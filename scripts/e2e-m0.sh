#!/usr/bin/env bash
# E2E M0: dua instance dekat di satu host, cek discovery + chat 2 arah.
set -u
cd /root/dekat || exit 1
BIN=./target/debug/dekat
rm -rf /tmp/lc-a /tmp/lc-b /tmp/a.in /tmp/b.in /tmp/a.log /tmp/b.log
mkfifo /tmp/a.in /tmp/b.in

$BIN --name ray --port 42001 --dir /tmp/lc-a </tmp/a.in >/tmp/a.log 2>&1 &
APID=$!
exec 3>/tmp/a.in

$BIN --name iphone --port 42002 --dir /tmp/lc-b </tmp/b.in >/tmp/b.log 2>&1 &
BPID=$!
exec 4>/tmp/b.in

sleep 9
echo "halo semua dari ray" >&3
sleep 2
echo "/dm iphone dm khusus buat iphone" >&3
sleep 2
echo "balasan dari iphone" >&4
sleep 2
echo "/peers" >&4

sleep 1
kill $APID $BPID 2>/dev/null
exec 3>&- 4>&-
wait $APID $BPID 2>/dev/null

echo "================ A (ray :42001) ================"
cat /tmp/a.log
echo "================ B (iphone :42002) =============="
cat /tmp/b.log
