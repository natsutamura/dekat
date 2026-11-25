/* LAN Chat — frontend Tauri. Semua data lewat invoke(); event dari Rust lewat listen(). */
const T = window.__TAURI__ || {};
const invoke = T.core ? T.core.invoke : async () => { throw new Error("Tauri API tidak tersedia"); };
const listen = T.event ? T.event.listen : async () => () => {};

const $ = (id) => document.getElementById(id);
const S = {
  me: null,
  threads: [],
  peers: [],
  messages: new Map(),
  active: null,
  settings: { theme: "light", accent: "#0b84ff", bubble_in: "", bubble_out: "", radius: 18, scale: 1 },
  typing: new Map(),
  filter: "",
};

function baseTitle() { return "LAN Chat" + (S.me ? " — " + S.me.name : ""); }

const STATUS_LABEL = { online: "Online", idle: "Idle", dnd: "Do Not Disturb" };
const EMOJIS = "😀 😁 😂 🤣 😊 😍 😘 😎 🤔 😴 😢 😭 😡 👍 👎 👏 🙏 💪 🔥 ✨ 🎉 ❤️ 💙 💚 💜 🤝 👀 🚀 🍕 ☕ 🌙 ✅ ❌ ⚡ 🧠 💬".split(" ");

/* ---------------- util ---------------- */
function h(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}
function initials(name) {
  const parts = (name || "?").trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return "?";
  return (parts[0][0] + (parts[1] ? parts[1][0] : "")).toUpperCase();
}
function hueOf(str) {
  let h = 0;
  for (const c of String(str)) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}
function avatarEl(name, extra = "", group = false) {
  const a = h("div", "avatar " + extra);
  a.textContent = initials(name);
  const hue = hueOf(name);
  a.style.background = `linear-gradient(160deg, hsl(${hue} 70% 62%), hsl(${(hue + 32) % 360} 68% 48%))`;
  if (group) a.classList.add("grp");
  return a;
}
function statusDot(status, online = true) {
  const d = h("span", "dot " + (online ? status || "online" : "offline"));
  return d;
}
const pad = (n) => String(n).padStart(2, "0");
function fmtTime(ts) {
  const d = new Date(ts);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
function fmtListTime(ts) {
  if (!ts) return "";
  const d = new Date(ts), now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) return fmtTime(ts);
  const y = new Date(now.getTime() - 864e5);
  if (d.toDateString() === y.toDateString()) return "Kemarin";
  const diff = (now - d) / 864e5;
  if (diff < 7) return ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"][d.getDay()];
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${String(d.getFullYear()).slice(2)}`;
}
function sepLabel(ts) {
  const d = new Date(ts), now = new Date();
  if (d.toDateString() === now.toDateString()) return "Hari ini " + fmtTime(ts);
  const y = new Date(now.getTime() - 864e5);
  if (d.toDateString() === y.toDateString()) return "Kemarin " + fmtTime(ts);
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()} ${fmtTime(ts)}`;
}

/* ---------------- render ---------------- */
function applySettings() {
  const s = S.settings;
  $("app").dataset.theme = s.theme === "dark" ? "dark" : "light";
  const root = document.documentElement.style;
  root.setProperty("--accent", s.accent || "#0b84ff");
  root.setProperty("--bg-bubble-out", s.bubble_out || s.accent || "#0b84ff");
  if (s.bubble_in) root.setProperty("--bg-bubble-in", s.bubble_in);
  root.setProperty("--radius", (Number(s.radius) || 18) + "px");
  root.setProperty("--scale", String(Number(s.scale) || 1));
}

function renderMe() {
  if (!S.me) return;
  $("me-name").textContent = S.me.name;
  const av = $("me-avatar");
  av.firstChild.textContent = initials(S.me.name);
  av.style.background = `linear-gradient(160deg,#0a84ff,#5e5ce6)`;
  const st = S.me.status || "online";
  $("me-status-dot").style.background = `var(--${st === "dnd" ? "dnd" : st})`;
  $("me-status-text").textContent = STATUS_LABEL[st] || "Online";
}

function threadUnread(t) {
  return (t.unread > 0) || t.manual_unread;
}

