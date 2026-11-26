#!/usr/bin/env bash
# E2E dua arah yang deterministik: window dicari lewat judul "Dekat — <nama>".
set -u
export PATH="$HOME/.cargo/bin:$PATH"
DISP="${DISP:-:98}"
export DISPLAY="$DISP"
cd /root/dekat || exit 1
pkill -f 'target/debug/dekat-desktop' 2>/dev/null
for _ in $(seq 1 20); do pgrep -f 'target/debug/dekat-desktop' >/dev/null || break; sleep 0.3; done
sleep 1
rm -rf /root/.dekat-ray /root/.dekat-iphone

spawn() {
  nohup env DEKAT_NAME="$1" DEKAT_PORT="$2" DEKAT_DIR="$3" \
    DISPLAY="$DISP" WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 \
    ./target/debug/dekat-desktop >"$4" 2>&1 &
}
spawn ray 42101 /root/.dekat-ray /tmp/app-ray.log
spawn iphone 42102 /root/.dekat-iphone /tmp/app-iphone.log
sleep 10

wid() { xdotool search --name "Dekat — $1" | head -1; }
RAY=$(wid ray); IPH=$(wid iphone)
echo "ray_win=${RAY:-none} iphone_win=${IPH:-none}"
[ -n "$RAY" ] && [ -n "$IPH" ] || { echo "GAGAL: window tidak ketemu"; exit 1; }
xdotool windowsize $RAY 900 620;  xdotool windowmove $RAY 10 40
xdotool windowsize $IPH 900 620;  xdotool windowmove $IPH 930 40
sleep 3
echo "$RAY $IPH" > /tmp/wids.txt

shot() { xwd -id "$1" -out /tmp/_s.xwd && convert /tmp/_s.xwd "$2"; }
shot $RAY /tmp/x-ray-1.png; shot $IPH /tmp/x-iphone-1.png

# klik baris peer pertama di sidebar masing-masing (bagian "DI JARINGAN INI")
klik_peer() { xdotool windowactivate "$1"; sleep 1; xdotool mousemove --window "$1" 120 196 click 1; sleep 2; }
klik_peer $RAY
shot $RAY /tmp/x-ray-2-open.png
klik_peer $IPH
shot $IPH /tmp/x-iphone-2-open.png

ketik() { # win teks
  xdotool windowactivate "$1"; sleep 1
  xdotool mousemove --window "$1" 560 585 click 1; sleep 1
  xdotool type --delay 25 "$2"; sleep 1
  xdotool key Return; sleep 3
}
ketik $RAY "halo iphone, ini ray"
ketik $IPH "halo ray, ini iphone"
sleep 2
shot $RAY /tmp/x-ray-3-chat.png; shot $IPH /tmp/x-iphone-3-chat.png

for d in /root/.dekat-ray /root/.dekat-iphone; do
  echo "--- $d"
  sqlite3 -separator ' | ' "$d/dekat.db" "select from_name, mine, delivered, body from messages order by ts;"
done
