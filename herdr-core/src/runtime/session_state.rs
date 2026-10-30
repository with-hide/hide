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
        if let Some(reason) = sessions::resolve_refusal(row) {
            self.push_diagnostic("session.resolve.refused", format!("Pane {pane}: {reason}"));
            return false;
        }
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
        // ☆ on an ask the operator has not read reads it: the ask goes to
        // 다시 볼 것 and stops calling for attention (needs-you-cleanup B13).
        self.read_ask_now(pane);
        mark.starred_at_unix_ms = Some(unix_milliseconds());
        mark.session = session;
        self.save_revisit(pane, Some(mark))
    }

    /// Records the unread ask on `pane` as read, as the operator's focus on
    /// it would, wherever the pane runs.
    fn read_ask_now(&mut self, pane: &str) {
        let target = if self
            .snapshot
            .navigator
            .agents
            .iter()
            .any(|row| row.pane_id == pane)
        {
            None
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| {
                    remote
                        .session
                        .as_ref()
                        .is_some_and(|session| session.agents.iter().any(|row| row.pane_id == pane))
                })
                .map(|remote| remote.target_id.clone())
        };
        let Some(row) = self
            .session_rows()
            .find(|row| row.pane_id == pane)
            .filter(|row| row.unread && matches!(row.demand.as_str(), "question" | "approval"))
            .cloned()
        else {
            return;
        };
        let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
        let change = crate::agent_state::read_now(
            &row,
            &mut records,
            self.read_connection(target.as_deref()),
        );
        self.snapshot.ui_state.pane_read_records = records;
        let Some(change) = change else {
            return;
        };
        self.record_read_record_changes(&[change]);
        // A device's rows take the record on its next sync, as its focus does.
        if target.is_none() {
            self.refresh_pane_read_state();
        }
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
            .and(sessions::ask_stamp(row));
        let next = sessions::RevisitMark {
            starred_at_unix_ms: None,
            dismissed_ask_since: ask.or(current.as_ref().and_then(|m| m.dismissed_ask_since)),
            session: row.lineage_session.clone(),
        };
        let next = (!next.is_empty()).then_some(next);
        if next == current || (next.is_none() && current.is_none()) {
            return false;
        }
        if let Some(since) = ask
            && current.as_ref().and_then(|m| m.dismissed_ask_since) != Some(since)
        {
            // An unanswered ask put away: a sign the question verdict may
            // have been wrong, with ids only (needs-you-cleanup D-06, D-15).
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "verdict.ask_put_away",
                "source": "revisit",
                "pane_id": pane,
                "turn": since,
                "ask": row.ask_subject.as_ref().map_or(row.demand.as_str(), |subject| match subject {
                    crate::model::AskSubject::PlanReview { .. } => "plan_review",
                    crate::model::AskSubject::MissionDecision { .. } => "mission_decision",
                }),
            }));
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
        let mut adopted_revisits = Vec::new();
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
                        (None, Some(now)) => {
                            adopted_revisits.push((row.pane_id.clone(), now.clone()));
                            true
                        }
                        _ => true,
                    };
                    if !current {
                        stale_revisits.push(row.pane_id.clone());
                    }
                    current
                });
            row.revisit = sessions::revisit_of(row, mark);
        }
        self.lay_session_pulls(&mut rows);
        if !stale_revisits.is_empty() || !adopted_revisits.is_empty() {
            for pane in &stale_revisits {
                self.snapshot.ui_state.revisit_sessions.remove(pane);
            }
            // The first conversation reported after the ★ becomes its own, so
            // a later one replacing it drops the mark.
            for (pane, session) in adopted_revisits {
                if let Some(mark) = self.snapshot.ui_state.revisit_sessions.get_mut(&pane) {
                    mark.session = Some(session);
                }
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
        if changed {
            self.settle_record_pulls();
        }
        changed
    }

    /// The session every row hosts, on its own machine, in
    /// [`Self::session_rows`] order: this machine's rows, then each device's.
    fn session_row_identities(&self) -> Vec<Option<crate::links::ledger::SessionIdentity>> {
        let identity = |device: &str, row: &SidebarAgentSnapshot| {
            Some(crate::links::ledger::SessionIdentity {
                device_id: device.to_owned(),
                agent: hide_session::Agent::from_kind(&row.agent_kind)?
                    .as_str()
                    .to_owned(),
                session_id: row.session_id.clone()?,
            })
        };
        self.snapshot
            .navigator
            .agents
            .iter()
            .map(|row| identity(self.node.as_str(), row))
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| Some((&remote.target_id, remote.session.as_ref()?)))
                    .flat_map(|(device, session)| {
                        session.agents.iter().map(move |row| identity(device, row))
                    }),
            )
            .collect()
    }

    /// Lays each row's pull requests (`agent_state::work::family`) across the
    /// rows of this machine and every device, which share one lineage. A list
    /// past either cap is logged once as it crosses.
    fn lay_session_pulls(&mut self, rows: &mut [SidebarAgentSnapshot]) {
        use crate::agent_state::work::family;
        let sessions = self.session_row_identities();
        let laid: Vec<family::SessionPulls> = {
            let sources = family::Sources::new(
                rows.iter(),
                &sessions,
                std::iter::once(&self.github).chain(self.device_github.values()),
            );
            (0..rows.len())
                .map(|index| {
                    sources.pulls(
                        Some(index),
                        sessions[index].as_ref(),
                        &[&self.recorded_pulls],
                    )
                })
                .collect()
        };
        for (row, list) in rows.iter_mut().zip(laid) {
            let (omitted, lineage_capped) = (list.omitted, list.lineage_capped);
            if omitted == 0 && !lineage_capped {
                self.pulls_over_cap.remove(&row.pane_id);
            } else if self.pulls_over_cap.insert(row.pane_id.clone()) {
                crate::diagnostic!(serde_json::json!({
                    "component": "session", "kind": "pulls.capacity",
                    "pane_id": row.pane_id, "omitted": omitted, "limit": family::PULL_LIMIT,
                    "lineage_capped": lineage_capped, "session_limit": family::SESSION_LIMIT,
                }));
            }
            row.pulls = (!list.pulls.is_empty()).then_some(list);
        }
    }

    /// Lays the open session record's `PR` sections from the same lists the
    /// rows draw (`agent_state::work::family`): what the link worker's
    /// summaries keep, which the rows read, then the record's own tree for a
    /// session no summary holds. Writes only when a section moved, and logs
    /// a list past either cap as it is laid.
    pub(super) fn settle_record_pulls(&mut self) {
        use crate::agent_state::work::family;
        let Some(held) = self.snapshot.session_record.as_ref() else {
            return;
        };
        let Some(record) = held.record.as_ref() else {
            return;
        };
        let sessions = self.session_row_identities();
        let tree = family::Recorded::from_record(record);
        let recorded = [&self.recorded_pulls, &tree];
        let sources = family::Sources::new(
            self.session_rows(),
            &sessions,
            std::iter::once(&self.github).chain(self.device_github.values()),
        );
        if sources.groups_match(record, &recorded) {
            return;
        }
        let mut next = crate::links::ledger::SessionRecordSnapshot::clone(held);
        if let Some(record) = next.record.as_mut() {
            let capped = sources.lay_groups(record, &recorded);
            if capped != family::Capped::default() {
                crate::diagnostic!(serde_json::json!({
                    "component": "session", "kind": "record_pulls.capacity",
                    "session_id": record.session.session_id, "omitted": capped.omitted,
                    "limit": family::PULL_LIMIT, "lineage_capped": capped.lineage,
                    "session_limit": family::SESSION_LIMIT,
                }));
            }
        }
        self.snapshot.session_record = Some(Arc::new(next));
    }

    pub(super) fn sync_session_state(&mut self) -> bool {
        let changed = self.project_session_state();
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
        if automatic.is_empty() {
            return changed;
        }
        if let Err(message) = self.write_ui_state() {
            for (pane, _, _) in automatic {
                self.pending_session_resolutions.remove(&pane);
            }
            self.push_diagnostic("session.resolve.save_failed", message);
            return changed;
        }
        // The queued resolutions resolve their rows, which only a second
        // projection shows.
        self.project_session_state() || changed
    }
}
