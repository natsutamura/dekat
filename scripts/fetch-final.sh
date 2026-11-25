#!/usr/bin/env bash
# Unduh artefak build 0f9ea30 lalu terbitkan ke release v0.2.0.
set -u
TOK=$(cat /tmp/tok.txt)
WIN=$(sed -n '1p' /tmp/urls2.txt)
LX=$(sed -n '2p' /tmp/urls2.txt)
curl -sL -H "Authorization: token $TOK" -o /tmp/wf.zip "$WIN"
echo "WIN=$(stat -c%s /tmp/wf.zip 2>/dev/null || echo 0)"
curl -sL -H "Authorization: token $TOK" -o /tmp/lf.zip "$LX"
echo "LX=$(stat -c%s /tmp/lf.zip 2>/dev/null || echo 0)"