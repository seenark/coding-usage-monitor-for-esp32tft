#!/bin/sh
set -eu
umask 077
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
ENV_FILE=${QUOTA_ENV_FILE:-deploy/.env}
[ -f "$ENV_FILE" ] || { printf '%s\n' 'Run scripts/setup.sh first.' >&2; exit 1; }
# Authentication comes from Mosquitto's protected config file, never argv/environment.
if [ "$#" -eq 0 ]; then
  exec docker compose --env-file "$ENV_FILE" -f deploy/compose.yaml --profile tools run --rm --no-deps subscriber
fi
[ "$#" -eq 2 ] || { printf '%s\n' 'Usage: scripts/subscriber.sh [message-count timeout-seconds]' >&2; exit 2; }
case "$1:$2" in *[!0-9:]*|:*|*:) exit 2 ;; esac
[ "$1" -gt 0 ] && [ "$2" -gt 0 ] || exit 2
exec docker compose --env-file "$ENV_FILE" -f deploy/compose.yaml --profile tools run --rm --no-deps subscriber -h mosquitto -p 1883 -t 'ai-quota/v1/#' -v -C "$1" -W "$2"