function renderThreads() {
  const ul = $("threads");
  ul.innerHTML = "";
  const q = S.filter.toLowerCase();
  const list = S.threads.filter((t) => !q || t.name.toLowerCase().includes(q));
  if (!list.length) {
    ul.appendChild(h("li", "list-empty", q ? "Tidak ada yang cocok" : "Belum ada percakapan"));
    return;
  }
  for (const t of list) {
    const li = h("li", "thread" + (t.id === S.active ? " active" : "") + (threadUnread(t) ? " has-unread" : ""));
    li.dataset.id = t.id;
    const av = avatarEl(t.name, "", t.kind === "group");
    const p = S.peers.find((x) => x.id === t.id);
    if (t.kind === "dm" && p) av.appendChild(statusDot(p.status, p.online));
    li.appendChild(av);

    const main = h("div", "tmain");
    const row = h("div", "trow");
    row.appendChild(h("span", "tname", t.name));
    const unread = threadUnread(t);
    if (unread) {
      const b = h("span", "unread" + (t.unread > 0 ? "" : " dot-only"), t.unread > 0 ? String(t.unread) : "");
      row.appendChild(b);
    }
    row.appendChild(h("span", "time", fmtListTime(t.last_ts)));
    main.appendChild(row);
    main.appendChild(h("div", "preview", t.preview || "Belum ada pesan"));
    li.appendChild(main);
    li.onclick = () => openThread(t.id);
    li.oncontextmenu = (e) => { e.preventDefault(); showThreadMenu(e, t); };
    ul.appendChild(li);
  }
}

function renderPeers() {
  const ul = $("peer-list");
  ul.innerHTML = "";
  const known = new Set(S.threads.map((t) => t.id));
  const q = S.filter.toLowerCase();
  const list = S.peers.filter((p) => !known.has(p.id) && (!q || p.name.toLowerCase().includes(q)));
  if (!list.length) {
    ul.appendChild(h("li", "list-empty", S.peers.length ? "Semua peer sudah ada di percakapan" : "Mencari peer di jaringan…"));
    return;
  }
  for (const p of list) {
    const li = h("li", "thread peer-row");
    const av = avatarEl(p.name);
    av.appendChild(statusDot(p.status, p.online));
    li.appendChild(av);
    const main = h("div", "tmain");
    const row = h("div", "trow");
    row.appendChild(h("span", "tname", p.name));
    main.appendChild(row);
    main.appendChild(h("div", "preview", (STATUS_LABEL[p.status] || "Online") + " · " + p.os));
    li.appendChild(main);
    li.onclick = async () => {
      const t = await invoke("open_dm", { peerId: p.id });
      await refreshThreads();
      openThread(t.id);
    };
    ul.appendChild(li);
  }
}

function renderHeader() {
  const t = S.threads.find((x) => x.id === S.active);
  const head = $("panel-head");
  if (!t) {
    $("head-name").textContent = "Pilih percakapan";
    $("head-status").textContent = "—";
    $("head-avatar").textContent = "?";
    $("chatbar").hidden = true;
    return;
  }
  $("chatbar").hidden = false;
  $("head-name").textContent = t.name;
  const av = $("head-avatar");
  av.textContent = initials(t.name);
  av.className = "avatar sm" + (t.kind === "group" ? " grp" : "");
  const hue = hueOf(t.name);
  av.style.background = `linear-gradient(160deg, hsl(${hue} 70% 62%), hsl(${(hue + 32) % 360} 68% 48%))`;

  const st = $("head-status");
  const typing = S.typing.get(t.id);
  if (typing) {
    st.className = "head-status typing";
    st.textContent = t.kind === "group" ? `${typing} sedang mengetik…` : "sedang mengetik…";
    return;
  }
  st.className = "head-status";
  if (t.kind === "group") {
    const online = S.peers.filter((p) => t.members.includes(p.id) && p.online).length;
    st.textContent = `${t.members.length} anggota · ${online} online`;
  } else if (t.kind === "broadcast") {
    st.textContent = "Semua peer di jaringan";
  } else {
    const p = S.peers.find((x) => x.id === t.id);
    st.innerHTML = "";
    if (p) {
      const sq = h("span", "sq");
      sq.style.background = `var(--${p.status === "dnd" ? "dnd" : p.status})`;
      st.appendChild(sq);
      st.appendChild(h("span", "", p.online ? STATUS_LABEL[p.status] || "Online" : "Offline"));
    } else {
      st.textContent = "Offline";
    }
  }
}

