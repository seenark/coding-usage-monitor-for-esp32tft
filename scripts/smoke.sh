#!/bin/sh
# Offline provider acceptance only. Requires Docker Compose v2, Rust/Cargo and Python 3.
# Runs ignored MQTT tests against an owned temporary broker; never use live accounts.
# Test environment: MQTT_TEST_HOST, MQTT_TEST_PORT, MQTT_TEST_SERVER_PASSWORD_FILE,
# MQTT_TEST_DEVICE_PASSWORD_FILE, MQTT_TEST_SERVER_USERNAME, MQTT_TEST_DEVICE_USERNAME,
# MQTT_TEST_RESTART_DIR. This script supplies all of them with generated local secrets.
set -eu
# Never enable shell tracing: setup generates protected local MQTT passwords.
umask 077
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
for program in docker cargo python3; do
  command -v "$program" >/dev/null 2>&1 || { printf '%s\n' "Required command unavailable: $program" >&2; exit 1; }
done
docker compose version >/dev/null
[ "$(id -u)" -ne 0 ] && [ "$(id -g)" -ne 0 ] || {
  printf '%s\n' 'Smoke requires non-root host UID and GID.' >&2
  exit 1
}
# Resolve macOS /var and /tmp symlinks before the setup ownership/path checks.
TEMP=$(python3 -c 'import pathlib,tempfile; print(pathlib.Path(tempfile.mkdtemp(prefix="ai-quota-smoke-")).resolve())')
export COMPOSE_PROJECT_NAME
COMPOSE_PROJECT_NAME=$(python3 -c 'import uuid; print("ai-quota-smoke-" + uuid.uuid4().hex)')
export QUOTA_ENV_FILE="$TEMP/test.env"
export STATE_DIR="$TEMP/state"
export HOST_UID="$(id -u)" HOST_GID="$(id -g)"
export MQTT_BIND_ADDRESS=127.0.0.1
export MQTT_HOST_PORT
MQTT_HOST_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
# MAX_BACKOFF_SECONDS must be >= every refresh interval.
export CODEX_ENABLED=true CLAUDE_PRIVATE_USAGE_ENABLED=false
export CLAUDE_POLL_INTERVAL_SECONDS=3600 CODEX_POLL_INTERVAL_SECONDS=3600
export REQUEST_TIMEOUT_SECONDS=12 STALE_AFTER_SECONDS=900 MAX_BACKOFF_SECONDS=3600
# The Admin app port is published by every server profile; never collide with a running install.
export ADMIN_BIND_ADDRESS=127.0.0.1
export ADMIN_PORT
ADMIN_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
TEST_PID=
WATCH_PID=
COMPOSE_STARTED=false
compose() {
  docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$QUOTA_ENV_FILE" -f deploy/compose.yaml --profile mock --profile tools "$@"
}
cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [ -n "$TEST_PID" ]; then kill "$TEST_PID" 2>/dev/null || :; wait "$TEST_PID" 2>/dev/null || :; fi
  if [ -n "$WATCH_PID" ]; then kill "$WATCH_PID" 2>/dev/null || :; wait "$WATCH_PID" 2>/dev/null || :; fi
  # No -v, prune, user volume deletion, or changes to deploy/.env.
  if [ "$COMPOSE_STARTED" = true ]; then
    if ! compose down --remove-orphans --timeout 10 >/dev/null 2>&1; then
      printf '%s\n' "Owned Compose cleanup failed; temporary state remains at $TEMP" >&2
      exit 1
    fi
  fi
  rm -rf -- "$TEMP"
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
python3 - <<'PY'
import os
from pathlib import Path
keys = (
    "HOST_UID", "HOST_GID", "STATE_DIR", "MQTT_BIND_ADDRESS", "MQTT_HOST_PORT",
    "ADMIN_BIND_ADDRESS", "ADMIN_PORT",
    "CODEX_ENABLED", "CLAUDE_PRIVATE_USAGE_ENABLED", "CLAUDE_POLL_INTERVAL_SECONDS",
    "CODEX_POLL_INTERVAL_SECONDS", "REQUEST_TIMEOUT_SECONDS", "STALE_AFTER_SECONDS",
    "MAX_BACKOFF_SECONDS",
)
Path(os.environ["QUOTA_ENV_FILE"]).write_text("".join(f"{key}={os.environ[key]}\n" for key in keys))
PY
COMPOSE_STARTED=true
./scripts/setup.sh
# Only the mock profile starts. No auth helper or provider collector runs.
compose up --build -d mosquitto quota-server-mock

