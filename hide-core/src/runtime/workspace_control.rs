//! Pane-scoped queries and actions against the live engine projection.

use super::Runtime;
use crate::view_layout::DisplayKind;
use crate::workspace_control::{
    Action, ActionMaterial, ActionPreparation, ActionResult, ActionSource, BrowserRouteSource,
    Caller, Context, Query, QueryResult, Refusal, View,
};
use crate::workspace_views::Place;

const ACTION_RESULTS_KEPT: usize = 128;
const ACTION_RETRY_WINDOW_MS: u64 = 10 * 60 * 1000;

pub(super) struct ReportedBrowserPage {
    pub load: u64,
    pub present: bool,
    pub loading: bool,
    pub failure: Option<String>,
}

pub(super) struct RecordedAction {
    device_id: String,
    pane_id: String,
    context: Context,
    request_id: String,
    at: u64,
    action: Action,
    caller_identity: (Option<String>, Option<String>),
    result: Option<Result<ActionResult, Refusal>>,
}

/// What a file read of this machine's checkout answers when its folder is
/// no longer where Hide opened it.
const CHECKOUT_FOLDER_GONE: &str = "The checkout's folder could not be found where Hide opened it (moved, renamed or deleted): open the folder from its new place in Hide, then retry";

impl Runtime {
    fn view_caller_identity(
        &self,
        context: &Context,
        caller: &str,
    ) -> (Option<String>, Option<String>) {
        let key = (context.device_id.clone(), context.checkout_path.clone());
        let session = self
            .machine_agents(&context.device_id)
            .iter()
            .find(|agent| agent.pane_id == caller)
            .and_then(|agent| agent.session_id.clone());
        (self.control_caller_tab(&key, caller), session)
    }

    fn check_browser_scope(&self, key: &(String, String), action: &Action) -> Result<(), Refusal> {
        let (view_id, expected_area) = match action {
            Action::Select {
                view_id,
                expected_browser_area: Some(area),
                ..
            }
            | Action::Close {
                view_id,
                expected_browser_area: Some(area),
            } => (view_id, area),
            _ => return Ok(()),
        };
        let in_scope = self
            .view_place(key, view_id)
            .and_then(|place| self.view_layout_at(key, &place))
            .and_then(|layout| layout.area(expected_area))
            .is_some_and(|area| {
                area.displays
                    .iter()
                    .any(|display| display.id == *view_id && display.kind == DisplayKind::Browser)
            });
        if !in_scope {
            return Err(Refusal {
                reason: "browser_scope_changed",
                next_action: "Refresh the browser target and retry",
            });
        }
        Ok(())
    }

