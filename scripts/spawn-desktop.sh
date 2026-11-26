#!/usr/bin/env bash
# Jalankan satu instance desktop di DISPLAY tertentu (foreground-safe: dipanggil dari terminal bg).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
export DISPLAY="${DISPLAY:-:99}"
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export LIBGL_ALWAYS_SOFTWARE=1
cd /root/dekat || exit 1
nohup env DEKAT_NAME="${NAME}" DEKAT_PORT="${PORT}" DEKAT_DIR="${DIR}" \
  ./target/debug/dekat-desktop >"${LOG}" 2>&1 &
echo "pid=$!"
