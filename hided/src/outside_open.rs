//! An agent's `hide file open` or `hide browser open` of a file outside every
//! registered checkout (PRD outside-path-in-hide, extended to agents). The
//! core never holds such a path (AGENTS.md), so hided, which has it from the
//! agent's own command, hands it to the desktop window on this machine, which
//! grants it as it grants a file the operator clicks, and the core receives
//! the grant alone in an `open_external` action from the same caller. That
//! action then takes the same prepare and commit as any open, so its tab is
//! recorded with its opener and waits for the operator as theirs do.
//!
//! The path passes through and is neither kept nor logged here.

use herdr_core::workspace_control::{Action, ExternalKind};
use serde::Deserialize;
use serde_json::Value;

use crate::boundary::Boundary;
use crate::browser_control::{BrowserControl, Failure};

/// The file a caller's open names outside every registered checkout of this
/// machine, in its wire spelling, and whether it asks for a page; None for
/// any other action, and for a path written under a checkout, which the core
/// opens or refuses as before. The roots follow the core only while a window
/// is open, so a path no root holds brings them current (`catch_up`) once
/// before it is called outside.
pub(crate) fn outside_target(
    action: &Action,
    boundary: &Boundary,
    catch_up: impl FnOnce() -> bool,
) -> Option<(String, bool)> {
    let (path, page) = match action {
        Action::OpenFile { path } => (
            hide_platform::path::to_wire(std::path::Path::new(path)).ok()?,
            false,
        ),
        // A page's own new target, bound to its area, is a gateway's and
        // never an outside file.
        Action::OpenBrowser {
            url,
            area_id: None,
            new_target: false,
        } => (crate::file_url::file_path(url)?.0, true),
        _ => return None,
    };
    if boundary.written_under_root(&path) || (catch_up() && boundary.written_under_root(&path)) {
        return None;
    }
    Some((path, page))
}

#[derive(Deserialize)]
struct Granted {
    id: String,
    name: String,
    kind: ExternalKind,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Answer {
    Grant { grant: Granted },
    Refused { refused: String },
}

/// The `open_external` action for `path`, once this machine's window has
/// granted it, or the refusal its caller acts on.
pub(crate) fn grant(control: &BrowserControl, path: &str, page: bool) -> Result<Action, Failure> {
    let answer: Value = control.external_open(path, page)?;
    match serde_json::from_value::<Answer>(answer) {
        Ok(Answer::Grant { grant }) => Ok(Action::OpenExternal {
            grant: grant.id,
            name: grant.name,
            kind: grant.kind,
        }),
        Ok(Answer::Refused { refused }) => Err(refusal(&refused)),
        Err(_) => Err(refusal("")),
    }
}

/// What the agent reads for the window's refusal `reason`.
fn refusal(reason: &str) -> Failure {
    match reason {
        "file_missing" => ("file_missing", "Check the path and retry"),
        "file_unavailable" => (
            "file_unavailable",
            "Check that the file is readable, then retry",
        ),
        "unsupported_kind" => (
            "unsupported_kind",
            "Open an HTML page, an image, a PDF, a video or a text file; Hide shows no folder, audio or program",
        ),
        "not_html" => ("not_html", "Use hide file open for a file that is not HTML"),
        "outside_files_full" => (
            "outside_files_full",
            "Ask the operator to close an outside file's tab in Hide, then retry",
        ),
        "invalid_path" => ("invalid_path", "Use an absolute path and retry"),
        _ => (
            "browser_control_unavailable",
            "Reconnect the Hide desktop app and retry",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boundary with one registered checkout, `home/repo`.
    fn rooted() -> (tempfile::TempDir, Boundary, String) {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        boundary.set_roots(vec![crate::boundary::Root {
            workspace_id: "w".into(),
            checkout_id: "c".into(),
            path: repo.clone(),
        }]);
        let repo = hide_platform::path::to_wire(&repo).unwrap();
        (home, boundary, repo)
    }

    #[test]
    fn only_an_open_written_outside_every_checkout_goes_to_the_window() {
        let (_home, boundary, repo) = rooted();
        let outside = format!("{}/scratch/r.png", repo.trim_end_matches("/repo"));
        let file = |path: &str| Action::OpenFile { path: path.into() };
        let page = |path: &str, area_id: Option<&str>| Action::OpenBrowser {
            url: crate::file_url::file_url(path, ""),
            area_id: area_id.map(str::to_owned),
            new_target: false,
        };
        let target = |action: &Action| outside_target(action, &boundary, || false);
        assert_eq!(target(&file(&outside)), Some((outside.clone(), false)));
        assert_eq!(target(&page(&outside, None)), Some((outside.clone(), true)));
        // A checkout's own file, even one that is not there, stays the core's.
        assert_eq!(target(&file(&format!("{repo}/gone.png"))), None);
        assert_eq!(target(&page(&format!("{repo}/a.html"), None)), None);
        // A page's own target, a web address and a diff are never outside files.
        assert_eq!(target(&page(&outside, Some("a1"))), None);
        let web = Action::OpenBrowser {
            url: "https://example.test/".into(),
            area_id: None,
            new_target: false,
        };
        assert_eq!(target(&web), None);
        assert_eq!(
            target(&Action::OpenDiff {
                path: outside.clone()
            }),
            None
        );
        // The roots are brought current before a path is called outside.
        let mut caught_up = false;
        assert!(
            outside_target(&file(&outside), &boundary, || {
                caught_up = true;
                true
            })
            .is_some()
        );
        assert!(caught_up);
    }

    #[test]
    fn a_window_answer_becomes_a_grant_or_a_named_refusal() {
        let granted: Answer = serde_json::from_value(serde_json::json!({
            "grant": {"id": "a".repeat(32), "name": "r.png", "kind": "image"}
        }))
        .unwrap();
        assert!(matches!(granted, Answer::Grant { grant } if grant.kind == ExternalKind::Image));
        for reason in [
            "file_missing",
            "file_unavailable",
            "unsupported_kind",
            "not_html",
            "outside_files_full",
        ] {
            assert_eq!(refusal(reason).0, reason);
        }
        assert_eq!(refusal("something new").0, "browser_control_unavailable");
    }
}
