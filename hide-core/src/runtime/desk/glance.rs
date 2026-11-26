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
//! records on the View it lands in ([`Runtime::desk_note_agent_view`]); each
//! View's own record says whether it was looked at.

use super::super::Runtime;
use super::jobs::{CardIndex, view_output};
use crate::model::{DeskGlanceFile, DeskGlanceSnapshot, DeskJobSnapshot};
use crate::view_feedback::{Opener, SessionScope, Unseen};
use crate::workspace_control::DESK_CALLER;

/// The Views 한눈에 lists; past it the section counts the rest.
const MAX_GLANCE_FILES: usize = 32;

impl Runtime {
    /// The operator opened `pane`'s session beside Desk: it is one of
    /// 한눈에's, the newest. Whether the store changed, for a save.
    pub(in crate::runtime) fn desk_glance_opened(&mut self, pane: &str) -> bool {
        self.desk.worker.is_some()
            && self
                .desk
                .stored
                .keep_glance(pane, crate::runtime::unix_milliseconds())
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

    fn is_desk_opener(&self, opener: &Opener) -> bool {
        opener.device_id == self.node.as_str() && opener.terminal_id == DESK_CALLER
    }

    /// Desk's agent's `hide file open` placed a file in front of Desk's
    /// desk: that View records Desk's agent as its opener and is one for the
    /// operator to look at, until it is in front of the desk while the desk
    /// shows (`desk_output_seen`) or opened from 한눈에.
    pub(in crate::runtime) fn desk_note_agent_view(&mut self) {
        let opener = self.desk_opener();
        let now = crate::runtime::unix_milliseconds();
        let marked = self.change_desk_views(|layout, stamp| {
            let Some(id) = layout.active_area().active.clone() else {
                return Ok(((), false));
            };
            let Some(display) = layout.display_mut(&id) else {
                return Ok(((), false));
            };
            display.opened_by(opener, stamp);
            display.unseen = Some(Unseen::opened(now, display.unseen.as_ref()));
            Ok(((), true))
        });
        if let Err(error) = marked {
            crate::diagnostic!(serde_json::json!({
                "component": "desk",
                "kind": "desk_glance.agent_view_unmarked",
                "error": error.kind(),
            }));
        }
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
