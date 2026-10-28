//! Which Views a Workspace's Files strip shows (PRD session-files): the ones
//! the session whose pane has the keyboard opened, and the operator's own.
//!
//! A View keeps every session that opened it (`Display::openers`) and
//! whether the operator did (`Display::operator`). The strip is the
//! checkout's one View list filtered for the session in front, which the core
//! settles after every event and before every snapshot read
//! (`settle_view_sessions`): a switch of session shows the View that session
//! last showed, else its first View, else the operator's (D-12). A session's
//! Views close with its pane or its archive, never with a sleep or an agent
//! that exits to its shell (D-14, D-27).

use std::collections::{HashMap, HashSet, VecDeque};

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::model::{SessionDraftSnapshot, SessionDraftsSnapshot};
use crate::view_feedback::SessionScope;
use crate::view_layout::{Display, Layout};

/// Sessions whose last front one checkout remembers; a further one forgets
/// the session that left the strip longest ago.
const REMEMBERED_SESSIONS: usize = 32;
/// Checkouts whose strip is remembered; past it only the front one stays.
const REMEMBERED_CHECKOUTS: usize = 64;
/// Device pane and tab closes waiting for their device's answer; past it
/// the oldest is forgotten and its Views stay open.
const DEVICE_CLOSES: usize = 32;

/// What the core holds for the strips that is not saved: a restart shows
/// each area's saved View and learns each session's front again.
#[derive(Default)]
pub(super) struct ViewSessions {
    strips: HashMap<WorkspaceKey, Strip>,
    device_closes: VecDeque<DeviceClose>,
}

struct Strip {
    /// The session the strip was last settled for: `None` while a plain
    /// shell, or no pane of the checkout, has the keyboard (D-28).
    scope: Option<SessionScope>,
    /// The View in front of each area when each session last left the
    /// strip, the most recent last.
    fronts: VecDeque<(Option<Identity>, Vec<String>)>,
}

/// Which session a strip is for: its machine and terminal. The native
/// session it learns later never makes it another one.
type Identity = (String, String);

fn identity(scope: Option<&SessionScope>) -> Option<Identity> {
    scope.map(|scope| (scope.device_id.clone(), scope.terminal_id.clone()))
}

struct DeviceClose {
    target: String,
    request: String,
    scopes: Vec<SessionScope>,
}

/// What a settle changes: the Views to bring to the front of their areas
/// and, when the area in use draws nothing, the area to use instead.
#[derive(Default)]
struct Plan {
    shows: Vec<String>,
    active_area: Option<String>,
}

/// The front of each area that draws something for `scope`. On a `switched`
/// strip each area shows the View the session last showed there, else the
/// first View it opened, else what it shows now; otherwise an area keeps its
/// View while the strip draws it. An area whose View left the strip shows
/// the one looked at last.
fn plan_fronts(
    layout: &Layout,
    scope: Option<&SessionScope>,
    switched: bool,
    remembered: &[String],
) -> Plan {
    let mut plan = Plan::default();
    let mut drawn: Vec<(String, u64)> = Vec::new();
    for area in layout.areas() {
        let shown: Vec<&Display> = area
            .displays
            .iter()
            .filter(|display| display.shown_to(scope))
            .collect();
        let current = area
            .active
            .as_deref()
            .and_then(|id| shown.iter().find(|display| display.id == id))
            .copied();
        let own = |display: &&&Display| {
            scope.is_some_and(|scope| display.openers.iter().any(|o| o.of_session(scope)))
        };
        let chosen = if switched {
            shown
                .iter()
                .find(|display| remembered.contains(&display.id))
                .or_else(|| shown.iter().find(own))
                .copied()
                .or(current)
        } else {
            current
        }
        .or_else(|| {
            shown
                .iter()
                .max_by_key(|display| display.last_focused_unix_ms)
                .copied()
        });
        let Some(chosen) = chosen else {
            continue;
        };
        if area.active.as_deref() != Some(chosen.id.as_str()) {
            plan.shows.push(chosen.id.clone());
        }
        drawn.push((area.id.clone(), chosen.last_focused_unix_ms));
    }
    if !drawn.iter().any(|(id, _)| *id == layout.active_area) {
        plan.active_area = drawn
            .iter()
            .max_by_key(|(_, stamp)| *stamp)
            .map(|(id, _)| id.clone());
    }
    plan
}

