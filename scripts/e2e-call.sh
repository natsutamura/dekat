#!/usr/bin/env bash
# E2E signalling voice call lewat CLI: offer → answer → ICE → end.
set -u
cd /root/lanchat || exit 1
BIN=./target/debug/lanchat
DIR=/tmp/call-e2e
rm -rf $DIR; mkdir -p $DIR/a $DIR/b
mkfifo $DIR/a.in $DIR/b.in

$BIN --name pemanggil --port 42401 --dir $DIR/a < $DIR/a.in > $DIR/a.log 2>&1 &
A=$!
exec 3> $DIR/a.in
$BIN --name penerima --port 42402 --dir $DIR/b < $DIR/b.in > $DIR/b.log 2>&1 &
B=$!
exec 4> $DIR/b.in

sleep 9
echo "=== panggil ==="
echo "/call penerima" >&3
sleep 3
echo "=== jawab (terima) ==="
echo "/answer pemanggil auto accept" >&4
sleep 3
echo "=== tutup ==="
echo "/hangup penerima auto" >&3
sleep 2

kill $A $B 2>/dev/null
exec 3>&- 4>&-
wait $A $B 2>/dev/null

echo "=== log pemanggil ==="; cat $DIR/a.log
echo "=== log penerima ==="; cat $DIR/b.log