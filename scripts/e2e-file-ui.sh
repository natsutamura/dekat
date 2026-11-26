#!/usr/bin/env bash
# E2E file transfer lewat GUI asli: klik tombol lampiran → dialog OS → ketik path → Enter.
# Bukti diambil dari SQLite kedua instance + sha256 file yang benar-benar diterima.
set -u
export DISPLAY=:11 WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
BIN=${BIN:-/root/dekat/target/debug/dekat-desktop}
OUT=/tmp/e2e-file-ui
rm -rf $OUT /root/.lc-f-ray /root/.lc-f-iphone; mkdir -p $OUT

pkill -9 -x dekat-desktop 2>/dev/null; sleep 2

head -c 900000 /dev/urandom > $OUT/foto-liburan.bin
SHA_SRC=$(sha256sum $OUT/foto-liburan.bin | cut -d' ' -f1)
echo "sha sumber: $SHA_SRC"

DEKAT_NAME=ray DEKAT_PORT=42301 DEKAT_DIR=/root/.lc-f-ray setsid nohup "$BIN" >/tmp/lc-f-ray.log 2>&1 &
DEKAT_NAME=iphone DEKAT_PORT=42302 DEKAT_DIR=/root/.lc-f-iphone setsid nohup "$BIN" >/tmp/lc-f-iphone.log 2>&1 &

for i in $(seq 1 30); do
  RAY=$(xdotool search --name "Dekat — ray" 2>/dev/null | head -1)
  [ -n "${RAY:-}" ] && break
  sleep 1
done
echo "win ray=$RAY"
xdotool windowsize $RAY 900 600; xdotool windowmove $RAY 0 30; sleep 3
xdotool windowactivate --sync $RAY; sleep 2

# buka DM ke iphone
xdotool mousemove --window $RAY 120 205 click 1; sleep 2
xdotool windowactivate --sync $RAY; sleep 1
import -window $RAY $OUT/01-dm.png

# klik tombol lampiran (+ kiri bawah)
# tombol "+" ada di panel chat (bukan sidebar): sidebar 300px, jadi x≈315, chatbar y≈570
xdotool mousemove --window $RAY 315 572 click 1; sleep 3
import -window root $OUT/02-dialog.png

# dialog GTK: Ctrl+L lalu ketik path, Enter dua kali
xdotool key --clearmodifiers ctrl+l; sleep 1
xdotool type --delay 30 "$OUT/foto-liburan.bin"; sleep 1
xdotool key --clearmodifiers Return; sleep 4
import -window $RAY $OUT/03-after-send.png

sleep 3
echo "=== DB ray (pengirim) ==="
python3 -c "
import sqlite3
c=sqlite3.connect('/root/.lc-f-ray/dekat.db',timeout=5)
print('threads:', c.execute('select id,kind,name from threads').fetchall())
print('msgs:', c.execute('select id,from_name,mine,body from messages').fetchall())
print('att:', c.execute('select msg_id,file_name,file_size,file_path,sha256 from attachments').fetchall())
"
echo "=== DB iphone (penerima) ==="
python3 -c "
import sqlite3
c=sqlite3.connect('/root/.lc-f-iphone/dekat.db',timeout=5)
print('threads:', c.execute('select id,kind,name,preview from threads').fetchall())
print('msgs:', c.execute('select id,from_name,mine,body from messages').fetchall())
print('att:', c.execute('select msg_id,file_name,file_size,file_path,sha256 from attachments').fetchall())
"

echo "=== file yang diterima di disk ==="
ls -la /root/.lc-f-iphone/files/ 2>/dev/null
GOT=$(ls /root/.lc-f-iphone/files/* 2>/dev/null | head -1)
if [ -n "$GOT" ]; then
  SHA_GOT=$(sha256sum "$GOT" | cut -d' ' -f1)
  echo "sha diterima: $SHA_GOT"
  [ "$SHA_SRC" = "$SHA_GOT" ] && echo "RESULT: SHA COCOK ✓" || echo "RESULT: SHA BEDA ✗"
else
  echo "RESULT: FILE TIDAK SAMPAI ✗"
fi
import -window $RAY $OUT/04-final.png
echo DONE