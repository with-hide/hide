//! Which Hide installation answers for a pane: the rule and the procedure
//! around it that a pane's `hide` (`hided`'s `workspace_cli::bootstrap`), its
//! agent hooks ([`super::send`]), this account's daemon (`hided`'s
//! `pane_auth`) and a device's node (`hide_host::panes`) all follow, kept
//! here once (with-hide/hide-private#497 and #575; docs/agent-hooks.md, More
//! than one Hide on an account).
//!
//! A pane names the installation that started it
//! ([`crate::route::INSTALL_VARIABLE`]) and every request from it carries
//! that name ([`PaneRequest`]). A daemon or device node of another
//! installation refuses the request with [`OTHER_INSTALL`] before it proves
//! or runs anything ([`refuse_other`]). An earlier build ignores the name it
//! does not know and binds the pane by its folder, so the answer names the
//! installation that gave it too ([`stamp`]), and the caller takes an answer
//! only when that name is the pane's own ([`confirm`]).
//!
//! The caller asks this account's daemon and this device's bridges in the
//! order the pane's installation calls for ([`order`]). Each node names its
//! installation in its bridge folder ([`declare_bridge`]), so a caller never
//! connects to a bridge that is not the pane's ([`may_answer`]). A hook's
//! command goes to at most one bridge, chosen before it is sent
//! ([`single_bridge`]); a `hide` credential, which runs nothing, asks each
//! remaining bridge and keeps the one answer that confirms the pane
//! ([`settle_bridges`]).

use std::io::{self, Read, Write};
use std::path::Path;

use hide_platform::fs::private;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::HookRequest;
use crate::route::APP_INSTALL;

/// An answer that does not name the installation the request named: a
/// daemon or node from before answers carried one, which ignores the
/// request's `install` and binds the pane by its folder, or one of another
/// installation. What it answered is never used: a reference expires
/// unclaimed with its issuer, a hook's result is dropped.
pub const INSTALL_UNCONFIRMED: &str = "install_unconfirmed";

/// The refusal of a daemon or node of another installation than the one the
/// pane names ([`refuse_other`]), given before it proves the caller or runs
/// anything: no answer about the pane.
pub const OTHER_INSTALL: &str = "other_install";

/// More than one bridge may be the pane's node, so none is asked to run a
/// hook's command, or more than one confirmed a `hide` credential.
pub const AMBIGUOUS_PANE: &str = "ambiguous_pane";

/// The file in a node's bridge folder that names the node's installation
/// ([`declare_bridge`]).
pub const BRIDGE_INSTALL_FILE: &str = "install";

/// The most bytes a bridge's installation file holds; an installation name
/// is at most 4 KiB (`route`).
const DECLARATION_LIMIT: u64 = 8 * 1024;

/// One request from a pane to this account's daemon (its bootstrap socket)
/// or to a device's node (its bridge): a credential for `hide`, or with
/// `hook` a hook's command answered at once. The pane id may be empty for a
/// caller that has none, whom the daemon or node may bind by its folder.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaneRequest {
    #[serde(default)]
    pub pane_id: String,
    pub nonce: String,
    #[serde(default)]
    pub one_shot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookRequest>,
    /// The Hide installation the caller's pane names (`HIDE_INSTALL`), when
    /// it names one: only that installation's daemon or node answers for it.
    #[serde(default)]
    pub install: Option<String>,
}

/// The refusal a daemon or node of installation `own` gives `request` before
/// it proves the caller or runs anything: a pane another installation
/// started is that installation's, even when its folder is a checkout this
/// one also serves, and binding it here by its folder would run its calls
/// against another Hide.
pub fn refuse_other(request: &PaneRequest, own: Option<&str>) -> Result<(), &'static str> {
    match request.install.as_deref() {
        Some(install) if own != Some(install) => Err(OTHER_INSTALL),
        _ => Ok(()),
    }
}

/// Names installation `own` in `answer`, a reference's or a hook's, which a
/// caller whose pane names one requires ([`confirm`]).
pub fn stamp(answer: &mut Value, own: Option<&str>) {
    if let Some(fields) = answer.as_object_mut() {
        fields.insert("install".to_owned(), json!(own));
    }
}

/// Whether the `ok` answer `answer` counts for a pane naming `install`: it
/// names that same installation. A pane that names none (an agent outside Hide, a
/// plain terminal) takes any answer, naming one or not.
pub fn confirm(answer: &Value, install: Option<&str>) -> Result<(), &'static str> {
    match install {
        Some(install) if answer["install"].as_str() != Some(install) => Err(INSTALL_UNCONFIRMED),
        _ => Ok(()),
    }
}

