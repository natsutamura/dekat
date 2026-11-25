// Mock Tauri API — dipakai HANYA untuk render preview UI di browser biasa.
// Menjalankan app.js asli dengan data contoh, jadi tampilan yang dihasilkan
// identik dengan app desktop (yang jalan di WebKit).
(function () {
  const now = Date.now();
  const min = 60 * 1000;
  const me = { id: "7fda069ad40267af", name: "ray", fp: "ray#7fda", port: 42101, status: "online" };

  const threads = [
    {
      id: "#g1", kind: "group", name: "Grup Warkop", members: ["7fda069ad40267af", "p-dimas", "p-sinta", "p-adit"],
      last_ts: now - 4 * min, unread: 0, manual_unread: false, preview: "nanti jam 8 ya, bawa laptop",
    },
    {
      id: "p-dimas", kind: "dm", name: "dimas", members: ["p-dimas"],
      last_ts: now - 18 * min, unread: 0, manual_unread: false, preview: "file sketch-nya udah kekirim?",
    },
    {
      id: "p-sinta", kind: "dm", name: "sinta", members: ["p-sinta"],
      last_ts: now - 52 * min, unread: 2, manual_unread: false, preview: "wkwk iya bener",
    },
    {
      id: "p-adit", kind: "dm", name: "adit (PC)", members: ["p-adit"],
      last_ts: now - 26 * 60 * min, unread: 1, manual_unread: true, preview: "lagi compile, jangan diganggu dulu",
    },
  ];

  const peers = [
    { id: "p-dimas", name: "dimas", os: "linux", fp: "dimas#a1b2", addr: "10.10.1.31:41235", status: "online", online: true },
    { id: "p-sinta", name: "sinta", os: "android", fp: "sinta#c3d4", addr: "10.10.1.44:41235", status: "online", online: true },
    { id: "p-adit", name: "adit (PC)", os: "windows", fp: "adit#e5f6", addr: "10.10.1.52:41235", status: "online", online: true },
    { id: "p-nadia", name: "nadia", os: "android", fp: "nadia#9988", addr: "10.10.1.61:41235", status: "idle", online: true },
    { id: "p-rian", name: "rian", os: "windows", fp: "rian#7766", addr: "10.10.1.72:41235", status: "dnd", online: true },
  ];

  function m(id, from_id, from_name, body, ts, mine, delivered) {
    return { id, thread_id: "#g1", from_id, from_name, body, ts, mine: !!mine, delivered: !!delivered };
  }
  const group = [
    m("m1", "p-dimas", "dimas", "besok jadi meeting jam 8?", now - 42 * min),
    m("m2", "7fda069ad40267af", "ray", "jadi. bawa laptop + kabel LAN ya", now - 41 * min, true, true),
    m("m3", "7fda069ad40267af", "ray", "wifi kampus biasanya blokir multicast, nanti kita tes dulu", now - 41 * min + 900, true, true),
    m("m4", "p-sinta", "sinta", "oke, aku bawa switch kecil juga biar aman", now - 38 * min),
    m("m5", "p-adit", "adit (PC)", "udah ada yang nyoba build APK-nya?", now - 12 * min),
    m("m6", "7fda069ad40267af", "ray", "udah, di HP-ku jalan. discovery-nya perlu multicast lock dulu", now - 11 * min, true, true),
    m("m7", "p-dimas", "dimas", "sip, aku bawa proyektor juga biar gampang", now - 6 * min),
    Object.assign(m("m8", "p-dimas", "dimas", "", now - 8 * min), {
      attachment: { name: "beeimgtmp-20261124-143936.png", size: 328908, path: "/tmp/foto1.png", sha256: "aa", mime: "" },
    }),
    m("m9", "7fda069ad40267af", "ray", "nanti jam 8 ya, bawa laptop", now - 4 * min, true, true),
  ];
  const dmDimas = [
    { id: "d1", thread_id: "p-dimas", from_id: "p-dimas", from_name: "dimas", body: "mockup iMessage-nya udah jadi?", ts: now - 30 * min, mine: false, delivered: false },
    { id: "d2", thread_id: "p-dimas", from_id: "7fda069ad40267af", from_name: "ray", body: "udah, tinggal wiring ke core Rust", ts: now - 28 * min, mine: true, delivered: true },
    { id: "d3", thread_id: "p-dimas", from_id: "p-dimas", from_name: "dimas", body: "file sketch-nya udah kekirim?", ts: now - 18 * min, mine: false, delivered: false },
  ];
  const byThread = { "#g1": group, "p-dimas": dmDimas, "p-sinta": [], "p-adit": [] };

  const results = {
    bootstrap: {
      me,
      threads,
      peers,
      settings: {
        theme: (location.search.includes("theme=dark") ? "dark" : "light"),
        accent: "#0b84ff", bubble_in: "", bubble_out: "", radius: 18, scale: 1,
      },
    },
    messages: (a) => byThread[a.threadId] || [],
    peers: () => peers,
    call_offer: () => null,
    call_answer: () => null,
    call_ice: () => null,
    call_end: () => null,
    open_path: () => null,
    read_image: () => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAGQAAABkCAYAAABw4pVUAAAAOklEQVR4nO3BMQEAAADCoPVPbQ0PoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAvg0hAAABmmDh1QAAAABJRU5ErkJggg==",
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        const r = results[cmd];
        return typeof r === "function" ? r(args || {}) : (r === undefined ? null : r);
      },
    },
    event: { listen: async () => () => {} },
  };
})();