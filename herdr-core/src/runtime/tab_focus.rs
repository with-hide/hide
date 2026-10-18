//! The tab moves Hide has sent Herdr and not yet seen answered.
//!
//! Hide owns the visible tab and tells Herdr afterwards. Herdr's session
//! stream has no cursor naming the request an event answers, so whether a
//! Herdr move is Hide's own answer or somebody focusing a tab outside Hide is
//! read from one list: the tab moves that have left the control lane and not
//! been answered, in the order they left. Two facts give that order meaning.
//! The lane sends one control at a time and the next only after Herdr has
//! answered, so Herdr applies the requests in the order they were sent; and
//! Herdr delivers its events in order within a subscription (Socket API,
//! Event subscriptions), so the moves they make arrive in that order too.
//! The replica keeps those moves, one per `tab_focused`, and each session
//! carries the ones it has not been read with (`SessionTabMoves`): a session
//! ending on t2 cannot say whether Herdr went t2 or t2, t3, t2.
//!
//! The rest is derived from the list:
//!
//! - Each Herdr move answers the earliest request on its tab, and every
//!   request sent before that one has been applied already, so they leave
//!   with it. A session alone never answers anything: until the moves of
//!   earlier requests have arrived it shows Herdr from before them.
//! - A request for the tab Herdr shows moves nothing, and the pinned Herdr
//!   publishes no event for it, so its answer is the whole answer. Herdr
//!   does publish one for any other tab, another workspace's active tab
//!   included, so which tab Herdr shows is one value, not one per workspace;
//!   each request records it as it leaves (the tab of the request before it,
//!   or with none outstanding, the tab the last session showed).
//! - A refused request leaves at once and moved nothing, so the request
//!   after it takes over where Herdr was; a lost or silent one waits out the
//!   deadline, since Herdr may still apply it.
//! - Herdr moving on its own is followed only when nothing Hide asked for in
//!   that checkout is outstanding, sent or still waiting on the lane.
//! - When moves were dropped between two sessions (more than
//!   `TAB_FOCUS_LIMIT`, or a new replica after a reconnect), the tabs newly
//!   active since the last session stand in for them.
//!
//! A request still waiting on the lane is not in the list. The lane holds it
//! (and the pane focus intent its turn will send), and nothing of it can
//! reach Herdr until it leaves.

use super::control_lane::LaneJob;
use super::*;

/// One tab move sent to Herdr and not yet answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabFocusRequest {
    pub(super) checkout_id: String,
    pub(super) tab_id: String,
    /// The tab Herdr shows just before applying this request, as Hide knew
    /// when it left.
    herdr_before: Option<String>,
    /// The pane focus that moves Herdr to this tab; `None` for a tab focus
    /// or a creation.
    pub(super) pane_control_serial: Option<u64>,
    /// A successful pane control on this connection, while its stream move
    /// is still outstanding. Unknown answers never confirm a retry.
    pub(super) pane_focus_confirmed: bool,
    /// When it left, or for a pane focus when Herdr answered it.
    pub(super) requested_at_unix_ms: u64,
}

impl TabFocusRequest {
    fn expired_at(&self, now_unix_ms: u64) -> bool {
        now_unix_ms.saturating_sub(self.requested_at_unix_ms) >= VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS
    }
}

/// Outstanding tab asks as `(checkout, tab)`, in the order Herdr applies
/// them (`Runtime::tab_asks`).
pub(super) struct TabAsks(Vec<(String, String)>);

impl TabAsks {
    /// Whether Hide has asked Herdr for this tab and not yet seen it answered.
    pub(super) fn asked(&self, tab_id: &str) -> bool {
        self.0.iter().any(|(_, tab)| tab == tab_id)
    }

    /// Whether Hide has a tab move outstanding in this checkout: Herdr is
    /// about to be moved again, so a move it makes now is not followed.
    pub(super) fn outstanding_in(&self, checkout_id: &str) -> bool {
        self.0.iter().any(|(checkout, _)| checkout == checkout_id)
    }

