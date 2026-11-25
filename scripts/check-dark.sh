#!/usr/bin/env bash
# Cek tema dark di app nyata: simpan setting theme=dark, restart app, screenshot.
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/lanchat/target/debug/lanchat-desktop}
pkill -9 -x lanchat-desktop 2>/dev/null; sleep 2

python3 - <<'PY'
import sqlite3, os
db="/root/.lanchat-ray/lanchat.db"
if not os.path.exists(db):
    print("db tidak ada, lewati"); raise SystemExit
c=sqlite3.connect(db, timeout=3)
c.execute("insert into settings(k,v) values('theme','dark') on conflict(k) do update set v='dark'")
c.commit(); print("theme diset:", c.execute("select v from settings where k='theme'").fetchall())
PY

LANCHAT_NAME=ray LANCHAT_PORT=42181 LANCHAT_DIR=/root/.lanchat-ray setsid nohup "$BIN" >/tmp/app-ray.log 2>&1 &
LANCHAT_NAME=iphone LANCHAT_PORT=42182 LANCHAT_DIR=/root/.lanchat-iphone setsid nohup "$BIN" >/tmp/app-iphone.log 2>&1 &
sleep 12
W=$(xdotool search --name "LAN Chat — ray" | head -1)
echo "win=$W"
xdotool windowsize $W 900 600; xdotool windowmove $W 0 30; sleep 2
xdotool windowactivate --sync $W; sleep 1
# buka thread dari PERCAKAPAN (y~151)
xdotool mousemove --window $W 120 151 click 1; sleep 2
import -window $W /tmp/e2e-ui/dark-real.png
echo DONE