function humanSize(bytes) {
  const n = Number(bytes) || 0;
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  if (n < 1024 * 1024 * 1024) return (n / (1024 * 1024)).toFixed(1) + " MB";
  return (n / (1024 * 1024 * 1024)).toFixed(2) + " GB";
}

function fileIcon(name) {
  const ext = (name.split(".").pop() || "").toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "heic"].includes(ext)) return "🖼";
  if (["mp4", "mkv", "mov", "webm"].includes(ext)) return "🎬";
  if (["mp3", "wav", "flac", "ogg", "m4a"].includes(ext)) return "🎵";
  if (["pdf"].includes(ext)) return "📕";
  if (["zip", "rar", "7z", "tar", "gz"].includes(ext)) return "🗜";
  if (["doc", "docx", "txt", "md"].includes(ext)) return "📄";
  if (["xls", "xlsx", "csv"].includes(ext)) return "📊";
  if (["apk"].includes(ext)) return "📦";
  return "📎";
}

const fileProgress = new Map(); // msgId -> {sentBytes,totalBytes}

function fileCard(m) {
  const a = m.attachment;
  const card = h("div", "file-card");
  card.dataset.msg = m.id;
  const icon = h("div", "file-icon", fileIcon(a.name));
  card.appendChild(icon);
  const meta = h("div", "file-meta");
  meta.appendChild(h("div", "file-name", a.name));
  const pr = fileProgress.get(m.id);
  const sub = h("div", "file-sub");
  if (pr && pr.totalBytes) {
    sub.textContent = `${humanSize(pr.sentBytes)} / ${humanSize(pr.totalBytes)}`;
  } else {
    sub.textContent = humanSize(a.size) + (a.path ? " · klik untuk buka" : " · menunggu…");
  }
  meta.appendChild(sub);
  const bar = h("div", "file-bar");
  const fill = h("div", "file-bar-fill");
  const pct = pr && pr.totalBytes ? Math.min(100, (pr.sentBytes / pr.totalBytes) * 100) : (a.path ? 100 : 0);
  fill.style.width = pct + "%";
  bar.appendChild(fill);
  meta.appendChild(bar);
  card.appendChild(meta);
  if (a.path) {
    const open = h("button", "file-open", "Buka");
    open.onclick = (e) => {
      e.stopPropagation();
      invoke("open_path", { path: a.path }).catch((err) => toast("Gagal buka file", String(err)));
    };
    card.appendChild(open);
  }
  card.onclick = () => {
    if (a.path) invoke("open_path", { path: a.path }).catch((err) => toast("Gagal buka file", String(err)));
    else toast("File belum siap", "Masih menunggu pengiriman selesai");
  };
  return card;
}

function bubbleRow(m, cont, showSender, group) {
  const row = h("div", "row" + (m.mine ? " me" : "") + (cont ? " cont" : ""));
  const stack = h("div", "stack");
  if (showSender && !m.mine && group && !cont) stack.appendChild(h("div", "sender", m.from_name));
  if (m.attachment) {
    stack.appendChild(fileCard(m));
  } else {
    const b = h("div", "bubble");
    b.textContent = m.body;
    stack.appendChild(b);
  }
  row.appendChild(stack);
  if (m.mine && !cont) {
    const meta = h("div", "meta");
    meta.dataset.for = m.id;
    meta.innerHTML = m.delivered ? 'Terkirim <span class="tick">✓✓</span>' : 'Terkirim <span class="tick">✓</span>';
    row.appendChild(meta);
  }
  return row;
}

