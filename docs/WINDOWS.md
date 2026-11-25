# Menjalankan LAN Chat di Windows

Binary yang kamu butuhkan: **`LANChat.exe`** (dari artifact CI `LANChat-windows-x64.zip`).
File Linux (`lanchat-desktop`) tidak jalan di Windows — beda platform, harus di-build ulang.

## Langkah

1. Unduh `LANChat-windows-x64.zip` dari tab **Actions** → run terakhir → bagian *Artifacts*.
2. Ekstrak ke folder mana pun, misal `C:\LANChat\`.
3. Klik dua kali `LANChat.exe`.
   - **Windows Defender / SmartScreen** akan protes karena binary tidak ditandatangani
     (belum ada code signing certificate). Pilih **More info → Run anyway**.
   - **Firewall** akan bertanya. **Wajib Allow** untuk *Private networks*, kalau tidak
     peer lain tidak bisa connect ke kamu.
4. Nama default = nama user Windows (`USERNAME`). Mau nama lain? Jalankan dari cmd:
   ```cmd
   set LANCHAT_NAME=ray & LANChat.exe
   ```
5. Aplikasi lain di jaringan yang sama cukup dibuka; keduanya otomatis saling menemukan.

## Yang perlu ada di Windows 10

- **WebView2 Runtime** — Windows 11 sudah punya; Windows 10 versi baru biasanya sudah.
  Kalau app tidak mau terbuka (jendela kosong/putih), install dari
  https://developer.microsoft.com/microsoft-edge/webview2/ (Evergreen Bootstrapper).
- Port yang dipakai: **TCP 41235** (chat/file/call signalling) dan **UDP 41234** (discovery).
  Kalau jaringan kantor memblok, chat tidak akan menemukan peer.

## Kalau peer tidak muncul

1. Pastikan dua perangkat di **subnet yang sama** (mis. `192.168.1.x`).
2. WiFi publik/kantor sering memakai **AP isolation** — perangkat tidak boleh bicara
   langsung satu sama lain. Ini di luar kendali aplikasi; pakai hotspot HP atau kabel LAN.
3. Cek firewall: `Windows Defender Firewall → Allow an app` → pastikan LANChat dicentang
   untuk Private.
4. Sebagian router memblok multicast → tambahkan seed manual lewat env (alamat peer lain,
   port UDP discovery 41234):
   ```cmd
   set LANCHAT_SEED=192.168.1.20:41234 & LANChat.exe
   ```
   Bisa lebih dari satu, pisahkan koma: `192.168.1.20:41234,192.168.1.21:41234`

## Data disimpan di mana

`%APPDATA%\LANChat\`
- `identity.json` — keypair perangkat (jangan dibagikan)
- `lanchat.db` — riwayat chat
- `files\` — file yang kamu terima

Hapus folder itu = reset total (identitas baru, riwayat hilang).

## Fitur yang sudah jalan di Windows

- Chat 1:1 dan grup, riwayat lokal, unread, typing
- **Centang dua abu** = terkirim, **centang dua biru** = dibaca
- Kirim file (tombol lampiran / drag & drop) dengan verifikasi sha256
- **Gambar tampil langsung di chat** (png/jpg/gif/webp), klik untuk memperbesar
- **Notifikasi Windows** saat pesan masuk + tombol taskbar berkedip
- Voice call 1:1 (butuh mikrofon + izin mikrofon Windows)
- Status Online / Idle / DND (klik status di sidebar)
- Tema dark/light, zoom `Ctrl +/-/0` dan `Ctrl + scroll`, window resize
- Emoji picker dengan kategori

## Kalau notifikasi tidak muncul

Windows 10 punya **Focus Assist** (azi) yang menahan notifikasi. Cek
`Settings → System → Focus assist` dan matikan, atau tambahkan LANChat ke pengecualian.
Notifikasi juga butuh aplikasi punya identitas AppUserModelID; tanpa installer `.msi`
Windows kadang menggabungkan notifikasi ke pemilik proses induk.

## Belum ada

- Tanda tangan digital → SmartScreen tetap protes
- Installer (`.msi`), jadi masih zip portable
- TLS: traffic di LAN masih plaintext
- Group call, video call