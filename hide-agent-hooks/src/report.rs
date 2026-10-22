//! Handing a pane's counts to Herdr as display-only metadata.
//!
//! `pane.report_metadata` is the whole integration: Herdr stores the tokens
//! on the pane, returns them in `PaneInfo`, and Hide's ordinary
//! `session.snapshot` carries them to the core. No event subscription or poll
//! is added by this path.
//!
//! The request goes over the socket through `hide-herdr-client`, the same
//! client hided's own Herdr connections use, rather than through the
//! `herdr` CLI. The CLI's argument grammar is checked by nothing in this
//! repository, and the first release of this hook shelled out with the pane
//! id in a position the pinned CLI refuses, so every report exited 2, the
//! helper swallowed it, and every pane on the machine read as
//! `session_predates_install` for a week. The socket method is the contract
//! `scripts/check-herdr-contract.sh` pins, and the fake server test below
//! asserts the exact request it sends.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hide_herdr_client::{ApiError, request_with_timeout};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::counters::PaneCounters;
use crate::runtime::{
    DONE_TOKEN, HOOK_SOURCE_NAME, HOOK_VERSION, INSTRUMENTED_TOKEN, SESSION_TOKEN, WORKING_TOKEN,
};

/// How long one report may hold the agent's hook slot. A hook runs inside
/// the agent's turn, so a Herdr that does not answer costs the operator this
/// long and no more.
const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// The socket the hook reports through.
///
/// Herdr exports `HERDR_SOCKET_PATH` to the panes of a server that was not
/// started on the default path, so a hook inside such a pane reaches the
/// server that owns it. Without the override the hook uses Herdr's default
/// for `home`, in Herdr's own order (`hide_platform::host`), which is the
/// same resolution hided applies; where that order has no answer the report
/// fails with the reason.
pub fn socket_path(home: &Path) -> Result<PathBuf, ApiError> {
    if let Some(path) = std::env::var_os("HERDR_SOCKET_PATH").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let variables = |name: &str| {
        if name == hide_platform::host::HOME_VARIABLE {
            Some(home.as_os_str().to_owned())
        } else {
            std::env::var_os(name)
        }
    };
    hide_platform::host::herdr_socket_default_from(&variables).map_err(|error| {
        ApiError::Transport(format!("Herdr's default socket has no location: {error}"))
    })
}

/// The metadata source every hook report is filed under.
///
/// It is the marker name without its version: Herdr limits a source to
/// ASCII letters, digits, `:`, `.`, `_` and `-`, so the `@` of
/// [`crate::runtime::hook_source_id`] cannot travel here. The version is
/// carried by the [`INSTRUMENTED_TOKEN`] instead.
pub fn metadata_source() -> &'static str {
    HOOK_SOURCE_NAME
}

/// What a report says about the two count tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Counts {
    /// These counts.
    Set(PaneCounters),
    /// Nothing: the tokens are left out of the report, so what Herdr holds
    /// stays, and a reader sees counts nobody reported as unknown and not as
    /// a zero. hided does that when it cannot tell that a file's counts
    /// belong to the session now in the pane.
    Leave,
    /// Remove them (`null` clears one token, and a report merges token by
    /// token, so leaving them out would keep the old values). hided does that
    /// when it put counts on a pane and the file then turned out to be
    /// another session's.
    Clear,
}