function renderMessages() {
  const box = $("messages");
  box.innerHTML = "";
  const t = S.threads.find((x) => x.id === S.active);
  if (!t) {
    const e = h("div", "empty");
    e.innerHTML = '<div class="empty-icon">💬</div><div>Pilih percakapan di kiri</div><div class="empty-sub">Semua jalan di jaringan lokal, tanpa internet</div>';
    box.appendChild(e);
    return;
  }
  const list = S.messages.get(t.id) || [];
  if (!list.length) {
    const e = h("div", "empty");
    e.innerHTML = '<div class="empty-icon">👋</div><div>Belum ada pesan</div><div class="empty-sub">Sapa dulu, nanti balas</div>';
    box.appendChild(e);
    return;
  }
  let prev = null;
  for (const m of list) {
    const newDay = !prev || new Date(prev.ts).toDateString() !== new Date(m.ts).toDateString()
      || (m.ts - prev.ts) > 20 * 60 * 1000;
    if (newDay) box.appendChild(h("div", "sep", sepLabel(m.ts)));
    const cont = !!prev && !newDay && prev.from_id === m.from_id && (m.ts - prev.ts) < 120000;
    box.appendChild(bubbleRow(m, cont, true, t.kind === "group"));
    prev = m;
  }
  box.scrollTop = box.scrollHeight;
}

function renderAll() {
  renderMe();
  renderThreads();
  renderPeers();
  renderHeader();
  renderMessages();
}

/* ---------------- actions ---------------- */
async function refreshThreads() {
  const b = await invoke("bootstrap");
  S.threads = b.threads;
  S.peers = b.peers;
  S.me = b.me;
  renderThreads();
  renderPeers();
  renderHeader();
}

async function openThread(id) {
  S.active = id;
  await invoke("set_active", { threadId: id });
  const msgs = await invoke("messages", { threadId: id, limit: 500 });
  S.messages.set(id, msgs);
  const t = S.threads.find((x) => x.id === id);
  if (t) { t.unread = 0; t.manual_unread = false; }
  await invoke("mark_read", { threadId: id }).catch(() => {});
  renderThreads();
  renderHeader();
  renderMessages();
  $("input").placeholder = t ? `Pesan ke ${t.name}` : "iMessage";
  $("input").focus();
}

async function sendMessage() {
  const input = $("input");
  const body = input.value.trim();
  if (!body || !S.active) return;
  input.value = "";
  autosize();
  updateSendState();
  try {
    await invoke("send", { threadId: S.active, body });
  } catch (err) {
    toast("Gagal kirim", String(err));
  }
  stopTyping();
}

function updateSendState() {
  $("btn-send").classList.toggle("ready", !!$("input").value.trim());
}
function autosize() {
  const i = $("input");
  i.style.height = "auto";
  i.style.height = Math.min(110, i.scrollHeight) + "px";
}

let typingTimer = null;
function startTyping() {
  if (!S.active) return;
  invoke("typing", { threadId: S.active, on: true }).catch(() => {});
  clearTimeout(typingTimer);
  typingTimer = setTimeout(stopTyping, 2500);
}
function stopTyping() {
  clearTimeout(typingTimer);
  if (!S.active) return;
  invoke("typing", { threadId: S.active, on: false }).catch(() => {});
}

function setScale(v) {
  const s = Math.min(1.6, Math.max(0.8, Math.round(v * 100) / 100));
  S.settings.scale = s;
  applySettings();
  invoke("set_setting", { key: "scale", value: String(s) }).catch(() => {});
  toast("Zoom chat", Math.round(s * 100) + "%", "zoom");
}

function setTheme(theme) {
  S.settings.theme = theme;
  applySettings();
  invoke("set_setting", { key: "theme", value: theme }).catch(() => {});
}

/* ---------------- ui kecil ---------------- */
function toast(title, body, key) {
  if (key) {
    const prev = document.querySelector(`.toast[data-key="${key}"]`);
    if (prev) {
      const b = prev.querySelector(".t-body");
      if (b && body) b.textContent = body;
      return;
    }
  }
  const t = h("div", "toast");
  if (key) t.dataset.key = key;
  const box = h("div", "");
  box.appendChild(h("div", "t-title", title));
  if (body) box.appendChild(h("div", "t-body", body));
  t.appendChild(box);
  $("toasts").appendChild(t);
  setTimeout(() => t.remove(), 4000);
}

