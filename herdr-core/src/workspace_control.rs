//! Pane-scoped, daemon-only Workspace command contract. The core remains the
//! sole owner of membership and View state; the transport proves the caller.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::changes::{self, ChangesRequest, ChannelRef};
use crate::files::{self, DocumentPlace, DocumentRoot, OpenFailure};
use crate::model::EditorDocumentSnapshot;
use crate::node_access::NodeLink;

pub use crate::view_layout::Edge;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Query {
    Info,
    ViewList,
}

/// Who is asking, as the transport proved it. hided hands the core one string
/// in the pane-id slot: a pane descendant's pane id as Herdr reports it, or,
/// for a local process it could only bind to a checkout by its kernel-reported
/// cwd, the encoded form `checkout:<key>:<canonical path>`. `key` is unique per
/// issued capability so two callers in one checkout never share a retry record.
/// A desktop window's browser gateway, acting for a CDP client on a page in
/// that checkout, is `gateway:<key>:<canonical path>`, its key one per window.
/// Herdr pane ids are `w..:p..` and remote ones `remote:<device>:pane:<id>`, so
/// neither prefix can collide with a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Caller<'a> {
    Pane(&'a str),
    Checkout {
        key: &'a str,
        path: &'a str,
        gateway: bool,
    },
}

const CHECKOUT_CALLER_PREFIX: &str = "checkout:";
const GATEWAY_CALLER_PREFIX: &str = "gateway:";

impl<'a> Caller<'a> {
    pub fn parse(id: &'a str) -> Self {
        id.strip_prefix(CHECKOUT_CALLER_PREFIX)
            .map(|rest| (rest, false))
            .or_else(|| {
                id.strip_prefix(GATEWAY_CALLER_PREFIX)
                    .map(|rest| (rest, true))
            })
            .and_then(|(rest, gateway)| {
                let (key, path) = rest.split_once(':')?;
                Some((key, path, gateway))
            })
            .filter(|(key, path, _)| {
                !key.is_empty()
                    && key.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    && hide_platform::path::is_wire_absolute(path)
            })
            .map_or(Caller::Pane(id), |(key, path, gateway)| Caller::Checkout {
                key,
                path,
                gateway,
            })
    }
}

/// The caller id hided records for a checkout-bound capability.
pub fn checkout_caller_id(key: &str, canonical_path: &str) -> String {
    format!("{CHECKOUT_CALLER_PREFIX}{key}:{canonical_path}")
}

/// The caller id a desktop window's browser gateway acts as on a checkout.
pub fn gateway_caller_id(key: &str, canonical_path: &str) -> String {
    format!("{GATEWAY_CALLER_PREFIX}{key}:{canonical_path}")
}