/// The `pane.report_metadata` parameters one report sends, as a value a test
/// can inspect.
///
/// Building them apart from sending them is what lets the contract be
/// asserted without a Herdr server (engineering rule 12). `version` is the
/// hook version the report speaks for: the running helper's own, or the one
/// a pane's file recorded when hided puts the tokens back after Herdr lost
/// them (`counters::restore_of`).
pub fn report_params(
    pane_id: &str,
    version: u32,
    counts: Counts,
    session: Option<&str>,
) -> serde_json::Value {
    let mut tokens = serde_json::Map::new();
    tokens.insert(INSTRUMENTED_TOKEN.to_owned(), json!(version.to_string()));
    match counts {
        Counts::Set(counters) => {
            tokens.insert(
                WORKING_TOKEN.to_owned(),
                json!(counters.working.to_string()),
            );
            tokens.insert(DONE_TOKEN.to_owned(), json!(counters.done.to_string()));
            tokens.insert(
                SESSION_TOKEN.to_owned(),
                json!(session.and_then(session_digest)),
            );
        }
        Counts::Leave => {}
        Counts::Clear => {
            tokens.insert(WORKING_TOKEN.to_owned(), serde_json::Value::Null);
            tokens.insert(DONE_TOKEN.to_owned(), serde_json::Value::Null);
            tokens.insert(SESSION_TOKEN.to_owned(), serde_json::Value::Null);
        }
    }
    json!({
        "pane_id": pane_id,
        "source": metadata_source(),
        "tokens": tokens,
    })
}

/// Reports one pane's counts to the Herdr behind `socket_path`.
pub fn report(
    socket_path: &Path,
    pane_id: &str,
    counters: PaneCounters,
    session: Option<&str>,
) -> Result<(), ApiError> {
    request_with_timeout(
        socket_path,
        "pane.report_metadata",
        report_params(pane_id, HOOK_VERSION, Counts::Set(counters), session),
        REPORT_TIMEOUT,
    )
    .map(|_| ())
}

/// Publishes the counts with their observation owner, then repairs an
/// out-of-order report with one bounded reread of the same record.
/// A session change matters even when both sessions reported zero.
pub fn report_latest(
    home: &Path,
    socket_path: &Path,
    pane_id: &str,
    counters: PaneCounters,
    session: Option<&str>,
) -> Result<(), ApiError> {
    report(socket_path, pane_id, counters, session)?;
    // A writer reports its own update; an unreadable record is not newer proof.
    match crate::counters::read_settled(home, pane_id) {
        Ok((now, owner)) if now != counters || owner.as_deref() != session => {
            report(socket_path, pane_id, now, owner.as_deref())
        }
        _ => Ok(()),
    }
}

