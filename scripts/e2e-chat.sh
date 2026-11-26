#!/usr/bin/env bash
# Klik alur chat beneran di dua instance: A klik peer -> ketik -> Enter. Lalu cek DB.
set -u
export DISPLAY="${DISP:-:98}"
A=$(cut -d' ' -f1 /tmp/wids.txt)
B=$(cut -d' ' -f2 /tmp/wids.txt)
echo "A=$A B=$B"

dbq() { sqlite3 "$1" "$2"; }

echo "--- sebelum: A msgs=$(dbq /root/.dekat-ray/dekat.db 'select count(*) from messages;') B msgs=$(dbq /root/.dekat-iphone/dekat.db 'select count(*) from messages;')"

# 1) aktifkan window A lalu klik baris peer di sidebar
xdotool windowactivate "$A"; sleep 1
# cari baris peer: klik di area daftar 'DI JARINGAN INI' (100,195 relatif window)
xdotool mousemove --window "$A" 120 196 click 1; sleep 2
xwd -id "$A" -out /tmp/_a.xwd && convert /tmp/_a.xwd /tmp/i-01-a-open.png

# 2) klik kolom input di bawah panel kanan lalu ketik
xdotool mousemove --window "$A" 560 585 click 1; sleep 1
xdotool type --delay 30 "halo iphone, tes dari ray"; sleep 1
xdotool key Return; sleep 3
xwd -id "$A" -out /tmp/_a.xwd && convert /tmp/_a.xwd /tmp/i-02-a-sent.png
xwd -id "$B" -out /tmp/_b.xwd && convert /tmp/_b.xwd /tmp/i-02-b-recv.png

echo "--- sesudah A: $(dbq /root/.dekat-ray/dekat.db 'select thread_id, from_name, body from messages;')"
echo "--- sesudah B: $(dbq /root/.dekat-iphone/dekat.db 'select thread_id, from_name, body from messages;')"
echo "--- threads A: $(dbq /root/.dekat-ray/dekat.db 'select id,kind,name from threads;')"
echo "--- threads B: $(dbq /root/.dekat-iphone/dekat.db 'select id,kind,name from threads;')"
