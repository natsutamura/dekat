# LAN Chat — iMessage-style, LAN-only, Rust

Chat P2P di jaringan lokal (tanpa server, tanpa internet) untuk **Windows, Linux, Android**.
Tampilan mengikuti iMessage: bubble chat, tema dark/light, status online/idle/DND.

Status sekarang: **M0 selesai & teruji** — discovery, transport, chat teks (broadcast + DM),
ack delivery, identitas stabil. UI belum ada (M2).

## Jalanin (M0, CLI)

```bash
cargo build
./target/debug/lanchat --name ray --port 42001 --dir ~/.lanchat
```

Di mesin lain (satu network): `--name iphone --port 42002`.
Keduanya otomatis saling menemukan via multicast. Di dalam sesi:

```
<teks>              broadcast ke semua peer
/dm <id|nama> <txt> chat pribadi
/peers              daftar peer + status koneksi
/ping               info node sendiri
```

Kalau jaringan memblok multicast, tambahkan seed manual: `--seed 10.10.1.28:41234` (UDP discovery port).

Test E2E dua instance di satu host: `bash scripts/e2e-m0.sh`.

## Struktur

```
crates/core/     lib: proto, identity, discovery, node   (dipakai UI apa pun)
crates/cli/      harness CLI buat nguji core
apps/            UI desktop+mobile (M2, rencana: Tauri v2)
docs/            PROTOCOL.md, ROADMAP.md, UI-SPEC.md
```

## Kenapa Rust di core

Network loop, peer table, dan enkripsi jalan di satu proses Rust; UI cuma konsumen event.
Binary kecil (LTO + strip), RAM rendah, dan bisa dipakai ulang apa pun framework UI-nya.

## Keamanan (status jujur)

M0 masih plaintext: id node berasal dari public key Ed25519, tapi frame belum ditandatangani
dan TLS belum aktif. Jangan dipakai di jaringan yang tidak dipercaya sampai M1 selesai
(TLS + pinning TOFU + signature per pesan).

## Mockup UI

`apps/mockup/index.html` — mockup statis (buka di browser, `?theme=dark` untuk tema gelap).
Screenshot: `apps/mockup/mockup-light.png` dan `mockup-dark.png`.
