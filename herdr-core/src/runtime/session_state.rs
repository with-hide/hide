//! Resolve is a durable UI decision, published only after the state save succeeds.
//! All projection work is in memory. The existing save worker owns disk I/O.
use super::*;
use crate::agent_state::{escalation, sessions};

impl Runtime {
    pub(super) fn session_fold_keys(&self) -> HashSet<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.workspaces),
            )
            .map(|workspace| workspace.id.clone())
            .chain(self.snapshot.navigator.devices.iter().flat_map(|device| {
                [
                    format!("cleanup/{}", device.id),
                    crate::project_context::no_agents_fold_key(&device.id),
                ]
            }))
            .collect()
    }
    /// Uses the existing runtime clock; idle ticks compare one deadline, the
    /// moment the oldest Resolved row leaves the 24-hour window (PRD D-23).
    pub(super) fn tick_session_window(&mut self, at: u64) -> bool {
        if at < self.session_window_deadline_unix_ms {
            return false;
        }
        self.project_session_state_at(at) | self.refresh_agent_scopes()
    }

    fn session_rows(&self) -> impl Iterator<Item = &SidebarAgentSnapshot> {
        self.snapshot.navigator.agents.iter().chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter_map(|remote| remote.session.as_ref())
                .flat_map(|session| &session.agents),
        )
    }

    pub(super) fn resolve_session(&mut self, pane: &str, source: sessions::ResolveSource) -> bool {
        let Some(row) = self.session_rows().find(|row| row.pane_id == pane) else {
            self.push_diagnostic(
                "session.resolve.unavailable",
                format!("Pane {pane} is no longer an agent"),
            );
            return false;
        };
        if self.snapshot.ui_state.resolved_sessions.contains_key(pane)
            || self.pending_session_resolutions.contains_key(pane)
        {
            return false;
        }
        let record = sessions::Resolution {
            at_unix_ms: unix_milliseconds(),
            source,
            session: row.lineage_session.clone(),
            activity: row.last_activity.clone(),
        };
        self.failed_session_resolutions.remove(pane);
        self.pending_session_resolutions
            .insert(pane.to_owned(), record);
        if let Err(message) = self.write_ui_state() {
            if let Some(record) = self.pending_session_resolutions.remove(pane) {
                self.failed_session_resolutions
                    .insert(pane.to_owned(), record);
            }
            self.push_diagnostic("session.resolve.save_failed", message);
        }
        // A managed runtime waits for the worker's actual save acknowledgement.
        // A standalone runtime has already acknowledged its synchronous write.
        self.sync_session_state();
        self.refresh_agent_scopes();
        let resolved = self.snapshot.ui_state.resolved_sessions.contains_key(pane)
            || self.pending_session_resolutions.contains_key(pane);
        // The operator who resolves a row has dealt with it, so it is read:
        // 쉬는 중 never holds a row that still says it is unread.
        if resolved && source == sessions::ResolveSource::Operator {
            self.open_result(pane);
        }
        resolved
    }

    pub(super) fn acknowledge_session_save(
        &mut self,
        saved: &UiStateSnapshot,
        succeeded: bool,
    ) -> bool {
        let mut changed = false;
        for (pane, record) in &saved.resolved_sessions {
            if self.pending_session_resolutions.get(pane) != Some(record) {
                continue;
            }
            self.pending_session_resolutions.remove(pane);
            if succeeded {
                self.snapshot
                    .ui_state
                    .resolved_sessions
                    .insert(pane.clone(), record.clone());
                changed = true;
            } else {
                self.failed_session_resolutions
                    .insert(pane.clone(), record.clone());
            }
        }
        if changed {
            self.project_session_state()
                | self.refresh_inactive_groups()
                | self.refresh_agent_scopes()
        } else {
            false
        }
    }

    pub(super) fn reopen_resolved_session(&mut self, pane: &str) -> bool {
        let changed = self
            .snapshot
            .ui_state
            .resolved_sessions
            .remove(pane)
            .is_some()
            | self.pending_session_resolutions.remove(pane).is_some();
        if changed {
            self.snapshot
                .ui_state
                .session_resolution_inputs
                .insert(pane.to_owned(), unix_milliseconds());
            self.failed_session_resolutions.remove(pane);
            self.persist_ui_state();
            self.project_session_state();
            self.refresh_inactive_groups();
            self.refresh_agent_scopes();
            if let Some(context) = &self.worker_context {
                context.notifier.notify();
            }
        }
        changed
    }

    pub(super) fn prune_resolved_sessions(&mut self, gone: impl Fn(&str) -> bool) {
        let before = (
            self.snapshot.ui_state.resolved_sessions.len(),
            self.snapshot.ui_state.session_resolution_inputs.len(),
        );
        self.snapshot
            .ui_state
            .resolved_sessions
            .retain(|pane, _| !gone(pane));
        self.pending_session_resolutions
            .retain(|pane, _| !gone(pane));
        self.failed_session_resolutions
            .retain(|pane, _| !gone(pane));
        self.snapshot
            .ui_state
            .session_resolution_inputs
            .retain(|pane, _| !gone(pane));
        if before
            != (
                self.snapshot.ui_state.resolved_sessions.len(),
                self.snapshot.ui_state.session_resolution_inputs.len(),
            )
        {
            self.persist_ui_state();
        }
    }

    /// The doorbell's reasons for each pending letter after a pass at `now`.
    /// A pass that moves neither a raise nor a waiting mark publishes
    /// nothing, so the mark costs one publish when it shows and one when it
    /// goes, never one per pass.
    pub(crate) fn publish_delivery_holds(
        &mut self,
        holds: BTreeMap<String, crate::delivery::doorbell::Hold>,
        now: u64,
    ) {
        let waiting = self
            .delivery_ledger
            .as_ref()
            .ok()
            .map(|ledger| {
                crate::delivery::doorbell::letters_waiting(
                    ledger,
                    &holds,
                    &self.delivery_observations,
                    now,
                )
            })
            .unwrap_or_default();
        let holds = escalation::retain_raised_holds(&self.delivery_holds, holds);
        if self.delivery_holds == holds && self.letters_waiting == waiting {
            return;
        }
        self.delivery_holds = holds;
        self.set_letters_waiting(waiting);
        if (self.sync_session_state() | self.refresh_agent_scopes())
            && let Some(context) = &self.worker_context
        {
            context.notifier.notify();
        }
    }

    /// Logs each pane whose waiting mark appears or goes, with its count and
    /// never a letter, so a held letter shown to the operator can be traced.
    pub(super) fn set_letters_waiting(&mut self, waiting: BTreeMap<String, u32>) {
        for (pane, letters) in &waiting {
            if self.letters_waiting.get(pane) != Some(letters) {
                crate::diagnostic!(serde_json::json!({"component":"delivery",
                    "kind":"letter_mark.shown","pane_id":pane,"letters":letters}));
            }
        }
        for pane in self.letters_waiting.keys() {
            if !waiting.contains_key(pane) {
                crate::diagnostic!(serde_json::json!({"component":"delivery",
                    "kind":"letter_mark.cleared","pane_id":pane}));
            }
        }
        self.letters_waiting = waiting;
    }

    fn project_session_state(&mut self) -> bool {
        self.project_session_state_at(unix_milliseconds())
    }

    fn project_session_state_at(&mut self, at: u64) -> bool {
        let mut rows = self.session_rows().cloned().collect::<Vec<_>>();
        let mut invalid = Vec::new();
        let mut deadline = u64::MAX;
        for row in &mut rows {
            row.escalation = escalation::of(
                row,
                self.delivery_ledger.as_ref().ok().map(AsRef::as_ref),
                &self.delivery_holds,
            );
            row.letters_waiting = self.letters_waiting.get(&row.pane_id).copied();
            let record = self.snapshot.ui_state.resolved_sessions.get(&row.pane_id);
            let pending = self.pending_session_resolutions.get(&row.pane_id);
            if record.or(pending).is_some_and(|record| {
                record.session != row.lineage_session
                    || (row.activity == "working" && record.activity != row.last_activity)
            }) {
                invalid.push(row.pane_id.clone());
                row.resolved = None;
            } else {
                row.resolved = record.cloned();
            }
            let leaves = row.resolved.as_ref().map(|record| {
                record
                    .at_unix_ms
                    .saturating_add(sessions::RESOLVED_WINDOW_MS)
            });
            row.resolved_recent = leaves.is_some_and(|leaves| leaves > at);
            if let Some(leaves) = leaves.filter(|&leaves| leaves > at) {
                deadline = deadline.min(leaves);
            }
        }
        let ledger = self.delivery_ledger.as_ref().ok();
        escalation::lift(&mut rows, |id| {
            ledger?
                .letters
                .iter()
                .find(|letter| letter.id == id)
                .and_then(|letter| crate::display_text::one_line(letter.body.lines().next()?, 80))
        });
        for row in &mut rows {
            // A raise brings a resolved root back, as a new letter would.
            if !row.raised.is_empty() && row.resolved.is_some() {
                invalid.push(row.pane_id.clone());
                row.resolved = None;
                row.resolved_recent = false;
            }
            crate::agent_state::rederive(row);
        }
        self.session_window_deadline_unix_ms = deadline;
        let mut changed = false;
        for (row, next) in self
            .snapshot
            .navigator
            .agents
            .iter_mut()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter_mut()
                    .filter_map(|remote| remote.session.as_mut())
                    .flat_map(|session| &mut session.agents),
            )
            .zip(rows)
        {
            if *row != next {
                *row = next;
                changed = true;
            }
        }
        if !invalid.is_empty() {
            for pane in invalid {
                self.snapshot.ui_state.resolved_sessions.remove(&pane);
                self.pending_session_resolutions.remove(&pane);
                self.snapshot
                    .ui_state
                    .session_resolution_inputs
                    .insert(pane, unix_milliseconds());
            }
            self.persist_ui_state();
        }
        changed
    }

    pub(super) fn sync_session_state(&mut self) -> bool {
        let mut changed = self.project_session_state();
        let automatic: Vec<_> = self
            .session_rows()
            .filter(|row| {
                row.resolved.is_none()
                    && row.escalation.is_none()
                    && row.raised.is_empty()
                    && sessions::auto_resolvable(
                        row,
                        self.snapshot
                            .ui_state
                            .session_resolution_inputs
                            .get(&row.pane_id)
                            .copied(),
                    )
                    && !self.pending_session_resolutions.contains_key(&row.pane_id)
                    && self
                        .failed_session_resolutions
                        .get(&row.pane_id)
                        .is_none_or(|record| {
                            record.session != row.lineage_session
                                || record.activity != row.last_activity
                        })
            })
            .map(|row| {
                (
                    row.pane_id.clone(),
                    row.lineage_session.clone(),
                    row.last_activity.clone(),
                )
            })
            .collect();
        // Queue one batch, rather than recursively deriving each row.
        let now = unix_milliseconds();
        for (pane, session, activity) in &automatic {
            self.pending_session_resolutions.insert(
                pane.clone(),
                sessions::Resolution {
                    at_unix_ms: now,
                    source: sessions::ResolveSource::Automatic,
                    session: session.clone(),
                    activity: activity.clone(),
                },
            );
        }
        if !automatic.is_empty()
            && let Err(message) = self.write_ui_state()
        {
            for (pane, _, _) in automatic {
                self.pending_session_resolutions.remove(&pane);
            }
            self.push_diagnostic("session.resolve.save_failed", message);
        }
        changed |= self.project_session_state();
        changed
    }
}
