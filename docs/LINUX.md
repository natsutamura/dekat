# Dekat di Linux

Semua build Linux dibuat di runner Ubuntu 22.04 (lihat `.github/workflows/build-linux.yml`).

## Pilih cara pasang

### 1. Arch / CachyOS / Manjaro — PKGBUILD

```bash
git clone https://github.com/natsutamura/dekat
cd dekat/packaging
makepkg -si
```

`packaging/PKGBUILD` memasang paket `dekat-bin` dari tar.gz rilis resmi
(bukan kompilasi ulang — build Tauri butuh 15+ menit).

Dependensi yang dipasang otomatis: `webkit2gtk-4.1`, `gtk3`, `libayatana-appindicator`,
`librsvg`.

### 2. AppImage — distro apa pun

```bash
chmod +x Dekat-x86_64.AppImage
./Dekat-x86_64.AppImage
```

Tidak perlu instalasi. Butuh FUSE; kalau tidak ada:
`./Dekat-x86_64.AppImage --appimage-extract` lalu jalankan `squashfs-root/AppRun`.

### 3. tar.gz portabel

```bash
tar -xzf dekat-0.3.1-linux-x64.tar.gz
sudo cp dekat-0.3.1-linux-x64/usr/bin/dekat /usr/local/bin/
```

### 4. Debian / Ubuntu

```bash
sudo apt install ./Dekat_0.3.1_amd64.deb
```

## Dependensi runtime

| distro | paket |
|---|---|
| Arch | `webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg` |
| Debian/Ubuntu | `libwebkit2gtk-4.1-0 libgtk-3-0 libayatana-appindicator3-1` |
| Fedora | `webkit2gtk4.1 gtk3 libayatana-appindicator-gtk3` |

## Jaringan

Supaya peer saling menemukan:

1. Kedua perangkat harus di **subnet yang sama** (satu WiFi/hotspot).
2. Firewall harus mengizinkan **UDP 41234** (discovery) dan **TCP 41235** (chat):

```bash
# firewalld (Fedora, sebagian Arch)
sudo firewall-cmd --add-port=41234/udp --add-port=41235/tcp
sudo firewall-cmd --add-port=41234/udp --add-port=41235/tcp --permanent

# ufw (Debian/Ubuntu)
sudo ufw allow 41234/udp
sudo ufw allow 41235/tcp
```

3. Banyak WiFi publik/kantor memakai **client isolation** — peer tidak akan saling
   melihat. Itu batasan jaringan, bukan aplikasi. Solusi: hotspot dari HP, atau
   isi alamat manual:

```bash
dekat --name ray --seed 192.168.43.20:41234
```

## Kalau suara tidak jalan

PipeWire/PulseAudio harus hidup. Cek mikrofon terdeteksi:

```bash
pactl list short sources
```

WebViewGTK kadang butuh `WEBKIT_DISABLE_DMABUF_RENDERER=1` pada GPU lama —
kalau jendela hitam, jalankan:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 dekat
```

## Batasan yang diketahui

- Belum ada paket AUR resmi; PKGBUILD disediakan di repo.
- Wayland: jalan lewat XWayland. Kalau ada masalah input, coba
  `GDK_BACKEND=x11 dekat`.
## Ikon

`apps/desktop/icons/icon.png` (RGBA 1024×1024) adalah sumber utama; `icon.ico`
dibuat dari situ untuk Windows. Versi vektornya ada di `dekat.svg`.

Gradasi ikon ini bukan radial biasa: warnanya bergantung pada jarak ke tepi
(terang di pinggir, pekat di tengah). Tool ekspor SVG umumnya membuang efek ini dan
menghasilkan gambar flat. `dekat.svg` menyelesaikannya dengan
`feGaussianBlur` → `feComponentTransfer` (tabel kurva gradasi asli, 64 titik) →
`feFlood`, jadi tetap vektor murni tanpa gambar tertanam.

Kecocokan terukur terhadap `icon.png`: beda rata-rata 1,7/255 pada 1080 px,
1,6 pada 256 px, 2,2 pada 64 px. Untuk hasil yang identik bit-per-bit, pakai PNG.
