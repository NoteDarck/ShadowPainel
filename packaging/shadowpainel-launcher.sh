#!/bin/sh
set -eu

CORE="/usr/lib/shadowpainel/shadowpainel-core"
export SHADOWPAINEL_FRONTEND_DIR="${SHADOWPAINEL_FRONTEND_DIR:-/usr/share/shadowpainel/frontend}"
LOG="${XDG_CACHE_HOME:-$HOME/.cache}/shadowpainel.log"
mkdir -p "$(dirname "$LOG")"
: > "$LOG"

"$CORE" "$@" >>"$LOG" 2>&1 &
PID=$!
URL=""
for _ in $(seq 1 50); do
  URL=$(sed -n 's/.*ShadowPainel disponível em //p' "$LOG" | tail -n 1 || true)
  [ -n "$URL" ] && break
  sleep 0.1
done

if [ -n "$URL" ] && command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$URL" >/dev/null 2>&1 || true
fi

wait "$PID"
