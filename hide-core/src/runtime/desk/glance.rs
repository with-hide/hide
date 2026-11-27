//! 한눈에, the panel at Desk's right while no session is open beside it: the
//! sessions Desk started and the ones the operator opened beside Desk, and
//! the Views they and Desk's own agent opened.
//!
//! The sessions are Desk's store's (`crate::desk::Stored::glance`, the
//! newest [`crate::desk::MAX_GLANCE_SESSIONS`]): a session Desk starts is
//! kept when its pane first shows (`refresh_desk_jobs`), and one the
//! operator opens beside Desk when its thread opens (`open_session_thread`),
//! so every window reads the same list and a restart keeps it.
//!
//! The files are no list of their own: they are read, on every build of the
//! cards, from the Views themselves, a session's by its openers as its `뷰 N`
//! list reads them, and Desk's agent's by the opener its `hide file open`
//! stamps on the View it lands in ([`Runtime::desk_opener`], in
//! `desk_commit_open`); each View's own record says whether it was looked at.

use super::super::Runtime;
use super::jobs::{CardIndex, view_output};
use crate::model::{DeskGlanceFile, DeskGlanceSnapshot, DeskJobSnapshot};
use crate::view_feedback::{Opener, SessionScope};
use crate::workspace_control::DESK_CALLER;

/// The Views 한눈에 lists; past it the section counts the rest.
const MAX_GLANCE_FILES: usize = 32;

impl Runtime {
    /// The operator opened `pane`'s session beside Desk: it is one of
    /// 한눈에's, the newest, while its pane has a row, this machine's or a
    /// connected device's; a session read from its record alone (its pane
    /// gone) is never listed, so it takes no place. Whether the store
    /// changed, for a save.
    pub(in crate::runtime) fn desk_glance_opened(&mut self, pane: &str) -> bool {
        self.desk.worker.is_some()
            && self.agent_rows().any(|row| row.pane_id == pane)
            && self.keep_desk_glance(pane, crate::runtime::unix_milliseconds())
    }

    /// Keeps `pane` among 한눈에's sessions as of `at_ms`. A new one that a
    /// full list takes (newer than its oldest) first takes the place of the
    /// oldest kept session that has ended (its state ended, or the last
    /// published 한눈에 no longer shows it: its tab closed and no job of
    /// Desk's), so a session at work or asking the operator goes last
    /// (engineering principle 15); one older than all of a full list is not
    /// taken and moves nothing. A refresh asks this once per Desk session
    /// (`JobMark::glanced`), so one pushed out is not put back. Whether the
    /// store changed.
    pub(in crate::runtime) fn keep_desk_glance(&mut self, pane: &str, at_ms: u64) -> bool {
        use crate::model::DeskJobState;
        let stored = &mut self.desk.stored;
        let new = !stored.glance.iter().any(|kept| kept.pane_id == pane);
        let taken = stored.glance.len() < crate::desk::MAX_GLANCE_SESSIONS
            || stored
                .glance
                .first()
                .is_some_and(|oldest| oldest.at_ms < at_ms);
        let room = new && taken && {
            let section = self.snapshot.desk_jobs.as_ref();
            let going = |kept: &str| {
                section.is_some_and(|section| {
                    section.glance.sessions.iter().any(|listed| listed == kept)
                        && [&section.jobs, &section.workers, &section.named]
                            .into_iter()
                            .flatten()
                            .find(|card| card.pane_id.as_deref() == Some(kept))
                            .is_some_and(|card| {
                                matches!(
                                    card.state,
                                    DeskJobState::Queued
                                        | DeskJobState::Starting
                                        | DeskJobState::Working
                                        | DeskJobState::Asking
                                )
                            })
                })
            };
            stored.glance_room(|kept| !going(kept))
        };
        stored.keep_glance(pane, at_ms) || room
    }

    /// The opener Desk's own agent is recorded as on a View it opened: the
    /// core's node and Desk's caller name, which no engine terminal is.
    pub(in crate::runtime) fn desk_opener(&self) -> Opener {
        Opener {
            device_id: self.node.as_str().to_owned(),
            terminal_id: DESK_CALLER.to_owned(),
            session: None,
            resume: None,
            resume_path: None,
            provider: self
                .desk
                .stored
                .selection
                .agent
                .map_or("desk", hide_desk::AdapterId::as_str)
                .to_owned(),
            name: "Desk".to_owned(),
            profile: None,
        }
    }

    /// Whether `opener` is Desk's own agent ([`Self::desk_opener`]).
    pub(in crate::runtime) fn is_desk_opener(&self, opener: &Opener) -> bool {
        opener.device_id == self.node.as_str() && opener.terminal_id == DESK_CALLER
    }

    /// 한눈에's section from the cards one build made (`carded`: the jobs,
    /// the workers and the named), so a session is listed only with its card.
    pub(super) fn desk_glance(
        &self,
        carded: &[&[DeskJobSnapshot]],
        index: &CardIndex<'_>,
    ) -> DeskGlanceSnapshot {
        let has_card = |pane: &str| {
            carded
                .iter()
                .flat_map(|cards| cards.iter())
                .any(|card| card.pane_id.as_deref() == Some(pane))
        };
        let sessions: Vec<String> = self
            .desk
            .stored
            .glance
            .iter()
            .rev()
            .filter(|kept| has_card(&kept.pane_id))
            .map(|kept| kept.pane_id.clone())
            .collect();
        let scopes: Vec<(&str, SessionScope)> = sessions
            .iter()
            .filter_map(|pane| Some((pane.as_str(), self.session_scope_of_pane(pane)?)))
            .collect();
        let mut files: Vec<(u64, DeskGlanceFile)> = Vec::new();
        for (key, display) in &index.displays {
            // The session that opened it last names its row; another of
            // 한눈에's that opened it before stands in when that one is not.
            let by_desk = display
                .opener
                .as_ref()
                .is_some_and(|opener| self.is_desk_opener(opener));
            let by_session = display
                .opener
                .iter()
                .chain(display.openers.iter().rev())
                .find_map(|opener| {
                    scopes
                        .iter()
                        .find(|(_, scope)| opener.of_session(scope))
                        .map(|(pane, _)| *pane)
                });
            let pane_id = match (by_desk, by_session) {
                (true, _) => None,
                (false, Some(pane)) => Some(pane.to_owned()),
                (false, None) => continue,
            };
            files.push((
                display.opened_stamp,
                DeskGlanceFile {
                    output: view_output(key, display),
                    pane_id,
                },
            ));
        }
        // Not looked at first, then the newest first.
        files.sort_by(|(at, file), (other_at, other)| {
            file.output
                .seen
                .cmp(&other.output.seen)
                .then(other_at.cmp(at))
        });
        let files_more = files.len().saturating_sub(MAX_GLANCE_FILES);
        files.truncate(MAX_GLANCE_FILES);
        DeskGlanceSnapshot {
            sessions,
            files: files.into_iter().map(|(_, file)| file).collect(),
            files_more,
        }
    }
}
