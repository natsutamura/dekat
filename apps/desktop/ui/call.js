/* Panggilan suara: 1:1 dan grup (mesh WebRTC).
 *
 * Rust hanya meneruskan SDP + ICE lewat koneksi TCP chat (command call_*),
 * audio-nya peer-to-peer langsung di LAN. Tidak ada server STUN/TURN karena
 * semua perangkat di subnet yang sama — host candidate cukup.
 *
 * Model grup = mesh penuh: tiap peserta punya satu RTCPeerConnection ke tiap
 * peserta lain, dan satu mikrofon lokal dibagi ke semuanya. Peserta dikenali
 * dari `peerId` karena setiap frame signalling membawa `from`.
 *
 * Anti-dobel: pemanggil menghubungi semua peserta. Saat menerima, peserta
 * menghubungi peserta lain yang id-nya lebih besar dan bukan pemanggil, jadi
 * tiap pasangan punya tepat satu koneksi (aturan yang sama dengan transport).
 */
(function () {
  const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
  const listen = (ev, cb) => window.__TAURI__.event.listen(ev, cb);

  const S = {
    callId: null,
    selfId: null,
    members: [],      // id peserta (termasuk diri sendiri)
    callerId: null,
    pcs: new Map(),   // peerId -> { pc, pendingIce, audio }
    local: null,      // MediaStream mikrofon, dibagi ke semua koneksi
    state: "idle",    // idle | offering | ringing | active
    muted: false,
    startedAt: 0,
    incoming: null,
  };

  let bar, btnMute, btnEnd, timerEl, nameEl, stateEl, iconEl;
  let nameOf = (id) => String(id || "?").slice(0, 6);

  /* ---------- elemen ---------- */
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

  function isGroup() {
    return S.members.length > 2;
  }

  function show(title) {
    buildBar();
    bar.hidden = false;
    nameEl.textContent = title;
    iconEl.textContent = (title || "?").trim().charAt(0).toUpperCase();
    const hue = [...String(title)].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 0);
    iconEl.style.background = `linear-gradient(160deg,hsl(${hue} 70% 62%),hsl(${(hue + 32) % 360} 68% 48%))`;
  }

  function startTimer() {
    if (S.startedAt) return;
    S.startedAt = Date.now();
    clearInterval(S._timer);
    S._timer = setInterval(() => {
      const s = Math.floor((Date.now() - S.startedAt) / 1000);
      const mm = String(Math.floor(s / 60)).padStart(2, "0");
      const ss = String(s % 60).padStart(2, "0");
      if (timerEl) timerEl.textContent = `${mm}:${ss}`;
      if (isGroup() && stateEl && bar.dataset.kind === "active") {
        stateEl.textContent = `${S.pcs.size + 1} peserta tersambung`;
      }
    }, 500);
  }

  /* ---------- koneksi WebRTC ---------- */
  function newPc(peerId) {
    const pc = new RTCPeerConnection({ iceServers: [] });
    const entry = { pc, pendingIce: [], audio: null };
    let iceSeen = 0;

    pc.onicecandidate = (e) => {
      if (!e.candidate) return;
      // Di LAN biasanya 2–3 kandidat sudah cukup; sisanya cuma membanjiri TCP.
      if (++iceSeen > 24) return;
      invoke("call_ice", {
        peerId,
        callId: S.callId,
        candidate: e.candidate.candidate,
        sdpMid: e.candidate.sdpMid || "0",
        sdpMlineIndex: e.candidate.sdpMLineIndex ?? 0,
      }).catch(() => {});
    };

    pc.ontrack = (e) => {
      const stream = e.streams[0] || new MediaStream([e.track]);
      const audio = document.createElement("audio");
      audio.autoplay = true;
      audio.srcObject = stream;
      audio.dataset.peer = peerId;
      document.body.appendChild(audio);
      audio.play().catch(() => {});
      entry.audio = audio;
      S.state = "active";
      setState(isGroup() ? `${S.pcs.size + 1} peserta tersambung` : "tersambung", "active");
      startTimer();
    };

    pc.onconnectionstatechange = () => {
      const st = pc.connectionState;
      if (st === "connected") {
        S.state = "active";
        setState(isGroup() ? `${S.pcs.size + 1} peserta tersambung` : "tersambung", "active");
        startTimer();
      } else if (st === "failed") {
        // Satu koneksi gagal tidak mematikan panggilan grup — keluarkan peer itu saja.
        dropPeer(peerId, "koneksi gagal", true);
      }
    };

    return entry;
  }

  function addPeerPc(peerId) {
    if (S.pcs.has(peerId)) return S.pcs.get(peerId);
    const entry = newPc(peerId);
    S.pcs.set(peerId, entry);
    if (S.local) S.local.getTracks().forEach((t) => entry.pc.addTrack(t, S.local));
    return entry;
  }

  async function getMic() {
    // echoCancellation bikin suara lawan tidak balik ke dia
    return navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      video: false,
    });
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

  async function offerTo(peerId) {
    if (S.pcs.has(peerId)) return; // sudah ada koneksi ke peer ini
    const entry = addPeerPc(peerId);
    const offer = await entry.pc.createOffer({ offerToReceiveAudio: true });
    await entry.pc.setLocalDescription(offer);
    await waitIce(entry.pc);
    await invoke("call_offer", {
      peerId,
      callId: S.callId,
      sdp: entry.pc.localDescription.sdp,
      members: S.members,
    });
  }

  /* ---------- aksi ---------- */
  function setNameResolver(fn) {
    if (typeof fn === "function") nameOf = fn;
  }

  /** Mulai panggilan. `peerIds` boleh satu (DM) atau banyak (grup). */
  async function startCall(peerIds, title) {
    if (S.state !== "idle") return;
    const ids = (Array.isArray(peerIds) ? peerIds : [peerIds]).filter(Boolean);
    if (!ids.length) return;

    S.callId = "c-" + Math.random().toString(16).slice(2, 10);
    S.callerId = null;
    S.members = ids.slice();
    if (S.selfId && !S.members.includes(S.selfId)) S.members.push(S.selfId);
    S.state = "offering";
    show(title || (ids.length > 1 ? `${ids.length} peserta` : nameOf(ids[0])));
    setState("memanggil…", "calling");

    try {
      S.local = await getMic();
    } catch (e) {
      setState("mikrofon ditolak", "error");
      setTimeout(() => cleanup(), 1500);
      return;
    }

    let ok = 0;
    for (const pid of ids) {
      try {
        await offerTo(pid);
        ok++;
      } catch (e) {
        console.warn("gagal memanggil", pid, e);
      }
    }
    if (!ok) {
      setState("gagal memanggil", "error");
      setTimeout(() => cleanup(), 1500);
      return;
    }
    setState("menunggu dijawab…", "calling");
  }

  async function acceptCall() {
    const inc = S.incoming;
    if (!inc) return;
    S.callId = inc.callId;
    S.callerId = inc.from;
    S.members = (inc.members && inc.members.length ? inc.members : [inc.from]).slice();
    if (!S.members.includes(inc.from)) S.members.push(inc.from);
    S.state = "active";
    show(S.members.length > 2 ? `${S.members.length} peserta` : inc.fromName);
    setState("menyambung…", "connecting");

    try {
      S.local = await getMic();
    } catch (e) {
      setState("mikrofon ditolak", "error");
      invoke("call_answer", {
        peerId: inc.from,
        callId: inc.callId,
        accept: false,
        sdp: "",
        reason: "mikrofon tidak tersedia",
      }).catch(() => {});
      setTimeout(() => cleanup(), 1500);
      return;
    }

    // 1. Jawab pemanggil.
    const entry = addPeerPc(inc.from);
    await entry.pc.setRemoteDescription({ type: "offer", sdp: inc.sdp });
    const answer = await entry.pc.createAnswer();
    await entry.pc.setLocalDescription(answer);
    await waitIce(entry.pc);
    await invoke("call_answer", {
      peerId: inc.from,
      callId: inc.callId,
      accept: true,
      sdp: entry.pc.localDescription.sdp,
      reason: "",
    });

    S.incoming = null;
    closeIncomingPopup();

    // 2. Sambung ke peserta lain (mesh). Hanya yang id-nya lebih besar dari kita,
    //    supaya pasangan yang sama tidak membuat dua koneksi.
    //    `selfId` wajib ada: tanpa itu kita bisa memanggil diri sendiri.
    if (!S.selfId) {
      try {
        const me = await invoke("me");
        S.selfId = me && me.id;
      } catch (_) {}
    }
    const self = S.selfId;
    if (!self) return; // tidak bisa menentukan urutan dial dengan aman
    for (const pid of S.members) {
      if (pid === inc.from) continue;
      if (pid === self) continue;
      if (self && pid <= self) continue;
      try {
        await offerTo(pid);
      } catch (e) {
        console.warn("mesh gagal", pid, e);
      }
    }
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

  async function onAccepted(from, sdp) {
    const entry = S.pcs.get(from);
    if (!entry) return;
    // Jawaban bisa datang terlambat untuk koneksi yang sudah selesai.
    if (entry.pc.signalingState === "stable" && entry.pc.remoteDescription) return;
    await entry.pc.setRemoteDescription({ type: "answer", sdp });
    const q = entry.pendingIce.splice(0);
    q.forEach((c) => entry.pc.addIceCandidate(c).catch(() => {}));
  }

  function onIce(from, candidate, sdpMid, sdpMlineIndex) {
    const entry = S.pcs.get(from);
    if (!entry) return;
    const c = { candidate, sdpMid: String(sdpMid || "0"), sdpMLineIndex: sdpMlineIndex ?? 0 };
    if (!entry.pc.remoteDescription) {
      entry.pendingIce.push(c);
      return;
    }
    entry.pc.addIceCandidate(c).catch(() => {});
  }

  function toggleMute() {
    if (!S.local) return;
    S.muted = !S.muted;
    S.local.getAudioTracks().forEach((t) => (t.enabled = !S.muted));
    if (btnMute) btnMute.classList.toggle("on", S.muted);
    setState(S.muted ? "mikrofon mati" : "tersambung", S.muted ? "muted" : "active");
  }

  /**
   * Keluarkan satu peer dari panggilan.
   * `notify=false` dipakai saat peer itu sendiri yang mengakhiri koneksi.
   */
  function dropPeer(peerId, reason, notify) {
    const entry = S.pcs.get(peerId);
    if (!entry) return;
    try {
      entry.pc.onicecandidate = null;
      entry.pc.ontrack = null;
      entry.pc.close();
    } catch (_) {}
    if (entry.audio) entry.audio.remove();
    S.pcs.delete(peerId);
    if (notify && S.callId) {
      invoke("call_end", { peerId, callId: S.callId, reason }).catch(() => {});
    }
    if (!S.pcs.size) cleanup();
  }

  function cleanup() {
    clearInterval(S._timer);
    S.pcs.forEach((e) => {
      try {
        e.pc.close();
      } catch (_) {}
      if (e.audio) e.audio.remove();
    });
    S.pcs.clear();
    if (S.local) S.local.getTracks().forEach((t) => t.stop());
    S.local = null;
    S.muted = false;
    S.state = "idle";
    S.callId = null;
    S.callerId = null;
    S.members = [];
    S.startedAt = 0;
    if (bar) bar.hidden = true;
    if (btnMute) btnMute.classList.remove("on");
  }

  function hangup(reason) {
    if (S.callId) {
      for (const pid of Array.from(S.pcs.keys())) {
        invoke("call_end", { peerId: pid, callId: S.callId, reason }).catch(() => {});
      }
    }
    cleanup();
  }

  /* ---------- panggilan masuk ---------- */
  function showIncomingPopup(inc) {
    S.incoming = inc;
    closeIncomingPopup();
    const group = (inc.members || []).length > 2;
    const pop = document.createElement("div");
    pop.className = "call-pop";
    pop.id = "call-pop";
    pop.innerHTML = `
      <div class="call-pop-av">${(inc.fromName || "?").charAt(0).toUpperCase()}</div>
      <div class="call-pop-meta">
        <div class="call-pop-name">${inc.fromName}</div>
        <div class="call-pop-sub">${
          group ? `panggilan grup · ${inc.members.length} peserta` : "panggilan suara masuk"
        }</div>
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
    try {
      const me = await invoke("me");
      S.selfId = me && me.id;
    } catch (_) {}

    await listen("lc://call-incoming", (ev) => {
      const p = ev.payload;
      if (S.state !== "idle") {
        // Sudah di panggilan: tolak dengan alasan jelas (merge-call belum ada).
        invoke("call_answer", {
          peerId: p.from,
          callId: p.callId,
          accept: false,
          sdp: "",
          reason: "sedang dalam panggilan lain",
        }).catch(() => {});
        return;
      }
      showIncomingPopup(p);
    });

    await listen("lc://call-accepted", (ev) => {
      const p = ev.payload;
      onAccepted(p.from, p.sdp).catch(() => {});
    });

    await listen("lc://call-rejected", (ev) => {
      const p = ev.payload;
      // Di grup, satu penolakan bukan akhir panggilan: keluarkan peer itu saja.
      if (S.pcs.size > 1) {
        const e = S.pcs.get(p.from);
        if (e) {
          try { e.pc.close(); } catch (_) {}
          if (e.audio) e.audio.remove();
          S.pcs.delete(p.from);
        }
        return;
      }
      setState("ditolak: " + (p.reason || "tidak dijawab"), "error");
      setTimeout(() => cleanup(), 1600);
    });

    await listen("lc://call-ice", (ev) => {
      const p = ev.payload;
      onIce(p.from, p.candidate, p.sdpMid, p.sdpMlineIndex);
    });

    await listen("lc://call-ended", (ev) => {
      if (S.state === "idle") return;
      const p = ev.payload;
      if (p && p.from) dropPeer(p.from, p.reason || "keluar");
    });
  }

  window.LanChatCall = {
    init,
    startCall,
    acceptCall,
    rejectCall,
    hangup,
    setNameResolver,
    state: () => S.state,
    isGroup,
    peerCount: () => S.pcs.size,
    members: () => S.members.slice(),
    // hook uji otomatis
    _pcs: () => Array.from(S.pcs.values()).map((e) => e.pc),
    _local: () => S.local,
  };
})();