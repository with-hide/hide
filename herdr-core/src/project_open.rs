//! `hide <path>`: a folder opened as a project from a terminal, the way
//! `code .` opens VS Code (docs/UI_BEHAVIOR.md, Opening a folder from a
//! terminal).
//!
//! The folder is registered through the same `create_workspace` the Add a
//! project dialog sends, judged by the same rules, unless a registered
//! checkout already holds it; either way the project comes forward. The
//! caller's machine is the credential's, never a field of the request.

use serde::Serialize;

/// Where one `hide <path>` stands, read by its request id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectOpenReceipt {
    /// The registration is on its way.
    Pending,
    /// The project is in front: `path` is the checkout that came forward,
    /// `checkout_id` its id when the catalog lists it (a device's new
    /// project is listed only once its helper confirms the folder), `added`
    /// whether this request registered it.
    Opened {
        label: String,
        path: String,
        checkout_id: Option<String>,
        added: bool,
    },
    /// The registration was refused; `reason` is the core's error kind,
    /// spelled with `_` for `.` as every `hide` reason is.
    Refused { reason: String },
}

/// The longest path a request may name, as the wire takes it.
pub const PATH_LIMIT: usize = 4096;

/// Whether `path` is one a request may carry: absolute in the wire
/// spelling, bounded, and with no control character.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_LIMIT
        && hide_platform::path::is_wire_absolute(path)
        && !path.chars().any(char::is_control)
}

/// What the caller does about a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "not_found" => "Check the path: no folder is there",
        "not_a_directory" => "Give a folder, not a file",
        "outside_home" | "needs_confirmation" | "workspace_create_refused_needs_confirmation" => {
            "Add it from Hide's Add a project, which asks before adding a folder outside your home folder"
        }
        "home_root" => "Give a project folder inside your home folder, not the home folder itself",
        "project_holds_home" | "workspace_create_refused_project_holds_home" => {
            "This folder is in a Git repository that holds your whole home folder; give a folder outside it"
        }
        "invalid_path" => "Check the path and its permissions, then retry",
        "workspace_create_refused_not_found" => "Check the path on that device: no folder is there",
        "workspace_create_refused_not_a_directory" => "Give a folder on that device, not a file",
        "workspace_remove_in_flight" => "Wait for the project's panes to close, then run it again",
        "workspace_control_unavailable" | "workspace_create_unavailable" => {
            "Check that Hide's Herdr server, or the device's connection, is running, then retry"
        }
        "project_open_busy" => "Wait for the other folders being added, then retry",
        "project_open_unconfirmed" => {
            "Run it again: a folder already added is only brought forward, never added twice"
        }
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_carries_only_an_absolute_bounded_path() {
        assert!(valid_path("/home/me/app"));
        assert!(valid_path("C:/work/app"));
        assert!(!valid_path("app"));
        assert!(!valid_path("./app"));
        assert!(!valid_path(""));
        assert!(!valid_path("/home/me/a\nb"));
        assert!(!valid_path(&format!("/{}", "a".repeat(PATH_LIMIT))));
    }
}
