#!/usr/bin/env bash
# Uji fungsi app desktop nyata: dua instance, klik, kirim, balas, cek DB.
# Semua dalam satu skrip supaya tidak putus kalau sesi tool timeout.
set -u
export DISPLAY=:11
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/lanchat/target/debug/lanchat-desktop}

# Aktifkan window dan PASTIKAN fokus benar sebelum mengetik.
# xdotool type mengirim ke window yang fokus, jadi kalau aktivasi gagal,
# teks bisa masuk ke window lain (ini yang bikin hasil uji jadi palsu).
focus_win() {
  local w=$1 i
  for i in 1 2 3 4 5 6; do
    xdotool windowactivate --sync "$w" 2>/dev/null
    sleep 0.5
    if [ "$(xdotool getwindowfocus)" = "$w" ]; then return 0; fi
    sleep 0.7
  done
  echo "FAIL: tidak bisa fokus ke window $w"
  return 1
}

OUT=/tmp/e2e-ui
rm -rf $OUT; mkdir -p $OUT

pkill -9 -x lanchat-desktop 2>/dev/null
sleep 2
rm -rf /root/.lanchat-ray /root/.lanchat-iphone

LANCHAT_NAME=ray LANCHAT_PORT=42171 LANCHAT_DIR=/root/.lanchat-ray \
  setsid nohup "$BIN" >/tmp/app-ray.log 2>&1 &
LANCHAT_NAME=iphone LANCHAT_PORT=42172 LANCHAT_DIR=/root/.lanchat-iphone \
  setsid nohup "$BIN" >/tmp/app-iphone.log 2>&1 &

# tunggu window muncul
for i in $(seq 1 30); do
  RAY=$(xdotool search --name "LAN Chat — ray" 2>/dev/null | head -1)
  IPH=$(xdotool search --name "LAN Chat — iphone" 2>/dev/null | head -1)
  [ -n "${RAY:-}" ] && [ -n "${IPH:-}" ] && break
  sleep 1
done
echo "ray=$RAY iphone=$IPH"
[ -z "${RAY:-}" ] && { echo "FAIL: window ray tidak muncul"; exit 1; }

xdotool windowsize $RAY 900 600;  xdotool windowmove $RAY 0 30
xdotool windowsize $IPH 900 600;  xdotool windowmove $IPH 930 30
sleep 3
focus_win $RAY || exit 1
import -window $RAY $OUT/01-ray-boot.png

# --- klik peer iphone di sidebar ray ---
xdotool mousemove --window $RAY 120 205 click 1; sleep 2
import -window $RAY $OUT/02-ray-dm.png

# --- kirim pesan ---
xdotool mousemove --window $RAY 400 570 click 1; sleep 1
xdotool type --delay 45 'halo dari ray lewat app Tauri'; sleep 1
xdotool key Return; sleep 3
import -window $RAY $OUT/03-ray-sent.png

# --- balas dari iphone ---
focus_win $IPH || exit 1
# iphone sudah punya thread di PERCAKAPAN (y~151), coba itu dulu
xdotool mousemove --window $IPH 120 151 click 1; sleep 1.5
xdotool mousemove --window $IPH 120 205 click 1; sleep 1.5
import -window $IPH $OUT/04-iphone-dm.png
xdotool mousemove --window $IPH 400 570 click 1; sleep 1
xdotool type --delay 45 'diterima, ini balasan iphone'; sleep 1
xdotool key Return; sleep 3
import -window $IPH $OUT/05-iphone-sent.png

focus_win $RAY || exit 1
import -window $RAY $OUT/06-ray-final.png

# --- zoom test ---
focus_win $RAY || exit 1
xdotool key --clearmodifiers --window $RAY ctrl+plus; sleep 0.6
xdotool key --clearmodifiers --window $RAY ctrl+plus; sleep 1.5
import -window $RAY $OUT/07-ray-zoom.png

# --- tema dark ---
xdotool key --clearmodifiers ctrl+0; sleep 1
xdotool mousemove --window $RAY 265 20 click 1; sleep 1.5
import -window $RAY $OUT/08-settings.png
# pilih tema dark di dropdown lalu tutup
xdotool mousemove --window $RAY 560 268 click 1; sleep 1
xdotool key Down Return; sleep 1.5
xdotool mousemove --window $RAY 620 430 click 1; sleep 1.5
import -window $RAY $OUT/09-ray-dark.png

echo "=== DB ray ==="
python3 - <<'PY'
import sqlite3
for d in ("ray","iphone"):
    try:
        c=sqlite3.connect(f"/root/.lanchat-{d}/lanchat.db", timeout=3)
        print(d, "threads:", c.execute("select id,kind,name,unread from threads").fetchall())
        print(d, "msgs:", c.execute("select from_name,body,mine,delivered from messages order by ts").fetchall())
        print(d, "scale:", c.execute("select v from settings where k='scale'").fetchall())
    except Exception as e:
        print(d, "ERR", e)
PY
echo "DONE"