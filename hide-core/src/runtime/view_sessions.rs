//! The sessions behind a checkout's Views (PRD session-files): a View keeps
//! every session that opened it (`Display::openers`) and whether the
//! operator did (`Display::operator`). Which Views are in front of the
//! operator is each Agent tab's tray's (PRD lane-desk-pair-view D-05), not
//! the keyboard's. A session's Views close with its pane or its archive,
//! never with a sleep or an agent that exits to its shell (D-14, D-27).

use std::collections::{HashSet, VecDeque};

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::model::{SessionDraftSnapshot, SessionDraftsSnapshot};
use crate::view_feedback::SessionScope;
use crate::view_layout::Display;
use crate::workspace_views::Place;

/// Device pane and tab closes waiting for their device's answer; past it
/// the oldest is forgotten and its Views stay open.
const DEVICE_CLOSES: usize = 32;

/// Device closes waiting for their device's answer. Runtime only.
#[derive(Default)]
pub(super) struct ViewSessions {
    device_closes: VecDeque<DeviceClose>,
}

struct DeviceClose {
    target: String,
    request: String,
    scopes: Vec<SessionScope>,
}

impl Runtime {
    /// The session `pane_id` stands for, on whichever machine it runs: its
    /// terminal and the native session of the agent in it. A pane Hide holds
    /// no terminal for stands for none.
    pub(super) fn session_scope_of_pane(&self, pane_id: &str) -> Option<SessionScope> {
        let device = crate::address::pane_device(pane_id)
            .unwrap_or(self.node.as_str())
            .to_owned();
        let terminal_id = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == pane_id)?
            .terminal_id
            .clone()?;
        let session = self
            .machine_agents(&device)
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .and_then(|agent| agent.lineage_session.clone());
        Some(SessionScope {
            device_id: device,
            terminal_id,
            session,
        })
    }

    /// The sessions whose Views a close of `pane_ids` takes with it once it
    /// completes: every one of an ordinary close or an archive's, none of a
    /// sleep's, which only Hide asks for and which keeps them (D-14, D-27).
    pub(super) fn closing_view_sessions(
        &self,
        request: &crate::recent_closed::capture::CloseCaptureRequest,
        pane_ids: &[String],
    ) -> Vec<SessionScope> {
        let archive = pane_ids.iter().any(|pane| self.archive_closes_pane(pane));
        if !request.retain_for_reopen && !archive {
            return Vec::new();
        }
        self.session_scopes_of_panes(pane_ids)
    }

    /// The sessions of `pane_ids`.
    fn session_scopes_of_panes(&self, pane_ids: &[String]) -> Vec<SessionScope> {
        pane_ids
            .iter()
            .filter_map(|pane| self.session_scope_of_pane(pane))
            .collect()
    }

    /// The panes of the checkout `key`, as the catalog lists them.
    fn view_checkout_panes(&self, key: &WorkspaceKey) -> Vec<String> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == key.1)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .map(|pane| pane.id.clone())
            .collect()
    }

    /// The sessions `scopes` ended, by their pane closing or their archive
    /// (D-14): each View forgets them as openers, and a View no other session
    /// and not the operator opened closes. One whose document is the last to
    /// hold unsaved work stays, as the operator's, so a close the operator
    /// was not asked about never drops text.
    pub(super) fn close_session_views(&mut self, scopes: &[SessionScope]) {
        if scopes.is_empty() {
            return;
        }
        let ended = |opener: &crate::view_feedback::Opener| {
            scopes.iter().any(|scope| opener.of_session(scope))
        };
        let trees: Vec<(WorkspaceKey, Place)> = self
            .every_view_layout()
            .filter(|(key, _, layout)| {
                scopes.iter().any(|scope| scope.device_id == key.0)
                    && layout.displays().any(|d| d.openers.iter().any(ended))
            })
            .map(|(key, place, _)| (key, place))
            .collect();
        for (key, place) in trees {
            let forgotten = self.change_view_layout(&key, &place, |layout, _| {
                let ids: Vec<String> = layout
                    .displays()
                    .filter(|display| display.openers.iter().any(ended))
                    .map(|display| display.id.clone())
                    .collect();
                let mut alone = Vec::new();
                for id in &ids {
                    let display = layout.display_mut(id).expect("listed above");
                    display.openers.retain(|opener| !ended(opener));
                    if display.openers.is_empty() && !display.operator {
                        alone.push(id.clone());
                    }
                }
                Ok((alone, !ids.is_empty()))
            });
            let alone = match forgotten {
                Ok(alone) => alone,
                Err(error) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "view_sessions",
                        "kind": "view_sessions.close_failed",
                        "device": key.0,
                        "error": error.kind(),
                    }));
                    continue;
                }
            };
            let (mut closed, mut kept) = (0, 0);
            for id in alone {
                if self.holds_last_unsaved(&key, &id) {
                    let _ = self.change_view_layout(&key, &place, |layout, _| {
                        let display = layout.display_mut(&id).ok_or_else(|| {
                            crate::view_layout::LayoutError::UnknownDisplay(id.clone())
                        })?;
                        display.operator = true;
                        Ok(((), true))
                    });
                    // The operator's now, it goes where the operator's
                    // Views are: on a desk, never on no desk with no pane to
                    // list it (PRD lane-desk-pair-view D-05).
                    if place == Place::Shelf {
                        let front = self.front_place(&key);
                        self.bring_view_to(&key, &id, &front);
                    }
                    kept += 1;
                } else {
                    self.close_view_display(&key, &id, None, false);
                    closed += 1;
                }
            }
            crate::diagnostic!(serde_json::json!({
                "component": "view_sessions",
                "kind": "view_sessions.closed",
                "device": key.0,
                "closed": closed,
                "kept_unsaved": kept,
            }));
        }
    }

    /// Whether the View `display_id` of `key` is the last to show a document
    /// with unsaved work, in any Workspace.
    fn holds_last_unsaved(&self, key: &WorkspaceKey, display_id: &str) -> bool {
        let Some(tab) = self
            .view_display(key, display_id)
            .and_then(|display| display.tab_id.clone())
        else {
            return false;
        };
        self.document_kept(&tab)
            && !self.every_view().any(|(view, other)| {
                other.tab_id.as_ref() == Some(&tab) && (&view != key || other.id != display_id)
            })
    }

    /// A device pane or tab close asked of `target`: the sessions in it
    /// whose Views close once the device says it closed.
    pub(super) fn expect_device_session_close(
        &mut self,
        target: &str,
        request: &str,
        pane_ids: Vec<String>,
    ) {
        let scopes = self.session_scopes_of_panes(&pane_ids);
        let closes = &mut self.view_sessions.device_closes;
        closes.retain(|close| !(close.target == target && close.request == request));
        if scopes.is_empty() {
            return;
        }
        if closes.len() >= DEVICE_CLOSES {
            closes.pop_front();
        }
        closes.push_back(DeviceClose {
            target: target.to_owned(),
            request: request.to_owned(),
            scopes,
        });
    }

    /// The device answered a close: its sessions' Views close when it closed.
    pub(super) fn settle_device_session_close(
        &mut self,
        target: &str,
        request: &str,
        closed: bool,
    ) {
        let closes = &mut self.view_sessions.device_closes;
        let Some(index) = closes
            .iter()
            .position(|close| close.target == target && close.request == request)
        else {
            return;
        };
        let close = closes.remove(index).expect("found above");
        if closed {
            self.close_session_views(&close.scopes);
        }
    }

    /// For each pane of the checkout `key`, the Views whose documents hold
    /// unsaved work that closing the pane would close, so the shell asks
    /// first (D-14). Empty, as it nearly always is, it costs one pass over
    /// the checkout's Views.
    pub(super) fn session_drafts(&self, key: &WorkspaceKey) -> Vec<SessionDraftsSnapshot> {
        if !self.view_displays(key).any(|display| {
            display
                .tab_id
                .as_deref()
                .is_some_and(|tab| self.document_kept(tab))
        }) {
            return Vec::new();
        }
        let mut drafts = Vec::new();
        for pane in self.view_checkout_panes(key) {
            let Some(scope) = self.session_scope_of_pane(&pane) else {
                continue;
            };
            let closing: HashSet<&str> = self
                .view_displays(key)
                .filter(|display| {
                    !display.operator
                        && !display.openers.is_empty()
                        && display.openers.iter().all(|o| o.of_session(&scope))
                })
                .map(|display| display.id.as_str())
                .collect();
            let views: Vec<SessionDraftSnapshot> = self
                .view_displays(key)
                .filter(|display| closing.contains(display.id.as_str()))
                .filter_map(|display| {
                    let tab = display.tab_id.as_deref()?;
                    if !self.document_kept(tab) {
                        return None;
                    }
                    let shared = self.every_view().any(|(view, other)| {
                        other.tab_id.as_deref() == Some(tab)
                            && !(&view == key && closing.contains(other.id.as_str()))
                    });
                    (!shared).then(|| SessionDraftSnapshot {
                        display_id: display.id.clone(),
                        tab_id: tab.to_owned(),
                    })
                })
                .collect();
            if !views.is_empty() {
                drafts.push(SessionDraftsSnapshot {
                    pane_id: pane,
                    views,
                });
            }
        }
        drafts
    }

    /// Keeps the open record's Views (its 결과물 gallery, PRD
    /// session-docs-and-assistant D-40) in step with every checkout's View
    /// list. Runs with each Workspace view sync and when a record arrives;
    /// while no record is shown it reads nothing, and an unchanged list
    /// leaves the record's snapshot as it was.
    pub(super) fn settle_record_views(&mut self) {
        let Some(held) = self.snapshot.session_record.as_ref() else {
            return;
        };
        let Some(record) = held.record.as_ref() else {
            return;
        };
        let (views, omitted) = self.record_views(record);
        if held.views == views && held.views_omitted == omitted {
            return;
        }
        let mut next = crate::links::ledger::SessionRecordSnapshot::clone(held);
        next.views = views;
        next.views_omitted = omitted;
        self.snapshot.session_record = Some(Arc::new(next));
    }

    /// The outputs among the Views `record`'s session and its descendants
    /// opened (`is_output`), in every checkout's list, the one looked at last
    /// first, each once; past `RECORD_VIEWS` only counted. A session is found by its native session,
    /// as `hide view list --session` finds it, on its own machine.
    fn record_views(
        &self,
        record: &crate::links::ledger::SessionRecord,
    ) -> (Vec<crate::model::RecordViewSnapshot>, usize) {
        use crate::links::ledger::{SessionIdentity, SessionRecord};
        fn tree<'a>(record: &'a SessionRecord, into: &mut Vec<&'a SessionIdentity>) {
            into.push(&record.session);
            for child in &record.children {
                tree(child, into);
            }
        }
        let mut sessions = Vec::new();
        tree(record, &mut sessions);
        let sessions: Vec<(&SessionIdentity, Option<String>)> = sessions
            .into_iter()
            .map(|session| {
                (
                    session,
                    crate::engine::tokens::session_digest(&session.session_id),
                )
            })
            .collect();
        let whose = |display: &Display| {
            sessions.iter().find_map(|(session, digest)| {
                display
                    .openers
                    .iter()
                    .any(|opener| {
                        opener.device_id == session.device_id
                            && (opener.resume.as_deref() == Some(session.session_id.as_str())
                                || (digest.is_some() && opener.session == *digest))
                    })
                    .then_some(*session)
            })
        };
        let mut found: Vec<(&WorkspaceKey, &Display, &SessionIdentity)> = Vec::new();
        let views: Vec<(WorkspaceKey, &Display)> = self.every_view().collect();
        for (key, display) in &views {
            let display: &Display = display;
            {
                if !is_output(display) {
                    continue;
                }
                let Some(session) = whose(display) else {
                    continue;
                };
                // The same file, diff or page in one checkout is one tile.
                if found
                    .iter()
                    .any(|(other, seen, _)| *other == key && seen.shown() == display.shown())
                {
                    continue;
                }
                found.push((key, display, session));
            }
        }
        found.sort_by_key(|(_, display, _)| std::cmp::Reverse(display.last_focused_unix_ms));
        let omitted = found.len().saturating_sub(RECORD_VIEWS);
        let views = found
            .into_iter()
            .take(RECORD_VIEWS)
            .map(|(key, display, session)| {
                let tile = self.view_tile(display);
                crate::model::RecordViewSnapshot {
                    device_id: key.0.clone(),
                    checkout_path: key.1.clone(),
                    display_id: display.id.clone(),
                    kind: display.kind,
                    label: tile.label,
                    path: display.path.clone(),
                    url: display.url.clone(),
                    document_kind: tile.document_kind,
                    lines: tile.lines,
                    session: session.clone(),
                }
            })
            .collect();
        (views, omitted)
    }

    /// What a View's tile shows of it from what Hide already holds: its tab's
    /// name, the kind of the file its editor tab holds and an open text
    /// document's first lines, never a read or a capture made for the tile.
    /// The session record's gallery draws it; the phone's 볼 것 list needs
    /// only the name (`phone_looks`), so it does not pay for the rest.
    fn view_tile(&self, display: &Display) -> ViewTile {
        use crate::view_layout::DisplayKind;
        let document = display
            .tab_id
            .as_deref()
            .and_then(|tab| self.editor_documents.get(tab));
        let text = display.kind == DisplayKind::File;
        ViewTile {
            label: super::view_areas::display_label(display),
            document_kind: document
                .filter(|_| text)
                .map(|document| document.document_kind),
            lines: document
                .filter(|document| text && document.document_kind.is_editable())
                .and_then(|document| document.contents_utf8.as_deref())
                .map(first_lines)
                .unwrap_or_default(),
        }
    }

    /// The Views the agent in `pane_id` opened that are open now, in every
    /// checkout's list, the one opened or looked at last first, each once,
    /// with whether the operator has looked at it: the phone's 볼 것 tab
    /// (PRD mobile-companion-ui D-07). Past `LOOK_VIEWS` only counted. This
    /// is the same set the pane's `N views` lists on the desktop (a session's
    /// Views by `Display::openers`), found for any pane, not only the front
    /// Workspace's.
    pub fn phone_looks(&self, pane_id: &str) -> crate::model::LookViews {
        let Some(scope) = self.session_scope_of_pane(pane_id) else {
            return crate::model::LookViews::default();
        };
        // The phone has no tool's cell, where the Views an agent a tool
        // keeps opened are listed, so none of them reach it (PRD
        // hide-external-control D-52).
        let device = crate::address::pane_device(pane_id).unwrap_or(self.node.as_str());
        if self
            .machine_agents(device)
            .iter()
            .any(|agent| agent.pane_id == pane_id && crate::agent_state::tool_keeps(agent))
        {
            return crate::model::LookViews::default();
        }
        let mut found: Vec<(WorkspaceKey, &Display)> = Vec::new();
        for (key, display) in self.every_view() {
            {
                if !display
                    .openers
                    .iter()
                    .any(|opener| opener.of_session(&scope))
                {
                    continue;
                }
                // Two tabs showing the same thing in one checkout are one
                // row, which is unseen while either is and then names the
                // unseen one: only that View being seen clears the eye.
                match found
                    .iter_mut()
                    .find(|(other, seen)| *other == key && seen.shown() == display.shown())
                {
                    Some(row) => {
                        if display.unseen.is_some() && row.1.unseen.is_none() {
                            row.1 = display;
                        }
                    }
                    None => found.push((key.clone(), display)),
                }
            }
        }
        let at = |display: &Display| {
            display
                .unseen
                .as_ref()
                .map_or(display.last_focused_unix_ms, |unseen| unseen.opened_unix_ms)
        };
        found.sort_by_key(|(_, display)| std::cmp::Reverse(at(display)));
        let omitted = found.len().saturating_sub(LOOK_VIEWS);
        let views = found
            .into_iter()
            .take(LOOK_VIEWS)
            .map(|(key, display)| crate::model::LookView {
                device_id: key.0,
                checkout_path: key.1,
                display_id: display.id.clone(),
                kind: display.kind,
                label: super::view_areas::display_label(display),
                path: display.path.clone(),
                url: display.url.clone(),
                committed: display.committed,
                output: (display.kind == crate::view_layout::DisplayKind::File)
                    .then(|| output_kind(hide_platform::path::wire_name(&display.path)))
                    .flatten(),
                unseen: display.unseen.is_some(),
                at_unix_ms: at(display),
            })
            .collect();
        crate::model::LookViews { views, omitted }
    }
}