function showMenu(x, y, items) {
  const m = $("ctx-menu");
  m.innerHTML = "";
  for (const it of items) {
    const b = h("button", it.danger ? "danger" : "", it.label);
    b.onclick = () => { m.hidden = true; it.run(); };
    m.appendChild(b);
  }
  m.style.left = Math.min(x, window.innerWidth - 210) + "px";
  m.style.top = Math.min(y, window.innerHeight - items.length * 36 - 12) + "px";
  m.hidden = false;
}

function showThreadMenu(e, t) {
  const unread = threadUnread(t);
  showMenu(e.clientX, e.clientY, [
    {
      label: unread ? "Tandai sudah dibaca" : "Tandai belum dibaca",
      run: async () => {
        await invoke("set_unread", { threadId: t.id, unread: !unread });
        await refreshThreads();
      },
    },
    {
      label: "Hapus semua pesan (clear chat)",
      danger: true,
      run: async () => {
        if (!confirm(`Hapus semua pesan di "${t.name}"? Riwayat hilang dari perangkat ini.`)) return;
        await invoke("clear_chat", { threadId: t.id });
        S.messages.set(t.id, []);
        await refreshThreads();
        if (S.active === t.id) renderMessages();
      },
    },
  ]);
}

function openModal(html) {
  $("modal-box").innerHTML = html;
  $("modal").hidden = false;
}
function closeModal() { $("modal").hidden = true; }

/* ---------------- modal: pengaturan ---------------- */
function settingsModal() {
  const s = S.settings;
  const colors = ["#0b84ff", "#34c759", "#ff375f", "#af52de", "#ff9f0a", "#5e5ce6", "#2c2c2e", "#00c7be"];
  openModal(`
    <h3>Pengaturan tampilan</h3>
    <div class="row-line"><span>Tema</span>
      <select id="st-theme">
        <option value="light">Light</option>
        <option value="dark">Dark</option>
      </select>
    </div>
    <div class="row-line"><span>Warna bubble keluar</span>
      <div class="swatches" id="st-colors">${colors.map((c) => `<button data-c="${c}" style="background:${c}" aria-pressed="${c === (s.bubble_out || s.accent)}"></button>`).join("")}</div>
    </div>
    <div class="row-line"><span>Sudut bubble</span>
      <input type="range" id="st-radius" min="6" max="22" step="1" value="${s.radius}">
    </div>
    <div class="row-line"><span>Zoom teks chat</span>
      <input type="range" id="st-scale" min="0.8" max="1.6" step="0.05" value="${s.scale}">
    </div>
    <div class="hint">Ctrl + scroll, Ctrl + / Ctrl − , atau Ctrl 0 buat zoom cepat. Window bisa di-resize bebas.</div>
    <div class="actions">
      <button class="btn" id="st-reset">Reset</button>
      <button class="btn primary" id="st-done">Selesai</button>
    </div>`);
  $("st-theme").value = s.theme;
  $("st-theme").onchange = (e) => setTheme(e.target.value);
  $("st-colors").onclick = (e) => {
    const c = e.target.dataset.c;
    if (!c) return;
    [...$("st-colors").children].forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.c === c)));
    S.settings.accent = c;
    S.settings.bubble_out = c;
    applySettings();
    invoke("set_setting", { key: "accent", value: c }).catch(() => {});
    invoke("set_setting", { key: "bubble_out", value: c }).catch(() => {});
  };
  $("st-radius").oninput = (e) => {
    S.settings.radius = Number(e.target.value);
    applySettings();
    invoke("set_setting", { key: "radius", value: String(S.settings.radius) }).catch(() => {});
  };
  $("st-scale").oninput = (e) => {
    S.settings.scale = Number(e.target.value);
    applySettings();
    invoke("set_setting", { key: "scale", value: String(S.settings.scale) }).catch(() => {});
  };
  $("st-reset").onclick = () => {
    S.settings = { theme: "light", accent: "#0b84ff", bubble_in: "", bubble_out: "", radius: 18, scale: 1 };
    applySettings();
    for (const [k, v] of Object.entries(S.settings)) invoke("set_setting", { key: k, value: String(v) }).catch(() => {});
    settingsModal();
  };
  $("st-done").onclick = closeModal;
}

