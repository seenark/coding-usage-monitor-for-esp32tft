"""Offline checks use only newly generated synthetic files in a temporary directory."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("quota_deployment_setup", Path(__file__).with_name("setup.py"))
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)

status_spec = importlib.util.spec_from_file_location("quota_claude_status", Path(__file__).with_name("check-claude-status.py"))
status_check = importlib.util.module_from_spec(status_spec)
status_spec.loader.exec_module(status_check)


class SecureSetupTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.uid = os.getuid() or 12345
        self.gid = os.getgid() or 12345
        self.environment = self.root / "test.env"
        self.environment.write_text(
            f"HOST_UID={self.uid}\nHOST_GID={self.gid}\nSTATE_DIR={self.root / 'state'}\nMQTT_BIND_ADDRESS=127.0.0.1\n"
        )
        self.environment.chmod(0o600)
        self.patch = patch.dict(os.environ, {"QUOTA_ENV_FILE": str(self.environment)}, clear=True)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def test_permissions_preservation_and_hash_installation(self):
        setup.prepare()
        state = self.root / "state"
        first = (state / "secrets/mqtt-server-password").read_bytes()
        setup.prepare()
        self.assertEqual(first, (state / "secrets/mqtt-server-password").read_bytes())
        self.assertRegex(first.decode().strip(), r"^[0-9a-f]{64}$")
        # The device password is typed on a phone keypad.
        self.assertRegex((state / "secrets/mqtt-device-password").read_text().strip(), r"^[0-9]{12}$")
        for path in state.rglob("*"):
            self.assertEqual(path.stat().st_uid, self.uid)
            self.assertEqual(path.stat().st_gid, self.gid)
            self.assertEqual(path.stat().st_mode & 0o777, 0o700 if path.is_dir() else 0o600)
        self.assertIn('-u esp32\n', (state / "secrets/subscriber-config").read_text())
        # Synthetic hashes test atomic install only; Docker's real converter is smoke-tested separately.
        (state / "secrets/mosquitto-passwords.new").write_text("quota-server:$7$synthetic\nesp32:$7$synthetic\n")
        setup.finish()
        self.assertTrue((state / "secrets/mosquitto-passwords").is_file())
        self.assertFalse((state / "secrets/mosquitto-passwords.new").exists())

    def test_symlink_and_public_listener_rejected(self):
        setup.prepare()
        password = self.root / "state/secrets/mqtt-server-password"
        password.unlink()
        password.symlink_to(self.environment)
        with self.assertRaises(ValueError):
            setup.prepare()
        password.unlink()
        self.environment.write_text(self.environment.read_text().replace("127.0.0.1", "0.0.0.0"))
        with self.assertRaises(ValueError):
            setup.prepare()

    def test_admin_listener_must_be_private_with_valid_port(self):
        original = self.environment.read_text()
        for line in ("ADMIN_BIND_ADDRESS=0.0.0.0", "ADMIN_BIND_ADDRESS=8.8.8.8", "ADMIN_BIND_ADDRESS=::1",
                     "ADMIN_PORT=0", "ADMIN_PORT=70000", "ADMIN_PORT=http"):
            self.environment.write_text(original + line + "\n")
            with self.assertRaises(ValueError, msg=line):
                setup.prepare()
        self.environment.write_text(original + "ADMIN_BIND_ADDRESS=192.168.1.112\nADMIN_PORT=8081\n")
        setup.prepare()
        # A shell value that disagrees with the file is refused, as for MQTT_BIND_ADDRESS.
        with patch.dict(os.environ, {"ADMIN_BIND_ADDRESS": "127.0.0.1"}):
            with self.assertRaises(ValueError):
                setup.prepare()

    def test_example_environment_is_valid_and_has_no_removed_variable(self):
        example = (Path(setup.__file__).parent / ".env.example").read_text()
        self.assertNotRegex(example, r"(?m)^POLL_INTERVAL_SECONDS=")
        for key in ("CLAUDE_POLL_INTERVAL_SECONDS=600", "CODEX_POLL_INTERVAL_SECONDS=300",
                    "ADMIN_BIND_ADDRESS=127.0.0.1", "ADMIN_PORT=8080"):
            self.assertIn(key, example)
        self.environment.write_text(example.replace("HOST_UID=1000", f"HOST_UID={self.uid}")
                                    .replace("HOST_GID=1000", f"HOST_GID={self.gid}")
                                    .replace("STATE_DIR=./state", f"STATE_DIR={self.root / 'state'}"))
        setup.prepare()

    def test_acl_gives_device_exactly_one_write_topic(self):
        acl = (Path(setup.__file__).parent / "mosquitto/acl").read_text()
        writes = [r for r in acl.split("user esp32")[1].splitlines() if r.startswith("topic write")]
        self.assertEqual(writes, ["topic write ai-quota/v1/device/status"])
        server_writes = {r.split()[2] for r in acl.split("user esp32")[0].splitlines() if r.startswith("topic write")}
        self.assertEqual(server_writes, {
            "ai-quota/v1/providers/codex", "ai-quota/v1/providers/claude",
            "ai-quota/v1/server/status", "ai-quota/v1/device/config"})
        self.assertIn("topic read ai-quota/v1/device/status", acl.split("user esp32")[0])
        self.assertIn("topic read ai-quota/v1/device/config", acl.split("user esp32")[1])

    def test_plaintext_hash_file_rejected_and_removed_on_abort(self):
        setup.prepare()
        with self.assertRaises(ValueError):
            setup.finish()
        setup.abort()
        self.assertFalse((self.root / "state/secrets/mosquitto-passwords.new").exists())

    def test_claude_subscription_status_rejects_other_auth_methods(self):
        with patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": "/synthetic/claude"}):
            status = {"authMethod": "claude.ai", "configDirectory": "/synthetic/claude"}
            self.assertTrue(status_check.subscription_login(status))
            for method in ("none", "api_key", "api_key_helper", "oauth_token", "third_party", "console"):
                self.assertFalse(status_check.subscription_login({**status, "authMethod": method}))
            self.assertFalse(status_check.subscription_login({**status, "configDirectory": "/wrong"}))


if __name__ == "__main__":
    unittest.main()