    fn check_browser_area(
        &self,
        key: &(String, String),
        area_id: Option<&str>,
    ) -> Result<(), Refusal> {
        if let Some(area_id) = area_id {
            let known = self.area_place(key, area_id).is_some()
                || crate::view_layout::Layout::default()
                    .area(area_id)
                    .is_some();
            if !known {
                return Err(layout_refusal(
                    crate::view_layout::LayoutError::UnknownArea(area_id.to_owned()),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn reveal_workspace_control(
        &mut self,
        context: &Context,
        key: &(String, String),
        request_id: &str,
        caller_id: &str,
    ) {
        self.change_look(|runtime| {
            runtime.reveal_workspace_control_steps(context, key, request_id, caller_id)
        });
    }

    /// The steps of `reveal_workspace_control`, which runs them as one selection (`look.rs`).
    fn reveal_workspace_control_steps(
        &mut self,
        context: &Context,
        key: &(String, String),
        request_id: &str,
        caller_id: &str,
    ) {
        let tab = self.control_caller_tab(key, caller_id);
        if context.device_id == self.node.as_str() {
            if let Some(tab_id) = tab {
                self.apply(super::Event::FocusTab(super::FocusTabPayload {
                    workspace_id: context.workspace_id.clone(),
                    checkout_id: context.checkout_id.clone(),
                    tab_id,
                    focus_device: true,
                }));
            } else {
                self.bring_device_forward(context.device_id.clone());
                self.focus_checkout(&context.workspace_id, &context.checkout_id);
            }
        } else {
            self.request_remote_control(super::RemoteControlPayload {
                target_id: context.device_id.clone(),
                request_id: format!("{request_id}-reveal"),
                report_pane_focus_outcome: false,
                focus_device: true,
                request: match tab {
                    Some(tab_id) => super::RemoteControlRequest::FocusTab { tab_id },
                    None => super::RemoteControlRequest::FocusWorkspace {
                        workspace_id: context.workspace_id.clone(),
                        checkout_id: Some(context.checkout_id.clone()),
                    },
                },
            });
        }
    }

    pub fn browser_route_source(
        &self,
        device_id: &str,
        checkout_path: &str,
        view_id: &str,
        load: u64,
    ) -> Option<BrowserRouteSource> {
        // A page of Desk's own desk is the core's own, on no checkout's desk.
        let desk =
            super::desk::views::is_desk_key(&(device_id.to_owned(), checkout_path.to_owned()))
                && device_id == self.node.as_str();
        let connected = if device_id == self.node.as_str() {
            self.snapshot.status.engine.state == "connected"
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == device_id && remote.state == "connected")
        };
        if !connected {
            return None;
        }
        let registered = self.catalog_workspaces().into_iter().any(|workspace| {
            workspace.device_id == device_id
                && workspace
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.path == checkout_path)
        });
        if !registered && !desk {
            return None;
        }
        let key = (device_id.to_owned(), checkout_path.to_owned());
        let display = self.view_displays(&key).find(|display| {
            display.id == view_id && display.kind == DisplayKind::Browser && display.load == load
        })?;
        let url = display.url.clone()?;
        // A local page's own files come from the checkout that holds it,
        // which an agent's open or a terminal link may have shown in another
        // checkout's Workspace.
        let root = hide_platform::path::from_file_url(&url).map_or_else(
            || checkout_path.to_owned(),
            |(path, _)| self.display_source_key(&key, &path).1,
        );
        Some(BrowserRouteSource {
            device_id: device_id.to_owned(),
            checkout_path: root,
            url,
            load,
        })
    }
    fn check_action_request(
        &mut self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
    ) -> Result<(Context, Option<Result<ActionResult, Refusal>>), Refusal> {
        // Membership is checked on every call, including a retry. A cached
        // answer must never keep a moved or closed pane, or an unregistered
        // checkout, authorized.
        let context = self
            .workspace_control_query(device_id, pane_id, Query::Info)?
            .context;
        let changed = caller_changed(pane_id);
        if &context != expected {
            return Err(changed);
        }
        let now = super::unix_milliseconds();
        let issued = request_id
            .split_once('-')
            .and_then(|(issued, suffix)| {
                (!suffix.is_empty())
                    .then(|| issued.parse::<u64>().ok())
                    .flatten()
            })
            .ok_or(Refusal {
                reason: "invalid_request_id",
                next_action: "Run the hide command again with a new request ID",
            })?;
        if issued > now.saturating_add(60_000)
            || now.saturating_sub(issued) > ACTION_RETRY_WINDOW_MS
        {
            return Err(Refusal {
                reason: "request_expired",
                next_action: "Run hide view list and start a new request with a fresh ID",
            });
        }
        // A read changes no View, so its retry repeats nothing: it keeps no
        // receipt, and so never takes a place among the kept results or
        // waits for one (PRD unseen-agent-views D-22).
        if !keeps_receipt(action) {
            return Ok((context, None));
        }
        self.workspace_actions
            .retain(|record| now.saturating_sub(record.at) <= ACTION_RETRY_WINDOW_MS);
        if let Some(record) = self.workspace_actions.iter().find(|record| {
            record.device_id == device_id
                && record.pane_id == pane_id
                && record.request_id == request_id
        }) {
            let cached = if record.context != context
                || record.caller_identity != self.view_caller_identity(&context, pane_id)
            {
                Some(Err(changed))
            } else if &record.action == action {
                record.result.clone()
            } else {
                Some(Err(Refusal {
                    reason: "request_id_reused",
                    next_action: "Use a new request ID for a different action",
                }))
            };
            return Ok((context, cached));
        }
        if self.workspace_actions.len() >= ACTION_RESULTS_KEPT {
            return Err(Refusal {
                reason: "request_capacity",
                next_action: "Wait for earlier requests to expire, then retry",
            });
        }
        Ok((context, None))
    }

    pub fn workspace_control_prepare_action(
        &mut self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
    ) -> Result<ActionPreparation, Refusal> {
        // Desk has no View: its file open waits for its own tab panel.
        if Caller::parse(pane_id) == Caller::Desk {
            return self.desk_prepare_open(device_id, expected, request_id, action);
        }
        let preparation =
            self.prepare_view_action(device_id, pane_id, expected, request_id, action)?;
        if keeps_receipt(action)
            && !self.workspace_actions.iter().any(|record| {
                record.device_id == device_id
                    && record.pane_id == pane_id
                    && record.request_id == request_id
            })
        {
            self.workspace_actions.push_back(RecordedAction {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                context: expected.clone(),
                request_id: request_id.to_owned(),
                at: super::unix_milliseconds(),
                action: action.clone(),
                caller_identity: self.view_caller_identity(expected, pane_id),
                result: None,
            });
        }
        Ok(preparation)
    }

    fn prepare_view_action(
        &mut self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
    ) -> Result<ActionPreparation, Refusal> {
        let (context, cached) =
            self.check_action_request(device_id, pane_id, expected, request_id, action)?;
        if let Some(cached) = cached {
            return Ok(ActionPreparation::Cached(cached));
        }
        let key = (context.device_id.clone(), context.checkout_path.clone());
        self.check_browser_scope(&key, action)?;
        if let Action::OpenBrowser { url, area_id, .. } = action {
            if !crate::view_layout::browser_address(url) {
                return Err(Refusal {
                    reason: "invalid_address",
                    next_action: "Use an http, https, or checkout HTML address",
                });
            }
            self.check_browser_area(&key, area_id.as_deref())?;
            if hide_platform::path::is_file_url(url) {
                let (path, _) = hide_platform::path::from_file_url(url).ok_or(Refusal {
                    reason: "invalid_address",
                    next_action: "Use a local file URL with an absolute path",
                })?;
                let local = context.device_id == self.node.as_str();
                let checkout = self.open_source(&key, &context, &path);
                let (root, channel) =
                    self.document_source(&checkout.0, &checkout.1)
                        .map_err(|_| Refusal {
                            reason: "host_unavailable",
                            next_action: if local {
                                CHECKOUT_FOLDER_GONE
                            } else {
                                "Reconnect the device and retry"
                            },
                        })?;
                return Ok(ActionPreparation::Read(ActionSource::browser(
                    root, checkout, channel, path, local,
                )));
            }
            return Ok(ActionPreparation::Ready);
        }
        if let Action::OpenExternal { grant, name, kind } = action {
            crate::view_layout::ExternalFile::admitted(grant, name, *kind)
                .map_err(|_| outside_file_refused())?;
            return Ok(ActionPreparation::Ready);
        }
        let (path, file) = match action {
            Action::OpenFile { path, .. } => (path, true),
            Action::OpenDiff { path, .. } => (path, false),
            _ => return Ok(ActionPreparation::Ready),
        };
        // The caller's machine spelled it on the wire: `C:/...` from a
        // Windows device is absolute here too.
        if !hide_platform::path::is_wire_absolute(path) {
            return Err(Refusal {
                reason: "invalid_path",
                next_action: "Resolve the path from the caller's cwd and retry",
            });
        }
        // A file of another checkout of the caller's device is read from
        // that checkout and stays its document, as a terminal link the
        // operator presses opens it (`open_file_in_front_view`); only the
        // View is the caller's. A diff is the caller's checkout's change.
        let checkout = if file {
            self.open_source(&key, &context, path)
        } else {
            (context.workspace_id.clone(), context.checkout_id.clone())
        };
        let already_open = file
            && self.snapshot.editor.tabs.iter().any(|tab| {
                tab.workspace_id == checkout.0
                    && tab.checkout_id == checkout.1
                    && tab.path == *path
                    && tab.kind == crate::model::EditorTabKind::File
                    && tab.unavailable_reason.is_none()
            });
        let local = context.device_id == self.node.as_str();
        // On this machine the read can only fail on the checkout's folder,
        // gone from where Hide opened it, as after a rename (#2624).
        let (root, channel) = self
            .document_source(&checkout.0, &checkout.1)
            .map_err(|_| Refusal {
                reason: "host_unavailable",
                next_action: if local {
                    CHECKOUT_FOLDER_GONE
                } else {
                    "Reconnect the device and check file access consent, then retry"
                },
            })?;
        let source = if file {
            ActionSource::file(root, checkout, channel, path.clone(), already_open, local)
        } else {
            ActionSource::diff(root, checkout, channel, path.clone(), local)
        };
        Ok(ActionPreparation::Read(source))
    }

    /// The checkout, by its catalog ids, a caller's open of `path` reads
    /// from: the caller's own for a path under its root, else the checkout of
    /// its device whose root holds the path (`display_source`), and the
    /// caller's own again when none does, whose read then refuses it.
    fn open_source(
        &self,
        key: &super::workspace_view::WorkspaceKey,
        context: &Context,
        path: &str,
    ) -> (String, String) {
        self.display_source(key, path)
            .unwrap_or_else(|| (context.workspace_id.clone(), context.checkout_id.clone()))
    }

    pub fn workspace_control_action(
        &mut self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: Action,
        material: Result<Option<ActionMaterial>, Refusal>,
    ) -> Result<ActionResult, Refusal> {
        if Caller::parse(pane_id) == Caller::Desk {
            return self.desk_commit_open(device_id, expected, request_id, &action, material);
        }
        let (context, cached) =
            self.check_action_request(device_id, pane_id, expected, request_id, &action)?;
        if let Some(cached) = cached {
            return cached;
        }
        let key = (context.device_id.clone(), context.checkout_path.clone());
        // A file read releases the owner lock, so its named area may have
        // gone since prepare. A browser capability may also name a display
        // moved since its query. Check both under this commit's owner lock,
        // after receipt lookup and before any state mutation.
        let browser_load = self
            .workspace_views
            .as_ref()
            .map(|store| store.browser_load);
        let result = (|| {
            let material = material?;
            self.check_browser_scope(&key, &action)?;
            if let Action::OpenBrowser { area_id, .. } = &action {
                self.check_browser_area(&key, area_id.as_deref())?;
            }
            self.reconcile_view_displays();
            // The agent of the pane the operator watches opens on that tab's
            // tray; any other caller's View waits in its pane's list (PRD
            // lane-desk-pair-view B1, B2). A browser action bound to a page's
            // own area stays in that page's tree.
            let place = match &action {
                Action::OpenBrowser {
                    area_id: Some(area),
                    ..
                } => self
                    .area_place(&key, area)
                    .unwrap_or_else(|| self.front_place(&key)),
                _ => self.control_place(&key, pane_id),
            };
            if self.open_adds_view(
                &key,
                &place,
                &action,
                material.as_ref().map(|material| material.path.as_str()),
            ) {
                self.make_view_room(&key, &place);
            }
            // The View comes in front only on a tray the operator sees now:
            // the lane open, or the tab in pair view, where the paired pane's
            // own View stands the desk beside it even while it is drawn
            // alone (D-53). A closed lane stays closed, and the tray's count
            // and the To look at mark say a tab came (B2).
            let in_view = match &place {
                Place::Tray(tab) => {
                    self.watched_caller(&key, pane_id)
                        && (self.tray_in_view(&key, tab)
                            || self.pair_left_of(&key, tab).as_deref() == Some(pane_id))
                }
                Place::Shelf | Place::Desk => false,
            };
            let kept = (!in_view || !lane_call(&action)).then(|| self.keep_front(&key));
            // A run's desk is drawn by the automation screen, which no lane
            // state opens or closes, so an open there leaves the lane alone.
            let run_desk = matches!(&place, Place::Tray(tab) if self.viewed_run_tab(&key, tab));
            let calls_lane = in_view && lane_call(&action) && !run_desk;
            let mut applied = self.apply_workspace_action(
                &key, &place, &context, pane_id, request_id, &action, material,
            );
            if let Ok(done) = &mut applied
                && calls_lane
            {
                self.open_tray_for(&key, &place);
                // A view already in front still calls the lane, which the
                // page sees only if the change is announced.
                done.changed = true;
            }
            if let Some(kept) = kept {
                self.restore_front(kept);
            }
            // Stamped on the screen the action leaves, so an open the
            // operator is looking at once it lands is seen in this same
            // event, and one put back behind is not (PRD unseen-agent-views
            // B5, B14).
            if let Ok(done) = &applied {
                self.stamp_control_opener(&key, pane_id, &action, &done.view_id);
            }
            applied
        })();
        if result.is_err()
            && let (Some(before), Some(store)) = (browser_load, self.workspace_views.as_mut())
        {
            store.browser_load = before;
        }
        if !keeps_receipt(&action) {
            return result;
        }
        if let Some(record) = self.workspace_actions.iter_mut().find(|record| {
            record.device_id == device_id
                && record.pane_id == pane_id
                && record.request_id == request_id
        }) {
            record.result = Some(result.clone());
        } else {
            let caller_identity = self.view_caller_identity(&context, pane_id);
            self.workspace_actions.push_back(RecordedAction {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                context,
                request_id: request_id.to_owned(),
                at: super::unix_milliseconds(),
                action,
                caller_identity,
                result: Some(result.clone()),
            });
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_workspace_action(
        &mut self,
        key: &(String, String),
        place: &Place,
        context: &Context,
        pane_id: &str,
        request_id: &str,
        action: &Action,
        material: Option<ActionMaterial>,
    ) -> Result<ActionResult, Refusal> {
        if let Action::OpenBrowser {
            url,
            area_id,
            new_target,
        } = action
        {
            if !crate::view_layout::browser_address(url) {
                return Err(Refusal {
                    reason: "invalid_address",
                    next_action: "Use an http, https, or checkout HTML address",
                });
            }
            if hide_platform::path::is_file_url(url) && material.is_none() {
                return Err(Refusal {
                    reason: "html_unavailable",
                    next_action: "Check the HTML file and retry",
                });
            }
            let load = self.next_browser_load();
            // The page the checkout holds already: a tray takes it (D-20),
            // no tray leaves it where it is; bound to an area, only a page of
            // that area counts.
            let existing = (!*new_target)
                .then(|| {
                    self.claim_existing_view(key, place, |display| {
                        display.kind == DisplayKind::Browser && display.url.as_deref() == Some(url)
                    })
                })
                .flatten()
                .filter(|(id, at)| {
                    area_id.as_ref().is_none_or(|area| {
                        self.view_layout_at(key, at)
                            .and_then(|layout| layout.area_of(id))
                            .is_some_and(|held| held.id == *area)
                    })
                });
            let at = existing
                .as_ref()
                .map_or_else(|| place.clone(), |(_, at)| at.clone());
            let view_id = self
                .change_view_layout(key, &at, |layout, stamp| {
                    if let Some((id, _)) = &existing {
                        layout.focus(id, stamp)?;
                        if let Some(display) = layout.display_mut(id) {
                            display.load = load;
                        }
                        return Ok((id.clone(), true));
                    }
                    let area_id = area_id
                        .as_ref()
                        .filter(|area| layout.area(area).is_some())
                        .unwrap_or(&layout.active_area().id)
                        .clone();
                    let display = layout.new_browser_display(url, load);
                    let id = display.id.clone();
                    layout.insert(&area_id, display, stamp)?;
                    Ok((id, true))
                })
                .map_err(layout_refusal)?;
            let area_id = self.view_area(key, &view_id);
            return Ok(ActionResult {
                context: context.clone(),
                request_id: request_id.to_owned(),
                changed: true,
                view_id,
                area_id,
                load: Some(load),
            });
        }
        if let Action::OpenExternal { grant, name, kind } = action {
            let file = crate::view_layout::ExternalFile::admitted(grant, name, *kind)
                .map_err(|_| outside_file_refused())?;
            if !self.separate_view_areas() {
                return Err(Refusal {
                    reason: "views_unavailable",
                    next_action: "Open this Workspace in the Hide desktop app and retry",
                });
            }
            let (view_id, load) = self
                .put_external(key, place, file)
                .map_err(layout_refusal)?;
            crate::diagnostic!(serde_json::json!({
                "component": "workspace_control",
                "kind": "external.open",
                "grant": grant,
                "file_kind": kind,
                "request_id": request_id,
            }));
            let area_id = self.view_area(key, &view_id);
            return Ok(ActionResult {
                context: context.clone(),
                request_id: request_id.to_owned(),
                changed: true,
                view_id,
                area_id,
                load: Some(load),
            });
        }
        if let Action::OpenFile { .. } | Action::OpenDiff { .. } = action {
            let Some(material) = material else {
                return Err(Refusal {
                    reason: "read_missing",
                    next_action: "Retry the command with the same request ID",
                });
            };
            let path = &material.path;
            let (workspace_id, checkout_id) = (&material.checkout.0, &material.checkout.1);
            // The read let go of the lock, and the checkout that holds the
            // file may have gone since; a tab for it would have no owner.
            if self.catalog_checkout(workspace_id, checkout_id).is_none() {
                return Err(Refusal {
                    reason: "file_unavailable",
                    next_action: "The checkout that holds the file is gone; open it from where it is now",
                });
            }
            let is_file = matches!(action, Action::OpenFile { .. });
            if !self.admit_view_open(
                key,
                place,
                path,
                if is_file {
                    DisplayKind::File
                } else {
                    DisplayKind::Diff
                },
                if is_file { None } else { Some(false) },
                false,
                false,
            ) {
                return Err(Refusal {
                    reason: "view_limit",
                    next_action: "Close an unused View or area and retry",
                });
            }
            let before_generation = self
                .workspace_views
                .as_ref()
                .map(|store| store.generation)
                .unwrap_or_default();
            let tab_id = if is_file {
                let existing = self.snapshot.editor.tabs.iter().find(|tab| {
                    tab.workspace_id == *workspace_id
                        && tab.checkout_id == *checkout_id
                        && tab.kind == crate::model::EditorTabKind::File
                        && tab.path == *path
                        && tab.unavailable_reason.is_none()
                });
                if let Some(existing) = existing {
                    let tab_id = existing.id.clone();
                    // An agent that opens a file again after writing it sees
                    // what the file says now, as a page opened again loads
                    // again; a tab holding the operator's edit, save or
                    // conflict keeps it (`follow_document`).
                    if let Some((document, place)) = material.file
                        && self.follows_disk(&tab_id, &document)
                    {
                        self.replace_document(&tab_id, document, place);
                    }
                    tab_id
                } else {
                    let Some((document, place)) = material.file else {
                        return Err(Refusal {
                            reason: "read_missing",
                            next_action: "Retry the command with the same request ID",
                        });
                    };
                    let tab_id = self.new_file_tab_id(workspace_id, checkout_id, path);
                    self.insert_file_tab(
                        super::editor::PreparedFileTab::Read {
                            tab_id: tab_id.clone(),
                            document: Box::new(document),
                            place,
                        },
                        workspace_id,
                        checkout_id,
                        path,
                        false,
                    )
                    .ok_or(Refusal {
                        reason: "file_unavailable",
                        next_action: "Retry after refreshing the Workspace",
                    })?
                }
            } else {
                let tab_id = Self::diff_tab_id(workspace_id, checkout_id, path, false);
                if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
                    self.insert_diff_tab(workspace_id, checkout_id, path, false, false);
                }
                tab_id
            };
            self.place_document(key, place, &tab_id, false, false, None, None);
            let kind = if is_file {
                DisplayKind::File
            } else {
                DisplayKind::Diff
            };
            let committed = if is_file { None } else { Some(false) };
            // The View that shows it: the one at `place`, else the one a tray
            // kept when no tray took it (D-20).
            let view_id = self
                .view_layout_at(key, place)
                .and_then(|layout| {
                    layout
                        .displays()
                        .find(|display| display.shows(path, kind, committed))
                })
                .or_else(|| {
                    self.view_displays(key)
                        .find(|display| display.shows(path, kind, committed))
                })
                .map(|display| display.id.clone())
                .ok_or(Refusal {
                    reason: "view_placement_failed",
                    next_action: "Run hide view list to inspect the Workspace and retry",
                })?;
            let area_id = self.view_area(key, &view_id);
            return Ok(ActionResult {
                context: context.clone(),
                request_id: request_id.to_owned(),
                changed: self
                    .workspace_views
                    .as_ref()
                    .is_some_and(|store| store.generation != before_generation),
                view_id,
                area_id,
                load: None,
            });
        }
        let (view_id, changed, area_id) = match action {
            Action::OpenFile { .. } | Action::OpenDiff { .. } => unreachable!("handled above"),
            Action::OpenBrowser { .. } | Action::OpenExternal { .. } => {
                unreachable!("handled above")
            }
            Action::Select {
                view_id,
                expected_browser_area,
            } => {
                let Some(at) = self.view_place(key, view_id) else {
                    return Err(layout_refusal(
                        crate::view_layout::LayoutError::UnknownDisplay(view_id.clone()),
                    ));
                };
                // A select follows the open rule (D-36): the watched pane's
                // agent brings the View onto its tab's tray; anyone else's
                // marks it To look at where it is. A page's own action
                // selects it in its own tree.
                let target = if expected_browser_area.is_some() {
                    at.clone()
                } else {
                    place.clone()
                };
                let moved = match &target {
                    Place::Tray(_) if target != at => {
                        if !self.bring_view_to(key, view_id, &target) {
                            return Err(layout_refusal(
                                crate::view_layout::LayoutError::DisplayLimit,
                            ));
                        }
                        true
                    }
                    _ => false,
                };
                let selected = match &target {
                    Place::Shelf | Place::Desk => false,
                    Place::Tray(_) => self
                        .change_view_layout(key, &target, |layout, stamp| {
                            layout
                                .focus(view_id, stamp)
                                .map(|changed| (changed, changed))
                        })
                        .map_err(layout_refusal)?,
                };
                // Selected where the operator does not see it (on no desk, or
                // on a desk whose lane is closed), an agent's select is To
                // look at (B2).
                let unseen = match place {
                    Place::Shelf | Place::Desk => true,
                    Place::Tray(tab) => !self.tray_in_view(key, tab),
                };
                if unseen
                    && expected_browser_area.is_none()
                    && let Caller::Pane(_) = Caller::parse(pane_id)
                {
                    self.mark_view_unseen(key, view_id);
                }
                (
                    view_id.clone(),
                    moved || selected,
                    self.view_area(key, view_id),
                )
            }
            Action::Split {
                view_id,
                area_id,
                edge,
            } => {
                let at = self.placed_view(key, view_id)?;
                let area = self
                    .change_view_layout(key, &at, |layout, stamp| {
                        layout
                            .split(view_id, area_id, *edge, stamp)
                            .map(|area| (area, true))
                    })
                    .map_err(layout_refusal)?;
                (view_id.clone(), true, Some(area))
            }
            Action::Move {
                view_id,
                area_id,
                index,
            } => {
                let at = self.placed_view(key, view_id)?;
                let changed = self
                    .change_view_layout(key, &at, |layout, stamp| {
                        layout
                            .move_display(view_id, area_id, *index, stamp)
                            .map(|changed| (changed, changed))
                    })
                    .map_err(layout_refusal)?;
                (view_id.clone(), changed, Some(area_id.clone()))
            }
            Action::Read { view_id } => {
                if self.view_display(key, view_id).is_none() {
                    return Err(Refusal {
                        reason: "view_unknown",
                        next_action: "Run hide view list to choose a current View and retry",
                    });
                }
                self.note_ancestor_read(key, view_id, pane_id);
                (view_id.clone(), false, None)
            }
            Action::Close { view_id, .. } => {
                if self.refuse_fixed_view(key, view_id, "close") {
                    return Err(Refusal {
                        reason: "view_fixed",
                        next_action: "A run's summary stays on its desk; close the Views its agent opened instead",
                    });
                }
                let Some(display) = self.view_display(key, view_id).cloned() else {
                    return Ok(ActionResult {
                        context: context.clone(),
                        request_id: request_id.to_owned(),
                        changed: false,
                        view_id: view_id.clone(),
                        area_id: None,
                        load: None,
                    });
                };
                let shared = self.view_displays(key).any(|other| {
                    other.id != *view_id
                        && match &display.tab_id {
                            Some(tab) => other.tab_id.as_ref() == Some(tab),
                            None => other.shows(&display.path, display.kind, display.committed),
                        }
                });
                if !shared
                    && display
                        .tab_id
                        .as_deref()
                        .is_some_and(|tab| self.document_kept(tab))
                {
                    return Err(Refusal {
                        reason: "unsaved_document",
                        next_action: "Save or discard the draft in Hide before closing its last View",
                    });
                }
                let area = self.view_area(key, view_id);
                let at = self.view_place(key, view_id);
                if !shared {
                    if let Some(tab) = display.tab_id.as_deref() {
                        self.close_file_tab_now(tab);
                        self.reconcile_view_displays();
                    } else {
                        self.change_view(key, view_id, |layout, _| {
                            Ok(((), layout.remove(view_id).is_some()))
                        })
                        .map_err(layout_refusal)?;
                        if display.kind == DisplayKind::File {
                            self.cancel_document_read(
                                &context.workspace_id,
                                &context.checkout_id,
                                &display.path,
                            );
                        }
                    }
                } else {
                    self.change_view(key, view_id, |layout, _| {
                        Ok(((), layout.remove(view_id).is_some()))
                    })
                    .map_err(layout_refusal)?;
                }
                // An agent's close leaves the desk shown as it was (PRD
                // right-lane B12, lane-desk-pair-view B33); a pair view stays
                // on and its desk settles (D-64).
                if matches!(at, Some(Place::Tray(_))) {
                    self.keep_pair_front_shown();
                }
                (view_id.clone(), true, area)
            }
        };
        Ok(ActionResult {
            context: context.clone(),
            request_id: request_id.to_owned(),
            changed,
            view_id,
            area_id,
            load: None,
        })
    }

    /// The tray `view_id` is on, which `split` and `move` act in; a View on
    /// no tray is the operator's to put there first (PRD
    /// lane-desk-pair-view B33, D-36).
    fn placed_view(&self, key: &(String, String), view_id: &str) -> Result<Place, Refusal> {
        match self.view_place(key, view_id) {
            Some(Place::Tray(tab)) => Ok(Place::Tray(tab)),
            Some(Place::Shelf | Place::Desk) => Err(VIEW_NOT_IN_LANE),
            None => Err(layout_refusal(
                crate::view_layout::LayoutError::UnknownDisplay(view_id.to_owned()),
            )),
        }
    }

    /// The area that holds `view_id`, on whichever tree it is.
    fn view_area(&self, key: &(String, String), view_id: &str) -> Option<String> {
        let place = self.view_place(key, view_id)?;
        self.view_layout_at(key, &place)?
            .area_of(view_id)
            .map(|area| area.id.clone())
    }

    /// The desk that holds the area `area_id`; the shelf's area is no
    /// command's to name.
    fn area_place(&self, key: &(String, String), area_id: &str) -> Option<Place> {
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)?
            .layouts()
            .find(|(place, layout)| {
                matches!(place, Place::Tray(_)) && layout.area(area_id).is_some()
            })
            .map(|(place, _)| place)
    }

    /// Whether the operator sees `tab`'s tray when a tab lands on it: as
    /// focus mode draws it beside the tab's pane, else while the lane is
    /// open (PRD lane-desk-pair-view B2, D-53), or while the automation
    /// screen shows the run whose terminal is in that tab, which draws its
    /// desk beside it (`drawn_tray_layout`; operator choice 2026-11-24).
    pub(super) fn tray_in_view(&self, key: &(String, String), tab: &str) -> bool {
        self.pair_desk_drawn(key, tab).unwrap_or_else(|| {
            self.workspace_views
                .as_ref()
                .and_then(|store| store.views.get(&key.0, &key.1))
                .is_some_and(|view| view.open)
        }) || self.viewed_run_tab(key, tab)
    }

    /// Whether the machine `device_id` is connected now: this machine's
    /// engine, or the device's link.
    pub(super) fn workspace_connected(&self, device_id: &str) -> bool {
        if self.node == device_id {
            self.snapshot.status.engine.state == "connected"
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == device_id && remote.state == "connected")
        }
    }

    /// Resolves the caller to its checkout among the connected workspaces of
    /// `device_id`. A pane is found by membership; a checkout caller by the
    /// longest registered checkout path that equals or contains its cwd.
    pub(super) fn resolve_caller(
        &self,
        device_id: &str,
        caller: Caller<'_>,
    ) -> Result<Context, Refusal> {
        if caller == Caller::Desk {
            return Err(DESK_REFUSAL);
        }
        // The folder a checkout caller, or an external tool, ran its command
        // in; a tool the kernel named no folder for resolves none (D-28).
        let folder = match caller {
            Caller::Checkout { path, .. } => Some(path),
            Caller::Access { path, .. } => path,
            Caller::Pane(_) | Caller::Desk => None,
        };
        let mut found: Option<Context> = None;
        for workspace in self.catalog_workspaces() {
            if workspace.device_id != device_id {
                continue;
            }
            if !self.workspace_connected(&workspace.device_id) {
                continue;
            }
            for checkout in &workspace.checkouts {
                let context = || Context {
                    device_id: workspace.device_id.clone(),
                    workspace_id: workspace.id.clone(),
                    checkout_id: checkout.id.clone(),
                    checkout_path: checkout.path.clone(),
                };
                match caller {
                    Caller::Pane(pane_id) => {
                        if checkout
                            .tabs
                            .iter()
                            .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                        {
                            if found.is_some() {
                                return Err(Refusal {
                                    reason: "ambiguous_pane",
                                    next_action: "Reconnect the pane and run hide workspace info again",
                                });
                            }
                            found = Some(context());
                        }
                    }
                    Caller::Checkout { .. } | Caller::Access { .. } => {
                        let Some(path) = folder else {
                            continue;
                        };
                        let root = checkout.path.trim_end_matches('/');
                        if root.is_empty() || !path_within(path, root) {
                            continue;
                        }
                        match &found {
                            Some(best)
                                if best.checkout_path.trim_end_matches('/').len() > root.len() => {}
                            Some(best)
                                if best.checkout_path.trim_end_matches('/').len() == root.len() =>
                            {
                                if best.checkout_id != checkout.id {
                                    return Err(Refusal {
                                        reason: "ambiguous_checkout",
                                        next_action: "Register this checkout once in Hide and retry",
                                    });
                                }
                            }
                            _ => found = Some(context()),
                        }
                    }
                    // Refused before the loop.
                    Caller::Desk => {}
                }
            }
        }
        found.ok_or(match caller {
            Caller::Desk => DESK_REFUSAL,
            Caller::Pane(_) => Refusal {
                reason: "pane_not_connected",
                next_action: "Reconnect the pane in Hide and run hide workspace info again",
            },
            Caller::Checkout { .. } => Refusal {
                reason: "checkout_not_registered",
                next_action: "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry",
            },
            Caller::Access { .. } => ACCESS_OUTSIDE_CHECKOUT,
        })
    }

    pub fn workspace_control_query(
        &self,
        device_id: &str,
        pane_id: &str,
        query: Query,
    ) -> Result<QueryResult, Refusal> {
        let context = self.resolve_caller(device_id, Caller::parse(pane_id))?;
        let key = (context.device_id.clone(), context.checkout_path.clone());
        if self.workspace_views.is_none() {
            return Err(Refusal {
                reason: "views_unavailable",
                next_action: "Open this Workspace in a Hide web or desktop window and retry",
            });
        }
        // An untouched Workspace has no stored row yet. Its View tree is the
        // default one-empty-area layout, even when it is not the front one.
        let empty = crate::view_layout::Layout::default();
        let trees: Vec<(Place, &crate::view_layout::Layout)> = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .map(|view| view.layouts().collect())
            .unwrap_or_else(|| vec![(self.front_place(&key), &empty)]);
        let front = self.front_place(&key);
        let caller_session = match Caller::parse(pane_id) {
            Caller::Pane(pane) => self.session_scope_of_pane(pane),
            Caller::Checkout { .. } | Caller::Access { .. } | Caller::Desk => None,
        };
        // Every View of the checkout, on a tray or not (D-19); the area in
        // use is the tray in front's.
        let views = (query == Query::ViewList).then(|| {
            let (context, caller_session) = (&context, &caller_session);
            trees
                .iter()
                .flat_map(|(place, layout)| {
                    let in_front = *place == front;
                    layout
                        .areas()
                        .into_iter()
                        .map(move |area| (in_front, layout, area))
                })
                .flat_map(|(in_front, layout, area)| {
                    // A run's summary is the operator's page, not a View an agent can act on: it cannot be closed or moved, so it is not listed.
                    area.displays
                        .iter()
                        .filter(|display| !display.is_run_summary())
                        .map(move |display| View {
                            area_id: area.id.clone(),
                            view_id: display.id.clone(),
                            kind: match display.kind {
                                DisplayKind::File => "file",
                                DisplayKind::Diff => "diff",
                                DisplayKind::Browser => "browser",
                                DisplayKind::External => "external",
                                DisplayKind::Card => "card",
                            },
                            // An outside file's path stays in the operator's
                            // window; an agent reads that a View is there and
                            // nothing of the file (PRD outside-path-in-hide D-09).
                            target: display.url.as_ref().unwrap_or(&display.path).clone(),
                            selected: area.active.as_deref() == Some(display.id.as_str()),
                            active_area: in_front && layout.active_area().id == area.id,
                            page: (display.kind == DisplayKind::Browser).then(|| {
                                let reported = self
                                    .browser_pages
                                    .get(&(
                                        context.device_id.clone(),
                                        context.checkout_path.clone(),
                                        display.id.clone(),
                                    ))
                                    .filter(|page| page.load == display.load);
                                crate::workspace_control::BrowserPage {
                                    load: display.load,
                                    state: match reported {
                                        None => "pending",
                                        // The host unloaded the page (hidden and idle,
                                        // or past its live-page cap); it loads again
                                        // when shown or driven.
                                        Some(page) if !page.present => "unloaded",
                                        Some(page) if page.failure.is_some() => "failed",
                                        Some(page) if page.loading => "loading",
                                        Some(_) => "loaded",
                                    },
                                    failure: reported.and_then(|page| page.failure.clone()),
                                }
                            }),
                            openers: display
                                .openers
                                .iter()
                                .map(|opener| {
                                    let now = self.view_opener_snapshot(opener);
                                    crate::workspace_control::ViewOpener {
                                        name: now.name,
                                        provider: now.provider,
                                        session: opener.resume.clone(),
                                        pane_id: now.pane_id,
                                        here: caller_session
                                            .as_ref()
                                            .is_some_and(|scope| opener.of_session(scope)),
                                    }
                                })
                                .collect(),
                            operator: display.operator || display.openers.is_empty(),
                            unseen: display.unseen.is_some(),
                        })
                })
                .collect()
        });
        let mut capabilities = vec![
            "workspace.info",
            "view.list",
            "view.select",
            "view.split",
            "view.move",
            "view.close",
            "browser.open",
            "browser.status",
        ];
        capabilities.extend(crate::workspace_control::BROWSER_PAGE_CAPABILITIES);
        // Delivery acts as one agent, which only a pane-bound caller names.
        if matches!(Caller::parse(pane_id), Caller::Pane(_)) {
            capabilities.extend([
                "request.send",
                "request.reply",
                "request.show",
                "request.ack",
                "request.cancel",
                "inbox",
                "inbox.hook",
                "inbox.confirm",
                "watch.start",
                "watch.assign",
                "watch.stop",
                "watch.list",
            ]);
        }
        if self.node == context.device_id
            || self
                .device_hosts
                .get(&context.device_id)
                .is_some_and(|host| matches!(host.phase, super::hosts::HostPhase::Ready { .. }))
        {
            capabilities.extend(["file.open", "diff.open"]);
        }
        Ok(QueryResult {
            context,
            capabilities,
            views,
        })
    }
}

/// `split` and `move` of a View on no tray (PRD lane-desk-pair-view D-36).
const VIEW_NOT_IN_LANE: Refusal = Refusal {
    reason: "view_not_in_lane",
    next_action: "The operator has not put this View on their tab's Views yet; it waits in your pane's Views list until they open it, so leave its layout to them",
};

/// Desk has no pane and no checkout: it acts through delivery commands
/// (`hide agent`, `hide request`, `hide inbox`, `hide watch`, `hide ps`) and
/// shows a file in its own tab panel (`desk/opens.rs`), never through a
/// Workspace or View command.
pub(super) const DESK_REFUSAL: Refusal = Refusal {
    reason: "desk_has_no_checkout",
    next_action: "Use hide agent, hide request, hide inbox or hide ps; show the operator a file with hide file open <path>",
};

/// The refusal for a caller whose checkout membership no longer matches what
/// its capability recorded.
fn caller_changed(caller_id: &str) -> Refusal {
    match Caller::parse(caller_id) {
        Caller::Desk => DESK_REFUSAL,
        Caller::Pane(_) => Refusal {
            reason: "pane_changed",
            next_action: "Reconnect the pane and run hide workspace info again",
        },
        Caller::Checkout { .. } => Refusal {
            reason: "checkout_not_registered",
            next_action: "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry",
        },
        Caller::Access { .. } => ACCESS_OUTSIDE_CHECKOUT,
    }
}

/// An external tool's Workspace command from a folder no registered checkout
/// holds: it names the project with `--repo` (PRD hide-external-control
/// D-57), or runs from inside it.
const ACCESS_OUTSIDE_CHECKOUT: Refusal = Refusal {
    reason: "checkout_not_registered",
    next_action: "Name the project with --repo <folder> (hide file open, diff open and browser open take it), or run the command from inside a project Hide shows",
};

/// `path` is `root` itself or a descendant of it, by whole path segments, so
/// `/checkouts/ab` is not within `/checkouts/a`.
pub(super) fn path_within(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// An `open_external` whose grant or name is not one the desktop host makes,
/// which only a broken hided would send.
fn outside_file_refused() -> Refusal {
    Refusal {
        reason: "outside_file_unavailable",
        next_action: "Reconnect the Hide desktop app and retry",
    }
}

fn layout_refusal(error: crate::view_layout::LayoutError) -> Refusal {
    Refusal {
        reason: error.kind(),
        next_action: "Run hide view list, choose an available View or area, and retry",
    }
}

/// Whether an action opens or selects a view for the operator to look at. A
/// browser page's own target, bound to its area, calls nothing: the page is
/// where it already was.
pub(super) fn lane_call(action: &Action) -> bool {
    match action {
        Action::OpenFile { .. } | Action::OpenDiff { .. } | Action::OpenExternal { .. } => true,
        Action::OpenBrowser { area_id, .. } => area_id.is_none(),
        Action::Select {
            expected_browser_area,
            ..
        } => expected_browser_area.is_none(),
        Action::Split { .. } | Action::Move { .. } | Action::Close { .. } | Action::Read { .. } => {
            false
        }
    }
}

/// Whether an action keeps a receipt for its retry: everything but a read.
fn keeps_receipt(action: &Action) -> bool {
    !matches!(action, Action::Read { .. })
}