/* ---------------- modal: chat / grup baru ---------------- */
function newChatModal() {
  const known = new Set(S.threads.map((t) => t.id));
  const avail = S.peers.filter((p) => !known.has(p.id));
  if (!avail.length) {
    openModal(`<h3>Chat baru</h3><div class="hint">Belum ada peer baru. Pastikan perangkat lain ada di WiFi/LAN yang sama dan aplikasinya terbuka.</div>
      <div class="actions"><button class="btn primary" id="nc-close">Oke</button></div>`);
    $("nc-close").onclick = closeModal;
    return;
  }
  openModal(`<h3>Chat / grup baru</h3>
    <div class="hint" style="margin:0 0 8px">Pilih 1 orang untuk chat pribadi, 2+ untuk grup.</div>
    <div id="nc-list">${avail.map((p) => `<label class="pick"><input type="checkbox" value="${p.id}"><span class="nm">${p.name}</span><span class="hint" style="margin:0">${p.os}</span></label>`).join("")}</div>
    <div class="row-line"><span>Nama grup</span><input type="text" id="nc-name" placeholder="mis. Warkop"></div>
    <div class="actions"><button class="btn" id="nc-cancel">Batal</button><button class="btn primary" id="nc-ok">Buat</button></div>`);
  $("nc-cancel").onclick = closeModal;
  $("nc-ok").onclick = async () => {
    const ids = [...$("nc-list").querySelectorAll("input:checked")].map((i) => i.value);
    if (!ids.length) return;
    try {
      if (ids.length === 1) {
        const t = await invoke("open_dm", { peerId: ids[0] });
        closeModal();
        await refreshThreads();
        openThread(t.id);
      } else {
        const t = await invoke("create_group", { name: $("nc-name").value, members: ids });
        closeModal();
        await refreshThreads();
        openThread(t.id);
      }
    } catch (err) {
      toast("Gagal", String(err));
    }
  };
}

