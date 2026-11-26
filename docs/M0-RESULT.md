# Hasil uji M0

Lingkungan: VPS Linux (KVM), rustc 1.100.0, dua instance `dekat` di host yang sama
(`scripts/e2e-m0.sh`), discover lewat multicast `239.255.42.99:41234`.

```
================ A (ray :42001) ================
dekat M0 — ray [7fda069ad40267af]
listening TCP :42001
  + iphone [linux] 4d95aa  10.10.1.28:42002
[02:34:25] aku → semua: halo semua dari ray
[02:34:25] ✓ terkirim (4ba15ab0)
[02:34:27] aku → 4d95aa: dm khusus buat iphone
[02:34:27] ✓ terkirim (7ae4e779)
[02:34:29] iphone: balasan dari iphone
================ B (iphone :42002) ==============
dekat M0 — iphone [4d95aa97ce217a04]
listening TCP :42002
  + ray [linux] 7fda06  10.10.1.28:42001
[02:34:25] ray: halo semua dari ray
[02:34:27] ray: dm khusus buat iphone
[02:34:29] aku → semua: balasan dari iphone
[02:34:29] ✓ terkirim (4ed00c24)
  ray          linux    7fda06 10.10.1.28:42001  connected
```

Yang terbukti:
- presence dua arah via multicast tanpa konfigurasi apa pun
- satu koneksi TCP per pasangan (aturan "id lebih kecil yang dial"), bidirectional
- broadcast dan DM terkirim, ack (`✓ terkirim`) diterima balik
- `/peers` menandai `connected`

Belum diuji di lingkungan nyata: dua mesin fisik / Windows / Android, WiFi dengan AP isolation.

Bug yang ketemu saat uji dan sudah diperbaiki: sisi inbound tidak mengirim `hello`
sehingga dialer menunggu selamanya — sekarang kedua sisi mengirim `hello` lebih dulu.
