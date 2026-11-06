//! Subscription transport on the machine holding the login. Credentials and
//! account identifiers stay here. The core owns parsing and freshness.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_ai::{CancelToken, ClaudeCliBackend, ClaudeConfig};
use hide_node_link::usage::{
    FailureDisposition, LimitFailure, LimitFetch, LimitSource, SessionLimits,
};
use hide_session::{newest_codex_session_files, parse_rfc3339, read_tail};
use serde_json::{Value, json};

const CODEX_TAIL_BYTES: u64 = 2 * 1024 * 1024;
const BODY_LIMIT: u64 = 256 * 1024;
const AUTH_LIMIT: u64 = 64 * 1024;
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_BINARY: &str = hide_agent_adapter::AgentId::ClaudeCode
    .adapter()
    .executables[0];
/// The keychain item Claude Code keeps its login in, under the login
/// account's name, for its default configuration folder.
#[cfg(target_os = "macos")]
const CLAUDE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// Scopes per-model rows; the endpoint names at most a handful today.
const CLAUDE_SCOPED_LIMIT: usize = 16;
/// A label or reset time the core accepts is at most 64 bytes; a longer
/// string never leaves the node.
const CLAUDE_TEXT_LIMIT: usize = 64;

/// One read per declared source, regardless of how many windows it serves.
pub fn fetch(source: LimitSource, root: &Path, report: &mut dyn FnMut() -> bool) -> LimitFetch {
    match source {
        LimitSource::ClaudeUsage => LimitFetch {
            result: claude_usage(claude_usage_api, || claude_usage_text(root, report)),
            fallback: None,
        },
        LimitSource::CodexUsage => {
            let result = codex_usage(root);
            let fallback = result
                .as_ref()
                .err()
                .filter(|failure| failure.disposition != FailureDisposition::Schema)
                .and_then(|_| codex_session_usage(root));
            LimitFetch { result, fallback }
        }
        LimitSource::AntigravityUsage => LimitFetch {
            result: antigravity_usage(root, report),
            fallback: None,
        },
    }
}

/// Fixed public endpoint, no credentials, query or usage payload. The core
/// owns the daily attempt budget; this node bounds each network operation.
pub fn fetch_prices() -> hide_node_link::usage::PriceFetch {
    use hide_node_link::usage::{PRICE_BODY_LIMIT, PriceFailure};
    const URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .max_redirects(0)
        .build()
        .into();
    agent
        .get(URL)
        .header("Accept", "application/json")
        .call()
        .map_err(|_| PriceFailure::Fetch)?
        .body_mut()
        .with_config()
        .limit(PRICE_BODY_LIMIT as u64)
        .read_to_string()
        .map_err(|_| PriceFailure::Body)
}

/// Deliberately private and not serializable or printable.
struct CodexCredentials {
    access_token: String,
    account_id: String,
}

/// A login file of at most [`AUTH_LIMIT`]; a missing or larger one is a
/// login this node cannot read.
fn read_credentials(path: &Path) -> Result<Vec<u8>, LimitFailure> {
    let failure =
        || LimitFailure::new(FailureDisposition::Authentication, "credentials_unreadable");
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| failure())?
        .take(AUTH_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() as u64 > AUTH_LIMIT {
        return Err(failure());
    }
    Ok(bytes)
}