    /// The tab Hide last asked for in this checkout.
    pub(super) fn newest_in(&self, checkout_id: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(checkout, _)| checkout == checkout_id)
            .map(|(_, tab)| tab.as_str())
    }
}

impl Runtime {
    /// A tab focus leaves the lane. A tab no checkout lists has nothing to
    /// follow, so nothing of it is kept.
    pub(super) fn send_tab_focus(&mut self, tab_id: &str) {
        if let Some(checkout_id) = self.checkout_holding_tab(tab_id) {
            self.send_tab_move(checkout_id, tab_id.to_owned(), None);
        }
    }

    /// A pane focus leaves the lane. A pane in a tab Herdr is not showing
    /// moves Herdr's tab too, and Herdr reports that move after answering
    /// the focus; one in the tab Herdr shows is settled by the answer.
    pub(super) fn send_pane_focus_tab(&mut self, pane_id: &str, serial: u64) {
        if let Some((checkout_id, tab_id)) = self.checkout_tab_holding_pane(pane_id) {
            self.send_tab_move(checkout_id, tab_id, Some(serial));
        }
    }

    /// Herdr created a tab with focus as part of the creation Hide asked
    /// for. When the session Hide last read already shows it, its move has
    /// arrived and nothing more of it will.
    pub(super) fn send_created_tab(&mut self, checkout_id: String, tab_id: String) {
        if !self.herdr_active_tabs_seen.contains(&tab_id) {
            self.send_tab_move(checkout_id, tab_id, None);
        }
    }

    fn send_tab_move(&mut self, checkout_id: String, tab_id: String, serial: Option<u64>) {
        let herdr_before = self
            .tab_focus_requests
            .last()
            .map(|earlier| earlier.tab_id.clone())
            .or_else(|| self.herdr_focused_tab_seen.clone());
        self.tab_focus_requests.push(TabFocusRequest {
            checkout_id,
            tab_id,
            herdr_before,
            pane_control_serial: serial,
            pane_focus_confirmed: false,
            requested_at_unix_ms: unix_milliseconds(),
        });
        if self.tab_focus_requests.len() > TAB_FOCUS_LIMIT {
            let evicted = self.tab_focus_requests.remove(0);
            crate::diagnostic!(serde_json::json!({
                "component": "view_state",
                "kind": "tab.focus.request_evicted",
                "checkout_id": evicted.checkout_id,
                "tab_id": evicted.tab_id,
                "limit": TAB_FOCUS_LIMIT,
            }));
        }
    }

    /// The control carrying the tab focus for `tab_id` never ran, so nothing
    /// of it reached Herdr.
    pub(super) fn unsend_tab_focus(&mut self, tab_id: &str) {
        if let Some(index) = self.tab_focus_index(tab_id) {
            self.withdraw_tab_move(index);
        }
    }

    /// The pane focus `serial` moves no tab: Herdr refused it, its pane went,
    /// or its control never ran.
    pub(super) fn drop_pane_focus_tab(&mut self, serial: Option<u64>) {
        if let Some(index) = serial.and_then(|serial| {
            self.tab_focus_requests
                .iter()
                .position(|request| request.pane_control_serial == Some(serial))
        }) {
            self.withdraw_tab_move(index);
        }
    }

    /// Removes a request that moved nothing; Herdr is where it was before
    /// it, so the request after it starts from there.
    fn withdraw_tab_move(&mut self, index: usize) -> TabFocusRequest {
        let withdrawn = self.tab_focus_requests.remove(index);
        if let Some(next) = self.tab_focus_requests.get_mut(index) {
            next.herdr_before.clone_from(&withdrawn.herdr_before);
        }
        withdrawn
    }

    /// Herdr answered the tab focus for `tab_id`.
    pub(super) fn answer_tab_focus(&mut self, tab_id: &str) {
        if let Some(index) = self.tab_focus_index(tab_id) {
            self.settle_where_herdr_already_was(index);
        }
    }