# Bounded authenticated subscriptions check actual Compose binary data, not just TCP readiness.
wait_state() {
  topic=$1
  destination=$2
  state=$3
  attempts=0
  while :; do
    if compose run --rm --no-deps subscriber -h mosquitto -p 1883 -t "$topic" -C 1 -W 10 > "$destination"; then
      if python3 - "$destination" "$state" <<'PY'; then
import json
from pathlib import Path
import sys
try:
    valid = json.loads(Path(sys.argv[1]).read_text()).get("state") == sys.argv[2]
except (ValueError, OSError):
    valid = False
sys.exit(0 if valid else 1)
PY
        return 0
      fi
    fi
    attempts=$((attempts + 1))
    [ "$attempts" -lt 6 ] || { printf '%s\n' "Timed out waiting for $topic state $state." >&2; return 1; }
    sleep 1
  done
}
wait_state ai-quota/v1/server/status "$TEMP/online.json" online
wait_state ai-quota/v1/providers/codex "$TEMP/codex.json" ok
wait_state ai-quota/v1/providers/claude "$TEMP/claude.json" ok
python3 - "$TEMP" <<'PY'
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
assert json.loads((root / "online.json").read_text()) == {"state": "online"}
for provider in ("codex", "claude"):
    snapshot = json.loads((root / f"{provider}.json").read_text())
    assert snapshot["provider"] == provider and snapshot["source"] == "mock"
    assert snapshot["state"] == "ok" and snapshot["fetched_at"] is not None
    assert snapshot["windows"], "Mock provider produced no quota windows"
print("Authenticated Compose mock subscriptions passed.")
PY
# Abrupt process death must publish LWT, not a graceful explicit offline message.
compose kill -s SIGKILL quota-server-mock
# Disable the mock container's restart policy so it cannot overwrite retained test data.
compose stop --timeout 5 quota-server-mock
wait_state ai-quota/v1/server/status "$TEMP/offline.json" offline
python3 - "$TEMP/offline.json" <<'PY'
import json
from pathlib import Path
import sys
assert json.loads(Path(sys.argv[1]).read_text()) == {"state": "offline"}
print("Compose server last will passed.")
PY

export MQTT_TEST_HOST=127.0.0.1 MQTT_TEST_PORT="$MQTT_HOST_PORT"
export MQTT_TEST_SERVER_USERNAME=quota-server MQTT_TEST_DEVICE_USERNAME=esp32
export MQTT_TEST_SERVER_PASSWORD_FILE="$STATE_DIR/secrets/mqtt-server-password"
export MQTT_TEST_DEVICE_PASSWORD_FILE="$STATE_DIR/secrets/mqtt-device-password"
export MQTT_TEST_RESTART_DIR="$TEMP/restart"
mkdir "$MQTT_TEST_RESTART_DIR"
# Compile before starting the bounded broker orchestration clock.
cargo test -p quota-server --test mqtt_integration --no-run

wait_marker() {
  marker=$1
  attempts=0
  while [ ! -f "$MQTT_TEST_RESTART_DIR/$marker" ]; do
    [ ! -f "$TEMP/tests-finished" ] || { printf '%s\n' "Tests ended before $marker." >&2; return 1; }
    attempts=$((attempts + 1))
    [ "$attempts" -lt 120 ] || { printf '%s\n' "Timed out waiting for $marker." >&2; return 1; }
    sleep 1
  done
}
restart_broker() {
  wait_marker persistence-ready
  compose stop --timeout 10 mosquitto
  compose start mosquitto
  : > "$MQTT_TEST_RESTART_DIR/persistence-restarted"
  wait_marker reconnect-ready
  compose stop --timeout 10 mosquitto
  : > "$MQTT_TEST_RESTART_DIR/broker-stopped"
  wait_marker publication-queued
  compose start mosquitto
  : > "$MQTT_TEST_RESTART_DIR/reconnect-restarted"
}
restart_broker &
WATCH_PID=$!
python3 - "$TEMP" <<'PY' &
import os
from pathlib import Path
import signal
import subprocess
import sys

process = subprocess.Popen(
    ["cargo", "test", "-p", "quota-server", "--test", "mqtt_integration",
     "--", "--ignored", "--test-threads=1"],
    start_new_session=True,
)

def terminate(signum, frame):
    del frame
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()
    sys.exit(128 + signum)

signal.signal(signal.SIGTERM, terminate)
signal.signal(signal.SIGINT, terminate)
try:
    result = process.wait(timeout=240)
except subprocess.TimeoutExpired:
    print("MQTT tests exceeded the 240-second smoke deadline.", file=sys.stderr)
    terminate(signal.SIGTERM, None)
finally:
    (Path(sys.argv[1]) / "tests-finished").touch()
sys.exit(result)
PY
TEST_PID=$!
status=0
wait "$TEST_PID" || status=$?
TEST_PID=
if [ "$status" -ne 0 ]; then exit "$status"; fi
wait "$WATCH_PID"
WATCH_PID=
printf '%s\n' 'MQTT smoke passed: mock binary, retained/reconnect, duplicate quota, ACL, provider isolation, LWT and broker persistence.'
printf '%s\n' 'Only this temporary Compose project and generated temporary files are removed. No provider credentials or user volumes are accessed.'
