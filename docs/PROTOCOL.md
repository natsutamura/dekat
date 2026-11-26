# Protokol Dekat v5 (M1 + M3 + M5 + TLS)

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

## 2. Transport (TLS + TCP + framing)

**Sejak v5 semua koneksi TCP dibungkus TLS 1.3.** Tidak ada mode plaintext kecuali
dinyalakan manual dengan `DEKAT_ALLOW_PLAINTEXT=1` (untuk bicara dengan build lama).

### 2.1 Handshake

```
TCP connect
  └─ TLS handshake (self-signed, rustls, TLS 1.3; sertifikat TIDAK dipakai memutuskan percaya)
      └─ hello   (plain, kedua arah, berisi id + pk)
          └─ auth (tanda tangan Ed25519 atas exporter sesi TLS)  ← kedua arah
              └─ frame biasa (text/ack/file/call/…)
```

Frame = `u32 BE length` + JSON body. Batas 8 MiB. Panjang di luar `1..=8 MiB` → koneksi diputus.

```json
{"Hello":{"v":3,"id":"7fda069ad40267af","name":"ray","fp":"ray#7fda","port":42001,"os":"linux","status":"online","sig":"","pk":""}}
{"Auth":{"pk":"<64 hex>","sig":"<128 hex>"}}
{"CallOffer":{"call_id":"c-1a2b","from":"...","from_name":"ray","to":"...","sdp":"v=0...","ts":...,"members":["id-a","id-b","id-c"]}}
{"Text":{"id":"4ba15ab0c1d2e3f4","from":"...","from_name":"ray","to":"*","ts":1795...,"body":"halo"}}
```

### 2.2 Kenapa sertifikat self-signed tidak jadi masalah

Sertifikat hanya menyediakan **kunci sesi**, bukan identitas. Verifikator sengaja
menerima sertifikat apa pun (`AcceptAnyCert`). Identitas dibuktikan di langkah `auth`:

1. Kedua ujung memanggil `export_keying_material(label="EXPORTER-dekat-channel-v1", 32 byte)`.
   Nilai ini diturunkan dari rahasia sesi TLS → **hanya diketahui kedua ujung**.
2. Tiap sisi menandatangani `HELLO_CTX ‖ exporter ‖ 0 ‖ min(id_a,id_b) ‖ 0 ‖ max(id_a,id_b)`
   dengan kunci Ed25519-nya, lalu mengirim `Auth{pk, sig}`.
3. Penerima memverifikasi: `sig` sah **dan** 8 byte pertama `pk` == `id` peer.

**Kenapa exporter wajib ada:** tanpa itu, penyerang bisa menjadi relay — meneruskan
tanda tangan asli A ke B dan sebaliknya. Dengan exporter, tanda tangan A terikat pada
sesi TLS A↔MITM, sehingga tidak sah di sesi MITM↔B. Relay gagal total.

**Batas yang jujur:** penyerang yang menguasai jaringan **saat pertama kali** dua node
bertemu bisa menyamar (tidak ada PKI/OOB pairing). Ini batas semua aplikasi LAN tanpa
server; mitigasinya pairing QR di M6, bukan TLS.

### 2.3 Status peer

| field | arti |
|---|---|
| `verified: true` | identitas dibuktikan tanda tangan di sesi TLS ini |
| `verified: false` | berasal dari announce multicast saja, atau peer lama (plaintext) |

`verified` hanya boleh naik, tidak pernah turun (announce tidak menghapus bukti TLS).
{"t":"typing","from":"...","to":"<peer-id>","on":true}
{"t":"read","from":"...","to":"<peer-id>","up_to_ts":1795...}
{"t":"presence","id":"...","status":"dnd"}
{"t":"file_offer","id":"f1","from":"...","from_name":"ray","to":"<peer>","name":"foto.jpg","size":700000,"sha256":"2c96...","chunks":6,"ts":1795...}
{"t":"file_accept","id":"f1","accept":true,"reason":""}
{"t":"file_chunk","id":"f1","idx":0,"data":"<base64 128 KiB>"}
{"t":"file_done","id":"f1","ok":true,"reason":""}
{"t":"call_offer","call_id":"c-9f2a","from":"...","from_name":"ray","to":"<peer>","sdp":"v=0...","ts":1795...}
{"t":"call_answer","call_id":"c-9f2a","from":"...","accept":true,"sdp":"v=0...","reason":""}
{"t":"call_ice","call_id":"c-9f2a","from":"...","candidate":"candidate:1 1 UDP ...","sdp_mid":"0","sdp_mline_index":0}
{"t":"call_end","call_id":"c-9f2a","from":"...","reason":"ditutup"}
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

