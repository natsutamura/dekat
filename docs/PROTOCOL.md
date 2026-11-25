# Protokol LAN Chat v1 (M0)

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
{"v":1,"id":"7fda069ad40267af","name":"ray","fp":"ray#7fda","os":"linux","port":42001,"ts":1795000000000}
```

- `id` — 8 byte pertama public key Ed25519 (hex 16 char), stabil antar restart
- `port` — port TCP tempat node menerima frame
- penerima mengabaikan announce dengan `id` miliknya sendiri

## 2. Transport (TCP + framing)

Frame = `u32 BE length` + JSON body. Batas 8 MiB (nanti untuk file chunk).

Handshake: **kedua sisi mengirim `hello` segera setelah koneksi terbuka**, lalu membaca
`hello` lawan. Frame pertama yang bukan `hello` → koneksi ditolak.

```json
{"t":"hello","v":1,"id":"...","name":"ray","fp":"ray#7fda","port":42001}
{"t":"text","id":"4ba15ab0c1d2e3f4","from":"...","from_name":"ray","to":"*","ts":1795...,"body":"halo"}
{"t":"ack","id":"4ba15ab0c1d2e3f4","ts":1795...}
```

- `to` = `"*"` → semua koneksi; atau node id peer
- penerima mengirim `ack` balik untuk tiap `text` yang ditujukan ke dia
- tag `t` memakai snake_case supaya gampang ditambah tipe baru (nanti: `file_offer`,
  `typing`, `presence`, `call_offer`, `call_answer`, `ice`)

## 3. Aturan koneksi (anti-dobel)

Untuk tiap pasangan node, **hanya node dengan `id` lebih kecil (lexicographic) yang melakukan
dial**. Hasilnya tepat satu koneksi TCP bidirectional per pasangan; dua arah menulis di
socket yang sama. Kalau koneksi putus, sisi yang lebih kecil akan dial ulang saat announce
berikutnya (≤5 s).

## 4. Batas M0

Belum ada: TLS, signature per frame, persistensi riwayat, group/room, file transfer, presence
selain up/down, reconnect backoff eksponensial. Semua masuk M1/M3 (lihat ROADMAP.md).
