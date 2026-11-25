#!/usr/bin/env bash
# E2E file transfer lewat CLI: dua node nyata, satu file 700 KB, verifikasi sha256.
set -u
cd /root/lanchat || exit 1
BIN=./target/debug/lanchat
DIR=/tmp/ft-e2e
rm -rf $DIR; mkdir -p $DIR/a $DIR/b
mkfifo $DIR/a.in $DIR/b.in

# file uji 700 KB (5+ chunk, tidak pas kelipatan biar chunk terakhir parsial)
head -c 700000 /dev/urandom > $DIR/kirim.bin
SHA_SRC=$(sha256sum $DIR/kirim.bin | cut -d' ' -f1)
echo "sha sumber: $SHA_SRC"

$BIN --name pengirim --port 42201 --dir $DIR/a < $DIR/a.in > $DIR/a.log 2>&1 &
A=$!
exec 3> $DIR/a.in
$BIN --name penerima --port 42202 --dir $DIR/b < $DIR/b.in > $DIR/b.log 2>&1 &
B=$!
exec 4> $DIR/b.in

sleep 9
echo "=== kirim ==="
echo "/file penerima $DIR/kirim.bin" >&3
sleep 6
echo "/peers" >&3
sleep 2

kill $A $B 2>/dev/null
exec 3>&- 4>&-
wait $A $B 2>/dev/null

echo "=== log pengirim ==="; cat $DIR/a.log
echo "=== log penerima ==="; cat $DIR/b.log

echo "=== file yang diterima ==="
ls -la $DIR/b/files/ 2>/dev/null
GOT=$(ls $DIR/b/files/* 2>/dev/null | head -1)
if [ -n "$GOT" ]; then
  SHA_GOT=$(sha256sum "$GOT" | cut -d' ' -f1)
  echo "sha diterima: $SHA_GOT"
  if [ "$SHA_SRC" = "$SHA_GOT" ]; then echo "RESULT: SHA COCOK ✓"; else echo "RESULT: SHA BEDA ✗"; fi
else
  echo "RESULT: FILE TIDAK DITERIMA ✗"
fi
