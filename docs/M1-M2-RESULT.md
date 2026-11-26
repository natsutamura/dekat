# Hasil uji M1 + M2 (app desktop Tauri)

Lingkungan: VPS Linux, X display :11 (Xvfb/xrdp), rustc 1.100.0, WebKitGTK 2.52.6.
Dua instance app desktop dijalankan bersamaan: `ray` (:42141) dan `iphone` (:42142),
lalu satu instance ketiga `android` (:42153) untuk uji temuan multi-peer.

## 1. Unit + integrasi (`cargo test`)

```
8 test unit    — proto (frame tagging, room_id deterministik, status roundtrip),
                 identity (id stabil antar restart), store (riwayat, unread, clear, settings)
4 test integrasi — dm_terkirim_dan_di_ack, pesan_grup_sampai_ke_anggota,
                 typing_dan_read_receipt_sampai, kirim_ke_room_tanpa_anggota_terhubung_ditolak
test result: ok. 12 passed
```

Integrasi memakai dua `Node` nyata dengan socket TCP nyata di loopback (bukan mock).

## 2. Discovery di app asli

Dua window app, keduanya saling menemukan tanpa konfigurasi:

```
window ray    → "DI JARINGAN INI: iphone — Online · linux"
window iphone → "DI JARINGAN INI: ray — Online · linux"
```

Dengan instance ketiga: window `ray` menampilkan **android** dan **iphone** sekaligus.

## 3. Chat dua arah (bukti dari SQLite lokal masing-masing instance)

Setelah mengetik di UI lalu Enter:

```
ray    : [('ray','halo iphone, ini pesan dari ray', mine=1, delivered=1),
          ('iphone','diterima, ini balasan dari iphone', mine=0, delivered=0)]
iphone : [('ray','halo iphone, ini pesan dari ray', mine=0, delivered=0),
          ('iphone','diterima, ini balasan dari iphone', mine=1, delivered=1)]
```

Bubble keluar di sisi pengirim menampilkan **Terkirim ✓✓** setelah `read` frame diterima.
UI menampilkan bubble biru rata kanan (pesan sendiri) dan abu rata kiri (pesan masuk),
dengan nama pengirim di atas bubble untuk grup.

## 4. Fitur UI yang diuji langsung di window

| uji | hasil |
|---|---|
| Klik peer → buka DM | ✅ thread `dm iphone` dibuat & panel kanan menampilkan header + chat bar |
| Kirim pesan via Enter | ✅ masuk DB, ack diterima, ✓✓ muncul |
| Balas dari instance lain | ✅ sampai ke window pertama dan tampil sebagai bubble masuk |
| Klik gear → modal Pengaturan | ✅ modal terbuka (Light/Dark, warna bubble, radius, zoom) |
| `Ctrl + +` 3× | ✅ `settings.scale` = 1.0 → **1.3**, toast "Zoom chat 130%" |
| `Ctrl + scroll` down 2× | ✅ 1.3 → **1.1** |
| `Ctrl + 0` | ✅ kembali ke **1.0** |
| Window resize 760×480 dan 1180×720 | ✅ layout ikut menyesuaikan, min-width 720 dipatuhi |
| Tema dark | ✅ warna & kontras terbaca, tidak ada elemen menutupi |
| Badge unread | ✅ instance penerima menampilkan badge 1 sebelum thread dibuka, hilang setelah dibuka |

## Bug yang ketemu saat uji dan sudah diperbaiki

1. **`.modal-backdrop` selalu tampil** — `display:grid` menimpa atribut `hidden`, jadi overlay
   transparan menutupi seluruh app dan **semua klik mati**. Perbaikan: `[hidden]{display:none!important}`.
   Ini yang bikin klik tidak responsif di percobaan pertama.
2. **Nama pengirim jadi flex-item di samping bubble** (bukan di atasnya) — perbaikan: wrapper
   `.stack` di dalam `.row`.
3. **Toast zoom menumpuk** setiap penekanan tombol — perbaikan: toast ber-`data-key` di-reuse.
4. **`cargo build` gagal karena path `dekat-core`** (`../core` → `../../crates/core`).
5. **Icon PNG harus RGBA** — `icon.png` awal 16-bit tanpa alpha; dibuat ulang dengan `PNG32`.
6. **`borrow of moved value`** di handler `Event::Typing` — `from.clone()`.
7. **Frame `hello` hanya dikirim sisi dialer** (bug M0 yang baru ketahuan di uji M0) — sekarang
   kedua sisi mengirim `hello` lebih dulu.

## Batas yang jujur

- Uji dilakukan di satu host (dua/tiga proses, satu network namespace), bukan dua mesin fisik.
- Belum diuji di Windows maupun Android.
- Frame masih plaintext (TLS + signature belum ada), jadi belum aman untuk jaringan publik.
- Voice/video call dan kirim file belum ada (M3/M5).
- Sesi uji GUI di X11 rapuh: restart `xfwm4` menghilangkan window app, jadi uji klik
  dilakukan sebelum window manager di-restart.

## 5. Build release

```
cargo build --release -p dekat-desktop   → 16m 17s
target/release/dekat-desktop             → 5.6 MB
```

Binary release diuji ulang dengan skrip E2E yang sama (bukan cuma compile):
`BIN=target/release/dekat-desktop bash scripts/e2e-ui.sh` → chat dua arah + ✓✓ OK.

## 6. Tangkapan layar dari app nyata

Semua diambil dari window aplikasi asli (WebKitGTK, display :11), bukan mockup:

| file | isi |
|---|---|
| `docs/screens/app-01-discovery.png` | ray menemukan iphone di "Di jaringan ini" tanpa konfigurasi |
| `docs/screens/app-02-dm-open.png` | DM dibuka, header + chat bar aktif |
| `docs/screens/app-03-chat.png` | dua bubble dua arah, pesan keluar ber-✓✓ |
| `docs/screens/app-04-zoom.png` | `Ctrl + +` → "Zoom chat 120%" |
| `docs/screens/app-05-settings.png` | modal Pengaturan (tema, warna bubble, radius, zoom) |
| `docs/screens/app-06-dark.png` | tema dark aktif setelah restart (setting persisten) |