# Roadmap

Target: LAN chat rasa iMessage, Rust di core, ringan, jalan di Windows/Linux/Android.

## Keputusan stack UI

**Rekomendasi: Tauri v2 + web UI (HTML/CSS/JS ringan, tanpa framework berat).**

Alasan:
- Core Rust yang sudah ada dipakai apa adanya (`lanchat-core` → command Tauri).
- Bubble/tema/animasi paling cepat dicapai dengan CSS; dark/light cuma ganti variabel.
- Tauri v2 resmi mendukung **Android** — satu basis kode untuk tiga target.
- Webview bawaan OS → binary kecil (~5 MB) dan update tidak perlu bundel Chromium.
- Voice call bisa pakai WebRTC bawaan webview, tanpa nulis RTP/Opus manual.

Alternatif kalau mau 100% Rust tanpa webview: **Slint** (ada target Android). Konsekuensi:
animasi/scroll chat dan WebRTC harus dikerjakan tangan. `lanchat-core` tidak perlu diubah —
keduanya cuma konsumen event.

## Milestone

| # | isi | output yang bisa dilihat |
|---|---|---|
| **M0** ✅ | proto, identity, discovery multicast+fallback, TCP transport, chat teks broadcast & DM, ack | CLI dua mesin saling chat (sudah diuji) |
| **M1** ✅ | **selesai**: SQLite riwayat, presence online/idle/dnd, typing, read receipt, unread count, model room/group, **TLS 1.3 + autentikasi Ed25519 terikat sesi (anti-relay)**, penolakan peer plaintext. **belum**: TOFU pinning antar sesi, reconnect backoff | core stabil |
| **M2** 🟡 | **selesai**: sidebar + thread, bubble, dark/light, chat bar (emoji berkategori/send), avatar inisial + status dot, clear chat, mark read/unread, toast + **notifikasi OS native + taskbar flash**, zoom (Ctrl +/-/0/scroll), window resize, custom bubble, centang abu/biru. **belum**: foto profil/grup (upload gambar) | app desktop bisa dipakai harian |
| **M3** 🟡 | **selesai**: kirim file chunked (128 KiB) + sha256 + progress + tolak chunk rusak/tidak berurutan + drag & drop + CLI `/file` + **gambar tampil inline & lightbox**. **belum**: resume kalau putus, kirim file di grup, paste gambar dari clipboard | kirim file antar perangkat |
| **M4** | Android: build Tauri mobile, `MulticastLock`, izin `NEARBY_WIFI_DEVICES` (API 33+), notifikasi lokal, foreground service | HP Android chat di LAN yang sama |
| **M5** 🟡 | **selesai**: WebRTC audio 1:1, signalling lewat frame TCP (`call_offer`/`call_answer`/`call_ice`/`call_end`), popup panggilan masuk, mute, timer, tanpa STUN/TURN. **belum**: group call, device picker, indikator speaking, video | panggilan suara antar perangkat |
| **M6** 🟡 | **selesai**: installer Windows **MSI + NSIS** (AUMID → notifikasi jalan). **belum**: pairing QR, pengaturan nama/foto, AppImage/deb, updater, code signing | rilis v1.0 |

## Pemetaan fitur yang diminta

| fitur | milestone | catatan |
|---|---|---|
| Regular chat | M0 ✅ / UI di M2 | protokol sudah jalan |
| Regular call | M5 | WebRTC audio; signalling lewat frame TCP yang sama |
| Group chat | M1 (model room) / M2 | room id + daftar anggota, pesan `to: "#room"` |
| Group call | M5 | mesh kalau ≤6 peserta |
| Clear chat | M2 | hapus lokal (SQLite); "clear for everyone" opsional nanti |
| Mark read/unread | M1 (state) / M2 (UI) | flag lokal + read receipt |
| Notification & popup | M2 / M4 | `tauri-plugin-notification` |
| Layout chat bar | M2 | attach kiri, input pill, emoji, tombol kirim muncul saat ada teks |
| Profile & group picture | M1 (kirim) / M2 (UI) | blob di SQLite, dikirim saat handshake |
| Status online/idle/dnd | M1 | dnd disembunyikan dari announce + auto-idle dari idle OS |

## Risiko yang sudah dipetakan

1. **AP/client isolation** — banyak WiFi kampus/kantor/router ISP memblok trafik peer-to-peer.
   Aplikasi tidak bisa menembus ini. Mitigasi M6: deteksi + saran bikin hotspot sendiri / kabel.
2. **Android multicast** — wajib `MulticastLock` (plugin kecil di Rust/JNI) + izin nearby devices,
   kalau tidak, presence tidak pernah masuk.
3. **Windows firewall** — popup izin saat pertama listen; dokumentasikan, jangan diakali.
4. **WebView2** — Windows butuh WebView2 runtime; Windows 11 sudah ada, Windows 10 lama perlu installer.
5. **Linux** — perlu `webkit2gtk-4.1` di mesin build & mesin user (`libwebkit2gtk-4.1-0`).
6. **Kualitas call di WiFi** — jitter; mitigasi: Opus + PLC, buffer adaptif, fallback ke push-to-talk.