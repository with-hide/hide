//! Pane-scoped queries and actions against the live Herdr projection.

use super::{AreaIntent, Runtime};
use crate::view_layout::DisplayKind;
use crate::workspace_control::{
    Action, ActionMaterial, ActionPreparation, ActionResult, ActionSource, BrowserRouteSource,
    Caller, Context, Query, QueryResult, Refusal, View,
};

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
            .view_layout_of(key)
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
            let empty = crate::view_layout::Layout::default();
            let known = self
                .view_layout_of(key)
                .map_or(empty.area(area_id), |layout| layout.area(area_id))
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
        let connected = if device_id == self.node.as_str() {
            self.snapshot.status.herdr.state == "connected"
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
        if !registered {
            return None;
        }
        let key = (device_id.to_owned(), checkout_path.to_owned());
        let display = self.view_displays(&key).find(|display| {
            display.id == view_id && display.kind == DisplayKind::Browser && display.load == load
        })?;
        Some(BrowserRouteSource {
            device_id: device_id.to_owned(),
            checkout_path: checkout_path.to_owned(),
            url: display.url.clone()?,
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
        let preparation =
            self.prepare_view_action(device_id, pane_id, expected, request_id, action)?;
        if !self.workspace_actions.iter().any(|record| {
            record.device_id == device_id
                && record.pane_id == pane_id
                && record.request_id == request_id
        }) {
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
            if crate::view_layout::is_file_address(url) {
                let path = crate::workspace_control::local_file_path(url).ok_or(Refusal {
                    reason: "invalid_address",
                    next_action: "Use a local file URL with an absolute path",
                })?;
                let (root, channel) = self
                    .document_source(&context.workspace_id, &context.checkout_id)
                    .map_err(|_| Refusal {
                        reason: "host_unavailable",
                        next_action: "Reconnect the device and retry",
                    })?;
                return Ok(ActionPreparation::Read(ActionSource::browser(
                    root, channel, path,
                )));
            }
            return Ok(ActionPreparation::Ready);
        }
        let (path, file) = match action {
            Action::OpenFile { path, .. } => (path, true),
            Action::OpenDiff { path, .. } => (path, false),
            _ => return Ok(ActionPreparation::Ready),
        };
        if !std::path::Path::new(path).is_absolute() {
            return Err(Refusal {
                reason: "invalid_path",
                next_action: "Resolve the path from the caller's cwd and retry",
            });
        }
        let already_open = file
            && self.snapshot.editor.tabs.iter().any(|tab| {
                tab.workspace_id == context.workspace_id
                    && tab.checkout_id == context.checkout_id
                    && tab.path == *path
                    && tab.kind == crate::model::EditorTabKind::File
                    && tab.unavailable_reason.is_none()
            });
        let (root, channel) = self
            .document_source(&context.workspace_id, &context.checkout_id)
            .map_err(|_| Refusal {
                reason: "host_unavailable",
                next_action: "Reconnect the device and check file access consent, then retry",
            })?;
        let source = if file {
            ActionSource::file(root, channel, path.clone(), already_open)
        } else {
            ActionSource::diff(root, channel, path.clone())
        };
        Ok(ActionPreparation::Read(source))
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
            if self.open_adds_view(
                &key,
                &action,
                material.as_ref().map(|material| material.path.as_str()),
            ) {
                self.make_view_room(&key);
            }
            // On the Workspace in front only the agent in the pane the
            // operator is in changes what they see; another pane, of the same
            // tab or not, leaves the selection, the lane and every tab's
            // bookmark as they were.
            let kept = self
                .caller_stays_behind(&key, pane_id)
                .then(|| self.keep_front(&key));
            // An open the operator is looking at calls the lane as a click
            // does (PRD right-lane B17).
            let calls_lane = kept.is_none()
                && self.front_workspace_key().as_ref() == Some(&key)
                && lane_call(&action);
            let mut applied =
                self.apply_workspace_action(&key, &context, request_id, &action, material);
            if let Ok(done) = &mut applied {
                self.stamp_control_opener(&key, pane_id, &action, &done.view_id);
                if calls_lane {
                    self.apply_area_intent_to(&key, AreaIntent::Documents);
                    // A view already in front still calls the lane, which
                    // the page sees only if the change is announced.
                    done.changed = true;
                }
            }
            if let Some(kept) = kept {
                self.restore_front(kept);
            }
            applied
        })();
        if result.is_err()
            && let (Some(before), Some(store)) = (browser_load, self.workspace_views.as_mut())
        {
            store.browser_load = before;
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

    fn apply_workspace_action(
        &mut self,
        key: &(String, String),
        context: &Context,
        request_id: &str,
        action: &Action,
        material: Option<ActionMaterial>,
    ) -> Result<ActionResult, Refusal> {
        if let Action::OpenBrowser {
            url,
            area_id,
            new_target,
            beside,
        } = action
        {
            if !crate::view_layout::browser_address(url) {
                return Err(Refusal {
                    reason: "invalid_address",
                    next_action: "Use an http, https, or checkout HTML address",
                });
            }
            if crate::view_layout::is_file_address(url) && material.is_none() {
                return Err(Refusal {
                    reason: "html_unavailable",
                    next_action: "Check the HTML file and retry",
                });
            }
            let load = self.next_browser_load();
            let view_id = if *beside && area_id.is_none() {
                self.change_view_layout(key, |layout, stamp| {
                    super::view_areas::place_page_beside(layout, url, load, stamp)
                        .map(|id| (id, true))
                })
                .map_err(|error| Refusal {
                    reason: error.kind(),
                    next_action: match error {
                        crate::view_layout::LayoutError::AreaLimit
                        | crate::view_layout::LayoutError::DepthLimit => {
                            "There is no room for another View area beside this one; close an area's last View with hide view close <view-id>, or open without --beside, and retry"
                        }
                        crate::view_layout::LayoutError::DisplayLimit => {
                            "The Workspace holds as many Views as it can; close one with hide view close <view-id> and retry"
                        }
                        _ => "Run hide view list, choose an available View or area, and retry",
                    },
                })?
            } else {
                self.change_view_layout(key, |layout, stamp| {
                    let existing = if *new_target {
                        None
                    } else {
                        layout
                            .areas()
                            .into_iter()
                            .filter(|area| area_id.as_ref().is_none_or(|id| area.id == *id))
                            .flat_map(|area| area.displays.iter())
                            .find(|display| {
                                display.kind == DisplayKind::Browser
                                    && display.url.as_deref() == Some(url)
                            })
                            .map(|display| display.id.clone())
                    };
                    if let Some(id) = existing {
                        layout.focus(&id, stamp)?;
                        if let Some(display) = layout.display_mut(&id) {
                            display.load = load;
                        }
                        return Ok((id, true));
                    }
                    let area_id = area_id.as_ref().unwrap_or(&layout.active_area().id).clone();
                    let display = layout.new_browser_display(url, load);
                    let id = display.id.clone();
                    layout.insert(&area_id, display, stamp)?;
                    Ok((id, true))
                })
                .map_err(layout_refusal)?
            };
            let area_id = self
                .view_layout_of(key)
                .and_then(|layout| layout.area_of(&view_id))
                .map(|area| area.id.clone());
            return Ok(ActionResult {
                context: context.clone(),
                request_id: request_id.to_owned(),
                changed: true,
                view_id,
                area_id,
                load: Some(load),
            });
        }
        if let Action::OpenFile { path: _, beside } | Action::OpenDiff { path: _, beside } = action
        {
            let Some(material) = material else {
                return Err(Refusal {
                    reason: "read_missing",
                    next_action: "Retry the command with the same request ID",
                });
            };
            let path = &material.path;
            let is_file = matches!(action, Action::OpenFile { .. });
            if !self.admit_view_open(
                key,
                path,
                if is_file {
                    DisplayKind::File
                } else {
                    DisplayKind::Diff
                },
                if is_file { None } else { Some(false) },
                false,
                *beside,
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
                    tab.workspace_id == context.workspace_id
                        && tab.checkout_id == context.checkout_id
                        && tab.kind == crate::model::EditorTabKind::File
                        && tab.path == *path
                        && tab.unavailable_reason.is_none()
                });
                if let Some(existing) = existing {
                    existing.id.clone()
                } else {
                    let Some((document, place)) = material.file else {
                        return Err(Refusal {
                            reason: "read_missing",
                            next_action: "Retry the command with the same request ID",
                        });
                    };
                    let tab_id =
                        self.new_file_tab_id(&context.workspace_id, &context.checkout_id, path);
                    self.insert_file_tab(
                        super::editor::PreparedFileTab::Read {
                            tab_id: tab_id.clone(),
                            document: Box::new(document),
                            place,
                        },
                        &context.workspace_id,
                        &context.checkout_id,
                        path,
                        false,
                    )
                    .ok_or(Refusal {
                        reason: "file_unavailable",
                        next_action: "Retry after refreshing the Workspace",
                    })?
                }
            } else {
                let tab_id =
                    Self::diff_tab_id(&context.workspace_id, &context.checkout_id, path, false);
                if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
                    self.insert_diff_tab(
                        &context.workspace_id,
                        &context.checkout_id,
                        path,
                        false,
                        false,
                    );
                }
                tab_id
            };
            self.place_document(key, &tab_id, false, *beside, None, None);
            let view_id = self
                .view_layout_of(key)
                .and_then(|layout| layout.active_area().active.clone())
                .ok_or(Refusal {
                    reason: "view_placement_failed",
                    next_action: "Run hide view list to inspect the Workspace and retry",
                })?;
            if !self.view_layout_of(key).is_some_and(|layout| {
                layout.display(&view_id).is_some_and(|display| {
                    display.shows(
                        path,
                        if is_file {
                            DisplayKind::File
                        } else {
                            DisplayKind::Diff
                        },
                        if is_file { None } else { Some(false) },
                    )
                })
            }) {
                return Err(Refusal {
                    reason: "view_placement_failed",
                    next_action: "Run hide view list to inspect the Workspace and retry",
                });
            }
            let area_id = self
                .view_layout_of(key)
                .map(|layout| layout.active_area().id.clone());
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
            Action::OpenBrowser { .. } => unreachable!("handled above"),
            Action::Select { view_id, .. } => {
                let selected = self
                    .change_view_layout(key, |layout, stamp| {
                        layout
                            .focus(view_id, stamp)
                            .map(|changed| (changed, changed))
                    })
                    .map_err(layout_refusal)?;
                let area = self
                    .view_layout_of(key)
                    .and_then(|layout| layout.area_of(view_id))
                    .map(|area| area.id.clone());
                (view_id.clone(), selected, area)
            }
            Action::Split {
                view_id,
                area_id,
                edge,
            } => {
                let area = self
                    .change_view_layout(key, |layout, stamp| {
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
                let changed = self
                    .change_view_layout(key, |layout, stamp| {
                        layout
                            .move_display(view_id, area_id, *index, stamp)
                            .map(|changed| (changed, changed))
                    })
                    .map_err(layout_refusal)?;
                (view_id.clone(), changed, Some(area_id.clone()))
            }
            Action::Close { view_id, .. } => {
                let Some(display) = self
                    .view_layout_of(key)
                    .and_then(|layout| layout.display(view_id))
                    .cloned()
                else {
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
                let area = self
                    .view_layout_of(key)
                    .and_then(|layout| layout.area_of(view_id))
                    .map(|area| area.id.clone());
                if !shared {
                    if let Some(tab) = display.tab_id.as_deref() {
                        self.close_file_tab_now(tab);
                        self.reconcile_view_displays();
                    } else {
                        self.change_view_layout(key, |layout, _| {
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
                    self.change_view_layout(key, |layout, _| {
                        Ok(((), layout.remove(view_id).is_some()))
                    })
                    .map_err(layout_refusal)?;
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

    /// Resolves the caller to its checkout among the connected workspaces of
    /// `device_id`. A pane is found by membership; a checkout caller by the
    /// longest registered checkout path that equals or contains its cwd.
    pub(super) fn resolve_caller(
        &self,
        device_id: &str,
        caller: Caller<'_>,
    ) -> Result<Context, Refusal> {
        let mut found: Option<Context> = None;
        for workspace in self.catalog_workspaces() {
            if workspace.device_id != device_id {
                continue;
            }
            let connected = if self.node == workspace.device_id {
                self.snapshot.status.herdr.state == "connected"
            } else {
                self.snapshot.status.remote.iter().any(|remote| {
                    remote.target_id == workspace.device_id && remote.state == "connected"
                })
            };
            if !connected {
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
                    Caller::Checkout { path, .. } => {
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
                }
            }
        }
        found.ok_or(match caller {
            Caller::Pane(_) => Refusal {
                reason: "pane_not_connected",
                next_action: "Reconnect the pane in Hide and run hide workspace info again",
            },
            Caller::Checkout { .. } => Refusal {
                reason: "checkout_not_registered",
                next_action: "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry",
            },
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
        let layout = self.view_layout_of(&key).unwrap_or(&empty);
        let caller_session = match Caller::parse(pane_id) {
            Caller::Pane(pane) => self.session_scope_of_pane(pane),
            Caller::Checkout { .. } => None,
        };
        let views = (query == Query::ViewList).then(|| {
            layout
                .areas()
                .into_iter()
                .flat_map(|area| {
                    area.displays.iter().map(|display| View {
                        area_id: area.id.clone(),
                        view_id: display.id.clone(),
                        kind: match display.kind {
                            DisplayKind::File => "file",
                            DisplayKind::Diff => "diff",
                            DisplayKind::Browser => "browser",
                            DisplayKind::External => "external",
                        },
                        // An outside file's path stays in the operator's
                        // window; an agent reads that a View is there and
                        // nothing of the file (PRD outside-path-in-hide D-09).
                        target: display.url.as_ref().unwrap_or(&display.path).clone(),
                        selected: area.active.as_deref() == Some(display.id.as_str()),
                        active_area: layout.active_area().id == area.id,
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

/// The refusal for a caller whose checkout membership no longer matches what
/// its capability recorded.
fn caller_changed(caller_id: &str) -> Refusal {
    match Caller::parse(caller_id) {
        Caller::Pane(_) => Refusal {
            reason: "pane_changed",
            next_action: "Reconnect the pane and run hide workspace info again",
        },
        Caller::Checkout { .. } => Refusal {
            reason: "checkout_not_registered",
            next_action: "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry",
        },
    }
}

/// `path` is `root` itself or a descendant of it, by whole path segments, so
/// `/checkouts/ab` is not within `/checkouts/a`.
fn path_within(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
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
fn lane_call(action: &Action) -> bool {
    match action {
        Action::OpenFile { .. } | Action::OpenDiff { .. } => true,
        Action::OpenBrowser { area_id, .. } => area_id.is_none(),
        Action::Select {
            expected_browser_area,
            ..
        } => expected_browser_area.is_none(),
        Action::Split { .. } | Action::Move { .. } | Action::Close { .. } => false,
    }
}
