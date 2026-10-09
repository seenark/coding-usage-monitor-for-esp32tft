use quota_core::{DetailsStatus, ErrorCode, MAX_KNOWN_EXPIRATIONS, MAX_WINDOWS, Provider};
use quota_server::providers::{Collector, codex::CodexCollector};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

fn fixture_cases() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../fixtures/codex-normalization.json")).unwrap()
}

fn simple_response() -> Value {
    json!({"rateLimits": {"primary": {"usedPercent": 40, "windowDurationMins": 300, "resetsAt": 1800000000}}, "rateLimitResetCredits": null})
}

fn collector(scenario: Value, timeout: Duration) -> (TempDir, CodexCollector) {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("scenario.json"),
        serde_json::to_vec(&scenario).unwrap(),
    )
    .unwrap();
    let collector = CodexCollector::new(
        PathBuf::from(env!("CARGO_BIN_EXE_fake-codex")),
        home.path().to_path_buf(),
        timeout,
    );
    (home, collector)
}

fn audit(home: &Path) -> Vec<Value> {
    std::fs::read_to_string(home.join("audit.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn launches(home: &Path) -> u64 {
    std::fs::read(home.join("launches.json"))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or(0)
}

#[tokio::test]
async fn normalizes_official_shapes_over_public_subprocess_interface() {
    let cases = fixture_cases();
    let responses: Vec<Value> = cases.iter().map(|case| case["response"].clone()).collect();
    let (home, mut collector) = collector(
        json!({"responses": responses, "noise": true, "fragmented": true}),
        Duration::from_secs(5),
    );
    for case in cases {
        let snapshot = collector.fetch().await.unwrap();
        assert_eq!(snapshot.provider, Provider::Codex);
        assert!(snapshot.fetched_at.is_some());
        snapshot.validate().unwrap();
        let published = serde_json::to_value(&snapshot).unwrap();
        let actual_windows: Vec<Value> = published["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|window| {
                json!({
                    "limit_id": window["limit_id"], "key": window["key"],
                    "duration_minutes": window["duration_minutes"], "used_percent": window["used_percent"],
                    "remaining_percent": window["remaining_percent"], "resets_at": window["resets_at"]
                })
            })
            .collect();
        assert_eq!(json!(actual_windows), case["windows"], "{}", case["name"]);
        assert_eq!(
            published["banked_resets"], case["banked_resets"],
            "{}",
            case["name"]
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        for forbidden in [
            "private-account-marker",
            "private-credit-title-marker",
            "opaque-credit-marker",
            "synthetic-personal-marker",
            "must-not-publish",
            "synthetic-stderr-secret",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }
    let audit = audit(home.path());
    let methods: Vec<&str> = audit
        .iter()
        .filter_map(|record| record["method"].as_str())
        .collect();
    assert_eq!(
        &methods[..4],
        &[
            "initialize",
            "initialized",
            "account/read",
            "account/rateLimits/read"
        ]
    );
    assert_eq!(
        methods
            .iter()
            .filter(|method| **method == "initialize")
            .count(),
        1
    );
    assert_eq!(launches(home.path()), 1);
    let ids: Vec<u64> = audit
        .iter()
        .filter_map(|record| record["id"].as_u64())
        .collect();
    assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len());
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    let args = audit[0]["args"].as_array().unwrap();
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--listen" && pair[1] == "stdio://")
    );
    assert!(
        args.iter()
            .any(|argument| argument == "cli_auth_credentials_store=\"file\"")
    );
    assert!(
        args.iter()
            .any(|argument| argument == "forced_login_method=\"chatgpt\"")
    );
}

#[tokio::test]
async fn polls_external_reset_changes_without_inventing_recovery() {
    let first = simple_response();
    let mut second = first.clone();
    second["rateLimits"]["primary"]["resetsAt"] = json!(1);
    second["rateLimits"]["primary"]["windowDurationMins"] = json!(73);
    let (home, mut collector) = collector(
        json!({"responses": [first, second]}),
        Duration::from_secs(5),
    );
    let first = collector.fetch().await.unwrap();
    let second = collector.fetch().await.unwrap();
    assert_eq!(first.windows[0].resets_at, Some(1800000000));
    assert_eq!(second.windows[0].resets_at, Some(1));
    assert_eq!(second.windows[0].duration_minutes, Some(73));
    assert_eq!(second.windows[0].remaining_percent, Some(60.0));
    assert_eq!(launches(home.path()), 1);
}

#[tokio::test]
async fn kills_failed_child_backs_off_then_reinitializes_with_new_ids() {
    let (home, mut collector) = collector(
        json!({"responses": [simple_response()], "mode": "exit", "failOnce": true}),
        Duration::from_secs(5),
    );
    let failure = collector.fetch().await.unwrap_err();
    assert_eq!(failure.code, ErrorCode::ProcessUnavailable);
    let backoff = failure.retry_after.unwrap();
    assert_eq!(backoff, Duration::from_secs(1));
    assert_eq!(
        collector.fetch().await.unwrap_err().code,
        ErrorCode::ProcessUnavailable
    );
    assert_eq!(launches(home.path()), 1);
    tokio::time::sleep(backoff + Duration::from_millis(20)).await;
    collector.fetch().await.unwrap();
    assert_eq!(launches(home.path()), 2);
    let records = audit(home.path());
    let ids: Vec<u64> = records
        .iter()
        .filter_map(|record| record["id"].as_u64())
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        records
            .iter()
            .filter(|record| record["method"] == "initialize")
            .count(),
        2
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["method"] == "initialized")
            .count(),
        2
    );
}