/* ---------------- events dari Rust ---------------- */
async function wireEvents() {
  await listen("lc://msg", (ev) => {
    const { threadId, message, thread, notify } = ev.payload;
    if (!S.messages.has(threadId)) S.messages.set(threadId, []);
    const arr = S.messages.get(threadId);
    if (!arr.some((m) => m.id === message.id)) arr.push(message);
    if (thread) {
      const i = S.threads.findIndex((t) => t.id === thread.id);
      if (i >= 0) S.threads[i] = thread; else S.threads.unshift(thread);
      S.threads.sort((a, b) => b.last_ts - a.last_ts);
    }
    if (S.active === threadId) {
      renderMessages();
      renderThreads();
      if (!message.mine) invoke("mark_read", { threadId }).catch(() => {});
    } else {
      renderThreads();
      if (notify) {
        toast(message.from_name, message.body.slice(0, 90));
        document.title = "● " + baseTitle() + " — pesan baru";
      }
    }
  });

  await listen("lc://ack", ({ payload }) => {
    for (const arr of S.messages.values()) {
      const m = arr.find((x) => x.id === payload.messageId);
      if (m) { m.delivered = true; }
    }
    const el = document.querySelector(`.meta[data-for="${payload.messageId}"]`);
    if (el) el.innerHTML = 'Terkirim <span class="tick">✓✓</span>';
  });

  await listen("lc://read", ({ payload }) => {
    const arr = S.messages.get(payload.threadId) || [];
    arr.forEach((m) => { if (m.mine && m.ts <= payload.upToTs) m.delivered = true; });
    document.querySelectorAll("#messages .meta").forEach((el) => {
      const m = arr.find((x) => x.id === el.dataset.for);
      if (m && m.delivered) el.innerHTML = 'Terkirim <span class="tick">✓✓</span>';
    });
  });

  await listen("lc://typing", ({ payload }) => {
    const key = payload.threadId;
    if (payload.on) {
      const p = S.peers.find((x) => x.id === payload.from);
      S.typing.set(key, p ? p.name : "seseorang");
      setTimeout(() => {
        if (S.typing.get(key)) { S.typing.delete(key); if (S.active === key) renderHeader(); }
      }, 4000);
    } else {
      S.typing.delete(key);
    }
    if (S.active === key) renderHeader();
  });

  await listen("lc://peers", ({ payload }) => {
    S.peers = payload.peers;
    renderPeers();
    renderHeader();
    renderThreads();
  });

  await listen("lc://file-sent", ({ payload }) => {
    const arr = S.messages.get(payload.threadId) || [];
    if (!arr.some((m) => m.id === payload.message.id)) arr.push(payload.message);
    S.messages.set(payload.threadId, arr);
    if (S.active === payload.threadId) renderMessages();
    refreshThreads().catch(() => {});
  });

  await listen("lc://file-offer", ({ payload }) => {
    const { threadId, message, thread } = payload;
    const arr = S.messages.get(threadId) || [];
    if (!arr.some((m) => m.id === message.id)) arr.push(message);
    S.messages.set(threadId, arr);
    if (thread) {
      const i = S.threads.findIndex((t) => t.id === thread.id);
      if (i >= 0) S.threads[i] = thread; else S.threads.unshift(thread);
      S.threads.sort((a, b) => b.last_ts - a.last_ts);
    }
    if (S.active === threadId) renderMessages();
    renderThreads();
    toast(`📎 ${message.from_name}`, "mengirim " + message.attachment.name);
  });

  await listen("lc://file-progress", ({ payload }) => {
    fileProgress.set(payload.id, { sentBytes: payload.sentBytes, totalBytes: payload.totalBytes });
    document.querySelectorAll(`.file-card[data-msg="${payload.id}"] .file-sub`).forEach((el) => {
      el.textContent = `${humanSize(payload.sentBytes)} / ${humanSize(payload.totalBytes)}`;
    });
    document.querySelectorAll(`.file-card[data-msg="${payload.id}"] .file-bar-fill`).forEach((el) => {
      const pct = payload.totalBytes ? Math.min(100, (payload.sentBytes / payload.totalBytes) * 100) : 0;
      el.style.width = pct + "%";
    });
  });

  await listen("lc://file-received", ({ payload }) => {
    const arr = S.messages.get(payload.threadId) || [];
    const i = arr.findIndex((m) => m.id === payload.message.id);
    if (i >= 0) arr[i] = payload.message; else arr.push(payload.message);
    S.messages.set(payload.threadId, arr);
    fileProgress.delete(payload.message.id);
    if (S.active === payload.threadId) renderMessages();
  });

  await listen("lc://file-failed", ({ payload }) => {
    toast("Transfer gagal", `${payload.name || payload.id}: ${payload.reason}`);
  });

  // drag & drop file dari OS → kirim ke thread aktif
  await listen("tauri://drag-drop", async (ev) => {
    const paths = (ev.payload && ev.payload.paths) || [];
    if (!paths.length) return;
    if (!S.active) return toast("Pilih chat dulu", "Jatuhkan file setelah membuka percakapan");
    const t = S.threads.find((x) => x.id === S.active);
    if (t && t.kind !== "dm") return toast("Belum bisa", "Kirim file untuk sementara hanya di chat pribadi");
    for (const path of paths) {
      try {
        await invoke("send_file_path", { threadId: S.active, path });
      } catch (err) {
        toast("Gagal kirim file", String(err));
      }
    }
  });
  await listen("tauri://drag-enter", () => {
    document.getElementById("messages").classList.add("drag-over");
  });
  await listen("tauri://drag-leave", () => {
    document.getElementById("messages").classList.remove("drag-over");
  });

  await listen("lc://warn", ({ payload }) => toast("Info", payload.message));
}

