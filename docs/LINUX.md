# LAN Chat di Linux

Semua build Linux dibuat di runner Ubuntu 22.04 (lihat `.github/workflows/build-linux.yml`).

## Pilih cara pasang

### 1. Arch / CachyOS / Manjaro — PKGBUILD

```bash
git clone https://github.com/natsutamura/lanchat
cd lanchat/packaging
makepkg -si
```

`packaging/PKGBUILD` memasang paket `lanchat-bin` dari tar.gz rilis resmi
(bukan kompilasi ulang — build Tauri butuh 15+ menit).

Dependensi yang dipasang otomatis: `webkit2gtk-4.1`, `gtk3`, `libayatana-appindicator`,
`librsvg`.

### 2. AppImage — distro apa pun

```bash
chmod +x LANChat-x86_64.AppImage
./LANChat-x86_64.AppImage
```

Tidak perlu instalasi. Butuh FUSE; kalau tidak ada:
`./LANChat-x86_64.AppImage --appimage-extract` lalu jalankan `squashfs-root/AppRun`.

### 3. tar.gz portabel

```bash
tar -xzf lanchat-0.2.0-linux-x64.tar.gz
sudo cp lanchat-0.2.0-linux-x64/usr/bin/lanchat /usr/local/bin/
```

### 4. Debian / Ubuntu

```bash
sudo apt install ./LANChat_0.2.0_amd64.deb
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
lanchat --name ray --seed 192.168.43.20:41234
```

## Kalau suara tidak jalan

PipeWire/PulseAudio harus hidup. Cek mikrofon terdeteksi:

```bash
pactl list short sources
```

WebViewGTK kadang butuh `WEBKIT_DISABLE_DMABUF_RENDERER=1` pada GPU lama —
kalau jendela hitam, jalankan:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 lanchat
```

## Batasan yang diketahui

- Belum ada paket AUR resmi; PKGBUILD disediakan di repo.
- Wayland: jalan lewat XWayland. Kalau ada masalah input, coba
  `GDK_BACKEND=x11 lanchat`.