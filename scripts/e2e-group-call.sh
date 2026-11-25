#!/usr/bin/env bash
# E2E panggilan grup: 3 instance app, A memanggil grup {B,C}.
# Yang dibuktikan: tiap instance membentuk 2 RTCPeerConnection (mesh penuh),
# audio track hidup, dan tidak ada koneksi dobel.
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/lanchat/target/debug/lanchat-desktop}
OUT=/tmp/e2e-group-call
rm -rf $OUT /root/.lc-g-*; mkdir -p $OUT
pkill -9 -x lanchat-desktop 2>/dev/null; sleep 2

# Grup butuh anggota yang sudah jadi thread; bentuk grupmu lewat DB supaya deterministik.
# Id node baru dibuat saat start, jadi start dulu lalu isi DB.
LANCHAT_NAME=alpha LANCHAT_PORT=44701 LANCHAT_DIR=/root/.lc-g-alpha setsid nohup "$BIN" >/tmp/gc-alpha.log 2>&1 &
LANCHAT_NAME=beta  LANCHAT_PORT=44702 LANCHAT_DIR=/root/.lc-g-beta  setsid nohup "$BIN" >/tmp/gc-beta.log 2>&1 &
LANCHAT_NAME=gamma LANCHAT_PORT=44703 LANCHAT_DIR=/root/.lc-g-gamma setsid nohup "$BIN" >/tmp/gc-gamma.log 2>&1 &

for i in $(seq 1 30); do
  n=$(xdotool search --name "^LAN Chat — " 2>/dev/null | wc -l)
  [ "$n" -ge 3 ] && break
  sleep 1
done
echo "window: $(xdotool search --name '^LAN Chat — ' | wc -l)"

# Id tiap node dari DB (tabel settings menyimpan id? kalau tidak, dari log/stdout).
for who in alpha beta gamma; do
  ID=$(sqlite3 /root/.lc-g-$who/lanchat.db "select v from settings where k='node_id';" 2>/dev/null)
  echo "$who id=${ID:-<none>}"
done
echo DONE
