#!/bin/sh
set -eu
umask 077
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
ENV_FILE=${QUOTA_ENV_FILE:-deploy/.env}
case "${1:-}" in codex|claude) provider=$1 ;; *) printf '%s\n' 'Usage: scripts/auth.sh codex|claude [login|status]' >&2; exit 2 ;; esac
operation=${2:-login}
case "$operation" in login|status) ;; *) printf '%s\n' 'Unsupported auth operation.' >&2; exit 2 ;; esac
[ "$#" -le 2 ] || exit 2
[ -f "$ENV_FILE" ] || { printf '%s\n' 'Run scripts/setup.sh first.' >&2; exit 1; }
compose() { docker compose --env-file "$ENV_FILE" -f deploy/compose.yaml "$@"; }
# The lock prevents concurrent manual CLI writers. Server stops before any CLI reads or refreshes.
mkdir deploy/.auth-lock 2>/dev/null || { printf '%s\n' 'Another auth command holds deploy/.auth-lock.' >&2; exit 1; }
was_running=false
completed=false
cleanup() {
  rmdir deploy/.auth-lock
  if [ "$was_running" = true ] && [ "$completed" != true ]; then
    printf '%s\n' 'Auth failed or was interrupted. Live server remains stopped; resolve auth before restarting.' >&2
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if [ -n "$(compose --profile live ps --status running -q quota-server)" ]; then
  was_running=true
fi
compose --profile live stop quota-server
compose --profile auth run --rm auth "$provider-$operation"
completed=true
if [ "$was_running" = true ]; then
  compose --profile live up -d quota-server
fi