    /// Herdr answered the pane focus `serial`. A move still to come gets
    /// its deadline from this answer.
    pub(super) fn answer_pane_focus_tab(&mut self, serial: u64) {
        let Some(index) = self
            .tab_focus_requests
            .iter()
            .position(|request| request.pane_control_serial == Some(serial))
        else {
            return;
        };
        if !self.settle_where_herdr_already_was(index) {
            self.tab_focus_requests[index].pane_focus_confirmed = true;
            self.tab_focus_requests[index].requested_at_unix_ms = unix_milliseconds();
        }
    }

    /// An answered pane focus need not be resent while its tab move is still
    /// on the stream. This is control evidence, not the stream's baseline.
    pub(super) fn pane_focus_tab_confirmed(&self, tab_id: &str) -> bool {
        self.queued_tab_ask().is_none()
            && self
                .tab_focus_requests
                .last()
                .is_some_and(|request| request.pane_focus_confirmed && request.tab_id == tab_id)
    }

    /// Herdr refused the tab focus for `tab_id`, or its answer was lost.
    /// A refusal moved nothing; a lost answer may still have been applied,
    /// so it waits out its deadline. Only the newest ask is reported: an
    /// older one is not what the screen shows. A refusal with no request
    /// left (the control failed before it left, or its deadline had passed)
    /// is newest unless something was asked after it.
    pub(super) fn refuse_tab_focus(&mut self, tab_id: &str, message: &str, definite: bool) {
        let index = self.tab_focus_index(tab_id);
        let sent_after = index.is_some_and(|index| index + 1 < self.tab_focus_requests.len());
        if definite && let Some(index) = index {
            self.withdraw_tab_move(index);
        }
        if !sent_after && self.queued_tab_ask().is_none() {
            self.report_refused_view_focus(ViewFocusSlot::Tab, tab_id, message);
        }
    }

    /// Stops waiting on tab moves Herdr never reported. The value Hide chose
    /// is kept; only the waiting stops, so Herdr's next move is read as its
    /// own. The move of a pane focus still running waits with it. Only the
    /// newest ask is reported.
    pub(super) fn expire_tab_focus_requests(&mut self, now_unix_ms: u64) -> bool {
        let running = self
            .pane_focus_in_flight
            .as_ref()
            .map(|control| control.serial);
        let newest = (self.queued_tab_ask().is_none())
            .then(|| self.tab_focus_requests.len().checked_sub(1))
            .flatten();
        let mut timed_out = None;
        let mut index = 0;
        self.tab_focus_requests.retain(|request| {
            let keep = (request.pane_control_serial.is_some()
                && request.pane_control_serial == running)
                || !request.expired_at(now_unix_ms);
            if !keep && Some(index) == newest {
                timed_out = Some(request.tab_id.clone());
            }
            index += 1;
            keep
        });
        let Some(tab_id) = timed_out else {
            return false;
        };
        self.report_timed_out_view_focus(ViewFocusSlot::Tab, &tab_id);
        true
    }

    /// Takes the moves Herdr has made since the last session as answers and
    /// returns the requests they answered, each move answering the earliest
    /// request on its tab with every request sent before it. A snapshot read
    /// outside the event stream has no moves and answers nothing; the
    /// stream's next session brings them. When some were dropped while a
    /// request waited, the tabs newly active since the last session, and
    /// Herdr's focus arriving at a tab, stand in for them, and the gap is
    /// recorded; with nothing waiting, a reconnect's new count is no gap.
    pub(super) fn take_tab_focus_answers(
        &mut self,
        herdr: &HerdrTabView,
        moves: Option<&crate::sidebar::SessionTabMoves>,
        herdr_focus_moved: bool,
    ) -> Vec<TabFocusRequest> {
        let Some(moves) = moves else {
            return Vec::new();
        };
        let active = herdr.active_tab_ids();
        let read = self
            .herdr_tab_moves_read
            .replace((moves.generation, moves.applied));
        // The first session from the stream starts the count; nothing before
        // it is known to have moved.
        let Some((generation, consumed)) = read else {
            self.herdr_active_tabs_seen = active;
            return Vec::new();
        };
        let mut answered = Vec::new();
        match moves.since(generation, consumed) {
            Some(fresh) => {
                for tab_id in fresh {
                    if let Some(position) = self.earliest_tab_move(tab_id) {
                        answered.extend(self.tab_focus_requests.drain(..=position));
                    }
                }
            }
            None if self.tab_focus_requests.is_empty() => {}
            None => {
                crate::diagnostic!(serde_json::json!({
                    "component": "view_state",
                    "kind": "tab.focus.moves_gap",
                    "read_generation": generation,
                    "read": consumed,
                    "generation": moves.generation,
                    "applied": moves.applied,
                    "kept": moves.recent.len(),
                }));
                let mut moved = active
                    .difference(&self.herdr_active_tabs_seen)
                    .cloned()
                    .collect::<Vec<_>>();
                if herdr_focus_moved && let Some(focused) = herdr.focused_tab_id.as_ref() {
                    moved.push(focused.clone());
                }
                if let Some(last) = moved
                    .iter()
                    .filter_map(|tab_id| self.earliest_tab_move(tab_id))
                    .max()
                {
                    answered.extend(self.tab_focus_requests.drain(..=last));
                }
            }
        }
        self.herdr_active_tabs_seen = active;
        answered
    }

