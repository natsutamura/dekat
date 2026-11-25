/* Voice call 1:1 pakai WebRTC bawaan webview.
 *
 * Rust hanya meneruskan SDP + ICE lewat koneksi TCP chat (command call_*),
 * audio-nya peer-to-peer langsung di LAN. Tidak ada server STUN/TURN karena
 * kedua perangkat di subnet yang sama — host candidate cukup.
 */
(function () {
  const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
  const listen = (ev, cb) => window.__TAURI__.event.listen(ev, cb);

  const S = {
    callId: null,
    peerId: null,
    peerName: "",
    pc: null,
    local: null,
    remote: null,
    state: "idle", // idle | offering | ringing | active
    muted: false,
    startedAt: 0,
    pendingIce: [],
  };

  /* ---------- elemen ---------- */
  let bar, btnMute, btnEnd, timerEl, nameEl, stateEl, iconEl;

  function buildBar() {
    if (bar) return;
    bar = document.createElement("div");
    bar.className = "callbar";
    bar.hidden = true;
    bar.innerHTML = `
      <div class="call-av" id="call-av">?</div>
      <div class="call-meta">
        <div class="call-name" id="call-name">—</div>
        <div class="call-state" id="call-state">menghubungkan…</div>
      </div>
      <div class="call-timer" id="call-timer">00:00</div>
      <button class="call-btn" id="call-mute" title="Bisukan mikrofon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0014 0M12 18v3"/>
        </svg>
      </button>
      <button class="call-btn end" id="call-end" title="Akhiri panggilan">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
          <path d="M2 14c4-4 16-4 20 0l-2 3c-3-2-5-2-8-2s-5 0-8 2z"/>
        </svg>
      </button>`;
    document.querySelector(".panel").appendChild(bar);
    btnMute = document.getElementById("call-mute");
    btnEnd = document.getElementById("call-end");
    timerEl = document.getElementById("call-timer");
    nameEl = document.getElementById("call-name");
    stateEl = document.getElementById("call-state");
    iconEl = document.getElementById("call-av");
    btnMute.onclick = toggleMute;
    btnEnd.onclick = () => hangup("ditutup");
  }

  function setState(text, kind) {
    buildBar();
    stateEl.textContent = text;
    bar.dataset.kind = kind || "connecting";
  }

  function show(name) {
    buildBar();
    bar.hidden = false;
    nameEl.textContent = name;
    iconEl.textContent = (name || "?").trim().charAt(0).toUpperCase();
    const hue = [...String(name)].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 0);
    iconEl.style.background = `linear-gradient(160deg,hsl(${hue} 70% 62%),hsl(${(hue + 32) % 360} 68% 48%))`;
  }

  function startTimer() {
    S.startedAt = Date.now();
    clearInterval(S._timer);
    S._timer = setInterval(() => {
      const s = Math.floor((Date.now() - S.startedAt) / 1000);
      const mm = String(Math.floor(s / 60)).padStart(2, "0");
      const ss = String(s % 60).padStart(2, "0");
      if (timerEl) timerEl.textContent = `${mm}:${ss}`;
    }, 500);
  }

  /* ---------- koneksi WebRTC ---------- */
  function newPc() {
    const pc = new RTCPeerConnection({ iceServers: [] });
    pc.onicecandidate = (e) => {
      if (!e.candidate) return;
      invoke("call_ice", {
        peerId: S.peerId,
        callId: S.callId,
        candidate: e.candidate.candidate,
        sdpMid: e.candidate.sdpMid || "0",
        sdpMlineIndex: e.candidate.sdpMLineIndex ?? 0,
      }).catch(() => {});
    };
    pc.ontrack = (e) => {
      S.remote = e.streams[0] || new MediaStream([e.track]);
      const audio = ensureAudio();
      audio.srcObject = S.remote;
      audio.play().catch(() => {});
      setState("tersambung", "active");
      startTimer();
    };
    pc.onconnectionstatechange = () => {
      const st = pc.connectionState;
      if (st === "connected") setState("tersambung", "active");
      else if (st === "failed" || st === "disconnected") {
        setState("koneksi terputus", "error");
        setTimeout(() => hangup("koneksi terputus"), 1200);
      }
    };
    return pc;
  }

  function ensureAudio() {
    let a = document.getElementById("remote-audio");
    if (!a) {
      a = document.createElement("audio");
      a.id = "remote-audio";
      a.autoplay = true;
      document.body.appendChild(a);
    }
    return a;
  }

  async function getMic() {
    // echoCancellation bikin suara lawan tidak balik ke dia
    return navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      video: false,
    });
  }

  /* ---------- aksi ---------- */
  async function startCall(peerId, peerName) {
    if (S.state !== "idle") return;
    S.callId = "c-" + Math.random().toString(16).slice(2, 10);
    S.peerId = peerId;
    S.peerName = peerName;
    S.state = "offering";
    show(peerName);
    setState("memanggil…", "calling");

    try {
      S.local = await getMic();
    } catch (e) {
      setState("mikrofon ditolak", "error");
      setTimeout(() => hangup("mikrofon tidak tersedia"), 1500);
      return;
    }

    S.pc = newPc();
    S.local.getTracks().forEach((t) => S.pc.addTrack(t, S.local));
    const offer = await S.pc.createOffer({ offerToReceiveAudio: true });
    await S.pc.setLocalDescription(offer);

    // tunggu ICE host terkumpul (LAN: cepat, tidak perlu trickle awal)
    await waitIce(S.pc);

    try {
      await invoke("call_offer", {
        peerId,
        callId: S.callId,
        sdp: S.pc.localDescription.sdp,
      });
      setState("menunggu dijawab…", "calling");
    } catch (e) {
      setState("gagal memanggil", "error");
      setTimeout(() => hangup(String(e)), 1500);
    }
  }

  function waitIce(pc, timeoutMs = 1500) {
    if (pc.iceGatheringState === "complete") return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        pc.removeEventListener("icegatheringstatechange", check);
        resolve();
      };
      const check = () => {
        if (pc.iceGatheringState === "complete") done();
      };
      pc.addEventListener("icegatheringstatechange", check);
      setTimeout(done, timeoutMs);
    });
  }

  async function acceptCall() {
    const inc = S.incoming;
    if (!inc) return;
    S.callId = inc.callId;
    S.peerId = inc.from;
    S.peerName = inc.fromName;
    S.state = "active";
    show(inc.fromName);
    setState("menyambung…", "connecting");

    try {
      S.local = await getMic();
    } catch (e) {
      setState("mikrofon ditolak", "error");
      invoke("call_answer", {
        peerId: S.peerId,
        callId: S.callId,
        accept: false,
        sdp: "",
        reason: "mikrofon tidak tersedia",
      }).catch(() => {});
      setTimeout(() => hangup("mikrofon tidak tersedia"), 1500);
      return;
    }

    S.pc = newPc();
    S.local.getTracks().forEach((t) => S.pc.addTrack(t, S.local));
    await S.pc.setRemoteDescription({ type: "offer", sdp: inc.sdp });
    const answer = await S.pc.createAnswer();
    await S.pc.setLocalDescription(answer);
    await waitIce(S.pc);

    await invoke("call_answer", {
      peerId: S.peerId,
      callId: S.callId,
      accept: true,
      sdp: S.pc.localDescription.sdp,
      reason: "",
    });
    S.incoming = null;
    closeIncomingPopup();
  }

  function rejectCall(reason = "ditolak") {
    const inc = S.incoming;
    if (!inc) return;
    invoke("call_answer", {
      peerId: inc.from,
      callId: inc.callId,
      accept: false,
      sdp: "",
      reason,
    }).catch(() => {});
    S.incoming = null;
    closeIncomingPopup();
  }

  async function onAccepted(sdp) {
    if (!S.pc || S.state === "idle") return;
    S.state = "active";
    await S.pc.setRemoteDescription({ type: "answer", sdp });
    setState("menyambung…", "connecting");
    flushIce();
  }

  function onIce(candidate, sdpMid, sdpMlineIndex) {
    const c = { candidate, sdpMid: String(sdpMid || "0"), sdpMLineIndex: sdpMlineIndex ?? 0 };
    if (!S.pc || !S.pc.remoteDescription) {
      S.pendingIce.push(c);
      return;
    }
    S.pc.addIceCandidate(c).catch(() => {});
  }

  function flushIce() {
    if (!S.pc) return;
    const q = S.pendingIce.splice(0);
    q.forEach((c) => S.pc.addIceCandidate(c).catch(() => {}));
  }

  function toggleMute() {
    if (!S.local) return;
    S.muted = !S.muted;
    S.local.getAudioTracks().forEach((t) => (t.enabled = !S.muted));
    if (btnMute) btnMute.classList.toggle("on", S.muted);
    setState(S.muted ? "mikrofon mati" : "tersambung", S.muted ? "muted" : "active");
  }

  function cleanup() {
    clearInterval(S._timer);
    if (S.pc) {
      S.pc.onicecandidate = null;
      S.pc.ontrack = null;
      S.pc.close();
    }
    if (S.local) S.local.getTracks().forEach((t) => t.stop());
    const a = document.getElementById("remote-audio");
    if (a) a.srcObject = null;
    S.pc = null;
    S.local = null;
    S.remote = null;
    S.pendingIce = [];
    S.muted = false;
    S.state = "idle";
    S.callId = null;
    S.peerId = null;
    S.peerName = "";
    S.startedAt = 0;
    if (bar) bar.hidden = true;
    if (btnMute) btnMute.classList.remove("on");
  }

  function hangup(reason) {
    if (S.callId && S.peerId) {
      invoke("call_end", { peerId: S.peerId, callId: S.callId, reason }).catch(() => {});
    }
    cleanup();
  }

  /* ---------- popup panggilan masuk ---------- */
  function showIncomingPopup(inc) {
    S.incoming = inc;
    closeIncomingPopup();
    const pop = document.createElement("div");
    pop.className = "call-pop";
    pop.id = "call-pop";
    pop.innerHTML = `
      <div class="call-pop-av">${(inc.fromName || "?").charAt(0).toUpperCase()}</div>
      <div class="call-pop-meta">
        <div class="call-pop-name">${inc.fromName}</div>
        <div class="call-pop-sub">panggilan suara masuk</div>
      </div>
      <button class="call-btn end" id="cp-reject" title="Tolak">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M2 14c4-4 16-4 20 0l-2 3c-3-2-5-2-8-2s-5 0-8 2z"/></svg>
      </button>
      <button class="call-btn ok" id="cp-accept" title="Terima">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M2 14c4-4 16-4 20 0l-2 3c-3-2-5-2-8-2s-5 0-8 2z"/></svg>
      </button>`;
    document.querySelector(".panel").appendChild(pop);
    document.getElementById("cp-accept").onclick = acceptCall;
    document.getElementById("cp-reject").onclick = () => rejectCall("ditolak");
  }

  function closeIncomingPopup() {
    const p = document.getElementById("call-pop");
    if (p) p.remove();
  }

  /* ---------- wiring event dari Rust ---------- */
  async function init() {
    buildBar();
    await listen("lc://call-incoming", (ev) => {
      const p = ev.payload;
      if (S.state !== "idle") {
        invoke("call_answer", {
          peerId: p.from,
          callId: p.callId,
          accept: false,
          sdp: "",
          reason: "sedang dalam panggilan lain",
        }).catch(() => {});
        return;
      }
      S.incoming = p;
      showIncomingPopup(p);
      setState("panggilan masuk", "calling");
    });
    await listen("lc://call-accepted", (ev) => onAccepted(ev.payload.sdp));
    await listen("lc://call-rejected", (ev) => {
      setState("ditolak: " + (ev.payload.reason || "tidak dijawab"), "error");
      setTimeout(() => cleanup(), 1600);
    });
    await listen("lc://call-ice", (ev) => {
      const p = ev.payload;
      onIce(p.candidate, p.sdpMid, p.sdpMlineIndex);
    });
    await listen("lc://call-ended", (ev) => {
      if (S.state !== "idle") {
        setState("panggilan berakhir", "error");
        setTimeout(() => cleanup(), 1200);
      }
    });
  }

  window.LanChatCall = {
    init,
    startCall,
    acceptCall,
    rejectCall,
    hangup,
    state: () => S.state,
    activePeer: () => S.peerId,
    // hook untuk uji otomatis: akses RTCPeerConnection supaya bisa baca getStats()
    _pc: () => S.pc,
    _local: () => S.local,
  };
})();