/// The same native-session identity used by the core's pane lineage.
/// Hash the exact value, including a path's spelling, rather than a display name.
pub fn session_digest(session: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    crate::delivery::valid_session(session).then(|| {
        Sha256::digest(session.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    })
}

/// The session Herdr reports for the agent in a pane, or none when it reports
/// none (no agent there, or an integration that has not reported one yet).
///
/// It is what tells the pane's agent from a session that only runs the same
/// hooks in the pane's environment: Herdr takes a nested `claude -p` session's
/// report for no more than a report, and keeps the pane's own (measured on the
/// pinned Herdr 0.9.3, where `/clear` replaced the pane's session and a nested
/// session's `startup` did not).
pub fn pane_session(socket_path: &Path, pane_id: &str) -> Result<Option<String>, ApiError> {
    request_with_timeout(
        socket_path,
        "pane.get",
        json!({ "pane_id": pane_id }),
        REPORT_TIMEOUT,
    )
    .map(|answer| session_of(&answer))
}

/// The session in a `pane.get` answer (`PaneInfo.agent_session`).
fn session_of(answer: &serde_json::Value) -> Option<String> {
    answer
        .pointer("/pane/agent_session/value")?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// The last report that did not reach Herdr, kept so the failure is a
/// sentence in `doctor` and in Settings rather than a pane that quietly
/// reads as uninstrumented (engineering rules 4 and 10).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFailure {
    pub pane_id: String,
    pub event: String,
    pub socket_path: String,
    pub error: String,
    pub at_unix_ms: u64,
}

impl ReportFailure {
    /// The one sentence both surfaces show.
    pub fn message(&self) -> String {
        format!(
            "The hook could not report to Herdr at {} for pane {} on {}: {}",
            self.socket_path, self.pane_id, self.event, self.error
        )
    }
}

fn failure_path(home: &Path) -> PathBuf {
    home.join(".hide")
        .join("agent-hooks")
        .join("last-report-failure.json")
}

/// Records the outcome of one report: a failure replaces the last one, a
/// success clears it, so the file describes the hook's current state rather
/// than its history.
pub fn record_outcome(
    home: &Path,
    pane_id: &str,
    event: &str,
    socket_path: &Path,
    outcome: &Result<(), ApiError>,
) -> io::Result<()> {
    let path = failure_path(home);
    match outcome {
        Ok(()) => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
        Err(error) => {
            let failure = ReportFailure {
                pane_id: pane_id.to_owned(),
                event: event.to_owned(),
                socket_path: socket_path.display().to_string(),
                error: error.to_string(),
                at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or(0),
            };
            if let Some(parent) = path.parent() {
                hide_platform::fs::private::create_dir_all(parent)?;
            }
            fs::write(&path, serde_json::to_vec(&failure)?)
        }
    }
}

/// The last recorded failure, or none when the last report succeeded or no
/// report has run. A file that cannot be read back is reported as a failure
/// of its own rather than read as "no failure".
pub fn last_failure(home: &Path) -> Option<ReportFailure> {
    let path = failure_path(home);
    match fs::read(&path) {
        Ok(raw) => match serde_json::from_slice(&raw) {
            Ok(failure) => Some(failure),
            Err(error) => Some(ReportFailure {
                pane_id: String::new(),
                event: String::new(),
                socket_path: String::new(),
                error: format!("{} could not be read back: {error}", path.display()),
                at_unix_ms: 0,
            }),
        },
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use hide_platform::ipc::LocalListener;
    use std::io::{BufRead, BufReader, Write};

    use super::*;
    use crate::runtime::HookEvent;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hide-agent-hooks-report-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        root
    }

    #[test]
    fn a_restored_report_carries_the_version_the_file_recorded() {
        let params = report_params(
            "w7B:pM",
            HOOK_VERSION - 1,
            Counts::Set(PaneCounters::default()),
            Some("session-a"),
        );
        assert_eq!(
            params["tokens"]["hide_hooks"],
            (HOOK_VERSION - 1).to_string()
        );
    }

    #[test]
    fn the_pane_session_is_the_one_herdr_reports_for_its_agent() {
        // A `pane.get` answer of the pinned Herdr 0.9.3 (private server,
        // 2026-10-15), trimmed to the fields around the one read.
        let answer = json!({"type":"pane_info","pane":{"pane_id":"w1:p1","agent":"claude",
            "agent_status":"idle","agent_session":{"source":"herdr:claude","agent":"claude",
            "kind":"id","value":"76e51bb1-98e0-46ff-9b61-e8eeace65386"},"revision":6}});
        assert_eq!(
            session_of(&answer).as_deref(),
            Some("76e51bb1-98e0-46ff-9b61-e8eeace65386")
        );
        // "If no native session reference is stored, the field is omitted."
        let none = json!({"type":"pane_info","pane":{"pane_id":"w1:p1","agent":"claude"}});
        assert_eq!(session_of(&none), None);
        assert_eq!(session_of(&json!({})), None);
    }

    #[test]
    fn a_report_without_counts_names_only_the_version() {
        let params = report_params("w7B:pM", HOOK_VERSION, Counts::Leave, None);
        assert_eq!(
            params["tokens"],
            json!({"hide_hooks": HOOK_VERSION.to_string()})
        );
    }

    #[test]
    fn a_report_can_clear_the_counts_with_null() {
        let params = report_params("w7B:pM", HOOK_VERSION, Counts::Clear, None);
        assert_eq!(
            params["tokens"],
            json!({"hide_hooks": HOOK_VERSION.to_string(), "hide_sub_working": null, "hide_sub_done": null, "hide_sub_session": null})
        );
    }

    #[test]
    fn the_report_names_the_pane_the_source_and_the_counts_owner() {
        let params = report_params(
            "w7B:pM",
            HOOK_VERSION,
            Counts::Set(PaneCounters {
                working: 2,
                done: 5,
            }),
            Some("abc"),
        );
        assert_eq!(params["pane_id"], "w7B:pM");
        assert_eq!(params["source"], "hide-subagents");
        assert_eq!(
            params["tokens"],
            json!({
                "hide_hooks": HOOK_VERSION.to_string(),
                "hide_sub_working": "2",
                "hide_sub_done": "5",
                "hide_sub_session": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            })
        );
    }

    #[test]
    fn ownerless_totals_remove_the_previous_owner_instead_of_leaving_its_proof() {
        for session in [None, Some("-invalid"), Some("")] {
            let params = report_params(
                "w1:p1",
                HOOK_VERSION,
                Counts::Set(PaneCounters::default()),
                session,
            );
            assert_eq!(params["tokens"][SESSION_TOKEN], serde_json::Value::Null);
            assert_eq!(params["tokens"][WORKING_TOKEN], "0");
        }
    }

    #[test]
    fn an_out_of_order_zero_report_republishes_the_new_sessions_owner() {
        use crate::counters::{Reporter, store};
        let home = tempfile::tempdir().unwrap();
        let short_root = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let socket_root = tempfile::Builder::new()
            .prefix("hah-owner-")
            .tempdir_in(short_root)
            .unwrap();
        let socket = socket_root.path().join("herdr.sock");
        let listener = LocalListener::bind(&socket).unwrap();
        let closer = listener.closer();
        let record_home = home.path().to_owned();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            while let Ok(mut stream) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                // B settles while A's equally empty report is in flight.
                if requests.is_empty() {
                    store(
                        &record_home,
                        "w1:p1",
                        PaneCounters::default(),
                        Reporter {
                            agent: Some("opencode"),
                            session: Some("session-b"),
                        },
                    )
                    .unwrap();
                }
                writeln!(
                    stream,
                    "{}",
                    json!({"id": request["id"], "result": {"type": "ok"}})
                )
                .unwrap();
                requests.push(request);
            }
            requests
        });
        let outcome = report_latest(
            home.path(),
            &socket,
            "w1:p1",
            PaneCounters::default(),
            Some("session-a"),
        );
        closer.close();
        let requests = server.join().unwrap();
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(
            requests.len(),
            2,
            "equal numbers do not hide a changed owner"
        );
        assert_eq!(
            requests[0]["params"]["tokens"][SESSION_TOKEN],
            session_digest("session-a").unwrap()
        );
        assert_eq!(
            requests[1]["params"]["tokens"][SESSION_TOKEN],
            session_digest("session-b").unwrap()
        );
    }

    #[test]
    fn the_source_and_token_names_are_ones_herdr_accepts() {
        // Herdr answers `invalid_metadata_source` for anything else; the
        // first hook shipped `hide-subagents@1` and every report was refused.
        assert!(
            metadata_source()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":._-".contains(&byte)),
            "metadata source may contain only ASCII letters, digits, colon, dot, underscore, and hyphen"
        );
        // `PaneReportMetadataParams.tokens` in contracts/herdr-api.schema.json.
        for name in [INSTRUMENTED_TOKEN, WORKING_TOKEN, DONE_TOKEN, SESSION_TOKEN] {
            assert!(
                (1..=32).contains(&name.len())
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
                "{name} does not match ^[A-Za-z0-9_-]{{1,32}}$"
            );
        }
    }

    #[test]
    fn a_report_is_one_pane_report_metadata_request_and_a_success_clears_the_failure() {
        let root = scratch("request");
        // A Unix socket path is capped at SUN_LEN (104 bytes on macOS), and a
        // harness that points TMPDIR into its run directory exceeds it, so the
        // socket alone binds under the short system root. Windows has no
        // such root and no such cap.
        let short_root = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let socket_root =
            short_root.join(format!("hide-agent-hooks-report-{}", std::process::id()));
        fs::create_dir_all(&socket_root).expect("socket directory");
        let socket = socket_root.join("herdr.sock");
        let _ = fs::remove_file(&socket);
        let listener = LocalListener::bind(&socket).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accept");
            let mut line = String::new();
            // One request line; nothing else is sent before the response.
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .expect("read request");
            let request: serde_json::Value = serde_json::from_str(&line).expect("request JSON");
            writeln!(
                stream,
                "{}",
                json!({"id": request["id"], "result": {"type": "ok"}})
            )
            .expect("write response");
            request
        });

        let outcome = report(
            &socket,
            "w1:p1",
            PaneCounters {
                working: 1,
                done: 0,
            },
            Some("abc"),
        );
        assert!(outcome.is_ok(), "{outcome:?}");
        let request = server.join().expect("fake server joins");
        assert_eq!(request["method"], "pane.report_metadata");
        assert_eq!(
            request["params"],
            json!({
                "pane_id": "w1:p1",
                "source": "hide-subagents",
                "tokens": {"hide_hooks": HOOK_VERSION.to_string(), "hide_sub_working": "1", "hide_sub_done": "0", "hide_sub_session": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            })
        );

        let stale = Err(ApiError::Transport("earlier".to_owned()));
        record_outcome(
            &root,
            "w1:p1",
            HookEvent::SessionStart.name(),
            &socket,
            &stale,
        )
        .expect("stale");
        assert!(last_failure(&root).is_some());
        record_outcome(&root, "w1:p1", HookEvent::Stop.name(), &socket, &outcome).expect("record");
        assert_eq!(last_failure(&root), None);
        fs::remove_dir_all(&root).ok();
        fs::remove_file(&socket).ok();
        fs::remove_dir(&socket_root).ok();
    }

    #[test]
    fn a_report_nobody_answers_is_recorded_with_its_pane_event_and_socket() {
        let root = scratch("failure");
        // Under the short system root, as the request test binds its socket:
        // a path past SUN_LEN fails to connect for its length, not because
        // nothing listens there.
        let short_root = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let socket = short_root
            .join(format!("hide-agent-hooks-absent-{}", std::process::id()))
            .join("absent.sock");
        let outcome = report(&socket, "w1:p1", PaneCounters::default(), None);
        assert!(
            matches!(outcome, Err(ApiError::NotRunning(_))),
            "{outcome:?}"
        );

        record_outcome(
            &root,
            "w1:p1",
            HookEvent::SessionStart.name(),
            &socket,
            &outcome,
        )
        .expect("record");
        let failure = last_failure(&root).expect("a failure is remembered");
        assert_eq!(failure.pane_id, "w1:p1");
        assert_eq!(failure.event, "SessionStart");
        assert_eq!(failure.socket_path, socket.display().to_string());
        assert!(failure.error.contains("not running"), "{}", failure.error);
        assert!(failure.message().contains("w1:p1"));

        record_outcome(&root, "w1:p1", HookEvent::Stop.name(), &socket, &Ok(())).expect("clear");
        assert_eq!(last_failure(&root), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unreadable_failure_record_is_a_failure_not_a_clean_bill() {
        let root = scratch("unreadable");
        let path = failure_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not json").unwrap();
        let failure = last_failure(&root).expect("reported");
        assert!(failure.error.contains("could not be read back"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_socket_comes_from_the_environment_or_herdrs_default() {
        // The override is process-global, so this test only checks the
        // default; the override branch is one `var_os` read.
        if std::env::var_os("HERDR_SOCKET_PATH").is_none()
            && std::env::var_os("XDG_CONFIG_HOME").is_none()
            && !cfg!(windows)
        {
            assert_eq!(
                socket_path(Path::new("/Users/example")).unwrap(),
                Path::new("/Users/example/.config/herdr/herdr.sock")
            );
        }
    }
}
