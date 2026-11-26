#!/usr/bin/env bash
# E2E: kirim PNG asli lewat dialog GUI → harus tampil inline (bukan kartu file).
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/dekat/target/debug/dekat-desktop}
OUT=/tmp/e2e-img
rm -rf $OUT /root/.lc-i-ray /root/.lc-i-iphone; mkdir -p $OUT
pkill -9 -x dekat-desktop 2>/dev/null; sleep 2

# PNG asli 640x400 dengan gradien + teks, biar kelihatan jelas di screenshot
convert -size 640x400 gradient:'#0a84ff-#30d158' \
  -gravity center -pointsize 54 -fill white -annotate 0 "foto liburan" \
  $OUT/foto-liburan.png
identify $OUT/foto-liburan.png
SHA=$(sha256sum $OUT/foto-liburan.png | cut -d' ' -f1)

DEKAT_NAME=ray DEKAT_PORT=42601 DEKAT_DIR=/root/.lc-i-ray setsid nohup "$BIN" >/tmp/lc-i-ray.log 2>&1 &
DEKAT_NAME=iphone DEKAT_PORT=42602 DEKAT_DIR=/root/.lc-i-iphone setsid nohup "$BIN" >/tmp/lc-i-iphone.log 2>&1 &

for i in $(seq 1 30); do
  RAY=$(xdotool search --name "Dekat — ray" 2>/dev/null | head -1)
  [ -n "${RAY:-}" ] && break
  sleep 1
done
echo "win=$RAY"
xdotool windowsize $RAY 900 600; xdotool windowmove $RAY 0 30; sleep 3
xdotool windowactivate --sync $RAY; sleep 2

# buka DM
xdotool mousemove --window $RAY 120 205 click 1; sleep 2
xdotool windowactivate --sync $RAY; sleep 1

# kirim PNG lewat dialog
xdotool mousemove --window $RAY 315 572 click 1; sleep 3
xdotool key --clearmodifiers ctrl+l; sleep 1
xdotool type --delay 25 "$OUT/foto-liburan.png"; sleep 1
xdotool key --clearmodifiers Return; sleep 5
import -window $RAY $OUT/01-ray-sent.png

# penerima: buka thread, screenshot
IPH=$(xdotool search --name "Dekat — iphone" | head -1)
xdotool windowsize $IPH 900 600; xdotool windowmove $IPH 930 30; sleep 2
xdotool windowactivate --sync $IPH; sleep 1
xdotool mousemove --window $IPH 120 151 click 1; sleep 3
import -window $IPH $OUT/02-iphone-received.png

echo "=== file diterima ==="
ls -la /root/.lc-i-iphone/files/ 2>/dev/null
GOT=$(ls /root/.lc-i-iphone/files/*.png 2>/dev/null | head -1)
if [ -n "$GOT" ]; then
  echo "sha asli : $SHA"
  echo "sha terima: $(sha256sum "$GOT" | cut -d' ' -f1)"
fi
echo DONE
