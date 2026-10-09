#!/usr/bin/env python3
"""Prepare local runtime files without printing passwords or reading provider auth."""
import os
from pathlib import Path
import re
import secrets
import sys
import tempfile

DEPLOY = Path(__file__).resolve().parent

def environment_file():
    return Path(os.environ.get("QUOTA_ENV_FILE", str(DEPLOY / ".env"))).absolute()


def settings():
    path = environment_file()
    if not path.exists():
        template = (DEPLOY / ".env.example").read_text()
        uid = int(os.environ.get("HOST_UID", os.getuid()))
        gid = int(os.environ.get("HOST_GID", os.getgid()))
        template = template.replace("HOST_UID=1000", f"HOST_UID={uid}")
        template = template.replace("HOST_GID=1000", f"HOST_GID={gid}")
        atomic_write(path, template, os.getuid(), os.getgid())
    if path.is_symlink() or not path.is_file():
        raise ValueError("deploy/.env must be a regular file")
    values = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        value = value.strip()
        if not separator or not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            raise ValueError("Use plain KEY=value lines in deploy/.env")
        # Compose also expands these forms; refusing them prevents path disagreement.
        if any(character in value for character in "$\"'`#\r\n"):
            raise ValueError("Use literal unquoted values in deploy/.env")
        values[key] = value
    for key in ("HOST_UID", "HOST_GID", "STATE_DIR", "MQTT_BIND_ADDRESS", "ADMIN_BIND_ADDRESS"):
        if key in os.environ and os.environ[key] != values.get(key):
            raise ValueError("Shell environment conflicts with deploy/.env")
    uid, gid = int(values["HOST_UID"]), int(values["HOST_GID"])
    if uid <= 0 or gid <= 0:
        raise ValueError("HOST_UID and HOST_GID must both be non-root IDs")
    if os.geteuid() != 0 and (uid != os.geteuid() or gid not in os.getgroups() + [os.getegid()]):
        raise ValueError("Configured IDs need matching host ownership; run setup with suitable permissions")
    for key in ("MQTT_BIND_ADDRESS", "ADMIN_BIND_ADDRESS"):
        bind_address(key, values.get(key, "127.0.0.1"))
    port = values.get("ADMIN_PORT", "8080")
    if not port.isascii() or not port.isdigit() or not 0 < int(port) < 65536:
        raise ValueError("ADMIN_PORT must be a TCP port")
    raw = Path(values.get("STATE_DIR", "./state"))
    state = raw if raw.is_absolute() else DEPLOY / raw
    if any(part.is_symlink() for part in [state, *state.parents]):
        raise ValueError("STATE_DIR and its parent directories must not be symlinks")
    state = state.resolve()
    if state in (Path(state.anchor), DEPLOY, DEPLOY.parent):
        raise ValueError("STATE_DIR must be a dedicated directory")
    return state, uid, gid


def bind_address(key, bind):
    """Both listeners are unauthenticated at the app layer: loopback or private IPv4 only."""
    import ipaddress
    address = ipaddress.ip_address(bind)
    if address.is_unspecified or address.is_multicast or (not address.is_loopback and not address.is_private):
        raise ValueError(f"{key} must be an explicit private LAN or loopback address")
    # Compose's short port syntax is deliberately IPv4-only here.
    if address.version != 4:
        raise ValueError(f"{key} must be an IPv4 LAN or loopback address")


def secure_directory(path, uid, gid):
    if path.is_symlink():
        raise ValueError("Runtime directories must not be symlinks")
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not path.is_dir():
        raise ValueError("Runtime paths must be directories")
    os.chown(path, uid, gid)
    path.chmod(0o700)


def atomic_write(path, text, uid, gid):
    if path.is_symlink():
        raise ValueError("Runtime files must not be symlinks")
    descriptor, temporary = tempfile.mkstemp(prefix=".setup-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            os.fchmod(stream.fileno(), 0o600)
            os.fchown(stream.fileno(), uid, gid)
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


# The server account keeps a 256-bit secret. The device account reads quota/config
# topics and writes only its own status on the LAN, and is typed on a phone keypad, so it uses 12 random digits.
SERVER_PASSWORD = (r"[0-9a-f]{64}", lambda: secrets.token_hex(32))
DEVICE_PASSWORD = (r"[0-9]{12}", lambda: f"{secrets.randbelow(10**12):012d}")


def mqtt_password(path, uid, gid, kind):
    pattern, generate = kind
    if path.exists():
        if path.is_symlink() or not path.is_file():
            raise ValueError("Password files must be regular files")
        metadata = path.stat()
        if metadata.st_uid != uid or metadata.st_gid != gid or metadata.st_mode & 0o777 != 0o600:
            raise ValueError("Existing password files need the configured owner/group and mode 0600")
        value = path.read_text().strip()
        if not re.fullmatch(pattern, value):
            raise ValueError(f"{path.name} does not contain a setup-generated password; delete it and rerun setup")
        return value
    value = generate()
    atomic_write(path, value + "\n", uid, gid)
    return value


def prepare():
    state, uid, gid = settings()
    for suffix in ("", "secrets", "broker", "cache", "mock-cache", "auth", "auth/home", "auth/codex", "auth/claude"):
        secure_directory(state / suffix, uid, gid)
    secret_dir = state / "secrets"
    server = mqtt_password(secret_dir / "mqtt-server-password", uid, gid, SERVER_PASSWORD)
    device = mqtt_password(secret_dir / "mqtt-device-password", uid, gid, DEVICE_PASSWORD)
    atomic_write(secret_dir / "mosquitto-passwords.new", f"quota-server:{server}\nesp32:{device}\n", uid, gid)
    atomic_write(secret_dir / "subscriber-config", f"-u esp32\n-P {device}\n", uid, gid)
    config = state / "auth/codex/config.toml"
    if not config.exists():
        atomic_write(config, 'cli_auth_credentials_store = "file"\nforced_login_method = "chatgpt"\n', uid, gid)
    claude_settings = state / "auth/claude/settings.json"
    if not claude_settings.exists():
        atomic_write(claude_settings, '{"forceLoginMethod":"claudeai"}\n', uid, gid)


def finish():
    state, uid, gid = settings()
    source = state / "secrets/mosquitto-passwords.new"
    if source.is_symlink() or not source.is_file():
        raise ValueError("Missing protected converted password file")
    lines = source.read_text().splitlines()
    if len(lines) != 2 or any(not re.fullmatch(r"(quota-server|esp32):\$[67]\$.+", line) for line in lines):
        raise ValueError("mosquitto_passwd did not produce password hashes")
    os.chown(source, uid, gid)
    source.chmod(0o600)
    os.replace(source, state / "secrets/mosquitto-passwords")


def abort():
    if environment_file().is_file():
        state, _, _ = settings()
        source = state / "secrets/mosquitto-passwords.new"
        if source.is_file() and not source.is_symlink():
            source.unlink()


if __name__ == "__main__":
    os.umask(0o077)
    try:
        {"prepare": prepare, "finish": finish, "abort": abort}[sys.argv[1]]()
    except (OSError, ValueError, KeyError, IndexError):
        # Never echo exception text from a secret-bearing file operation.
        print("Setup failed: check non-root IDs, literal paths, file ownership, and protected runtime files.", file=sys.stderr)
        sys.exit(1)
