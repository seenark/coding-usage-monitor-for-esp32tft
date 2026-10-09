use std::process::Command;

#[test]
fn invalid_configuration_is_rejected_without_exposing_values() {
    let cases = [
        ("MOCK_MODE", "private-test-sentinel"),
        ("MQTT_PORT", "private-test-sentinel"),
        ("CLAUDE_POLL_INTERVAL_SECONDS", "0"),
        ("CODEX_POLL_INTERVAL_SECONDS", "59"),
        ("ADMIN_LISTEN", "private-test-sentinel"),
        ("CLAUDE_PRIVATE_USAGE_ENABLED", "private-test-sentinel"),
    ];
    for (key, value) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_quota-server"))
            .env_clear()
            .env(key, value)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains(key), "missing safe configuration name");
        assert!(
            !text.contains("private-test-sentinel"),
            "configuration value leaked"
        );
    }
}

#[test]
fn mqtt_tls_requires_explicit_trust_anchor() {
    let output = Command::new(env!("CARGO_BIN_EXE_quota-server"))
        .env_clear()
        .env("MQTT_TLS", "true")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("MQTT_CA_FILE"));
}
