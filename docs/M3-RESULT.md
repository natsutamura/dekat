# Hasil uji M3 — kirim file

## 1. Unit + integrasi (`cargo test`)

```
21 unit test — termasuk 9 test modul files:
  prepare_menghitung_chunk_dan_hash, prepare_menolak_file_kosong,
  transfer_utuh_antar_dua_sisi, chunk_rusak_ditolak_dan_file_dihapus,
  potongan_tidak_berurutan_ditolak, abort_menghapus_file_setengah_jadi,
  nama_file_dari_jaringan_tidak_bisa_keluar_folder, progress_terbaca,
  sha_tidak_berubah_untuk_file_sama
  + safe_filename_strips_paths_and_traversal, human_size_formats, file_offer_roundtrip,
  + attachment_tersimpan_dan_ikut_terhapus (store)
7 integrasi dua node nyata — termasuk kirim_file_sampai_dan_utuh,
  kirim_file_tanpa_koneksi_gagal_dengan_jelas
test result: ok. 28 passed
```

## 2. E2E lewat CLI (tanpa GUI)

`bash scripts/e2e-file.sh` — dua node, file 700.000 byte (5 chunk + sisa):

```
pengirim: ↑ kirim.bin: 100%   ✓ file terkirim (53f5ce11)
penerima: ↓ kirim.bin: 18% → 37% → 56% → 74% → 93%
          ✓ file diterima: kirim.bin → /tmp/ft-e2e/b/files/53f5ce11d4d5041c-kirim.bin
sha sumber  : 2c961a47a140b5cf...
sha diterima: 2c961a47a140b5cf...
RESULT: SHA COCOK ✓
```

## 3. E2E lewat GUI asli (dialog file OS)

`bash scripts/e2e-file-ui.sh` — dua instance Tauri, file 900.000 byte:

```
=== DB ray (pengirim) ===
threads: [('18c289a3...', 'dm', 'iphone')]
att: [('046efe2c...', 'foto-liburan.bin', 900000, '/tmp/e2e-file-ui/foto-liburan.bin', '19fcf1bb...')]

=== DB iphone (penerima) ===
threads: [('7a9c2f82...', 'dm', 'ray', '📎 foto-liburan.bin')]
att: [('046efe2c...', 'foto-liburan.bin', 900000, '/root/.lc-f-iphone/files/046efe2c...-foto-liburan.bin', '19fcf1bb...')]

sha sumber  : 19fcf1bb4ca66598e878d59cd04496d4019c732ee03c05684c5a499e18dd032a
sha diterima: 19fcf1bb4ca66598e878d59cd04496d4019c732ee03c05684c5a499e18dd032a
RESULT: SHA COCOK ✓
```

Yang terbukti:
- tombol lampiran membuka dialog file OS; memilih file langsung mengirim
- pengirim menyimpan pesan + lampiran (path, ukuran, sha256) di SQLite
- penerima mencatat thread dengan preview `📎 foto-liburan.bin`, menyimpan file ke
  `<data_dir>/files/<msg_id>-<nama>`, dan sha256 file di disk identik dengan aslinya
- kartu file tampil di kedua sisi dengan nama, ukuran, progress bar penuh, dan tombol "Buka"
- `delivered`/✓✓ tidak dipakai untuk file (file punya konfirmasi `FileDone` sendiri)

## 4. Yang sengaja belum ada

- **Resume** kalau koneksi putus di tengah transfer (file setengah jadi dihapus, bukan dilanjutkan)
- Kirim file di **group chat** (ditolak dengan pesan jelas)
- Preview gambar / thumbnail, paste gambar dari clipboard
- Batas kecepatan (throttle): saat ini mengirim secepat TCP lokal mengizinkan
- Beberapa file sekaligus dalam satu pesan

## 5. Build release

```
cargo build --release -p dekat-desktop  → EXIT=0
target/release/dekat-desktop            → 6.7 MB
```

Binary **release** (bukan debug) diuji ulang dengan `BIN=target/release/dekat-desktop
bash scripts/e2e-file-ui.sh`: file 900.000 byte terkirim lewat dialog GUI asli,
`sha256 = cd058c7b2feca4b7…` identik di pengirim, penerima, dan file di disk.
