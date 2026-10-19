//! `세션 보관` and `이어가기` (PRD session-archive): the runtime half of the
//! session archive in `crate::session_archive`.
//!
//! An archive saves its record first and closes nothing until the coalesced
//! writer acknowledges it; the close is the subtree close, deepest first,
//! and none of its closes enters Reopen Closed (D-02, D-03, B9, B10). A
//! resume starts each member through start-in-checkout with the agent's
//! resume arguments on the member's own machine, one at a time through the
//! task slot, and a member leaves the archive when a live row reports its
//! conversation, which also covers a session resumed by hand (B22, B32).
//!
//! Everything here is bookkeeping under the runtime lock; the saves, the
//! closes, the starts and the record deletes run on their own workers.

use super::*;
use crate::session_archive::{
    ArchiveId, ArchivePhase, ArchiveRecord, ArchivedMember, ArchivedPull,
    RESUME_CONFIRM_TIMEOUT_MS, Resume, SessionArchive,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ArchiveSessionPayload {
    pub(super) pane_id: String,
    /// The operator confirmed stopping the work listed (B6).
    #[serde(default)]
    pub(super) confirmed: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArchiveTargetPayload {
    pub(super) archive_id: String,
    /// One member to resume; absent resumes every archived member (B23, B24).
    #[serde(default)]
    pub(super) session_id: Option<String>,
}

/// The request id a member's start carries, so its answer is found.
fn resume_request(id: &ArchiveId, session: &str) -> String {
    format!("archive:{}:{session}", id.as_str())
}

impl Runtime {
    /// `archive_session`: save the root and its live descendants, then close
    /// them once the save is acknowledged.
    pub(super) fn request_archive(&mut self, payload: ArchiveSessionPayload) -> bool {
        match self.admit_archive(&payload.pane_id, payload.confirmed) {
            Ok(()) => {}
            Err((kind, message)) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.refused",
                    "reason": kind, "pane_id": payload.pane_id,
                }));
                self.set_error(kind, message, false);
            }
        }
        true
    }

    /// Why `세션 보관` cannot run on this row now, as an error kind and the
    /// operator's words (B2-B4, B13), or `None` when it can.
    pub(super) fn archive_refusal(
        &self,
        agent: &SidebarAgentSnapshot,
    ) -> Option<(&'static str, String)> {
        if agent.hide_ai.is_some() {
            return Some((
                "archive.not_offered",
                "This agent's work is kept by its own home and is not archived".into(),
            ));
        }
        if agent.delegated || agent.lineage_parent_pane_id.is_some() {
            return Some((
                "archive.delegated",
                "A delegated session is archived with its root session".into(),
            ));
        }
        if let Some(device) = super::agent_sleep::pane_device(&agent.pane_id)
            && !self.archive_device_connected(device)
        {
            return Some((
                "archive.device_disconnected",
                format!("{device} is not connected; archive it once it reconnects"),
            ));
        }
        if self.archive_session_of(agent).is_none() {
            return Some((
                "archive.session_unknown",
                "Hide does not know this agent's conversation yet".into(),
            ));
        }
        None
    }

    /// The native session a row reports, proven where this machine reads it.
    fn archive_session_of(&self, agent: &SidebarAgentSnapshot) -> Option<String> {
        let session = agent.session_id.clone().filter(|id| !id.is_empty())?;
        match super::agent_sleep::pane_device(&agent.pane_id) {
            // This machine's rows carry the reference-proven facts.
            None => agent.row_facts.as_ref().map(|_| session),
            Some(_) => Some(session),
        }
    }

    /// The machine a row runs on: this core's node or its device.
    fn archive_machine_of(&self, pane_id: &str) -> String {
        super::agent_sleep::pane_device(pane_id)
            .unwrap_or(self.node.as_str())
            .to_owned()
    }

    /// The machine is reachable, its panes have been read since, and the
    /// session those panes and rows come from is loaded: before that every
    /// pane there would read as gone.
    fn archive_machine_read(&self, machine: &str) -> bool {
        self.archive_device_connected(machine)
            && self
                .delivery_panes
                .get(machine)
                .is_some_and(|read| read.panes.is_some())
            && (machine == self.node.as_str()
                || self
                    .snapshot
                    .status
                    .remote
                    .iter()
                    .any(|remote| remote.target_id == machine && remote.session.is_some()))
    }

    fn archive_device_connected(&self, device: &str) -> bool {
        device == self.node.as_str()
            || self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == device && remote.state == "connected")
    }

    fn admit_archive(
        &mut self,
        pane_id: &str,
        confirmed: bool,
    ) -> Result<(), (&'static str, String)> {
        let Some(root) = self
            .archive_rows()
            .find(|agent| agent.pane_id == pane_id)
            .cloned()
        else {
            return Err((
                "archive.target_unknown",
                format!("{pane_id} is no longer open; nothing was archived"),
            ));
        };
        if let Some(refusal) = self.archive_refusal(&root) {
            return Err(refusal);
        }
        let machine = self.archive_machine_of(pane_id);
        let session = self
            .archive_session_of(&root)
            .expect("checked by the refusal");
        let existing = self
            .snapshot
            .ui_state
            .session_archive
            .find(&machine, &root.agent_kind, &session)
            .filter(|(_, index)| *index == 0)
            .map(|(id, _)| id.clone());
        if let Some(id) = &existing {
            let record = &self.snapshot.ui_state.session_archive.records[id];
            if record.phase.in_flight() || record.deleting {
                // The same press again converges on the archive running (B11).
                return Ok(());
            }
        }
        let rows = self.archive_rows().cloned().collect::<Vec<_>>();
        let by_pane = rows
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect::<HashMap<_, _>>();
        // The root, then every live descendant the subtree close would take.
        let mut tree = vec![(&root, None::<String>, 0u8)];
        let mut index = 0;
        while index < tree.len() {
            let (agent, _, depth) = tree[index];
            let parent_session = self.archive_session_of(agent);
            for child in &agent.lineage_child_pane_ids {
                let Some(child) = by_pane.get(child.as_str()) else {
                    continue;
                };
                if root.close_descendant_pane_ids.contains(&child.pane_id)
                    && !tree
                        .iter()
                        .any(|(held, _, _)| held.pane_id == child.pane_id)
                {
                    tree.push((child, parent_session.clone(), depth.saturating_add(1)));
                }
            }
            index += 1;
        }
        for pane in &root.close_descendant_pane_ids {
            if !tree.iter().any(|(held, _, _)| held.pane_id == *pane)
                && let Some(agent) = by_pane.get(pane.as_str())
            {
                tree.push((agent, Some(session.clone()), 1));
            }
        }
        // One subtree close carries the whole tree; a larger one is refused
        // here, before anything is saved (B12).
        if tree.len() > super::tree_close::TREE_CLOSE_PANE_LIMIT {
            return Err((
                "archive.too_many",
                format!(
                    "A session with more than {} agents cannot be archived at once; nothing was closed",
                    super::tree_close::TREE_CLOSE_PANE_LIMIT
                ),
            ));
        }
        for (agent, _, _) in &tree {
            if agent.requires_close_status_check {
                return Err((
                    "archive.status_unknown",
                    format!(
                        "The activity of {} is unknown; check its status before archiving. Nothing was closed.",
                        agent.identity_label
                    ),
                ));
            }
            if self.archive_session_of(agent).is_none() {
                return Err((
                    "archive.session_unknown",
                    format!(
                        "Hide does not know {}'s conversation yet; nothing was archived",
                        agent.identity_label
                    ),
                ));
            }
        }
        if !confirmed
            && tree
                .iter()
                .any(|(agent, _, _)| agent.state.subtree == "working")
        {
            return Err((
                "archive.confirmation_required",
                "An agent in this session is working; archive_session requires confirmed=true"
                    .into(),
            ));
        }
        if self.tree_closes_full() {
            return Err((
                "archive.busy",
                "Wait for the closes already running to finish, then archive again".into(),
            ));
        }
        let Some((project_root, project_label, checkout_path)) = self.archive_place(pane_id) else {
            return Err((
                "archive.context_unavailable",
                "This session's checkout is not listed; nothing was archived".into(),
            ));
        };
        let mut members = Vec::new();
        for (agent, parent, depth) in &tree {
            let checkout = self
                .archive_place(&agent.pane_id)
                .map_or_else(|| checkout_path.clone(), |(_, _, path)| path);
            members.push(ArchivedMember {
                machine: self.archive_machine_of(&agent.pane_id),
                pane_id: agent.pane_id.clone(),
                kind: agent.agent_kind.clone(),
                native_session_id: self.archive_session_of(agent).expect("checked above"),
                source_reference: agent
                    .row_facts
                    .as_ref()
                    .and_then(|facts| facts.native_reference.clone()),
                cwd: self.archive_pane_cwd(&agent.pane_id),
                checkout_path: checkout,
                title: agent.identity_label.clone(),
                last_line: crate::session_archive::last_line(agent.message.as_deref()),
                pulls: archived_pulls(agent),
                registration: self.delivery_registration_of(&agent.pane_id),
                parent: parent.clone(),
                depth: *depth,
                closed: false,
                resume: Resume::Idle,
            });
        }
        let id = match existing {
            Some(id) => id,
            None => ArchiveId::new().map_err(|reason| ("archive.identity", reason.to_owned()))?,
        };
        // A retry keeps the members an earlier close already archived (B11).
        if let Some(earlier) = self.snapshot.ui_state.session_archive.records.get(&id) {
            for member in earlier.held() {
                if member.closed
                    && !members.iter().any(|held| {
                        held.is(&member.machine, &member.kind, &member.native_session_id)
                    })
                {
                    members.push(member.clone());
                }
            }
        }
        let record = ArchiveRecord {
            phase: ArchivePhase::Saving,
            revision: self
                .snapshot
                .ui_state
                .session_archive
                .records
                .get(&id)
                .map_or(1, |record| record.revision + 1),
            archived_at_unix_ms: unix_milliseconds(),
            project_root,
            project_label,
            members,
            reason: None,
            deleting: false,
        };
        self.snapshot
            .ui_state
            .session_archive
            .admit(id.clone(), record)
            .map_err(|reason| ("archive.full", reason.to_owned()))?;
        crate::diagnostic!(serde_json::json!({
            "component": "session_archive", "kind": "session_archive.saving",
            "archive_id": id.as_str(), "pane_id": pane_id, "members": tree.len(),
        }));
        self.persist_ui_state();
        self.refresh_archived_rows();
        Ok(())
    }

    /// Every agent row on this machine and each connected device.
    fn archive_rows(&self) -> impl Iterator<Item = &SidebarAgentSnapshot> {
        self.snapshot.navigator.agents.iter().chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.state == "connected")
                .filter_map(|remote| remote.session.as_ref())
                .flat_map(|session| session.agents.iter()),
        )
    }

    /// The live conversations, by machine, kind and session, with their pane.
    fn archive_live(&self) -> Vec<(String, String, String, String)> {
        self.archive_rows()
            .filter_map(|agent| {
                Some((
                    self.archive_machine_of(&agent.pane_id),
                    agent.agent_kind.clone(),
                    agent.session_id.clone()?,
                    agent.pane_id.clone(),
                ))
            })
            .collect()
    }

    fn archive_workspaces(&self, device: Option<&str>) -> &[WorkspaceSnapshot] {
        match device {
            None => &self.snapshot.navigator.workspaces,
            Some(device) => self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())
                .map_or(&[], |session| session.workspaces.as_slice()),
        }
    }

    /// The project path, its label and the checkout path a pane is in.
    fn archive_place(&self, pane_id: &str) -> Option<(String, String, String)> {
        self.archive_workspaces(super::agent_sleep::pane_device(pane_id))
            .iter()
            .find_map(|workspace| {
                workspace.checkouts.iter().find_map(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                        .then(|| {
                            (
                                workspace.path.clone(),
                                workspace.label.clone(),
                                checkout.path.clone(),
                            )
                        })
                })
            })
    }

    fn archive_pane_cwd(&self, pane_id: &str) -> Option<String> {
        self.archive_workspaces(super::agent_sleep::pane_device(pane_id))
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.trim().is_empty())
    }

    /// The writer's receipt: a saved archive starts its close, a saved
    /// delete removes its row; a failed save closes and removes nothing.
    pub(super) fn ingest_archive_saved(&mut self, saved: &SessionArchive, ok: bool) -> bool {
        let mut changed = false;
        for (id, written) in &saved.records {
            let Some(current) = self.snapshot.ui_state.session_archive.records.get(id) else {
                continue;
            };
            if current.revision != written.revision {
                continue;
            }
            if written.deleting && current.deleting {
                changed = true;
                if ok {
                    let removed = self.snapshot.ui_state.session_archive.records.remove(id);
                    self.release_archive_holds(removed.iter().flat_map(|record| &record.members));
                    crate::diagnostic!(serde_json::json!({
                        "component": "session_archive", "kind": "session_archive.deleted",
                        "archive_id": id.as_str(),
                    }));
                } else {
                    if let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(id)
                    {
                        record.deleting = false;
                    }
                    self.set_error(
                        "archive.delete_failed",
                        "The archived session could not be removed; it stays in the archive",
                        true,
                    );
                }
                continue;
            }
            if written.phase != ArchivePhase::Saving || current.phase != ArchivePhase::Saving {
                continue;
            }
            changed = true;
            if !ok {
                self.abandon_archive(id, "The archive could not be saved, so nothing was closed");
                continue;
            }
            self.start_archive_close(id);
        }
        if changed {
            self.refresh_archived_rows();
        }
        changed
    }

    /// Drops what a save could not hold: a retry's earlier members stay as
    /// they were, a new record goes.
    fn abandon_archive(&mut self, id: &ArchiveId, message: &str) {
        let archive = &mut self.snapshot.ui_state.session_archive;
        let Some(record) = archive.records.get_mut(id) else {
            return;
        };
        if !record.members.iter().any(|member| member.closed) {
            archive.records.remove(id);
        } else {
            // A retry of a partial close: the root stays open and keeps its
            // place at the head of the record, with the members that closed
            // before, so the record still loads as one tree (B24).
            let mut index = 0;
            record.members.retain(|member| {
                index += 1;
                index == 1 || member.closed
            });
            record.phase = if record.members[0].closed {
                ArchivePhase::Archived
            } else {
                ArchivePhase::CloseFailed
            };
            record.reason = Some(message.to_owned());
            record.revision += 1;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "session_archive", "kind": "session_archive.abandoned",
            "archive_id": id.as_str(), "message": message,
        }));
        self.set_error("archive.save_failed", message, true);
    }

    fn start_archive_close(&mut self, id: &ArchiveId) {
        let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(id) else {
            return;
        };
        record.phase = ArchivePhase::Closing;
        let root = record.members[0].pane_id.clone();
        let descendants = record
            .members
            .iter()
            .skip(1)
            .filter(|member| !member.closed)
            .map(|member| member.pane_id.clone())
            .collect::<Vec<_>>();
        crate::diagnostic!(serde_json::json!({
            "component": "session_archive", "kind": "session_archive.closing",
            "archive_id": id.as_str(),
        }));
        self.close_tree_for_archive(id.clone(), root, descendants);
    }

    /// Whether an archive is closing this pane, so its close is not kept
    /// for Reopen Closed (B9).
    pub(super) fn archive_closes_pane(&self, pane_id: &str) -> bool {
        self.snapshot
            .ui_state
            .session_archive
            .records
            .values()
            .filter(|record| record.phase == ArchivePhase::Closing)
            .flat_map(|record| &record.members)
            .any(|member| !member.closed && member.pane_id == pane_id)
    }

    /// The subtree close of an archive ended: `closed` are the panes
    /// confirmed gone.
    pub(super) fn finish_archive_close(
        &mut self,
        id: &ArchiveId,
        closed: &HashSet<String>,
        cause: Option<&'static str>,
    ) {
        let Some(record) = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .get(id)
            .filter(|record| record.phase == ArchivePhase::Closing)
        else {
            return;
        };
        // A listed pane already gone on a connected machine closed too; a
        // disconnected device's absence is not a close.
        let gone = record
            .members
            .iter()
            .map(|member| member.pane_id.clone())
            .filter(|pane| {
                closed.contains(pane)
                    || (self.archive_device_connected(&self.archive_machine_of(pane))
                        && !self.pane_listed(pane))
            })
            .collect::<HashSet<_>>();
        let record = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .get_mut(id)
            .expect("checked above");
        for member in &mut record.members {
            member.closed |= gone.contains(&member.pane_id);
        }
        record.revision += 1;
        let total = record.members.len();
        let open = record
            .members
            .iter()
            .filter(|member| !member.closed)
            .count();
        if record.members[0].closed {
            // The root waited on every listed descendant, so it closed last.
            record.members.retain(|member| member.closed);
            record.phase = ArchivePhase::Archived;
            record.reason = None;
            crate::diagnostic!(serde_json::json!({
                "component": "session_archive", "kind": "session_archive.archived",
                "archive_id": id.as_str(), "members": record.members.len(),
            }));
        } else {
            let (kind, why) = match cause {
                Some("device_disconnected") => {
                    ("archive.incomplete_disconnected", "its device disconnected")
                }
                Some("timed_out") => ("archive.incomplete_timeout", "the close timed out"),
                _ => ("archive.incomplete", "the close was refused"),
            };
            let message = format!(
                "{open} of {total} could not be closed because {why}, so the session was not archived. Choose Archive session again to retry."
            );
            record.phase = ArchivePhase::CloseFailed;
            record.reason = Some(message.clone());
            if !record.members.iter().any(|member| member.closed) {
                self.snapshot.ui_state.session_archive.records.remove(id);
            }
            crate::diagnostic!(serde_json::json!({
                "component": "session_archive", "kind": "session_archive.close_failed",
                "archive_id": id.as_str(), "open": open, "members": total,
                "cause": cause,
            }));
            self.set_error(kind, message, true);
        }
        self.persist_ui_state();
        self.refresh_archived_rows();
    }

    /// The subtree close was refused before anything started.
    pub(super) fn fail_archive_close(&mut self, id: &ArchiveId, message: String) -> bool {
        if let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(id)
            && record.phase == ArchivePhase::Closing
        {
            record.revision += 1;
            if record.members.iter().any(|member| member.closed) {
                record.phase = ArchivePhase::CloseFailed;
                record.reason = Some(message.clone());
            } else {
                self.snapshot.ui_state.session_archive.records.remove(id);
            }
            self.persist_ui_state();
            self.refresh_archived_rows();
        }
        self.set_error("archive.incomplete", message, true);
        true
    }

    /// `archive_resume`: every archived member of a record, or one (B21, B24).
    pub(super) fn request_archive_resume(&mut self, payload: ArchiveTargetPayload) -> bool {
        let Some(id) = ArchiveId::parse(&payload.archive_id) else {
            self.set_error("archive.unknown", "That archived session is gone", false);
            return true;
        };
        let Some(record) = self.snapshot.ui_state.session_archive.records.get(&id) else {
            self.set_error("archive.unknown", "That archived session is gone", false);
            return true;
        };
        // A delete the link worker is carrying out takes the record away;
        // nothing starts that its answer would then strand.
        if record.phase.in_flight() || record.deleting || self.archive_deletes_sent.contains(&id) {
            return true;
        }
        if record.phase == ArchivePhase::CloseUnknown {
            self.set_error(
                "archive.close_unknown",
                "Hide cannot say whether this session closed; check its status before resuming it",
                true,
            );
            return true;
        }
        let device = record.members[0].machine.clone();
        if !self.archive_device_connected(&device) {
            self.set_error(
                "archive.device_disconnected",
                format!("{device} is not connected; resume it once it reconnects"),
                true,
            );
            return true;
        }
        let only = payload.session_id.as_deref();
        let mut front = true;
        let record = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .get_mut(&id)
            .expect("checked above");
        let mut asked = 0;
        for member in &mut record.members {
            if !member.closed
                || only.is_some_and(|session| member.native_session_id != session)
                || member.resume.in_flight()
                || matches!(member.resume, Resume::Done { .. })
            {
                continue;
            }
            member.resume = Resume::Queued { front };
            front = false;
            asked += 1;
        }
        if asked == 0 {
            // A second press while one starts starts nothing (B21).
            return true;
        }
        record.revision += 1;
        crate::diagnostic!(serde_json::json!({
            "component": "session_archive", "kind": "session_archive.resume_requested",
            "archive_id": id.as_str(), "members": asked,
        }));
        self.advance_session_archive();
        self.persist_ui_state();
        true
    }

    /// `archive_check`: after a restart, says what an unknown archive or
    /// resume left, and repeats nothing by itself (B34).
    pub(super) fn check_archive_status(&mut self, payload: ArchiveTargetPayload) -> bool {
        let Some(id) = ArchiveId::parse(&payload.archive_id) else {
            return false;
        };
        let Some(record) = self.snapshot.ui_state.session_archive.records.get(&id) else {
            self.set_error("archive.unknown", "That archived session is gone", false);
            return true;
        };
        // Only what a restart left unknown is checked; a save, close or
        // resume still in flight answers for itself.
        if record.phase != ArchivePhase::CloseUnknown
            && !record
                .members
                .iter()
                .any(|member| matches!(member.resume, Resume::Unknown { .. }))
        {
            return false;
        }
        // A pane is read as closed only from a fresh read of a machine Hide
        // can reach: before the first read, or while a device is away, every
        // pane would look gone and a session still open would be archived.
        if let Some(member) = record
            .members
            .iter()
            .find(|member| !self.archive_machine_read(&member.machine))
        {
            self.set_error(
                "archive.device_disconnected",
                format!(
                    "{} cannot be checked until its machine is connected and read",
                    member.title
                ),
                true,
            );
            return true;
        }
        let rows = self
            .archive_rows()
            .filter_map(|agent| {
                Some((
                    self.archive_machine_of(&agent.pane_id),
                    agent.agent_kind.clone(),
                    agent.session_id.clone()?,
                ))
            })
            .collect::<Vec<_>>();
        let live = |member: &ArchivedMember| {
            rows.iter()
                .any(|(machine, kind, session)| member.is(machine, kind, session))
        };
        let listed = |pane: &str| self.pane_listed(pane);
        let mut panes = HashMap::new();
        if let Some(record) = self.snapshot.ui_state.session_archive.records.get(&id) {
            for member in &record.members {
                panes.insert(member.pane_id.clone(), listed(&member.pane_id));
                if let Some(pane) = member.resume.made_pane() {
                    panes.insert(pane.to_owned(), listed(pane));
                }
            }
        }
        let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(&id) else {
            return false;
        };
        if record.phase == ArchivePhase::CloseUnknown {
            for member in &mut record.members {
                // A pane that is gone with its conversation not live closed.
                member.closed |= !panes[&member.pane_id] && !live(member);
            }
            if record.members[0].closed {
                record.members.retain(|member| member.closed);
                record.phase = ArchivePhase::Archived;
            } else {
                record.phase = ArchivePhase::CloseFailed;
                record.reason = Some(
                    "The session is still open; choose Archive session again to archive it".into(),
                );
            }
        }
        for member in &mut record.members {
            if let Resume::Unknown { pane_id } = &member.resume {
                let pane_id = pane_id.clone().filter(|pane| panes[pane]);
                member.resume = Resume::Failed {
                    reason: "The resume did not finish; resume again to retry".into(),
                    pane_id,
                };
            }
        }
        record.revision += 1;
        if record.members.iter().all(|member| !member.closed) {
            self.snapshot.ui_state.session_archive.records.remove(&id);
        }
        self.persist_ui_state();
        self.advance_session_archive();
        self.refresh_archived_rows();
        true
    }

    /// `archive_delete`: Hide's record lines of the session and its archived
    /// descendants first, then the archive entry (D-15, B29, B30).
    pub(super) fn request_archive_delete(&mut self, payload: ArchiveTargetPayload) -> bool {
        let Some(id) = ArchiveId::parse(&payload.archive_id) else {
            return false;
        };
        let Some(record) = self.snapshot.ui_state.session_archive.records.get(&id) else {
            return false;
        };
        if record.phase.in_flight()
            || record.deleting
            || record
                .members
                .iter()
                .any(|member| member.resume.in_flight())
        {
            self.set_error(
                "archive.delete_busy",
                "Wait for this session's archive or resume to finish, then delete it",
                true,
            );
            return true;
        }
        let sessions = record
            .held()
            .filter(|member| member.closed)
            .map(|member| crate::links::ledger::SessionIdentity {
                device_id: member.machine.clone(),
                agent: super::links::session_agent(&member.kind)
                    .unwrap_or(member.kind.as_str())
                    .to_owned(),
                session_id: member.native_session_id.clone(),
            })
            .collect::<Vec<_>>();
        let sent = match self.links_work.client.as_ref() {
            Some(client) => client.delete_sessions(id.as_str().to_owned(), sessions),
            None => Err("links_store_unavailable".to_owned()),
        };
        if let Err(code) = sent {
            crate::diagnostic!(serde_json::json!({
                "component": "session_archive", "kind": "session_archive.delete_failed",
                "archive_id": id.as_str(), "code": code,
            }));
            self.set_error(
                "archive.delete_failed",
                "Hide's record of this session could not be deleted; it stays in the archive",
                true,
            );
            return true;
        }
        self.archive_deletes_sent.insert(id);
        self.refresh_archived_rows();
        true
    }

    /// The link worker's answer to a delete.
    pub(crate) fn ingest_archive_record_deleted(
        &mut self,
        request: String,
        answer: Result<(), String>,
    ) -> bool {
        let Some(id) = ArchiveId::parse(&request) else {
            return false;
        };
        if !self.archive_deletes_sent.remove(&id) {
            return false;
        }
        match answer {
            Ok(()) => {
                if let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(&id) {
                    record.deleting = true;
                    record.revision += 1;
                    self.persist_ui_state();
                }
            }
            Err(code) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.delete_failed",
                    "archive_id": id.as_str(), "code": code,
                }));
                self.set_error(
                    "archive.delete_failed",
                    "Hide's record of this session could not be deleted; it stays in the archive",
                    true,
                );
            }
        }
        self.refresh_archived_rows();
        true
    }

    /// The link record pruned these sessions, or retention expired archived
    /// sessions it held no line for (B31).
    pub(crate) fn ingest_link_pruned(
        &mut self,
        sessions: Vec<crate::links::ledger::SessionIdentity>,
    ) -> bool {
        let keys = sessions
            .into_iter()
            .map(|session| (session.device_id, session.agent, session.session_id))
            .collect::<Vec<_>>();
        let before = self.snapshot.ui_state.session_archive.clone();
        if !self.snapshot.ui_state.session_archive.drop_pruned(&keys) {
            return false;
        }
        let kept = self
            .snapshot
            .ui_state
            .session_archive
            .held_registrations()
            .map(|registration| registration.id.clone())
            .collect::<HashSet<_>>();
        self.release_archive_holds(
            before
                .records
                .values()
                .flat_map(|record| &record.members)
                .filter(|member| {
                    member
                        .registration
                        .as_ref()
                        .is_some_and(|registration| !kept.contains(&registration.id))
                }),
        );
        crate::diagnostic!(serde_json::json!({
            "component": "session_archive", "kind": "session_archive.pruned",
            "sessions": keys.len(),
        }));
        self.persist_ui_state();
        self.refresh_archived_rows();
        true
    }

    /// Moves every archive as far as the current rows and task slot allow:
    /// a live row reporting an archived conversation takes it out (B22,
    /// B32), a started resume is confirmed or times out, and the next queued
    /// member starts once the slot is free. Runs on the operation tick and
    /// after each session ingest; cheap while the archive is empty.
    pub(super) fn advance_session_archive(&mut self) -> bool {
        if self.snapshot.ui_state.session_archive.is_empty() {
            return false;
        }
        let now = unix_milliseconds();
        let mut changed = false;
        let live = self.archive_live();
        let task = self.snapshot.task_operation.clone();
        let ids = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in &ids {
            let record = self
                .snapshot
                .ui_state
                .session_archive
                .records
                .get_mut(id)
                .expect("listed above");
            if record.phase.in_flight() || record.phase == ArchivePhase::CloseUnknown {
                continue;
            }
            for member in &mut record.members {
                if !member.closed || matches!(member.resume, Resume::Done { .. }) {
                    continue;
                }
                if let Some((_, _, _, pane)) = live
                    .iter()
                    .find(|(machine, kind, session, _)| member.is(machine, kind, session))
                {
                    crate::diagnostic!(serde_json::json!({
                        "component": "session_archive", "kind": "session_archive.resumed",
                        "archive_id": id.as_str(), "pane_id": pane,
                    }));
                    member.resume = Resume::Done {
                        pane_id: pane.clone(),
                    };
                    record.revision += 1;
                    changed = true;
                    continue;
                }
                let next = match &member.resume {
                    Resume::Starting {
                        task: started,
                        front,
                        ..
                    } => match task.as_ref() {
                        Some(operation) if operation.id == *started => {
                            match (operation.phase.as_str(), operation.agent_phase.as_deref()) {
                                ("failed", _) => Some(Resume::Failed {
                                    reason: operation.message.clone().unwrap_or_else(|| {
                                        "The resume tab could not be opened".into()
                                    }),
                                    pane_id: None,
                                }),
                                (_, Some("started")) => Some(Resume::Confirming {
                                    pane_id: operation.pane_id.clone().unwrap_or_default(),
                                    since_unix_ms: now,
                                }),
                                (_, Some("failed")) => Some(Resume::Failed {
                                    reason: operation.agent_message.clone().unwrap_or_else(|| {
                                        "The saved conversation could not be resumed".into()
                                    }),
                                    pane_id: operation.pane_id.clone(),
                                }),
                                (_, Some("unknown")) => Some(Resume::Unknown {
                                    pane_id: operation.pane_id.clone(),
                                }),
                                _ => None,
                            }
                        }
                        // Another start took the slot: this one's answer is lost.
                        _ => {
                            let _ = front;
                            Some(Resume::Unknown { pane_id: None })
                        }
                    },
                    Resume::Confirming {
                        pane_id,
                        since_unix_ms,
                    } if now.saturating_sub(*since_unix_ms) > RESUME_CONFIRM_TIMEOUT_MS => {
                        Some(Resume::Failed {
                            reason: "The agent was not ready in time; resume again to retry".into(),
                            pane_id: Some(pane_id.clone()),
                        })
                    }
                    _ => None,
                };
                if let Some(next) = next {
                    if let Resume::Failed { reason, .. } = &next {
                        crate::diagnostic!(serde_json::json!({
                            "component": "session_archive", "kind": "session_archive.resume_failed",
                            "archive_id": id.as_str(), "reason": reason,
                        }));
                    }
                    member.resume = next;
                    record.revision += 1;
                    changed = true;
                }
            }
        }
        changed |= self.continue_archived_registrations(now);
        changed |= self.start_next_resume(now);
        // A record whose every member is live again is done.
        let before = self.snapshot.ui_state.session_archive.records.len();
        self.snapshot
            .ui_state
            .session_archive
            .records
            .retain(|_, record| {
                !(matches!(
                    record.phase,
                    ArchivePhase::Archived | ArchivePhase::CloseFailed
                ) && record
                    .members
                    .iter()
                    .filter(|member| member.closed)
                    .all(|member| {
                        matches!(member.resume, Resume::Done { .. })
                            && member.registration.is_none()
                    }))
            });
        changed |= before != self.snapshot.ui_state.session_archive.records.len();
        if changed {
            self.persist_ui_state();
        }
        changed | self.refresh_archived_rows()
    }

    /// A resumed member's registration moves to its new pane through the
    /// delivery store's rebind, as a sleep wake's does.
    fn continue_archived_registrations(&mut self, now: u64) -> bool {
        let waiting = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .iter()
            .flat_map(|(id, record)| {
                record
                    .members
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, member)| {
                        match (&member.resume, &member.registration) {
                            (Resume::Done { pane_id }, Some(registration)) => {
                                Some((id.clone(), index, pane_id.clone(), registration.clone()))
                            }
                            _ => None,
                        }
                    })
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        self.archive_registration_waits.retain(|agent, _| {
            waiting
                .iter()
                .any(|(_, _, _, registration)| registration.id == *agent)
        });
        for (id, index, pane, registration) in waiting {
            // The pane it resumed in closed before the delivery store saw it
            // there, or the store has not seen it within the hour a woken
            // sleeper gets: nothing will take the registration over, so it
            // is let go rather than held for a pane that will not answer.
            let since = *self
                .archive_registration_waits
                .entry(registration.id.clone())
                .or_insert(now);
            let gone =
                !self.pane_listed(&pane) && !self.archive_rows().any(|agent| agent.pane_id == pane);
            let expired = now.saturating_sub(since) >= super::agent_sleep::WOKEN_BINDING_LIMIT_MS;
            let released = gone || expired;
            if released {
                crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.registration_released",
                    "archive_id": id.as_str(), "agent_id": registration.id, "pane_id": pane,
                    "reason": if gone { "pane_gone" } else { "unbound" },
                }));
            }
            if (released || self.continue_dormant_registration(&registration, &pane))
                && let Some(record) = self.snapshot.ui_state.session_archive.records.get_mut(&id)
            {
                record.members[index].registration = None;
                record.revision += 1;
                changed = true;
                self.archive_registration_waits.remove(&registration.id);
                if released {
                    self.registration_rebinds.remove(&registration.id);
                    self.release_dormant_hold(&registration.id);
                }
            }
        }
        changed
    }

    /// Starts the next queued member once the task slot is free: the root
    /// first, each descendant after the one above it (B23).
    fn start_next_resume(&mut self, now: u64) -> bool {
        let starting = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .values()
            .flat_map(|record| &record.members)
            .any(|member| matches!(member.resume, Resume::Starting { .. }));
        if starting
            || self.pending_start_focus()
            || self
                .snapshot
                .task_operation
                .as_ref()
                .is_some_and(|operation| {
                    operation.phase == "working"
                        || operation.agent_phase.as_deref() == Some("starting")
                })
        {
            return false;
        }
        let next = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .iter()
            .find_map(|(id, record)| {
                record
                    .members
                    .iter()
                    .position(|member| matches!(member.resume, Resume::Queued { .. }))
                    .map(|index| (id.clone(), index))
            });
        let Some((id, index)) = next else {
            return false;
        };
        let member = self.snapshot.ui_state.session_archive.records[&id].members[index].clone();
        let Resume::Queued { front } = member.resume else {
            unreachable!("found queued above");
        };
        let request = resume_request(&id, &member.native_session_id);
        let outcome = self.start_member_resume(&member, &request, front);
        let record = self
            .snapshot
            .ui_state
            .session_archive
            .records
            .get_mut(&id)
            .expect("found above");
        record.revision += 1;
        record.members[index].resume = match outcome {
            Ok(task) => Resume::Starting {
                task,
                since_unix_ms: now,
                front,
            },
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.resume_refused",
                    "archive_id": id.as_str(), "reason": reason,
                }));
                Resume::Failed {
                    reason,
                    pane_id: member.resume.made_pane().map(str::to_owned),
                }
            }
        };
        true
    }

    /// Starts one member: in the tab an earlier attempt made when it is
    /// still there and holds no agent (B26), else in a new tab of its
    /// checkout on its machine.
    fn start_member_resume(
        &mut self,
        member: &ArchivedMember,
        request: &str,
        front: bool,
    ) -> Result<u64, String> {
        let local = member.machine == self.node.as_str();
        if !self.archive_device_connected(&member.machine) {
            return Err(format!("{} is not connected", member.machine));
        }
        if !std::path::Path::new(&member.checkout_path).is_absolute() && local {
            return Err("This session's folder is gone".into());
        }
        let reuse = self.archived_resume_pane(member);
        if let Some(pane) = reuse {
            return self.restart_in_pane(member, &pane, request);
        }
        let resume_session_path = member
            .source_reference
            .as_ref()
            .filter(|reference| reference.kind == "path")
            .map(|reference| reference.value.clone());
        self.start_in_checkout(
            super::events::AgentStartInCheckoutPayload {
                target: None,
                confirm_only: false,
                checkout_path: Some(member.checkout_path.clone()),
                device_id: (!local).then(|| member.machine.clone()),
                home: false,
                provider: member.kind.clone(),
                prompt: None,
                model: None,
                request_id: Some(request.to_owned()),
                resume_session_id: Some(member.native_session_id.clone()),
                resume_session_path,
                // A descendant opens behind the root resumed before it (B23).
                stay_behind: !front,
            },
            false,
            member.cwd.clone(),
        );
        if let Some(operation) = self
            .snapshot
            .task_operation
            .as_mut()
            .filter(|operation| operation.request_id.as_deref() == Some(request))
        {
            return Ok(operation.id);
        }
        Err(self
            .snapshot
            .status
            .last_error
            .as_ref()
            .filter(|error| error.request_id.as_deref() == Some(request))
            .map_or_else(
                || "The saved conversation could not be resumed".to_owned(),
                |error| error.message.clone(),
            ))
    }

    /// The tab an earlier resume made, when it is still listed and holds no
    /// agent.
    fn archived_resume_pane(&self, member: &ArchivedMember) -> Option<String> {
        let pane = member.resume.made_pane()?;
        (self.pane_listed(pane) && !self.archive_rows().any(|agent| agent.pane_id == pane))
            .then(|| pane.to_owned())
    }

    /// Starts the agent again in a pane an earlier resume made, through the
    /// task slot's own agent start.
    fn restart_in_pane(
        &mut self,
        member: &ArchivedMember,
        pane: &str,
        request: &str,
    ) -> Result<u64, String> {
        let Some(mut arguments) =
            super::projects::resume_session_arguments(&member.kind, &member.native_session_id)
        else {
            return Err("This session cannot be resumed".into());
        };
        arguments.extend(super::agent_choice::agent_arguments(
            Some(member.kind.as_str()),
            None,
            &[],
        ));
        let local = member.machine == self.node.as_str();
        let context = if local {
            self.local_worktree_target()?
        } else {
            self.device_worktree_target(&member.machine)?
        };
        let id = self.begin_task_operation(
            "agent_start",
            Some(member.checkout_path.clone()),
            None,
            None,
            Some(member.kind.clone()),
        )?;
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.request_id = Some(request.to_owned());
            operation.device_id = (!local).then(|| member.machine.clone());
            operation.phase = "ready".into();
            operation.path = Some(member.checkout_path.clone());
            operation.pane_id = Some(pane.to_owned());
            operation.agent_phase = Some("starting".into());
        }
        // As the checkout start an archived member's first attempt went
        // through: no model or effort of its own.
        self.set_task_agent_launch(
            id,
            None,
            arguments,
            hide_agent_adapter::LaunchChoice::default(),
        );
        if let Some(launch) = self.task_agent_launch.as_mut() {
            launch.resume_reference = member.source_reference.clone();
        }
        if let Err(message) = live::spawn_task_agent_start(context, id) {
            self.ingest_task_agent_result(id, live::TaskAgentOutcome::Failed(message));
        }
        Ok(id)
    }

    /// A record that goes takes its hold on its members' registrations with
    /// it; one whose pane is already gone ends now, as a woken sleeper's
    /// does, instead of waiting for a pane change that will not come.
    fn release_archive_holds<'a>(&mut self, members: impl Iterator<Item = &'a ArchivedMember>) {
        let ids = members
            .filter_map(|member| member.registration.as_ref())
            .map(|registration| registration.id.clone())
            // A move to the pane it resumed in finishes on its own.
            .filter(|id| !self.registration_rebinds.contains_key(id))
            .collect::<Vec<_>>();
        for id in ids {
            self.release_dormant_hold(&id);
        }
    }

    /// Whether the registration of an archived session holds this actor, so
    /// a watch on it is not ended while it is away.
    pub(super) fn archive_holds_actor(&self, actor: &crate::delivery::Actor) -> bool {
        self.snapshot
            .ui_state
            .session_archive
            .held_registrations()
            .any(|registration| registration.actor.same_identity(actor))
    }

    /// Hands the archive's sessions to a link worker that just started.
    pub(super) fn resend_archive_retention(&mut self) {
        self.archive_retention_sent = Arc::default();
        self.refresh_archived_rows();
        if let Some(client) = self.links_work.client.as_ref() {
            client.archived(Arc::clone(&self.archive_retention_sent));
        }
    }

    /// The archived rows the shell draws; true when they changed.
    pub(super) fn refresh_archived_rows(&mut self) -> bool {
        let archive = &self.snapshot.ui_state.session_archive;
        let rows = if archive.is_empty() {
            Vec::new()
        } else {
            // Pull requests are looked up only for records that have any.
            let any_pulls = archive
                .records
                .values()
                .any(|record| record.members.iter().any(|member| !member.pulls.is_empty()));
            let pulls = (if any_pulls {
                self.snapshot.navigator.workspaces.as_slice()
            } else {
                &[]
            })
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter(|_| any_pulls)
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.workspaces.iter()),
            )
            .flat_map(|workspace| &workspace.pull_requests)
            .map(|pull| (pull.url.as_str(), pull.state()))
            .collect::<HashMap<_, _>>();
            let mut rows = archive.snapshots(&|url| pulls.get(url).copied());
            let live = self.archive_live();
            for row in &mut rows {
                let id = ArchiveId::parse(&row.id);
                if id
                    .as_ref()
                    .is_some_and(|id| self.archive_deletes_sent.contains(id))
                {
                    row.phase = "deleting";
                }
                // Members kept by a partial close or a failed resume show in
                // their live root's record; once that root is gone on a
                // machine Hide can see, the record stands in 보관됨 itself,
                // so a saved conversation is never left without a row (B24).
                let root_gone = id
                    .as_ref()
                    .and_then(|id| archive.records.get(id))
                    .filter(|record| !record.phase.in_flight())
                    .map(|record| record.root())
                    .is_some_and(|root| {
                        // A root row whose conversation is not reported yet
                        // still holds its pane.
                        self.archive_machine_read(&root.machine)
                            && !self.pane_listed(&root.pane_id)
                            && !root
                                .resume
                                .made_pane()
                                .is_some_and(|pane| self.pane_listed(pane))
                            && !live
                                .iter()
                                .any(|(machine, kind, session, _)| root.is(machine, kind, session))
                    });
                row.listed |=
                    root_gone && row.members.iter().any(|member| member.resume != "resumed");
            }
            rows
        };
        // The link record's retention answers for these sessions too.
        let held = archive
            .records
            .values()
            .flat_map(|record| {
                record.held().filter(|member| member.closed).map(|member| {
                    crate::links::store::ArchivedSession {
                        session: crate::links::ledger::SessionIdentity {
                            device_id: member.machine.clone(),
                            agent: super::links::session_agent(&member.kind)
                                .unwrap_or(member.kind.as_str())
                                .to_owned(),
                            session_id: member.native_session_id.clone(),
                        },
                        project_root: record.project_root.clone(),
                        archived_at_unix_ms: record.archived_at_unix_ms,
                    }
                })
            })
            .collect::<Vec<_>>();
        if held != *self.archive_retention_sent {
            self.archive_retention_sent = Arc::new(held);
            if let Some(client) = self.links_work.client.as_ref() {
                client.archived(Arc::clone(&self.archive_retention_sent));
            }
        }
        if self.snapshot.navigator.archived_sessions == rows {
            return false;
        }
        self.snapshot.navigator.archived_sessions = rows;
        true
    }
}

/// The pull requests a row holds the duty of, as kept with its archive.
fn archived_pulls(agent: &SidebarAgentSnapshot) -> Vec<ArchivedPull> {
    let Some(request) = agent.request.as_ref() else {
        return Vec::new();
    };
    agent
        .state
        .pr
        .iter()
        .flat_map(|summary| &summary.pulls)
        .filter_map(|own| request.pull_requests.get(own.index))
        .map(|pull| ArchivedPull {
            url: pull.url.clone(),
            number: pull.number,
            title: pull.title.clone(),
            state: pull.state(),
        })
        .collect()
}