## 6. Voice call (M5)

WebRTC audio 1:1, **peer-to-peer langsung**, tanpa server STUN/TURN — di subnet yang sama,
host candidate sudah cukup. Rust hanya jadi pipa signalling; audio tidak pernah lewat Rust.

Alur:

1. Penelepon: `getUserMedia({audio})` → `RTCPeerConnection` → `createOffer` →
   tunggu ICE gathering selesai (timeout 1,5 s) → kirim `call_offer` berisi SDP.
2. Penerima: dapat `call_offer` → popup → kalau diterima: `setRemoteDescription(offer)` →
   `createAnswer` → kirim `call_answer` berisi SDP.
3. Kedua sisi mengirim `call_ice` untuk kandidat tambahan (trickle). Kandidat yang tiba
   sebelum remote description disimpan di antrean dan di-flush setelahnya.
4. `ontrack` → audio diputar lewat `<audio>` tersembunyi; timer mulai.
5. Salah satu sisi menekan tombol tutup → `call_end` → kedua sisi menutup RTCPeerConnection
   dan mematikan track mikrofon.

Detail penting:
- `call_id` dibuat acak (`c-` + 8 hex) per panggilan, dipakai untuk semua frame panggilan itu
- `iceServers: []` disengaja: tidak ada STUN publik, karena LAN-only dan tidak butuh internet
- `echoCancellation`, `noiseSuppression`, `autoGainControl` aktif
- Penerima yang sedang dalam panggilan lain otomatis menolak dengan alasan
  "sedang dalam panggilan lain"
- Batas M5: audio saja, 1:1, tanpa rekaman, tanpa device picker (pakai default OS)

## 7. Batas M1

Belum ada: TLS, signature per frame, file transfer, store-and-forward untuk anggota grup
yang offline, dan call. Riwayat sudah persisten di SQLite lokal per perangkat.
## 6. Panggilan suara (WebRTC mesh)

Rust hanya meneruskan SDP + ICE lewat frame `CallOffer`/`CallAnswer`/`CallIce`/`CallEnd`.
Audio mengalir langsung peer-to-peer; tidak ada STUN/TURN karena semua peserta satu subnet.

### Panggilan grup

`CallOffer.members` berisi seluruh peserta (termasuk pemanggil). Isi kosong = panggilan 1:1.

Aturan pembentukan **mesh penuh** (tiap peserta terhubung langsung ke tiap peserta lain):

1. Pemanggil mengirim `CallOffer` ke **semua** peserta.
2. Setelah menerima, tiap peserta menjawab pemanggil (`CallAnswer`), lalu mengirim
   `CallOffer` sendiri ke peserta lain yang **id-nya lebih besar** dari dirinya.
3. Karena aturan langkah 2 hanya ke arah id lebih besar, tiap pasangan menghasilkan
   **tepat satu** koneksi — tidak ada koneksi dobel.

Beban bandwidth naik kuadratik: untuk N peserta, tiap perangkat mengirim suaranya N−1 kali.
Praktis nyaman sampai ~5–6 peserta; di atas itu perlu SFU (di luar cakupan, karena tanpa server).

### Batas

- Satu perangkat hanya boleh berada di satu panggilan pada satu waktu. Undangan yang datang
  saat sedang menelepon dijawab `accept=false` dengan alasan `sedang dalam panggilan lain`.
- Belum ada video, device picker, dan indikator siapa yang sedang bicara.
- Satu peserta keluar tidak mengakhiri panggilan grup; koneksinya saja yang ditutup.