/// One pane request is one core transition. The transport supplies the pane
/// identity; no Workspace selector is accepted from the caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    OpenFile {
        path: String,
        beside: bool,
    },
    OpenDiff {
        path: String,
        beside: bool,
    },
    OpenBrowser {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area_id: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        new_target: bool,
        /// Open to the side, as a file or diff `--beside` does.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        beside: bool,
    },
    Select {
        view_id: String,
        /// When supplied by a browser capability, the target must still be
        /// a browser display in this area at commit time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_browser_area: Option<String>,
    },
    Split {
        view_id: String,
        area_id: String,
        edge: Edge,
    },
    Move {
        view_id: String,
        area_id: String,
        index: usize,
    },
    Close {
        view_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_browser_area: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionResult {
    pub context: Context,
    pub request_id: String,
    pub changed: bool,
    pub view_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Context {
    pub device_id: String,
    pub workspace_id: String,
    pub checkout_id: String,
    pub checkout_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct View {
    pub area_id: String,
    pub view_id: String,
    pub kind: &'static str,
    pub target: String,
    pub selected: bool,
    pub active_area: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<BrowserPage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserPage {
    pub load: u64,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// The current browser target an authenticated desktop host may resolve.
/// It is read from core-owned View state, never accepted from an IPC caller;
/// a node whose core runs elsewhere reads it from that core over the link.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserRouteSource {
    pub device_id: String,
    pub checkout_path: String,
    pub url: String,
    pub load: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QueryResult {
    pub context: Context,
    pub capabilities: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<Vec<View>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Refusal {
    pub reason: &'static str,
    pub next_action: &'static str,
}

/// The owner thread supplies a confined host source and releases its lock.
/// The daemon reads it on a request worker before asking the owner to commit.
pub struct ActionSource {
    root: DocumentRoot,
    channel: Arc<dyn NodeLink>,
    path: String,
    kind: SourceKind,
    already_open: bool,
}

enum SourceKind {
    File,
    Diff,
    Browser,
}

pub struct ActionMaterial {
    pub(crate) file: Option<(EditorDocumentSnapshot, DocumentPlace)>,
    pub(crate) path: String,
}

pub enum ActionPreparation {
    Cached(Result<ActionResult, Refusal>),
    Ready,
    Read(ActionSource),
}

impl ActionSource {
    pub(crate) fn file(
        root: DocumentRoot,
        channel: Arc<dyn NodeLink>,
        path: String,
        already_open: bool,
    ) -> Self {
        Self {
            root,
            channel,
            path,
            kind: SourceKind::File,
            already_open,
        }
    }

    pub(crate) fn diff(root: DocumentRoot, channel: Arc<dyn NodeLink>, path: String) -> Self {
        Self {
            root,
            channel,
            path,
            kind: SourceKind::Diff,
            already_open: false,
        }
    }

    pub(crate) fn browser(root: DocumentRoot, channel: Arc<dyn NodeLink>, path: String) -> Self {
        Self {
            root,
            channel,
            path,
            kind: SourceKind::Browser,
            already_open: false,
        }
    }

    pub fn read(self) -> Result<ActionMaterial, Refusal> {
        let relative = files::relative_in_root(self.channel.as_ref(), &self.root.path, &self.path)
            .map_err(|_| Refusal {
                reason: "path_outside_checkout",
                next_action: "Choose a path inside the calling pane's checkout",
            })?;
        let path = hide_platform::path::wire_join(&self.root.path, &relative);
        if matches!(self.kind, SourceKind::Browser) {
            let extension = std::path::Path::new(&path)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("");
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            ) {
                return Err(Refusal {
                    reason: "not_html",
                    next_action: "Choose a readable HTML file inside this checkout",
                });
            }
            files::open_document(self.channel.as_ref(), &self.root, &path).map_err(|_| {
                Refusal {
                    reason: "html_unavailable",
                    next_action: "Check that the HTML file is readable and retry",
                }
            })?;
            return Ok(ActionMaterial { file: None, path });
        }
        if matches!(self.kind, SourceKind::File) {
            if self.already_open {
                return Ok(ActionMaterial { file: None, path });
            }
            let opened = files::open_document(self.channel.as_ref(), &self.root, &path).map_err(
                |failure| match failure {
                    OpenFailure::Missing => Refusal {
                        reason: "file_missing",
                        next_action: "Check the path and retry",
                    },
                    OpenFailure::Failed(_) => Refusal {
                        reason: "file_unavailable",
                        next_action: "Check checkout access and file permissions, then retry",
                    },
                },
            )?;
            return Ok(ActionMaterial {
                file: Some(opened),
                path,
            });
        }
        let (changes, _) = changes::read(&ChangesRequest {
            root_path: self.root.path.clone(),
            root: self.root,
            channel: Ok(ChannelRef(self.channel)),
            selected_path: Some(path.clone()),
            selected_committed: false,
            base_branch: None,
            diffs: Vec::new(),
        });
        if changes.unavailable_reason.is_some() {
            return Err(Refusal {
                reason: "diff_unavailable",
                next_action: "Check Git access on this device and retry",
            });
        }
        if !changes.entries.iter().any(|entry| entry.path == path) {
            return Err(Refusal {
                reason: "diff_unchanged",
                next_action: "Choose a file with a working-tree change and retry",
            });
        }
        if changes.diff.is_none() {
            return Err(Refusal {
                reason: "diff_unavailable",
                next_action: "Refresh the changed file and retry",
            });
        }
        Ok(ActionMaterial { file: None, path })
    }
}

/// Decode a file URL without accepting an authority or an encoded NUL.
pub fn local_file_path(url: &str) -> Option<String> {
    if !url.get(..7)?.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = &url[7..];
    let (host, located) = rest.split_at(rest.find('/')?);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let path = located.split(['?', '#']).next()?;
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    (decoded.starts_with('/') && !decoded.contains('\0')).then(|| decoded.into_owned())
}

#[cfg(test)]
mod tests {
    use super::{Action, Caller, checkout_caller_id, gateway_caller_id, local_file_path};

    #[test]
    fn legacy_browser_open_decodes_and_serializes_without_target_options() {
        let legacy = serde_json::json!({
            "action": "open_browser",
            "url": "https://example.test/page",
        });
        let action: Action = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            action,
            Action::OpenBrowser {
                url: "https://example.test/page".into(),
                area_id: None,
                new_target: false,
                beside: false,
            }
        );
        assert_eq!(serde_json::to_value(action).unwrap(), legacy);
    }

    #[test]
    fn browser_open_target_options_round_trip() {
        let action = Action::OpenBrowser {
            url: "https://example.test/page".into(),
            area_id: Some("a1".into()),
            new_target: true,
            beside: false,
        };
        let wire = serde_json::to_value(&action).unwrap();
        assert_eq!(wire["area_id"], "a1");
        assert_eq!(wire["new_target"], true);
        assert_eq!(serde_json::from_value::<Action>(wire).unwrap(), action);
    }

    #[test]
    fn browser_scope_is_additive_to_legacy_select_and_close_wire_actions() {
        for legacy in [
            serde_json::json!({"action": "select", "view_id": "v2"}),
            serde_json::json!({"action": "close", "view_id": "v2"}),
        ] {
            let action: Action = serde_json::from_value(legacy.clone()).unwrap();
            assert!(matches!(
                &action,
                Action::Select {
                    expected_browser_area: None,
                    ..
                } | Action::Close {
                    expected_browser_area: None,
                    ..
                }
            ));
            assert_eq!(serde_json::to_value(&action).unwrap(), legacy);
            let mut scoped = legacy;
            scoped["expected_browser_area"] = serde_json::json!("a1");
            let action: Action = serde_json::from_value(scoped.clone()).unwrap();
            assert_eq!(serde_json::to_value(action).unwrap(), scoped);
        }
    }

    #[test]
    fn caller_ids_round_trip_and_pane_ids_are_left_alone() {
        let id = checkout_caller_id("0123abcd", "/srv/project");
        assert_eq!(
            Caller::parse(&id),
            Caller::Checkout {
                key: "0123abcd",
                path: "/srv/project",
                gateway: false
            }
        );
        let colon = checkout_caller_id("k", "/mnt/a:b/c");
        assert_eq!(
            Caller::parse(&colon),
            Caller::Checkout {
                key: "k",
                path: "/mnt/a:b/c",
                gateway: false
            }
        );
        // A checkout on a Windows screen machine, in the spelling between
        // machines, is a checkout caller as the attestation that made it is.
        let windows = checkout_caller_id("k", "C:/work/app");
        assert_eq!(
            Caller::parse(&windows),
            Caller::Checkout {
                key: "k",
                path: "C:/work/app",
                gateway: false
            }
        );
        // A window's gateway is a checkout caller that says so.
        let gateway = gateway_caller_id("k", "/srv/project");
        assert_eq!(
            Caller::parse(&gateway),
            Caller::Checkout {
                key: "k",
                path: "/srv/project",
                gateway: true
            }
        );
        assert_eq!(Caller::parse("w8P:pM"), Caller::Pane("w8P:pM"));
        assert_eq!(
            Caller::parse("remote:mini:pane:w1:p2"),
            Caller::Pane("remote:mini:pane:w1:p2")
        );
        assert_eq!(Caller::parse("checkout:"), Caller::Pane("checkout:"));
        assert_eq!(
            Caller::parse("checkout:k:relative"),
            Caller::Pane("checkout:k:relative")
        );
        assert_eq!(
            Caller::parse("gateway:k:relative"),
            Caller::Pane("gateway:k:relative")
        );
    }

    #[test]
    fn file_address_scheme_is_case_insensitive_at_the_workspace_boundary() {
        assert_eq!(
            local_file_path("FILE:///checkout/page.html"),
            Some("/checkout/page.html".to_owned())
        );
        assert_eq!(
            local_file_path("FiLe://localhost/checkout/page.html"),
            Some("/checkout/page.html".to_owned())
        );
        assert_eq!(local_file_path("FILE://elsewhere/checkout/page.html"), None);
    }
}