#[tokio::test]
async fn bounds_frames_notifications_and_failure_envelopes() {
    for (mode, expected) in [
        ("oversized", ErrorCode::InvalidResponse),
        ("malformed", ErrorCode::InvalidResponse),
        ("unterminated", ErrorCode::ProcessUnavailable),
        ("flood", ErrorCode::InvalidResponse),
        ("both", ErrorCode::InvalidResponse),
        ("server_request", ErrorCode::Unsupported),
    ] {
        let (_home, mut collector) = collector(
            json!({"responses": [simple_response()], "mode": mode}),
            Duration::from_secs(5),
        );
        let failure = collector.fetch().await.unwrap_err();
        assert_eq!(failure.code, expected, "{mode}");
        assert!(failure.retry_after.is_some());
        assert!(!failure.to_string().contains("synthetic"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn deadline_kills_hung_child_and_restarts_after_backoff() {
    let (home, mut collector) = collector(
        json!({"responses": [simple_response()], "mode": "timeout", "failOnce": true}),
        Duration::from_millis(500),
    );
    let failure = collector.fetch().await.unwrap_err();
    assert_eq!(failure.code, ErrorCode::Timeout);
    assert_child_stopped(home.path()).await;
    tokio::time::sleep(failure.retry_after.unwrap() + Duration::from_millis(20)).await;
    collector.fetch().await.unwrap();
    assert_eq!(launches(home.path()), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_inflight_fetch_does_not_leave_poisoned_child() {
    let (home, mut collector) = collector(
        json!({"responses": [simple_response()], "mode": "timeout", "failOnce": true}),
        Duration::from_secs(30),
    );
    let mut fetching = Box::pin(collector.fetch());
    tokio::select! {
        result = &mut fetching => panic!("the hung fake child returned early: {result:?}"),
        _ = wait_for_quota_request(home.path()) => {}
    }
    drop(fetching);
    assert_child_stopped(home.path()).await;
    collector.fetch().await.unwrap();
    assert_eq!(launches(home.path()), 2);
}

#[cfg(unix)]
async fn wait_for_quota_request(home: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(home.join("audit.jsonl"))
                .ok()
                .is_some_and(|contents| {
                    contents
                        .lines()
                        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                        .any(|record| record["method"] == "account/rateLimits/read")
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the fake child did not reach the quota request");
}

#[cfg(unix)]
async fn assert_child_stopped(home: &Path) {
    let records = audit(home);
    let pid = records
        .iter()
        .find(|record| record["event"] == "launch")
        .unwrap()["pid"]
        .as_u64()
        .unwrap();
    for _ in 0..100 {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the fake Codex child survived collector teardown");
}

#[tokio::test]
async fn failed_handshake_never_sends_initialized_or_quota_reads() {
    for scenario in [
        json!({"responses": [simple_response()], "errorAt": "initialize", "error": {"code": -32601, "message": "synthetic-handshake-secret"}}),
        json!({"responses": [simple_response()], "initialization": {}}),
    ] {
        let (home, mut collector) = collector(scenario, Duration::from_secs(5));
        let failure = collector.fetch().await.unwrap_err();
        assert!(matches!(
            failure.code,
            ErrorCode::Unsupported | ErrorCode::InvalidResponse
        ));
        assert!(!failure.to_string().contains("synthetic-handshake-secret"));
        let records = audit(home.path());
        assert_eq!(
            records
                .iter()
                .filter_map(|record| record["method"].as_str())
                .collect::<Vec<_>>(),
            vec!["initialize"]
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_collector_stops_idle_child() {
    let (home, mut collector) = collector(
        json!({"responses": [simple_response()]}),
        Duration::from_secs(5),
    );
    collector.fetch().await.unwrap();
    drop(collector);
    assert_child_stopped(home.path()).await;
}

#[tokio::test]
async fn missing_program_and_auth_failures_have_safe_codes() {
    let home = tempfile::tempdir().unwrap();
    let mut absent = CodexCollector::new(
        home.path().join("missing-program"),
        home.path().to_path_buf(),
        Duration::from_secs(1),
    );
    assert_eq!(
        absent.fetch().await.unwrap_err().code,
        ErrorCode::ProcessUnavailable
    );
    for (account, expected) in [
        (Value::Null, ErrorCode::Unauthenticated),
        (json!({"type": "apiKey"}), ErrorCode::Unsupported),
        (json!(42), ErrorCode::InvalidResponse),
    ] {
        let (home, mut collector) = collector(
            json!({"responses": [simple_response()], "account": account}),
            Duration::from_secs(5),
        );
        assert_eq!(collector.fetch().await.unwrap_err().code, expected);
        assert!(
            !audit(home.path())
                .iter()
                .any(|record| record["method"] == "account/rateLimits/read")
        );
    }
    for (code, expected) in [
        (-32600, ErrorCode::Unauthenticated),
        (-32601, ErrorCode::Unsupported),
        (-32001, ErrorCode::RateLimited),
        (-32603, ErrorCode::Upstream),
    ] {
        let (_home, mut collector) = collector(
            json!({"responses": [simple_response()], "mode": "error", "error": {"code": code, "message": "synthetic-upstream-secret"}}),
            Duration::from_secs(5),
        );
        let failure = collector.fetch().await.unwrap_err();
        assert_eq!(failure.code, expected);
        assert!(!failure.to_string().contains("synthetic-upstream-secret"));
    }
}

#[tokio::test]
async fn malformed_quota_values_do_not_publish_fabricated_data() {
    for response in [
        json!({}),
        json!({"rateLimits": {"primary": {"usedPercent": -1}}}),
        json!({"rateLimits": {"primary": {"usedPercent": "20"}}}),
        json!({"rateLimits": {"primary": {"windowDurationMins": 4294967296_u64}}}),
        json!({"rateLimits": {"primary": {"resetsAt": "later"}}}),
        json!({"rateLimits": {}, "rateLimitResetCredits": {"availableCount": -1, "credits": null}}),
        json!({"rateLimitsByLimitId": {"codex": {"limitId": "other"}}}),
    ] {
        let (_home, mut collector) =
            collector(json!({"responses": [response]}), Duration::from_secs(5));
        assert_eq!(
            collector.fetch().await.unwrap_err().code,
            ErrorCode::InvalidResponse
        );
    }
}

#[tokio::test]
async fn capped_credit_details_are_partial_without_changing_count() {
    let credits: Vec<Value> = (0..MAX_KNOWN_EXPIRATIONS + 3)
        .map(|index| json!({"id": format!("synthetic-{index}"), "resetType": "codexRateLimits", "status": "available", "expiresAt": 1800000000_i64 + index as i64}))
        .collect();
    let count = credits.len() as u32;
    let response = json!({"rateLimits": {}, "rateLimitResetCredits": {"availableCount": count, "credits": credits}});
    let (_home, mut collector) =
        collector(json!({"responses": [response]}), Duration::from_secs(5));
    let bank = collector.fetch().await.unwrap().banked_resets.unwrap();
    assert_eq!(bank.available_count, Some(count));
    assert_eq!(bank.details_status, DetailsStatus::Partial);
    assert_eq!(bank.known_expirations_at.len(), MAX_KNOWN_EXPIRATIONS);
    assert_eq!(bank.known_nonexpiring_count, 0);
}

#[tokio::test]
async fn duplicate_credit_ids_are_not_double_counted() {
    let credit = json!({"id": "synthetic-same", "resetType": "codexRateLimits", "status": "available", "expiresAt": 1800000000});
    let response = json!({"rateLimits": {}, "rateLimitResetCredits": {"availableCount": 2, "credits": [credit.clone(), credit]}});
    let (_home, mut collector) =
        collector(json!({"responses": [response]}), Duration::from_secs(5));
    let bank = collector.fetch().await.unwrap().banked_resets.unwrap();
    assert_eq!(bank.available_count, Some(2));
    assert_eq!(bank.known_expirations_at, vec![1800000000]);
    assert_eq!(bank.details_status, DetailsStatus::Partial);
}

#[tokio::test]
async fn excess_windows_fail_instead_of_silent_truncation() {
    let mut buckets = serde_json::Map::new();
    for index in 0..=MAX_WINDOWS {
        buckets.insert(
            format!("bucket-{index}"),
            json!({"primary": {"usedPercent": 10, "resetsAt": null}}),
        );
    }
    let (_home, mut collector) = collector(
        json!({"responses": [{"rateLimitsByLimitId": buckets}]}),
        Duration::from_secs(5),
    );
    assert_eq!(
        collector.fetch().await.unwrap_err().code,
        ErrorCode::InvalidResponse
    );
}