fn codex_credentials(home: &Path) -> Result<CodexCredentials, LimitFailure> {
    let failure =
        || LimitFailure::new(FailureDisposition::Authentication, "credentials_unreadable");
    let bytes = read_credentials(&home.join("auth.json"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
    let token = |name: &str| {
        value
            .get("tokens")
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(failure)
    };
    Ok(CodexCredentials {
        access_token: token("access_token")?,
        account_id: token("account_id")?,
    })
}

fn codex_usage(home: &Path) -> Result<String, LimitFailure> {
    let credentials = codex_credentials(home)?;
    let value = get_json(
        CODEX_USAGE_URL,
        &[
            (
                "Authorization",
                &format!("Bearer {}", credentials.access_token),
            ),
            ("ChatGPT-Account-Id", &credentials.account_id),
            ("User-Agent", "codex-cli"),
            ("OpenAI-Beta", "codex-1"),
            ("originator", "Codex Desktop"),
        ],
    )?;
    Ok(safe_codex_body(&value, false))
}

/// One bounded GET of a usage endpoint. A refusal carries only its status,
/// a fixed code and a bounded Retry-After, never the server's text.
fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, LimitFailure> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .call()
        .map_err(|_| LimitFailure::new(FailureDisposition::Offline, "transport"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(LimitFailure {
            disposition: if status == 401 {
                FailureDisposition::Authentication
            } else {
                FailureDisposition::Offline
            },
            kind: if status == 429 {
                "rate_limited"
            } else {
                "http"
            }
            .into(),
            status: Some(status),
            retry_after: response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .filter(|v| {
                    v.len() <= 128
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b" ,:-".contains(&b))
                })
                .map(str::to_owned),
        });
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(BODY_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| LimitFailure::new(FailureDisposition::Offline, "body_read"))?;
    if bytes.len() as u64 > BODY_LIMIT {
        return Err(LimitFailure::new(FailureDisposition::Schema, "body_limit"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| LimitFailure::new(FailureDisposition::Schema, "response_json"))
}

/// Copy only numeric window fields. Unknown fields, account IDs, server
/// messages and echoed headers can never reach a snapshot or diagnostic.
/// A malformed field stays null for the core to classify as unreadable.
fn safe_codex_body(value: &Value, session: bool) -> String {
    let limits = if session {
        value
    } else {
        value.get("rate_limit").unwrap_or(value)
    };
    let names = if session {
        ["primary", "secondary"]
    } else {
        ["primary_window", "secondary_window"]
    };
    let window = |name| {
        limits.get(name).filter(|v| !v.is_null()).map(|v| {
            let duration = if session {
                v.get("window_minutes")
                    .and_then(Value::as_u64)
                    .and_then(|n| n.checked_mul(60))
                    .map(Value::from)
            } else {
                v.get("limit_window_seconds")
                    .filter(|v| v.is_number())
                    .cloned()
            };
            json!({
                "limit_window_seconds": duration,
                "used_percent": v.get("used_percent").filter(|v| v.is_number()),
                "reset_at": v.get(if session { "resets_at" } else { "reset_at" })
                    .or_else(|| v.get("resets_at")).filter(|v| v.is_number()),
            })
        })
    };
    json!({"rate_limit": {"primary_window": window(names[0]), "secondary_window": window(names[1])}}).to_string()
}

fn codex_session_usage(home: &Path) -> Option<SessionLimits> {
    newest_codex_session_files(&home.join("sessions"))
        .ok()?
        .into_iter()
        .find_map(|path| {
            parse_latest_session_limits(
                read_tail(&path, CODEX_TAIL_BYTES)
                    .ok()?
                    .records_newest_first(),
            )
        })
}
fn parse_latest_session_limits<'a>(
    mut records: impl Iterator<Item = &'a str>,
) -> Option<SessionLimits> {
    records.find_map(|line| {
        let value: Value = serde_json::from_str(line).ok()?;
        if value.get("type")?.as_str()? != "event_msg"
            || value.pointer("/payload/type")?.as_str()? != "token_count"
        {
            return None;
        }
        let limits = value.pointer("/payload/rate_limits")?.as_object()?;
        let source_at_unix_ms =
            parse_rfc3339(value.get("timestamp")?.as_str()?)?.checked_mul(1000)?;
        Some(SessionLimits {
            body: safe_codex_body(&Value::Object(limits.clone()), true),
            source_at_unix_ms,
        })
    })
}

/// Claude Code's windows from the account's usage endpoint, with the login
/// Claude Code keeps; when that read fails for any reason, from the text
/// Claude Code's own `/usage` prints, which is slower and starts the CLI.
/// The answer says which one it is, because the core reads them differently.
fn claude_usage(
    api: impl FnOnce() -> Result<Value, LimitFailure>,
    text: impl FnOnce() -> Result<String, LimitFailure>,
) -> Result<String, LimitFailure> {
    match api() {
        Ok(windows) => Ok(json!({ "usage_api": windows }).to_string()),
        Err(failure) => {
            eprintln!(
                "{}",
                json!({
                    "component": "usage",
                    "kind": "claude_usage_api_failed",
                    "reason": failure.kind,
                    "status": failure.status,
                })
            );
            text().map(|text| json!({ "usage_text": text }).to_string())
        }
    }
}

/// Deliberately private and not serializable or printable.
struct ClaudeToken(String);

fn claude_usage_api() -> Result<Value, LimitFailure> {
    let home = hide_platform::host::home_dir()
        .map_err(|_| LimitFailure::new(FailureDisposition::Authentication, "home_unknown"))?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    let token = claude_token(&claude_login(&home)?, now_ms)?;
    let value = get_json(
        CLAUDE_USAGE_URL,
        &[
            ("Authorization", &format!("Bearer {}", token.0)),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("Accept", "application/json"),
            ("User-Agent", "hide"),
        ],
    )?;
    safe_claude_body(&value)
}

/// The bytes of Claude Code's login: on macOS the keychain item it keeps,
/// and otherwise, or on macOS when there is no such item, the
/// `.credentials.json` Claude Code writes in `~/.claude`.
fn claude_login(home: &Path) -> Result<Vec<u8>, LimitFailure> {
    #[cfg(target_os = "macos")]
    if let Some(bytes) = claude_keychain_item()? {
        return Ok(bytes);
    }
    read_credentials(&home.join(".claude").join(".credentials.json"))
}

/// Claude Code's keychain item for the login account, read through
/// `/usr/bin/security`: Claude Code writes the item with that program, and
/// its access list lets only that program read it without asking, so the
/// read never shows a prompt that names Hide, whatever Hide's own signature.
/// Claude Code names the item's account by `USER`.
#[cfg(target_os = "macos")]
fn claude_keychain_item() -> Result<Option<Vec<u8>>, LimitFailure> {
    /// `security`'s exit status for a missing item (`errSecItemNotFound`).
    const ITEM_NOT_FOUND: i32 = 44;
    let Some(account) = std::env::var_os("USER").filter(|account| !account.is_empty()) else {
        return Ok(None);
    };
    let mut command = std::process::Command::new("/usr/bin/security");
    command
        .args(["find-generic-password", "-s", CLAUDE_KEYCHAIN_SERVICE, "-a"])
        .arg(account)
        .arg("-w");
    let stop = std::sync::atomic::AtomicBool::new(false);
    match hide_platform::process::run_to_end(&mut command, Duration::from_secs(5), &stop) {
        Ok(finished) if finished.succeeded() => Ok(Some(finished.stdout.into_bytes())),
        Ok(finished) if finished.code == Some(ITEM_NOT_FOUND) => Ok(None),
        _ => Err(LimitFailure::new(
            FailureDisposition::Offline,
            "keychain_unreadable",
        )),
    }
}

/// The access token in Claude Code's login. An expired one is not a
/// signed-out account: Claude Code renews it the next time it runs.
fn claude_token(bytes: &[u8], now_ms: u64) -> Result<ClaudeToken, LimitFailure> {
    let unreadable =
        || LimitFailure::new(FailureDisposition::Authentication, "credentials_unreadable");
    if bytes.len() as u64 > AUTH_LIMIT {
        return Err(unreadable());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| unreadable())?;
    let login = value.get("claudeAiOauth").ok_or_else(unreadable)?;
    let token = login
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(unreadable)?;
    if login
        .get("expiresAt")
        .and_then(Value::as_u64)
        .is_some_and(|expires_at| expires_at <= now_ms)
    {
        return Err(LimitFailure::new(
            FailureDisposition::Offline,
            "token_expired",
        ));
    }
    Ok(ClaudeToken(token.to_owned()))
}

/// Copy only each window's percent and reset time, and a model window's
/// model name, which `/usage` prints too. Spend, credits, account fields and
/// server text never leave the node; a malformed field stays null for the
/// core to read as unreadable. An answer with neither account window is not
/// this endpoint's.
fn safe_claude_body(value: &Value) -> Result<Value, LimitFailure> {
    if value.get("five_hour").is_none() && value.get("seven_day").is_none() {
        return Err(LimitFailure::new(
            FailureDisposition::Schema,
            "response_shape",
        ));
    }
    let short = |text: Option<&Value>| {
        text.and_then(Value::as_str)
            .filter(|text| text.len() <= CLAUDE_TEXT_LIMIT)
            .map_or(Value::Null, Value::from)
    };
    let window = |window: &Value, percent: &str| {
        json!({
            "used_percent": window.get(percent).filter(|p| p.is_number()),
            "resets_at": short(window.get("resets_at")),
        })
    };
    let account = |name: &str| {
        value
            .get(name)
            .filter(|window| window.is_object())
            .map_or(Value::Null, |w| window(w, "utilization"))
    };
    let scoped: Vec<Value> = value
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|limit| limit.get("kind").and_then(Value::as_str) == Some("weekly_scoped"))
        .take(CLAUDE_SCOPED_LIMIT)
        .map(|limit| {
            let mut scoped = window(limit, "percent");
            scoped["label"] = short(limit.pointer("/scope/model/display_name"));
            scoped
        })
        .collect();
    Ok(json!({
        "five_hour": account("five_hour"),
        "seven_day": account("seven_day"),
        "scoped": scoped,
    }))
}

fn claude_usage_text(cwd: &Path, report: &mut dyn FnMut() -> bool) -> Result<String, LimitFailure> {
    let backend = ClaudeCliBackend::new(ClaudeConfig {
        binary: PathBuf::from(CLAUDE_BINARY),
        cwd: cwd.to_path_buf(),
        ..ClaudeConfig::default()
    });
    let cancel = CancelToken::new();
    let running = cancel.clone();
    crate::reporting::run_reporting(
        "hide-node-claude-usage",
        move || backend.usage_text(&running),
        &|| cancel.cancel(),
        report,
    )
    .unwrap_or_else(|_| Err(hide_ai::UsageError::Failed("usage_worker".into())))
    .map_err(LimitFailure::cli)
    .and_then(|text| {
        if text.len() as u64 <= BODY_LIMIT {
            Ok(text)
        } else {
            Err(LimitFailure::new(FailureDisposition::Schema, "body_limit"))
        }
    })
}

/// agy's `/usage`, run by the backend that keeps an agy child from opening a
/// sign-in page or updating itself, on a worker that ends with the read.
fn antigravity_usage(cwd: &Path, report: &mut dyn FnMut() -> bool) -> Result<String, LimitFailure> {
    let backend = hide_ai::AntigravityCliBackend::new(hide_ai::TextCliConfig {
        cwd: cwd.to_path_buf(),
        ..hide_ai::TextCliConfig::new(hide_ai::CLI_DEFAULT_MODEL)
    });
    let cancel = CancelToken::new();
    let running = cancel.clone();
    crate::reporting::run_reporting(
        "hide-node-antigravity-usage",
        move || backend.usage_text(&running),
        &|| cancel.cancel(),
        report,
    )
    .unwrap_or_else(|_| Err(hide_ai::UsageError::Failed("usage_worker".into())))
    .map_err(LimitFailure::cli)
    .and_then(|text| {
        if text.len() as u64 <= BODY_LIMIT {
            Ok(text)
        } else {
            Err(LimitFailure::new(FailureDisposition::Schema, "body_limit"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_window_fields_of_the_claude_endpoint_leave_the_node() {
        let answer: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/claude-oauth-usage-2026-11-06.json"
        ))
        .unwrap();
        assert_eq!(
            safe_claude_body(&answer).unwrap(),
            json!({
                "five_hour": null,
                "seven_day": {"used_percent": 67.0, "resets_at": "2026-11-11T17:00:00.104969+00:00"},
                "scoped": [{"used_percent": 19, "resets_at": "2026-11-11T17:00:00.105141+00:00", "label": "Fable"}],
            })
        );
        let odd = json!({
            "five_hour": {"utilization": "12", "resets_at": 5, "account": "private"},
            "seven_day": {"utilization": 3, "resets_at": "x".repeat(65)},
            "limits": [{"kind": "weekly_scoped", "percent": 1, "resets_at": "2026-11-11T17:00:00Z", "scope": {"model": null, "surface": "cowork"}}],
        });
        assert_eq!(
            safe_claude_body(&odd).unwrap(),
            json!({
                "five_hour": {"used_percent": null, "resets_at": null},
                "seven_day": {"used_percent": 3, "resets_at": null},
                "scoped": [{"used_percent": 1, "resets_at": "2026-11-11T17:00:00Z", "label": null}],
            })
        );
        let refused = safe_claude_body(&json!({"type": "error", "error": {"message": "private"}}));
        assert_eq!(
            refused.map_err(|f| (f.disposition, f.kind)),
            Err((FailureDisposition::Schema, "response_shape".to_owned()))
        );
    }
    #[test]
    fn a_claude_login_without_a_live_token_is_never_sent() {
        let login = |token: &str, expires_at: u64| {
            json!({"claudeAiOauth": {"accessToken": token, "expiresAt": expires_at}, "mcpOAuth": {}})
                .to_string()
        };
        let now_ms = 1_793_959_945_000;
        let read = |bytes: String| {
            claude_token(bytes.as_bytes(), now_ms)
                .map(|token| token.0)
                .map_err(|f| (f.disposition, f.kind))
        };
        assert_eq!(read(login("live", now_ms + 1)), Ok("live".to_owned()));
        assert_eq!(
            read(login("old", now_ms)),
            Err((FailureDisposition::Offline, "token_expired".to_owned()))
        );
        // Claude Code leaves items like this behind under other account names.
        let unreadable = Err((
            FailureDisposition::Authentication,
            "credentials_unreadable".to_owned(),
        ));
        assert_eq!(read(login("", 0)), unreadable);
        assert_eq!(read(json!({"mcpOAuth": {}}).to_string()), unreadable);
        assert_eq!(read("not json".to_owned()), unreadable);
    }
    #[test]
    fn claude_usage_starts_the_cli_only_when_the_endpoint_read_failed() {
        let windows = json!({"five_hour": null, "seven_day": null, "scoped": []});
        assert_eq!(
            claude_usage(|| Ok(windows.clone()), || panic!("the CLI started")),
            Ok(json!({"usage_api": windows}).to_string())
        );
        let offline = || Err(LimitFailure::new(FailureDisposition::Offline, "transport"));
        assert_eq!(
            claude_usage(offline, || Ok(
                "Current week (all models): 1% used".to_owned()
            )),
            Ok(json!({"usage_text": "Current week (all models): 1% used"}).to_string())
        );
        assert_eq!(
            claude_usage(offline, || Err(LimitFailure::cli(
                hide_ai::UsageError::Timeout
            ))),
            Err(LimitFailure::new(FailureDisposition::Offline, "timeout"))
        );
    }
    #[test]
    fn session_fallback_preserves_both_windows_and_requires_a_source_time() {
        let frame = json!({"timestamp":"2026-09-14T12:01:00Z", "type":"event_msg", "payload": {
            "type":"token_count", "rate_limits": {"primary":{"used_percent":90,"window_minutes":300,"resets_at":4102444800_u64},
                "secondary":{"used_percent":58,"window_minutes":10080,"resets_at":4102444801_u64}}, "message":"private"}});
        let read =
            parse_latest_session_limits(std::iter::once(frame.to_string().as_str())).unwrap();
        let body: Value = serde_json::from_str(&read.body).unwrap();
        assert_eq!(
            body.pointer("/rate_limit/primary_window/limit_window_seconds"),
            Some(&json!(18000))
        );
        assert_eq!(
            body.pointer("/rate_limit/secondary_window/used_percent"),
            Some(&json!(58))
        );
        assert_eq!(read.source_at_unix_ms, 1_789_387_260_000);
        assert!(!read.body.contains("private"));
        let mut missing_time = frame;
        missing_time.as_object_mut().unwrap().remove("timestamp");
        assert!(
            parse_latest_session_limits(std::iter::once(missing_time.to_string().as_str()))
                .is_none()
        );
    }
    #[test]
    fn only_numeric_usage_fields_leave_the_node() {
        let body = safe_codex_body(
            &json!({"access_token":"secret", "account_id":"private", "rate_limit":{
            "primary_window":{"used_percent":"secret", "limit_window_seconds":18000,"reset_at":123,"account":"private"},
            "secondary_window":null}}),
            false,
        );
        assert!(!body.contains("secret"));
        assert!(!body.contains("private"));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert!(
            body.pointer("/rate_limit/primary_window/used_percent")
                .unwrap()
                .is_null()
        );
        assert!(
            body.pointer("/rate_limit/secondary_window")
                .unwrap()
                .is_null()
        );
    }
    #[test]
    fn missing_and_malformed_credentials_are_authentication_failures() {
        let home = tempfile::tempdir().unwrap();
        assert!(matches!(
            codex_credentials(home.path()),
            Err(LimitFailure {
                disposition: FailureDisposition::Authentication,
                ..
            })
        ));
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"tokens":{"access_token":"t","account_id":" "}}"#,
        )
        .unwrap();
        assert!(codex_credentials(home.path()).is_err());
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"tokens":{"access_token":"t","account_id":"a"}}"#,
        )
        .unwrap();
        let credentials = codex_credentials(home.path()).unwrap();
        assert_eq!(credentials.access_token, "t");
        assert_eq!(credentials.account_id, "a");
    }
    #[test]
    fn device_commands_cannot_fetch_subscription_credentials_or_limits() {
        assert!(!hide_node_link::protocol::Call::UsagePriceFetch.answered_by_device());
        for source in LimitSource::ALL {
            let call = hide_node_link::protocol::Call::UsageLimitFetch {
                source,
                root: "/fixture".into(),
            };
            assert!(!call.answered_by_device());
        }
    }
}
