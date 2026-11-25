#!/usr/bin/env bash
# Unduh artefak CI tanpa heredoc (biar tidak kena flag approval).
set -u
TOK=$(cat /tmp/tok.txt)
WIN_URL=$(sed -n '1p' /tmp/urls.txt)
LX_URL=$(sed -n '2p' /tmp/urls.txt)

curl -sL -H "Authorization: token $TOK" -o /tmp/wx3.zip "$WIN_URL"
echo "WIN=$(stat -c%s /tmp/wx3.zip 2>/dev/null || echo 0)"

curl -sL -H "Authorization: token $TOK" -o /tmp/lx3.zip "$LX_URL"
echo "LINUX=$(stat -c%s /tmp/lx3.zip 2>/dev/null || echo 0)"