/// A View's tile.
struct ViewTile {
    label: String,
    document_kind: Option<crate::model::DocumentKind>,
    lines: Vec<String>,
}

/// Whether a View is a 결과물, something made for a person to look at, by
/// its kind and, for a file, its extension: a page always, a diff never, a
/// file when `OUTPUT_EXTENSIONS` names its extension. A source file, a log or
/// a configuration file the agent only opened is not one. Interim until the
/// agent names its outputs itself (`hide output add`, Desk phase 2).
pub(super) fn is_output(display: &Display) -> bool {
    use crate::view_layout::DisplayKind;
    let name = match display.kind {
        DisplayKind::Browser => return true,
        DisplayKind::Diff | DisplayKind::Card => return false,
        DisplayKind::File => hide_platform::path::wire_name(&display.path),
        DisplayKind::External => match &display.external {
            Some(file) => file.name.as_str(),
            None => return false,
        },
    };
    output_kind(name).is_some()
}

/// What kind of 결과물 a file name is by its extension: the name of its
/// `OUTPUT_EXTENSIONS` row (`image`, `pdf`, `video`, `board`, `document`), or
/// none for any other file. The one table decides it for the Desk's outputs
/// and for what a phone does with a View.
fn output_kind(name: &str) -> Option<&'static str> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    OUTPUT_EXTENSIONS
        .iter()
        .find(|(_, extensions)| {
            extensions
                .iter()
                .any(|output| output.eq_ignore_ascii_case(extension))
        })
        .map(|(kind, _)| *kind)
}

