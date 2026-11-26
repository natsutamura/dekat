#!/usr/bin/env bash
# Menjalankan dua instance app desktop di display :11 untuk uji E2E (Linux, satu host).
set -u
cd /root/dekat || exit 1
pkill -x dekat-desktop 2>/dev/null
sleep 1
rm -rf /root/.dekat-ray /root/.dekat-iphone

export DISPLAY=:11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export LIBGL_ALWAYS_SOFTWARE=1

DEKAT_NAME=ray DEKAT_PORT=42101 DEKAT_DIR=/root/.dekat-ray \
  ./target/debug/dekat-desktop >/tmp/app-ray.log 2>&1 &
DEKAT_NAME=iphone DEKAT_PORT=42102 DEKAT_DIR=/root/.dekat-iphone \
  ./target/debug/dekat-desktop >/tmp/app-iphone.log 2>&1 &

sleep 8
xdotool search --name "Dekat" > /tmp/wins.txt 2>/dev/null
echo "windows:"; cat /tmp/wins.txt
mapfile -t W < /tmp/wins.txt
if [ "${#W[@]}" -ge 2 ]; then
  xdotool windowsize "${W[0]}" 940 620; xdotool windowmove "${W[0]}" 10 40
  xdotool windowsize "${W[1]}" 940 620; xdotool windowmove "${W[1]}" 970 40
fi
sleep 3
DISPLAY=:11 import -window root /tmp/e2e-boot.png 2>/dev/null
echo "screenshot: /tmp/e2e-boot.png"
tail -3 /tmp/app-ray.log
