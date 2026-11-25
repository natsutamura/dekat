#!/usr/bin/env bash
# Menjalankan dua instance app desktop di display :11 untuk uji E2E (Linux, satu host).
set -u
cd /root/lanchat || exit 1
pkill -x lanchat-desktop 2>/dev/null
sleep 1
rm -rf /root/.lanchat-ray /root/.lanchat-iphone

export DISPLAY=:11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export LIBGL_ALWAYS_SOFTWARE=1

LANCHAT_NAME=ray LANCHAT_PORT=42101 LANCHAT_DIR=/root/.lanchat-ray \
  ./target/debug/lanchat-desktop >/tmp/app-ray.log 2>&1 &
LANCHAT_NAME=iphone LANCHAT_PORT=42102 LANCHAT_DIR=/root/.lanchat-iphone \
  ./target/debug/lanchat-desktop >/tmp/app-iphone.log 2>&1 &

sleep 8
xdotool search --name "LAN Chat" > /tmp/wins.txt 2>/dev/null
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