    fn earliest_tab_move(&self, tab_id: &str) -> Option<usize> {
        self.tab_focus_requests
            .iter()
            .position(|request| request.tab_id == tab_id)
    }

    /// Every tab move Hide has asked for and not seen answered, in the
    /// order Herdr will apply them: the sent ones, then those still waiting
    /// on the lane. Collected once for a pass that asks about each checkout.
    pub(super) fn tab_asks(&self) -> TabAsks {
        TabAsks(
            self.tab_focus_requests
                .iter()
                .map(|request| (request.checkout_id.clone(), request.tab_id.clone()))
                .chain(self.queued_tab_asks())
                .collect(),
        )
    }

    /// The tab Hide last asked for anywhere.
    pub(super) fn newest_tab_ask(&self) -> Option<String> {
        self.queued_tab_ask().or_else(|| {
            self.tab_focus_requests
                .last()
                .map(|request| request.tab_id.clone())
        })
    }

    fn queued_tab_ask(&self) -> Option<String> {
        self.queued_tab_asks().last().map(|(_, tab)| tab)
    }

    /// The tab moves waiting on the lane, in the order they will leave, with
    /// their checkouts: tab focuses, and the tab of a pane focus whose turn
    /// has not come.
    fn queued_tab_asks(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.control_lane.queued_jobs().filter_map(|job| match job {
            LaneJob::Tab(RemoteControlAction::FocusTab { tab_id }) => self
                .checkout_holding_tab(tab_id)
                .map(|checkout_id| (checkout_id, tab_id.clone())),
            LaneJob::PaneFocus => self
                .pending_pane_focus
                .as_ref()
                .filter(|pending| !pending.sent)
                .and_then(|pending| self.checkout_tab_holding_pane(&pending.target_id)),
            LaneJob::Tab(_) => None,
        })
    }

    /// The newest sent tab focus for `tab_id`: the lane runs one control at
    /// a time, so the one Herdr is answering is the last that left.
    fn tab_focus_index(&self, tab_id: &str) -> Option<usize> {
        self.tab_focus_requests
            .iter()
            .rposition(|request| request.pane_control_serial.is_none() && request.tab_id == tab_id)
    }

    /// Ends the request at `index` when Herdr was already on its tab as it
    /// applied it, and says whether it did.
    fn settle_where_herdr_already_was(&mut self, index: usize) -> bool {
        let request = &self.tab_focus_requests[index];
        let herdr_there = request.herdr_before.as_ref() == Some(&request.tab_id);
        if herdr_there {
            let request = self.tab_focus_requests.remove(index);
            crate::diagnostic!(serde_json::json!({
                "component": "view_state",
                "kind": "tab.focus.confirmed_by_answer",
                "checkout_id": request.checkout_id,
                "tab_id": request.tab_id,
            }));
        }
        herdr_there
    }

    pub(super) fn checkout_holding_tab(&self, tab_id: &str) -> Option<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id))
            })
            .map(|checkout| checkout.id.clone())
    }
}
