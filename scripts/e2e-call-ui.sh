#!/usr/bin/env bash
# Buktikan call.js termuat di binary RELEASE: jalankan app, buka DM, klik tombol panggilan.
# Mesin ini tidak punya mikrofon, jadi yang diharapkan muncul adalah call bar dengan
# status "mikrofon ditolak" — itu justru bukti call.js jalan dan getUserMedia dipanggil.
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/lanchat/target/release/lanchat-desktop}
OUT=/tmp/e2e-call-ui
rm -rf $OUT /root/.lc-c-ray /root/.lc-c-iphone; mkdir -p $OUT
pkill -9 -x lanchat-desktop 2>/dev/null; sleep 2

LANCHAT_NAME=ray LANCHAT_PORT=42501 LANCHAT_DIR=/root/.lc-c-ray setsid nohup "$BIN" >/tmp/lc-c-ray.log 2>&1 &
LANCHAT_NAME=iphone LANCHAT_PORT=42502 LANCHAT_DIR=/root/.lc-c-iphone setsid nohup "$BIN" >/tmp/lc-c-iphone.log 2>&1 &

for i in $(seq 1 30); do
  RAY=$(xdotool search --name "LAN Chat — ray" 2>/dev/null | head -1)
  [ -n "${RAY:-}" ] && break
  sleep 1
done
echo "win=$RAY"
xdotool windowsize $RAY 900 600; xdotool windowmove $RAY 0 30; sleep 3
xdotool windowactivate --sync $RAY; sleep 2

# buka DM ke iphone (baris peer pertama di sidebar)
xdotool mousemove --window $RAY 120 205 click 1; sleep 2
import -window $RAY $OUT/01-dm.png

# tombol telepon (ikon kamera) di header: pusat (833,26); info ada di (875,26)
xdotool mousemove --window $RAY 833 26 click 1; sleep 3
import -window $RAY $OUT/02-after-call-click.png
echo "--- log app ---"
tail -5 /tmp/lc-c-ray.log
echo DONE