impl Runtime {
    /// The session `pane_id` stands for, on whichever machine it runs: its
    /// terminal and the native session of the agent in it. A pane Hide holds
    /// no terminal for stands for none.
    pub(super) fn session_scope_of_pane(&self, pane_id: &str) -> Option<SessionScope> {
        let device = super::agent_sleep::pane_device(pane_id)
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
        request: &live::CloseCaptureRequest,
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
    fn strip_pane_ids(&self, key: &WorkspaceKey) -> Vec<String> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == key.1)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .map(|pane| pane.id.clone())
            .collect()
    }

    /// The session of the pane with the keyboard, when that pane is one of
    /// the checkout `key`'s own (D-13) and its session opened a View there.
    /// A session with none shows the operator's strip, so moving between
    /// plain shells, or to an agent that opened nothing, moves no View (D-12,
    /// D-28).
    fn keyboard_session(&self, key: &WorkspaceKey) -> Option<SessionScope> {
        let pane = self.operator_pane(&key.0)?.to_owned();
        let scope = self
            .strip_pane_ids(key)
            .contains(&pane)
            .then(|| self.session_scope_of_pane(&pane))
            .flatten()?;
        let layout = self.view_layout_of(key)?;
        layout
            .areas()
            .iter()
            .flat_map(|area| &area.displays)
            .any(|display| display.openers.iter().any(|o| o.of_session(&scope)))
            .then_some(scope)
    }

    /// The session the strip of `key` was last settled for.
    pub(super) fn strip_session(&self, key: &WorkspaceKey) -> Option<&SessionScope> {
        self.view_sessions.strips.get(key)?.scope.as_ref()
    }

    /// Brings the front Workspace's strip to the session whose pane has the
    /// keyboard (D-10, D-12, D-13). With nothing moved since the last pass
    /// it reads the catalog once and changes nothing.
    pub(super) fn settle_view_sessions(&mut self) {
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let scope = self.keyboard_session(&key);
        let known = self.view_sessions.strips.get(&key);
        // The first settle after a start keeps the saved selection.
        let left = known
            .filter(|strip| identity(strip.scope.as_ref()) != identity(scope.as_ref()))
            .map(|strip| strip.scope.clone());
        let (plan, leaving) = {
            let Some(layout) = self.view_layout_of(&key) else {
                return;
            };
            let remembered = left
                .as_ref()
                .and(known)
                .and_then(|strip| {
                    let id = identity(scope.as_ref());
                    strip.fronts.iter().find(|(known, _)| *known == id)
                })
                .map(|(_, fronts)| fronts.clone())
                .unwrap_or_default();
            let leaving = left.as_ref().map(|old| {
                layout
                    .areas()
                    .into_iter()
                    .filter_map(|area| area.active.clone())
                    .filter(|id| {
                        layout
                            .display(id)
                            .is_some_and(|display| display.shown_to(old.as_ref()))
                    })
                    .collect::<Vec<_>>()
            });
            (
                plan_fronts(layout, scope.as_ref(), left.is_some(), &remembered),
                leaving,
            )
        };
        let sessions = &mut self.view_sessions;
        if !sessions.strips.contains_key(&key) && sessions.strips.len() >= REMEMBERED_CHECKOUTS {
            sessions.strips.clear();
        }
        let strip = sessions.strips.entry(key.clone()).or_insert_with(|| Strip {
            scope: None,
            fronts: VecDeque::new(),
        });
        if let (Some(old), Some(fronts)) = (left, leaving) {
            let id = identity(old.as_ref());
            strip.fronts.retain(|(known, _)| *known != id);
            if strip.fronts.len() >= REMEMBERED_SESSIONS {
                strip.fronts.pop_front();
            }
            strip.fronts.push_back((id, fronts));
        }
        strip.scope = scope;
        if plan.shows.is_empty() && plan.active_area.is_none() {
            return;
        }
        let settled = self.change_view_layout(&key, |layout, stamp| {
            let mut changed = false;
            for id in &plan.shows {
                changed |= layout.show(id, stamp)?;
            }
            if let Some(area) = &plan.active_area {
                changed |= layout.focus_area(area, stamp)?;
            }
            Ok(((), changed))
        });
        if let Err(error) = settled {
            crate::diagnostic!(serde_json::json!({
                "component": "view_sessions",
                "kind": "view_sessions.settle_failed",
                "device": key.0,
                "error": error.kind(),
            }));
        }
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
        let keys: Vec<WorkspaceKey> = self
            .every_view_layout()
            .filter(|(key, layout)| {
                scopes.iter().any(|scope| scope.device_id == key.0)
                    && layout.displays().any(|d| d.openers.iter().any(ended))
            })
            .map(|(key, _)| key)
            .collect();
        for key in keys {
            let forgotten = self.change_view_layout(&key, |layout, _| {
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
                    let _ = self.change_view_layout(&key, |layout, _| {
                        let display = layout.display_mut(&id).ok_or_else(|| {
                            crate::view_layout::LayoutError::UnknownDisplay(id.clone())
                        })?;
                        display.operator = true;
                        Ok(((), true))
                    });
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
            .view_layout_of(key)
            .and_then(|layout| layout.display(display_id))
            .and_then(|display| display.tab_id.clone())
        else {
            return false;
        };
        self.document_kept(&tab)
            && !self.every_view_layout().any(|(view, layout)| {
                layout.displays().any(|other| {
                    other.tab_id.as_ref() == Some(&tab) && (&view != key || other.id != display_id)
                })
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
    /// the layout.
    pub(super) fn session_drafts(
        &self,
        key: &WorkspaceKey,
        layout: &Layout,
    ) -> Vec<SessionDraftsSnapshot> {
        if !layout.displays().any(|display| {
            display
                .tab_id
                .as_deref()
                .is_some_and(|tab| self.document_kept(tab))
        }) {
            return Vec::new();
        }
        let mut drafts = Vec::new();
        for pane in self.strip_pane_ids(key) {
            let Some(scope) = self.session_scope_of_pane(&pane) else {
                continue;
            };
            let closing: HashSet<&str> = layout
                .displays()
                .filter(|display| {
                    !display.operator
                        && !display.openers.is_empty()
                        && display.openers.iter().all(|o| o.of_session(&scope))
                })
                .map(|display| display.id.as_str())
                .collect();
            let views: Vec<SessionDraftSnapshot> = layout
                .displays()
                .filter(|display| closing.contains(display.id.as_str()))
                .filter_map(|display| {
                    let tab = display.tab_id.as_deref()?;
                    if !self.document_kept(tab) {
                        return None;
                    }
                    let shared = self.every_view_layout().any(|(view, other_layout)| {
                        other_layout.displays().any(|other| {
                            other.tab_id.as_deref() == Some(tab)
                                && !(&view == key && closing.contains(other.id.as_str()))
                        })
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

    /// The pane of this machine whose session opened `display_id`, when the
    /// strip of `key` does not show that View.
    pub(super) fn hidden_view_pane(&self, key: &WorkspaceKey, display_id: &str) -> Option<String> {
        if key.0 != self.node.as_str() {
            return None;
        }
        let display = self.view_layout_of(key)?.display(display_id)?;
        if display.shown_to(self.strip_session(key)) {
            return None;
        }
        let pane = self
            .view_opener_snapshot(display.opener.as_ref()?)
            .pane_id?;
        self.strip_pane_ids(key).contains(&pane).then_some(pane)
    }

    /// The index a View dragged in the drawn strip lands at in its area's
    /// whole list: before the drawn View now at `index`, else after the last
    /// one drawn.
    pub(super) fn drawn_index(
        &self,
        key: &WorkspaceKey,
        display_id: &str,
        area_id: &str,
        index: usize,
    ) -> usize {
        let scope = self.strip_session(key);
        let Some(area) = self
            .view_layout_of(key)
            .and_then(|layout| layout.area(area_id))
        else {
            return index;
        };
        let rest: Vec<&Display> = area
            .displays
            .iter()
            .filter(|display| display.id != display_id)
            .collect();
        let drawn: Vec<usize> = rest
            .iter()
            .enumerate()
            .filter(|(_, display)| display.shown_to(scope))
            .map(|(at, _)| at)
            .collect();
        match drawn.get(index) {
            Some(at) => *at,
            None => drawn.last().map_or(rest.len(), |last| last + 1),
        }
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
            .map(|session| (session, crate::wire::session_digest(&session.session_id)))
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
        let layouts: Vec<(WorkspaceKey, &Layout)> = self.every_view_layout().collect();
        for (key, layout) in &layouts {
            for display in layout.displays() {
                if !is_output(display) {
                    continue;
                }
                let Some(session) = whose(display) else {
                    continue;
                };
                // The same file, diff or page in one checkout is one tile.
                if found.iter().any(|(other, seen, _)| {
                    *other == key
                        && seen.kind == display.kind
                        && seen.path == display.path
                        && seen.committed == display.committed
                        && seen.url == display.url
                }) {
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
                let document = display
                    .tab_id
                    .as_deref()
                    .and_then(|tab| self.editor_documents.get(tab));
                let text = display.kind == crate::view_layout::DisplayKind::File;
                crate::model::RecordViewSnapshot {
                    device_id: key.0.clone(),
                    checkout_path: key.1.clone(),
                    display_id: display.id.clone(),
                    kind: display.kind,
                    label: match display.kind {
                        crate::view_layout::DisplayKind::Browser => {
                            super::view_areas::browser_label(display)
                        }
                        crate::view_layout::DisplayKind::External => display
                            .external
                            .as_ref()
                            .map_or_else(String::new, |file| file.name.clone()),
                        crate::view_layout::DisplayKind::File => {
                            super::workspace_view::file_label(&display.path)
                        }
                        crate::view_layout::DisplayKind::Diff => super::editor::diff_label(
                            &display.path,
                            display.committed.unwrap_or(false),
                        ),
                    },
                    path: display.path.clone(),
                    url: display.url.clone(),
                    document_kind: document
                        .filter(|_| text)
                        .map(|document| document.document_kind),
                    lines: document
                        .filter(|document| text && document.document_kind.is_editable())
                        .and_then(|document| document.contents_utf8.as_deref())
                        .map(first_lines)
                        .unwrap_or_default(),
                    session: session.clone(),
                }
            })
            .collect();
        (views, omitted)
    }
}

/// Whether a View is a 결과물, something made for a person to look at, by
/// its kind and, for a file, its extension: a page always, a diff never, a
/// file when `OUTPUT_EXTENSIONS` names its extension. A source file, a log or
/// a configuration file the agent only opened is not one. Interim until the
/// agent names its outputs itself (`hide output add`, Desk phase 2).
fn is_output(display: &Display) -> bool {
    use crate::view_layout::DisplayKind;
    let name = match display.kind {
        DisplayKind::Browser => return true,
        DisplayKind::Diff => return false,
        DisplayKind::File => hide_platform::path::wire_name(&display.path),
        DisplayKind::External => match &display.external {
            Some(file) => file.name.as_str(),
            None => return false,
        },
    };
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && OUTPUT_EXTENSIONS
                .iter()
                .flat_map(|(_, extensions)| extensions.iter())
                .any(|output| output.eq_ignore_ascii_case(extension))
    })
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
    use crate::view_layout::DisplayKind;

    fn opener(terminal: &str, session: Option<&str>) -> Opener {
        Opener {
            device_id: "local".into(),
            terminal_id: terminal.into(),
            session: session.map(str::to_owned),
            resume: None,
            resume_path: None,
            provider: "claude".into(),
            name: terminal.into(),
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
                Some(terminal) => display.opened_by(opener(terminal, Some(terminal))),
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
    fn a_session_sees_its_own_views_and_the_operators_only() {
        let layout = layout(&[
            ("prd.md", Some("t1")),
            ("auth.rs", Some("t2")),
            ("notes.md", None),
        ]);
        let a = scope("t1", Some("t1"));
        let shown: Vec<&str> = layout
            .displays()
            .filter(|display| display.shown_to(Some(&a)))
            .map(|display| display.path.as_str())
            .collect();
        assert_eq!(shown, ["/repo/prd.md", "/repo/notes.md"]);
        let shell: Vec<&str> = layout
            .displays()
            .filter(|display| display.shown_to(None))
            .map(|display| display.path.as_str())
            .collect();
        assert_eq!(
            shell,
            ["/repo/notes.md"],
            "a plain shell shows the operator's only"
        );
    }

    #[test]
    fn a_session_found_in_another_terminal_keeps_its_views() {
        let layout = layout(&[("prd.md", Some("t1"))]);
        let moved = scope("t9", Some("t1"));
        assert!(
            layout
                .displays()
                .all(|display| display.shown_to(Some(&moved)))
        );
    }

    #[test]
    fn switching_to_a_session_shows_the_view_it_last_showed_else_its_first() {
        let mut layout = layout(&[
            ("prd.md", Some("t1")),
            ("auth.rs", Some("t2")),
            ("auth.diff", Some("t2")),
            ("notes.md", None),
        ]);
        let b = scope("t2", Some("t2"));
        // First time: the session's first View, not the operator's in front.
        let plan = plan_fronts(&layout, Some(&b), true, &[]);
        assert_eq!(plan.shows, [id(&layout, "auth.rs")]);
        // Remembered: the View it last showed.
        let remembered = [id(&layout, "auth.diff")];
        let plan = plan_fronts(&layout, Some(&b), true, &remembered);
        assert_eq!(plan.shows, remembered);
        // A session with no View of its own shows the operator's.
        let notes = id(&layout, "notes.md");
        layout.show(&notes, 99).unwrap();
        let plan = plan_fronts(&layout, Some(&scope("t3", None)), true, &[]);
        assert!(plan.shows.is_empty(), "the operator's View in front stays");
    }

    #[test]
    fn a_view_that_left_the_strip_gives_way_to_the_one_looked_at_last() {
        let layout = layout(&[("notes.md", None), ("prd.md", Some("t1"))]);
        // prd.md is in front; a plain shell has the keyboard now.
        let plan = plan_fronts(&layout, None, false, &[]);
        assert_eq!(plan.shows, [id(&layout, "notes.md")]);
    }

    #[test]
    fn two_sessions_that_open_one_view_both_keep_it_and_the_last_is_its_mark() {
        let mut layout = layout(&[("README.md", Some("t1"))]);
        let readme = id(&layout, "README.md");
        let display = layout.display_mut(&readme).unwrap();
        display.opened_by(opener("t2", Some("t2")));
        assert!(display.shown_to(Some(&scope("t1", Some("t1")))));
        assert!(display.shown_to(Some(&scope("t2", Some("t2")))));
        assert_eq!(display.opener.as_ref().unwrap().terminal_id, "t2");
        // Opening it again moves that session to the end, once.
        display.opened_by(opener("t1", Some("t1")));
        let order: Vec<&str> = display
            .openers
            .iter()
            .map(|o| o.terminal_id.as_str())
            .collect();
        assert_eq!(order, ["t2", "t1"]);
    }
}
