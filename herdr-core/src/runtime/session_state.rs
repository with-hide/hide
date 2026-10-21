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
        // 해결 puts the task away, its ★ and read ask with it (D-14).
        if source == sessions::ResolveSource::Operator
            && self.revisit_mark(pane).is_some_and(|mark| !mark.is_empty())
        {
            self.pending_revisit.insert(pane.to_owned(), None);
        }
        self.failed_session_resolutions.remove(pane);
        self.pending_session_resolutions
            .insert(pane.to_owned(), record);
        if let Err(message) = self.write_ui_state() {
            self.pending_revisit.remove(pane);
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
        self.snapshot.ui_state.resolved_sessions.contains_key(pane)
            || self.pending_session_resolutions.contains_key(pane)
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
        changed |= self.acknowledge_revisit_save(saved, succeeded);
        if changed {
            self.project_session_state()
                | self.refresh_inactive_groups()
                | self.refresh_agent_scopes()
        } else {
            false
        }
    }

    /// The mark as it stands, a change waiting on its save included.
    pub(super) fn revisit_mark(&self, pane: &str) -> Option<&sessions::RevisitMark> {
        match self.pending_revisit.get(pane) {
            Some(pending) => pending.as_ref(),
            None => self.snapshot.ui_state.revisit_sessions.get(pane),
        }
    }

    /// ★ on a task (PRD revisit-group D-03, B6). Pressing it twice changes
    /// nothing more (B12); past the cap it is refused and said once (B22).
    pub(super) fn star_session(&mut self, pane: &str) -> bool {
        let Some(row) = self.session_rows().find(|row| row.pane_id == pane) else {
            self.push_diagnostic(
                "revisit.star.unavailable",
                format!("Pane {pane} is no longer an agent"),
            );
            return false;
        };
        if row.lineage_parent_pane_id.is_some() {
            self.push_diagnostic(
                "revisit.star.not_root",
                format!("Pane {pane} is a delegated child"),
            );
            return false;
        }
        let session = row.lineage_session.clone();
        let mut mark = self.revisit_mark(pane).cloned().unwrap_or_default();
        if mark.starred_at_unix_ms.is_some() {
            return false;
        }
        let starred = self
            .snapshot
            .ui_state
            .revisit_sessions
            .keys()
            .chain(self.pending_revisit.keys())
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|key| {
                self.revisit_mark(key)
                    .is_some_and(|m| m.starred_at_unix_ms.is_some())
            })
            .count();
        if starred >= sessions::MAX_STARRED_SESSIONS {
            self.set_error(
                "revisit.capacity",
                format!(
                    "At most {} tasks can be starred to revisit",
                    sessions::MAX_STARRED_SESSIONS
                ),
                false,
            );
            return true;
        }
        mark.starred_at_unix_ms = Some(unix_milliseconds());
        mark.session = session;
        self.save_revisit(pane, Some(mark))
    }

    /// 다 봤음 (PRD revisit-group D-05, B11): the ★ goes and the read ask
    /// standing now is put away, so the task rests until something new.
    pub(super) fn finish_revisit(&mut self, pane: &str) -> bool {
        let Some(row) = self.session_rows().find(|row| row.pane_id == pane) else {
            self.push_diagnostic(
                "revisit.done.unavailable",
                format!("Pane {pane} is no longer an agent"),
            );
            return false;
        };
        let current = self.revisit_mark(pane).cloned();
        let ask = row
            .revisit
            .filter(|revisit| revisit.read_ask)
            .and(row.changed_at_unix_ms);
        let next = sessions::RevisitMark {
            starred_at_unix_ms: None,
            dismissed_ask_since: ask.or(current.as_ref().and_then(|m| m.dismissed_ask_since)),
            session: row.lineage_session.clone(),
        };
        let next = (!next.is_empty()).then_some(next);
        if next == current || (next.is_none() && current.is_none()) {
            return false;
        }
        self.save_revisit(pane, next)
    }

    /// The mark changes only once the save holds it (B23).
    fn save_revisit(&mut self, pane: &str, mark: Option<sessions::RevisitMark>) -> bool {
        self.pending_revisit.insert(pane.to_owned(), mark);
        // A synchronous save already settled and reported its failure.
        if let Err(message) = self.write_ui_state()
            && self.pending_revisit.remove(pane).is_some()
        {
            self.report_revisit_save_failed(&message);
        }
        self.sync_session_state();
        self.refresh_agent_scopes();
        true
    }

    fn report_revisit_save_failed(&mut self, message: &str) {
        self.push_diagnostic("revisit.save_failed", message.to_owned());
        self.set_error(
            "revisit.save_failed",
            "The revisit mark could not be saved",
            true,
        );
    }

    fn acknowledge_revisit_save(&mut self, saved: &UiStateSnapshot, succeeded: bool) -> bool {
        let settled: Vec<String> = self
            .pending_revisit
            .iter()
            .filter(|(pane, pending)| saved.revisit_sessions.get(*pane) == pending.as_ref())
            .map(|(pane, _)| pane.clone())
            .collect();
        let mut changed = false;
        let mut failed = false;
        for pane in settled {
            let Some(pending) = self.pending_revisit.remove(&pane) else {
                continue;
            };
            if !succeeded {
                failed = true;
                continue;
            }
            let revisits = &mut self.snapshot.ui_state.revisit_sessions;
            changed |= match pending {
                Some(mark) => revisits.insert(pane, mark.clone()).as_ref() != Some(&mark),
                None => revisits.remove(&pane).is_some(),
            };
        }
        if failed {
            self.report_revisit_save_failed("The UI state save failed");
        }
        changed
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
        let revisits = self.snapshot.ui_state.revisit_sessions.len();
        self.snapshot
            .ui_state
            .revisit_sessions
            .retain(|pane, _| !gone(pane));
        self.pending_revisit.retain(|pane, _| !gone(pane));
        if revisits != self.snapshot.ui_state.revisit_sessions.len()
            || before
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
        let mut stale_revisits = Vec::new();
        for row in &mut rows {
            // A raise brings a resolved root back, as a new letter would.
            if !row.raised.is_empty() && row.resolved.is_some() {
                invalid.push(row.pane_id.clone());
                row.resolved = None;
                row.resolved_recent = false;
            }
            crate::agent_state::rederive(row);
            // A mark belongs to the conversation it was written for (D-11).
            let mark = self
                .snapshot
                .ui_state
                .revisit_sessions
                .get(&row.pane_id)
                .filter(|mark| {
                    // Only a known conversation replacing another drops it:
                    // one not yet reported, or reported late, keeps the ★.
                    let current = match (&mark.session, &row.lineage_session) {
                        (Some(marked), Some(now)) => marked == now,
                        _ => true,
                    };
                    if !current {
                        stale_revisits.push(row.pane_id.clone());
                    }
                    current
                });
            row.revisit = sessions::revisit_of(row, mark);
        }
        if !stale_revisits.is_empty() {
            for pane in &stale_revisits {
                self.snapshot.ui_state.revisit_sessions.remove(pane);
            }
            self.persist_ui_state();
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
