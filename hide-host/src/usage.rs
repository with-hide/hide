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
const CLAUDE_BINARY: &str = hide_agent_adapter::AgentId::ClaudeCode
    .adapter()
    .executables[0];

/// One read per declared source, regardless of how many windows it serves.
pub fn fetch(source: LimitSource, root: &Path, report: &mut dyn FnMut() -> bool) -> LimitFetch {
    match source {
        LimitSource::ClaudeUsageText => LimitFetch {
            result: claude_usage_text(root, report),
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

fn codex_credentials(home: &Path) -> Result<CodexCredentials, LimitFailure> {
    let failure =
        || LimitFailure::new(FailureDisposition::Authentication, "credentials_unreadable");
    let mut bytes = Vec::new();
    std::fs::File::open(home.join("auth.json"))
        .map_err(|_| failure())?
        .take(AUTH_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() as u64 > AUTH_LIMIT {
        return Err(failure());
    }
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
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(CODEX_USAGE_URL)
        .header(
            "Authorization",
            &format!("Bearer {}", credentials.access_token),
        )
        .header("ChatGPT-Account-Id", &credentials.account_id)
        .header("User-Agent", "codex-cli")
        .header("OpenAI-Beta", "codex-1")
        .header("originator", "Codex Desktop")
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
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| LimitFailure::new(FailureDisposition::Schema, "response_json"))?;
    Ok(safe_codex_body(&value, false))
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
            parse_latest_session_limits(&read_tail(&path, CODEX_TAIL_BYTES).ok()?.contents)
        })
}
fn parse_latest_session_limits(contents: &str) -> Option<SessionLimits> {
    contents.lines().rev().find_map(|line| {
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
    fn session_fallback_preserves_both_windows_and_requires_a_source_time() {
        let frame = json!({"timestamp":"2026-09-14T12:01:00Z", "type":"event_msg", "payload": {
            "type":"token_count", "rate_limits": {"primary":{"used_percent":90,"window_minutes":300,"resets_at":4102444800_u64},
                "secondary":{"used_percent":58,"window_minutes":10080,"resets_at":4102444801_u64}}, "message":"private"}});
        let read = parse_latest_session_limits(&frame.to_string()).unwrap();
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
        assert!(parse_latest_session_limits(&missing_time.to_string()).is_none());
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
