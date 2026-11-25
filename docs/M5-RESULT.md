# Hasil uji M5 — voice call 1:1 (WebRTC)

## 1. Uji otomatis (`cargo test`)

Integrasi antar dua `Node` nyata (socket TCP sungguhan), 3 test baru:

```
signalling_call_offer_answer_ice_end        offer → incoming → answer → accepted → ICE → end
call_ditolak_menghasilkan_event_rejected    accept=false → CallRejected dengan alasan
call_tanpa_koneksi_gagal_dengan_jelas       error jelas, bukan diam
test result: ok. 31 passed (21 unit + 10 integrasi)
```

## 2. E2E signalling lewat CLI (tanpa GUI)

`bash scripts/e2e-call.sh`:

```
pemanggil: ☎ memanggil penerima (c-86df6749)
           ☎ panggilan c-86df6749 diterima
           ☎ tutup c-86df6749
penerima : ☎ PANGGILAN MASUK dari pemanggil (id c-86df6749)
           ☎ jawab c-86df6749 : accept
           ☎ panggilan c-86df6749 berakhir: ditutup dari CLI
```

Empat tahap (`offer` → `answer` → `ice` → `end`) terbukti lewat dua proses nyata.

## 3. Uji WebRTC sungguhan (dua peer, audio mengalir)

`apps/desktop/ui/test-webrtc.html` menjalankan `call.js` asli di dua tab, dengan
`BroadcastChannel` sebagai pengganti transport Rust dan **osilator sintetis** sebagai
pengganti mikrofon (mesin uji tidak punya perangkat audio).

Hasil `RTCPeerConnection.getStats()` di **kedua sisi**:

```
PENELEPON (A): conn=connected  rtt=0.062s  inBytes=57.447  inPackets=709
                                          outBytes=57.505  outPackets=709
PENERIMA  (B): conn=connected  rtt=0.013s  inBytes=64.737  inPackets=799
                                          outBytes=64.633  outPackets=797
```

Artinya: bukan cuma "connected" — **byte audio benar-benar mengalir dua arah**, dengan
RTT 13–62 ms (loopback LAN).

Uji loopback murni di satu halaman juga membuktikan jalur media:
`inBytes=9646, inPackets=118` dari osilator 440 Hz.

## 4. Mute

Sebelum mute: `trackEnabled=[true]`, laju keluar ~700 byte/s.
Sesudah mute: `trackEnabled=[false]`, delta 471 byte dalam 6 detik (~157/s — paket
senyap/keepalive yang normal di WebRTC, audio tidak dikirim).

## 5. Tutup panggilan

`hangup()` di sisi A → A `idle`, B `idle`, RTCPeerConnection ditutup, track mikrofon
di-stop, timer berhenti, call bar hilang.

## 6. Tampilan

- Call bar: avatar, nama peer, status "tersambung" (hijau), timer `00:01`, tombol mute,
  tombol tutup merah.
- Popup panggilan masuk: avatar, nama, "panggilan suara masuk", tombol tolak merah dan
  terima hijau.

## Batas yang jujur

- Uji memakai **osilator sintetis**, bukan mikrofon sungguhan. Jalur media (encode, kirim,
  terima, decode, stats) terbukti; kualitas suara nyata belum dinilai.
- Tidak ada perangkat audio di mesin uji, jadi `getUserMedia` asli belum dijalankan di sini.
- Diuji di satu host (dua proses/tab), belum dua mesin fisik atau Windows.
- Group call (mesh >2) belum ada.
- Tanpa STUN/TURN: kalau dua perangkat beda subnet (mis. WiFi vs kabel dengan VLAN),
  koneksi media bisa gagal walau chat tetap jalan. Itu konsekuensi LAN-only yang disengaja.

## 7. Binary release

```
cargo build --release -p lanchat-desktop → EXIT=0
target/release/lanchat-desktop           → 6.7 MB
```

Dijalankan lewat `bash scripts/e2e-call-ui.sh` (binary release, bukan debug): klik tombol
panggilan di header → call bar muncul dengan status "mikrofon ditolak", dan log app mencatat

```
Audio capture was requested but no device was found amongst 0 devices
```

Itu bukti `call.js` benar-benar termuat dan `getUserMedia` dipanggil di build release —
mesin uji ini memang tidak punya perangkat audio.

Tangkapan layar: `docs/screens/call-01-active-bar.png`,
`docs/screens/call-02-incoming-popup.png`, `docs/screens/call-03-release-no-mic.png`.
