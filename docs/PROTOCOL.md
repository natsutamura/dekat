# Protokol LAN Chat v3 (M1 + M3)

Tanpa server. Semua node setara (peer-to-peer). Angka port ada di `crates/core/src/proto.rs`.

## 1. Discovery (UDP)

| item | nilai |
|---|---|
| port | `41234` |
| multicast group | `239.255.42.99` |
| fallback 1 | limited broadcast `255.255.255.255:41234` |
| fallback 2 | seed unicast `ip:41234` dari argumen CLI |
| interval announce | 5 s |
| TTL peer | 16 s (3 interval → hapus dari list) |

Socket dibuka dengan `SO_REUSEADDR` + `SO_REUSEPORT` supaya beberapa instance di satu host
(atau beberapa user di satu mesin) tetap bisa menerima announce.

Payload `Announce` (JSON, UTF-8):

```json
{"v":2,"id":"7fda069ad40267af","name":"ray","fp":"ray#7fda","os":"linux","port":42001,"status":"online","ts":1795000000000}
```

- `id` — 8 byte pertama public key Ed25519 (hex 16 char), stabil antar restart
- `port` — port TCP tempat node menerima frame
- `status` — `online` | `idle` | `dnd` (field opsional, klien lama dianggap `online`)
- penerima mengabaikan announce dengan `id` miliknya sendiri

## 2. Transport (TCP + framing)

Frame = `u32 BE length` + JSON body. Batas 8 MiB (nanti untuk file chunk).

Handshake: **kedua sisi mengirim `hello` segera setelah koneksi terbuka**, lalu membaca
`hello` lawan. Frame pertama yang bukan `hello` → koneksi ditolak.

```json
{"t":"hello","v":2,"id":"...","name":"ray","fp":"ray#7fda","port":42001,"status":"online"}
{"t":"text","id":"4ba15ab0c1d2e3f4","from":"...","from_name":"ray","to":"*","ts":1795...,"body":"halo"}
{"t":"ack","id":"4ba15ab0c1d2e3f4","ts":1795...}
{"t":"typing","from":"...","to":"<peer-id>","on":true}
{"t":"read","from":"...","to":"<peer-id>","up_to_ts":1795...}
{"t":"presence","id":"...","status":"dnd"}
{"t":"file_offer","id":"f1","from":"...","from_name":"ray","to":"<peer>","name":"foto.jpg","size":700000,"sha256":"2c96...","chunks":6,"ts":1795...}
{"t":"file_accept","id":"f1","accept":true,"reason":""}
{"t":"file_chunk","id":"f1","idx":0,"data":"<base64 128 KiB>"}
{"t":"file_done","id":"f1","ok":true,"reason":""}
```

- `to` = `"*"` → semua koneksi; `#xxxxxxxx` → room/grup; atau node id peer
- pesan grup menyertakan `members` (daftar node id) supaya penerima bisa memverifikasi
  keanggotaan tanpa server
- penerima mengirim `ack` balik untuk tiap `text` yang ditujukan ke dia
- `read` = "semua pesanmu sampai `up_to_ts` sudah kubaca" → pengirim menandai ✓✓
- tag `t` memakai snake_case supaya gampang ditambah tipe baru (nanti: `file_offer`,
  `call_offer`, `ice`)

## 3. Aturan koneksi (anti-dobel)

Untuk tiap pasangan node, **hanya node dengan `id` lebih kecil (lexicographic) yang melakukan
dial**. Hasilnya tepat satu koneksi TCP bidirectional per pasangan; dua arah menulis di
socket yang sama. Kalau koneksi putus, sisi yang lebih kecil akan dial ulang saat announce
berikutnya (≤5 s).

## 4. Grup / room

Tidak ada server dan tidak ada undangan. Room id dihitung deterministik dari himpunan
anggota (id diri sendiri wajib ikut):

```
room_id = "#" + hex(fnv1a64(sorted(member_ids).join("|")))[..8]
```

Semua node dengan himpunan anggota sama menghasilkan id sama. Pesan grup dikirim
langsung (fan-out) ke tiap anggota yang terkoneksi, dengan `to` = room id dan `members`
disertakan untuk verifikasi. Konsekuensi: anggota yang offline saat pesan dikirim tidak
menerimanya (belum ada store-and-forward — masuk M3).

## 5. Transfer file (M3)

Chunk default **128 KiB** byte mentah, di-encode base64 di dalam JSON (≈175 KiB per frame,
aman di bawah `MAX_FRAME` 8 MiB). Batas file 512 MiB.

Alur:

1. Pengirim: baca file, hitung `sha256` + `chunks`, kirim `file_offer`.
2. Penerima: buat `<data_dir>/files/<msg_id>-<nama-aman>` lalu balas `file_accept`.
   Nama file dibersihkan dulu (`safe_filename`): komponen path dibuang, traversal
   (`../`) tidak bisa keluar folder, karakter kontrol & `:*?"<>|` dibuang, max 180 char.
3. Pengirim: stream `file_chunk` berurutan (`idx` mulai 0). Penerima **menolak** chunk yang
   tidak berurutan atau yang membuat ukuran melebihi yang dijanjikan.
4. Penerima: setelah byte terakhir, verifikasi `sha256`. Kalau beda → **file dihapus** dan
   `file_done{ok:false}` dikirim ke pengirim.
5. Pengirim: menunggu `file_done`. Semua langkah punya timeout (30 s) supaya UI tidak
   menggantung kalau peer diam.

Batas M3: satu file per pesan, penerima harus online, tidak ada resume kalau putus,
kirim file hanya untuk DM (grup belum).

## 6. Batas M1

Belum ada: TLS, signature per frame, file transfer, store-and-forward untuk anggota grup
yang offline, dan call. Riwayat sudah persisten di SQLite lokal per perangkat.
