#!/usr/bin/env bash
# Verifikasi zoom lewat keyboard di app nyata: cek nilai scale di SQLite.
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/lanchat/target/release/lanchat-desktop}
pkill -9 -x lanchat-desktop 2>/dev/null; sleep 2

LANCHAT_NAME=ray LANCHAT_PORT=42191 LANCHAT_DIR=/root/.lanchat-ray setsid nohup "$BIN" >/tmp/app-ray.log 2>&1 &
sleep 10
W=$(xdotool search --name "LAN Chat — ray" | head -1)
echo "win=$W"
xdotool windowsize $W 900 600; xdotool windowmove $W 0 30; sleep 2
for i in 1 2 3 4 5 6; do
  xdotool windowactivate --sync "$W" 2>/dev/null; sleep 0.6
  [ "$(xdotool getwindowfocus)" = "$W" ] && break
done
echo "focus=$(xdotool getwindowfocus) target=$W"

echo "scale awal: $(python3 -c "import sqlite3;print(sqlite3.connect('/root/.lanchat-ray/lanchat.db',timeout=3).execute(\"select v from settings where k='scale'\").fetchall())")"

# Ctrl + '+' tiga kali  (plus butuh shift pada layout US)
xdotool key --clearmodifiers ctrl+equal; sleep 0.5
xdotool key --clearmodifiers ctrl+equal; sleep 0.5
xdotool key --clearmodifiers ctrl+equal; sleep 1.5
import -window $W /tmp/e2e-ui/zoom-a.png
echo "scale setelah 3x: $(python3 -c "import sqlite3;print(sqlite3.connect('/root/.lanchat-ray/lanchat.db',timeout=3).execute(\"select v from settings where k='scale'\").fetchall())")"

# Ctrl + scroll down (zoom out) di atas area chat
xdotool mousemove --window $W 600 300; sleep 0.3
xdotool keydown ctrl; xdotool click 5; xdotool click 5; xdotool keyup ctrl; sleep 1.5
import -window $W /tmp/e2e-ui/zoom-b.png
echo "scale setelah scroll: $(python3 -c "import sqlite3;print(sqlite3.connect('/root/.lanchat-ray/lanchat.db',timeout=3).execute(\"select v from settings where k='scale'\").fetchall())")"

# Ctrl+0 reset
xdotool key --clearmodifiers ctrl+0; sleep 1.5
echo "scale setelah reset: $(python3 -c "import sqlite3;print(sqlite3.connect('/root/.lanchat-ray/lanchat.db',timeout=3).execute(\"select v from settings where k='scale'\").fetchall())")"
import -window $W /tmp/e2e-ui/zoom-c.png
echo DONE