/// Where a pane's caller asks, without a reference that names its route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ask {
    /// This account's daemon, by the bootstrap socket its state folder names.
    Daemon,
    /// The device bridges in this account's state folder, one per node.
    Bridges,
}

/// The order a pane naming `install` asks in. A pane another installation's
/// node started, a device helper's, is that node's, and this account's own
/// daemon would bind it by its folder alone, so it asks the bridges first; a
/// pane the desktop app started, or one that names no installation, asks the
/// daemon first.
pub fn order(install: Option<&str>) -> [Ask; 2] {
    if install.is_some_and(|install| install != APP_INSTALL) {
        [Ask::Bridges, Ask::Daemon]
    } else {
        [Ask::Daemon, Ask::Bridges]
    }
}

/// What a bridge folder says about its node's installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Declared {
    /// The node names its installation, or `None` for a build run from
    /// anywhere else, whose panes name none.
    Install(Option<String>),
    /// No readable declaration: a node from before bridges declared one.
    Unknown,
}

/// Names installation `own` in the bridge folder `folder`, before the node
/// binds its socket there, so a live bridge always says whose it is.
pub fn declare_bridge(folder: &Path, own: Option<&str>) -> io::Result<()> {
    let mut file = private::create_new_file(&folder.join(BRIDGE_INSTALL_FILE))?;
    file.write_all(json!({ "install": own }).to_string().as_bytes())
}

/// What the bridge folder `folder` declares. A file that is missing, not
/// this account's own, too long or not the declaration is [`Declared::Unknown`],
/// which may still be the pane's and is counted so.
pub fn bridge_declared(folder: &Path) -> Declared {
    #[derive(Deserialize)]
    struct Declaration {
        install: Option<String>,
    }
    let Ok(file) = private::open_own_file(&folder.join(BRIDGE_INSTALL_FILE), false) else {
        return Declared::Unknown;
    };
    let mut bytes = Vec::new();
    if file
        .take(DECLARATION_LIMIT + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > DECLARATION_LIMIT
    {
        return Declared::Unknown;
    }
    serde_json::from_slice::<Declaration>(&bytes)
        .map(|declaration| Declared::Install(declaration.install))
        .unwrap_or(Declared::Unknown)
}

/// Whether a bridge that declares `declared` may answer for a pane naming
/// `install`. A pane that names none takes any node. A pane that names one
/// is never the pane of a node declaring another, or none: such a node is
/// not asked at all. A node that declares nothing may be the pane's.
pub fn may_answer(declared: &Declared, install: Option<&str>) -> bool {
    match (declared, install) {
        (_, None) | (Declared::Unknown, Some(_)) => true,
        (Declared::Install(own), Some(install)) => own.as_deref() == Some(install),
    }
}

/// The one bridge a hook's command goes to, from those that may answer for
/// its pane ([`may_answer`]): a command runs where it lands, so with more
/// than one it goes to none ([`AMBIGUOUS_PANE`]), and with none the caller
/// asks its next route.
pub fn single_bridge<T>(mut bridges: Vec<T>) -> Result<Option<T>, &'static str> {
    if bridges.len() > 1 {
        return Err(AMBIGUOUS_PANE);
    }
    Ok(bridges.pop())
}

/// A `hide` credential from the answers of the bridges that may answer for
/// its pane, read in turn: the one confirmed answer, [`AMBIGUOUS_PANE`] at a
/// second, or the last refusal of a node that may be the pane's. A node of
/// another installation ([`OTHER_INSTALL`]) is not the pane's, so its refusal
/// is no answer and never replaces the pane's own installation's reason.
pub fn settle_bridges<T>(
    answers: impl IntoIterator<Item = Result<T, String>>,
) -> Result<Option<T>, String> {
    let mut success = None;
    let mut reason = None;
    for answer in answers {
        match answer {
            Ok(found) => {
                if success.is_some() {
                    // A bridge may return a persistent reference already held
                    // by a live pane. Ambiguity cannot revoke either holder.
                    return Err(AMBIGUOUS_PANE.to_owned());
                }
                success = Some(found);
            }
            Err(error) if error == OTHER_INSTALL => {}
            Err(error) => reason = Some(error),
        }
    }
    match (success, reason) {
        (Some(found), _) => Ok(Some(found)),
        (None, Some(reason)) => Err(reason),
        (None, None) => Ok(None),
    }
}

