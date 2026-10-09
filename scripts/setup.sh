#!/bin/sh
set -eu
# Never enable shell tracing: runtime files contain local passwords.
umask 077
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
ENV_FILE=${QUOTA_ENV_FILE:-deploy/.env}
command -v python3 >/dev/null 2>&1 || { printf '%s\n' 'Python 3 is required.' >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { printf '%s\n' 'Docker Compose v2 is required.' >&2; exit 1; }
mkdir deploy/.setup-lock 2>/dev/null || { printf '%s\n' 'Another setup holds deploy/.setup-lock.' >&2; exit 1; }
cleanup() {
  python3 deploy/setup.py abort >/dev/null 2>&1 || :
  rmdir deploy/.setup-lock
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
python3 deploy/setup.py prepare
docker compose --env-file "$ENV_FILE" -f deploy/compose.yaml --profile tools run --rm --no-deps passwords -U /auth/mosquitto-passwords.new
python3 deploy/setup.py finish
printf '%s\n' 'Setup complete. Passwords stay in protected local files. See docs/OPERATIONS.md.'