/// The files that are a 결과물, by extension: each row a kind of output.
const OUTPUT_EXTENSIONS: &[(&str, &[&str])] = &[
    (
        "image",
        &["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif", "svg"],
    ),
    ("pdf", &["pdf"]),
    ("video", &["mp4", "webm", "mov"]),
    ("board", &["pen", "excalidraw", "tldraw", "drawio"]),
    (
        "document",
        &["md", "markdown", "mdx", "html", "htm", "txt", "rst", "adoc"],
    ),
];

/// Views one record lists; the rest are counted, not sent.
const RECORD_VIEWS: usize = 64;
/// Views the phone's 볼 것 list holds for one agent; the rest are counted.
const LOOK_VIEWS: usize = 64;
/// Lines of an open document a record's tile shows, and the characters
/// kept of each.
const PREVIEW_LINES: usize = 6;
const PREVIEW_CHARS: usize = 120;

/// A document's first lines for its tile: from its first line with words,
/// each cut to `PREVIEW_CHARS`.
fn first_lines(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| line.trim().is_empty())
        .take(PREVIEW_LINES)
        .map(|line| line.trim_end().chars().take(PREVIEW_CHARS).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_feedback::Opener;
    use crate::view_layout::{DisplayKind, Layout};

    fn opener(terminal: &str, session: Option<&str>) -> Opener {
        Opener {
            device_id: "local".into(),
            terminal_id: terminal.into(),
            session: session.map(str::to_owned),
            resume: None,
            resume_path: None,
            provider: "claude".into(),
            name: terminal.into(),
            profile: None,
        }
    }

    fn scope(terminal: &str, session: Option<&str>) -> SessionScope {
        SessionScope {
            device_id: "local".into(),
            terminal_id: terminal.into(),
            session: session.map(str::to_owned),
        }
    }

    /// One area holding, in order, a View of each `(name, opener terminal)`;
    /// `None` is the operator's.
    fn layout(views: &[(&str, Option<&str>)]) -> Layout {
        let mut layout = Layout::default();
        for (stamp, (name, terminal)) in views.iter().enumerate() {
            let mut display =
                layout.new_display(&format!("/repo/{name}"), DisplayKind::File, None, false);
            match terminal {
                Some(terminal) => {
                    display.opened_by(opener(terminal, Some(terminal)), stamp as u64 + 1)
                }
                None => display.operator = true,
            }
            layout.insert("a1", display, stamp as u64 + 1).unwrap();
        }
        layout
    }

    fn id(layout: &Layout, name: &str) -> String {
        layout
            .displays()
            .find(|display| display.path == format!("/repo/{name}"))
            .unwrap()
            .id
            .clone()
    }

    #[test]
    fn two_sessions_that_open_one_view_both_keep_it_and_the_last_is_its_mark() {
        let mut layout = layout(&[("README.md", Some("t1"))]);
        let readme = id(&layout, "README.md");
        let display = layout.display_mut(&readme).unwrap();
        display.opened_by(opener("t2", Some("t2")), 0);
        let both = |terminal: &str| {
            let scope = scope(terminal, Some(terminal));
            display.openers.iter().any(|o| o.of_session(&scope))
        };
        assert!(both("t1") && both("t2"));
        assert_eq!(display.opener.as_ref().unwrap().terminal_id, "t2");
        // Opening it again moves that session to the end, once.
        display.opened_by(opener("t1", Some("t1")), 0);
        let order: Vec<&str> = display
            .openers
            .iter()
            .map(|o| o.terminal_id.as_str())
            .collect();
        assert_eq!(order, ["t2", "t1"]);
    }
}