/* ---------------- boot ---------------- */
async function boot() {
  const b = await invoke("bootstrap");
  S.me = b.me;
  S.threads = b.threads;
  S.peers = b.peers;
  S.settings = Object.assign(S.settings, b.settings || {});
  applySettings();
  renderAll();
  await wireEvents();

  // input & tombol
  const input = $("input");
  input.addEventListener("input", () => { autosize(); updateSendState(); startTyping(); });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); sendMessage(); }
  });
  $("btn-send").onclick = sendMessage;
  $("search").addEventListener("input", (e) => { S.filter = e.target.value; renderThreads(); renderPeers(); });
  $("btn-new").onclick = newChatModal;
  $("btn-settings").onclick = settingsModal;
  $("btn-attach").onclick = async () => {
    if (!S.active) return toast("Pilih chat dulu", "Lampiran hanya bisa dikirim di chat pribadi");
    const t = S.threads.find((x) => x.id === S.active);
    if (t && t.kind !== "dm") return toast("Belum bisa", "Kirim file untuk sementara hanya di chat pribadi (DM)");
    try {
      const r = await invoke("send_file", { threadId: S.active });
      if (r === "dibatalkan") return;
    } catch (err) {
      toast("Gagal kirim file", String(err));
    }
  };
  $("btn-call").onclick = () => {
    const t = S.threads.find((x) => x.id === S.active);
    if (!t) return toast("Pilih chat dulu", "Panggilan hanya untuk chat pribadi");
    if (t.kind !== "dm") return toast("Belum bisa", "Panggilan grup masuk M5-lanjut");
    const p = S.peers.find((x) => x.id === t.id);
    if (p && !p.online) return toast("Peer offline", "Tidak bisa memanggil peer yang tidak online");
    window.LanChatCall.startCall(t.id, t.name);
  };
  $("btn-info").onclick = () => {
    const t = S.threads.find((x) => x.id === S.active);
    if (!t) return;
    openModal(`<h3>${t.name}</h3>
      <div class="row-line"><span>Jenis</span><span>${t.kind}</span></div>
      <div class="row-line"><span>Id</span><span>${t.id}</span></div>
      <div class="row-line"><span>Anggota</span><span>${t.members.length || "-"}</span></div>
      <div class="hint">Zoom: Ctrl + scroll / Ctrl +/-. Window bisa di-resize.</div>
      <div class="actions"><button class="btn primary" id="info-ok">Tutup</button></div>`);
    $("info-ok").onclick = closeModal;
  };

  // menu status
  $("status-btn").onclick = (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(r.left, r.bottom + 4, ["online", "idle", "dnd"].map((s) => ({
      label: STATUS_LABEL[s],
      run: async () => {
        await invoke("set_status", { status: s });
        S.me.status = s;
        renderMe();
      },
    })));
  };

  // emoji
  const pop = $("emoji-pop");
  pop.innerHTML = EMOJIS.map((e) => `<button>${e}</button>`).join("");
  pop.onclick = (e) => {
    if (e.target.tagName !== "BUTTON") return;
    input.value += e.target.textContent;
    pop.hidden = true;
    input.focus();
    autosize();
    updateSendState();
  };
  $("btn-emoji").onclick = (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    pop.hidden = !pop.hidden;
    pop.style.left = Math.min(r.left - 240, window.innerWidth - 300) + "px";
    pop.style.top = (r.top - 232) + "px";
  };

  // klik di luar menu / emoji
  document.addEventListener("mousedown", (e) => {
    if (!$("ctx-menu").hidden && !$("ctx-menu").contains(e.target)) $("ctx-menu").hidden = true;
    if (!pop.hidden && !pop.contains(e.target) && e.target.id !== "btn-emoji") pop.hidden = true;
  });
  $("modal").addEventListener("mousedown", (e) => { if (e.target.id === "modal") closeModal(); });

  // zoom: Ctrl +/-/0 dan Ctrl+wheel
  window.addEventListener("keydown", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key === "+" || e.key === "=") { e.preventDefault(); setScale(S.settings.scale + 0.1); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); setScale(S.settings.scale - 0.1); }
    else if (e.key === "0") { e.preventDefault(); setScale(1); }
    else if (e.key.toLowerCase() === "k") { e.preventDefault(); $("search").focus(); }
  });
  // Ctrl + scroll di mana saja (chat, sidebar, header) ikut men-zoom teks chat
  window.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    setScale(S.settings.scale + (e.deltaY > 0 ? -0.1 : 0.1));
  }, { passive: false });

  document.title = baseTitle();
  window.addEventListener("focus", () => { document.title = baseTitle(); });

  // voice call
  if (window.LanChatCall) await window.LanChatCall.init();

  // buka thread terakhir kalau ada
  if (S.threads.length) openThread(S.threads[0].id);
  setInterval(refreshThreads, 15000); // sinkron ringan (presence/status)
}

boot().catch((err) => {
  document.body.innerHTML = `<pre style="padding:24px;font:13px monospace">Gagal start: ${err}</pre>`;
});