/// What the caller can do about a refusal this rule gives, for a `hide` or a
/// node that names the next step.
pub fn next_action(reason: &str) -> Option<&'static str> {
    match reason {
        INSTALL_UNCONFIRMED => Some(
            "An earlier build of Hide answered and cannot confirm this pane is its own: quit and reopen the Hide that started this pane so it runs its current build, then retry the same command",
        ),
        OTHER_INSTALL => Some(
            "Only another Hide on this account answered, and this pane belongs to the Hide that started it: open that Hide, or reconnect this device in it, then retry the same command",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_or_node_refuses_only_another_installations_pane() {
        let request = |install: Option<&str>| PaneRequest {
            pane_id: "w1:p1".to_owned(),
            nonce: "0".repeat(32),
            one_shot: false,
            hook: None,
            install: install.map(str::to_owned),
        };
        assert_eq!(refuse_other(&request(Some("app")), Some("app")), Ok(()));
        assert_eq!(refuse_other(&request(None), Some("app")), Ok(()));
        assert_eq!(refuse_other(&request(None), None), Ok(()));
        assert_eq!(
            refuse_other(&request(Some("root:/helper")), Some("app")),
            Err(OTHER_INSTALL)
        );
        assert_eq!(
            refuse_other(&request(Some("app")), None),
            Err(OTHER_INSTALL)
        );
    }

    #[test]
    fn a_device_pane_asks_its_bridges_first() {
        assert_eq!(order(Some("root:/helper")), [Ask::Bridges, Ask::Daemon]);
        assert_eq!(order(Some(APP_INSTALL)), [Ask::Daemon, Ask::Bridges]);
        assert_eq!(order(None), [Ask::Daemon, Ask::Bridges]);
    }

    /// A bridge folder says whose node it is, and a pane that names an
    /// installation asks only a node of that one or one that says nothing.
    #[test]
    fn a_bridge_declares_its_installation_and_answers_only_for_it() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(bridge_declared(folder.path()), Declared::Unknown);
        declare_bridge(folder.path(), Some("root:/helper")).unwrap();
        let declared = bridge_declared(folder.path());
        assert_eq!(declared, Declared::Install(Some("root:/helper".to_owned())));
        assert!(may_answer(&declared, Some("root:/helper")));
        assert!(!may_answer(&declared, Some(APP_INSTALL)));
        assert!(may_answer(&declared, None));

        let none = tempfile::tempdir().unwrap();
        declare_bridge(none.path(), None).unwrap();
        assert_eq!(bridge_declared(none.path()), Declared::Install(None));
        assert!(!may_answer(&Declared::Install(None), Some(APP_INSTALL)));
        assert!(may_answer(&Declared::Unknown, Some(APP_INSTALL)));

        // A damaged declaration is no declaration.
        let damaged = tempfile::tempdir().unwrap();
        std::fs::write(damaged.path().join(BRIDGE_INSTALL_FILE), b"{").unwrap();
        assert_eq!(bridge_declared(damaged.path()), Declared::Unknown);
    }

    #[test]
    fn a_hooks_command_goes_to_one_bridge_or_none() {
        assert_eq!(single_bridge::<u8>(vec![]), Ok(None));
        assert_eq!(single_bridge(vec![1]), Ok(Some(1)));
        assert_eq!(single_bridge(vec![1, 2]), Err(AMBIGUOUS_PANE));
    }

    /// Another installation's refusal never makes or breaks the pane's
    /// answer, in any order.
    #[test]
    fn another_installations_refusal_is_no_bridge_answer() {
        let other = || Err(OTHER_INSTALL.to_owned());
        let unconfirmed = || Err(INSTALL_UNCONFIRMED.to_owned());
        let found = || Ok("reference");
        assert_eq!(
            settle_bridges([unconfirmed(), other()]),
            Err(INSTALL_UNCONFIRMED.to_owned())
        );
        assert_eq!(
            settle_bridges([other(), unconfirmed()]),
            Err(INSTALL_UNCONFIRMED.to_owned())
        );
        assert_eq!(settle_bridges::<&str>([other(), other()]), Ok(None));
        assert_eq!(
            settle_bridges([other(), found(), unconfirmed()]),
            Ok(Some("reference"))
        );
        assert_eq!(
            settle_bridges([found(), other(), found()]),
            Err(AMBIGUOUS_PANE.to_owned())
        );
    }

    #[test]
    fn each_install_refusal_has_its_own_next_step() {
        assert!(next_action(OTHER_INSTALL).is_some_and(|action| action.contains("another Hide")));
        assert!(
            next_action(INSTALL_UNCONFIRMED).is_some_and(|action| action.contains("earlier build"))
        );
        assert_ne!(next_action(OTHER_INSTALL), next_action(INSTALL_UNCONFIRMED));
        assert_eq!(next_action("pane_not_found"), None);
    }
}
