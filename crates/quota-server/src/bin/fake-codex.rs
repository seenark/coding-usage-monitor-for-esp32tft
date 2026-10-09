//! Offline subprocess used by Codex protocol integration tests. No network or auth access.
//! It refuses to start without an explicitly supplied synthetic scenario in CODEX_HOME.

use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

fn emit(output: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = PathBuf::from(std::env::var_os("CODEX_HOME").ok_or("missing synthetic home")?);
    assert_eq!(
        std::env::var("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED")?,
        "1"
    );
    assert_eq!(
        std::env::var_os("CODEX_SQLITE_HOME").as_deref(),
        Some(home.as_os_str())
    );
    assert!(std::env::var_os("OPENAI_API_KEY").is_none());
    assert!(std::env::var_os("CODEX_API_KEY").is_none());
    let scenario: Value = serde_json::from_slice(&fs::read(home.join("scenario.json"))?)?;
    let launches_path = home.join("launches.json");
    let launches: u64 = fs::read(&launches_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<u64>(&bytes).ok())
        .unwrap_or(0)
        + 1;
    fs::write(launches_path, serde_json::to_vec(&launches)?)?;
    let mut audit = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("audit.jsonl"))?;
    writeln!(
        audit,
        "{}",
        json!({"event": "launch", "pid": std::process::id(), "args": std::env::args().skip(1).collect::<Vec<_>>() })
    )?;
    audit.flush()?;
    // This marker is synthetic and must be discarded by the adapter's stderr policy.
    eprintln!("synthetic-stderr-secret-must-not-leak");
    let stdin = io::stdin();
    let mut output = io::stdout().lock();
    let mut initialized = false;
    let mut initialization_requested = false;
    let mut seen_ids = HashSet::new();
    let mut polls = 0;
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        writeln!(audit, "{request}")?;
        audit.flush()?;
        assert!(
            request.get("jsonrpc").is_none(),
            "wire omits the jsonrpc header"
        );
        let method = request["method"].as_str().ok_or("missing method")?;
        if method == "initialized" {
            assert!(initialization_requested && !initialized);
            assert!(request.get("id").is_none());
            initialized = true;
            continue;
        }
        let id = request["id"].as_u64().ok_or("missing request ID")?;
        assert!(seen_ids.insert(id), "request IDs must be unique");
        if scenario["errorAt"].as_str() == Some(method) {
            emit(&mut output, &json!({"id": id, "error": scenario["error"]}))?;
            continue;
        }
        let result = match method {
            "initialize" => {
                assert!(!initialization_requested);
                initialization_requested = true;
                assert_eq!(request["params"]["capabilities"]["experimentalApi"], false);
                scenario.get("initialization").cloned().unwrap_or_else(|| json!({"userAgent": "synthetic", "codexHome": home, "platformFamily": "unix", "platformOs": "linux"}))
            }
            "account/read" => {
                assert!(initialized);
                assert_eq!(request["params"]["refreshToken"], true);
                json!({
                    "account": scenario.get("account").cloned().unwrap_or_else(|| json!({"type": "chatgpt", "email": "synthetic-personal-marker", "planType": "plus"})),
                    "requiresOpenaiAuth": true
                })
            }
            "account/rateLimits/read" => {
                assert!(initialized);
                assert_eq!(request["params"]["supportsLunaReserve"], false);
                assert_eq!(request["params"]["excludeResetCreditDetails"], false);
                let should_fail = !scenario["failOnce"].as_bool().unwrap_or(false) || launches == 1;
                if should_fail {
                    match scenario["mode"].as_str().unwrap_or("") {
                        "timeout" => std::thread::sleep(Duration::from_secs(30)),
                        "exit" => return Ok(()),
                        "oversized" => {
                            output.write_all(&vec![b'x'; 128 * 1024 + 1])?;
                            output.flush()?;
                            std::thread::sleep(Duration::from_secs(30));
                        }
                        "malformed" => {
                            output.write_all(b"{not-json}\n")?;
                            output.flush()?;
                            continue;
                        }
                        "unterminated" => {
                            output.write_all(b"{\"id\":1,\"result\":{}}")?;
                            output.flush()?;
                            return Ok(());
                        }
                        "server_request" => {
                            emit(
                                &mut output,
                                &json!({"id": "server-1", "method": "account/chatgptAuthTokens/refresh", "params": {"reason": "unauthorized"}}),
                            )?;
                            continue;
                        }
                        "flood" => {
                            for _ in 0..130 {
                                emit(
                                    &mut output,
                                    &json!({"method": "account/updated", "params": {}}),
                                )?;
                            }
                            continue;
                        }
                        "error" => {
                            emit(&mut output, &json!({"id": id, "error": scenario["error"]}))?;
                            continue;
                        }
                        "both" => {
                            emit(
                                &mut output,
                                &json!({"id": id, "result": {}, "error": {"code": -32603, "message": "synthetic-error-secret"}}),
                            )?;
                            continue;
                        }
                        _ => {}
                    }
                }
                let responses = scenario["responses"]
                    .as_array()
                    .ok_or("missing synthetic responses")?;
                let response = responses
                    .get(polls)
                    .or_else(|| responses.last())
                    .ok_or("empty responses")?
                    .clone();
                polls += 1;
                response
            }
            _ => panic!(
                "the monitor must not invoke write, login, reset, purchase, or inference methods"
            ),
        };
        if scenario["noise"].as_bool().unwrap_or(false) {
            emit(
                &mut output,
                &json!({"method": "account/rateLimits/updated", "params": {"syntheticSecret": "must-not-publish"}}),
            )?;
            emit(
                &mut output,
                &json!({"id": "stray", "result": {"syntheticSecret": "must-not-publish"}}),
            )?;
            emit(
                &mut output,
                &json!({"id": id + 1000, "error": {"code": -32603, "message": "must-not-publish"}}),
            )?;
        }
        if scenario["fragmented"].as_bool().unwrap_or(false) {
            let bytes = serde_json::to_vec(&json!({"id": id, "result": result}))?;
            for chunk in bytes.chunks(7) {
                output.write_all(chunk)?;
                output.flush()?;
            }
            output.write_all(b"\r\n")?;
            output.flush()?;
        } else {
            emit(&mut output, &json!({"id": id, "result": result}))?;
        }
    }
    Ok(())
}
