//! The Project tool's 흘린 것 actions (PRD project-panel-results D-06..D-08,
//! D-17, B11-B16): 마무리 맡기기 writes fixed words to the session that last
//! worked in the worktree, or starts an agent there with them; 놔두기 and
//! 되돌리기 put a row away and back. Which worktrees are rows is
//! `agent_state::tally::project_status`'s.

use super::*;
use crate::agent_state::tally::project_status::{MAX_STRAY_DISMISSALS, Stray, StrayDismissal};
use crate::model::StrayHandOffSnapshot;
use serde::Deserialize;

/// The 마무리 맡기기 receipts the status keeps, oldest dropped first.
const HAND_OFF_RECEIPTS: usize = 16;

/// `stray_dismiss` and `stray_restore`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StrayPayload {
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
}

/// `stray_hand_off`: the row's own request, so a second press of the same
/// one sends nothing again (B12).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StrayHandOffPayload {
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    pub(super) request_id: String,
}

/// The words a hand-off sends for what is left (B11): one sentence per
/// kind, together when there are several.
pub(crate) fn hand_off_words(stray: &Stray) -> String {
    let mut words = Vec::new();
    if stray.changed_files > 0 {
        words.push("커밋하고 PR 열어줘".to_owned());
    } else if stray.unpushed > 0 {
        words.push("브랜치 푸시하고 PR 열어줘".to_owned());
    }
    if let Some(number) = stray.failed_pr {
        words.push(format!("#{number} CI 실패 원인 고쳐줘"));
    }
    words.join(". ")
}

impl Runtime {
    /// The row the project publishes for `checkout_id` now, with its
    /// checkout's path and device.
    fn published_stray(
        &self,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Option<(Stray, String, String)> {
        let project = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|project| project.id == workspace_id)?;
        let stray = project
            .agent_scope
            .status
            .strays
            .iter()
            .find(|row| row.checkout_id == checkout_id)?
            .clone();
        let checkout = project
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)?;
        Some((stray, checkout.path.clone(), project.device_id.clone()))
    }

    /// 놔두기 (B15): the row goes until a new commit or a new CI failure.
    /// Past the cap the oldest 놔두기 goes and its row stands again (B16).
    pub(super) fn dismiss_stray(&mut self, payload: StrayPayload) -> bool {
        let Some((stray, _, _)) = self.published_stray(&payload.workspace_id, &payload.checkout_id)
        else {
            crate::diagnostic!(serde_json::json!({
                "component": "project_status", "kind": "stray.dismiss_unavailable",
                "project": payload.workspace_id, "checkout": payload.checkout_id,
            }));
            return false;
        };
        let dismissals = &mut self.snapshot.ui_state.stray_dismissals;
        dismissals.insert(
            stray.checkout_id.clone(),
            StrayDismissal {
                head: stray.head.clone(),
                failed_pr: stray.failed_pr,
                at_unix_ms: unix_milliseconds(),
            },
        );
        while dismissals.len() > MAX_STRAY_DISMISSALS {
            let Some(oldest) = dismissals
                .iter()
                .min_by_key(|(_, dismissal)| dismissal.at_unix_ms)
                .map(|(checkout, _)| checkout.clone())
            else {
                break;
            };
            dismissals.remove(&oldest);
            crate::diagnostic!(serde_json::json!({
                "component": "project_status", "kind": "stray.dismissal_capacity",
                "checkout": oldest, "limit": MAX_STRAY_DISMISSALS,
            }));
        }
        self.persist_ui_state();
        self.refresh_agent_scopes();
        true
    }

    /// 되돌리기 (B15): the 놔두기 goes and the row stands again.
    pub(super) fn restore_stray(&mut self, payload: StrayPayload) -> bool {
        if self
            .snapshot
            .ui_state
            .stray_dismissals
            .remove(&payload.checkout_id)
            .is_none()
        {
            return false;
        }
        self.persist_ui_state();
        self.refresh_agent_scopes();
        true
    }

    /// 마무리 맡기기 (B11-B13): the words go to the live session that last
    /// worked in the worktree, or a new agent of its kind (else the one the
    /// operator last started) starts there with them. A request already
    /// taken sends nothing again; a failure keeps the row and goes to the
    /// diagnostic log.
    pub(super) fn hand_off_stray(&mut self, payload: StrayHandOffPayload) -> bool {
        if self
            .snapshot
            .status
            .stray_hand_offs
            .iter()
            .any(|receipt| receipt.request_id == payload.request_id)
        {
            return false;
        }
        let Some((stray, path, device_id)) =
            self.published_stray(&payload.workspace_id, &payload.checkout_id)
        else {
            self.settle_stray_hand_off(
                &payload.request_id,
                &payload.checkout_id,
                Err("The worktree has nothing left to finish".to_owned()),
            );
            return true;
        };
        let words = hand_off_words(&stray);
        self.note_stray_hand_off(&payload.request_id, &payload.checkout_id, "sending");
        let live = stray
            .pane_id
            .as_deref()
            .filter(|pane| self.agent_row(pane).is_some());
        if let Some(pane) = live {
            let started = self.pane_api_route(pane).and_then(|route| {
                spawn_stray_prompt(
                    route,
                    payload.request_id.clone(),
                    words,
                    Arc::clone(&self.input_gate),
                )
            });
            if let Err(reason) = started {
                self.settle_stray_hand_off(&payload.request_id, &payload.checkout_id, Err(reason));
            }
            return true;
        }
        let provider = stray
            .agent
            .clone()
            .or_else(|| self.snapshot.ui_state.agent_start.kind.clone())
            .unwrap_or_else(|| crate::model::AGENT_KINDS[0].to_owned());
        let local = device_id == self.node.as_str();
        self.agent_start_in_checkout(super::events::AgentStartInCheckoutPayload {
            confirm_only: false,
            target: None,
            checkout_path: Some(path),
            device_id: (!local).then_some(device_id),
            home: false,
            provider,
            prompt: Some(words),
            model: None,
            request_id: Some(payload.request_id.clone()),
            resume_session_id: None,
            resume_session_path: None,
            stay_behind: true,
        });
        let refused = self
            .snapshot
            .status
            .last_error
            .as_ref()
            .filter(|error| error.request_id.as_deref() == Some(payload.request_id.as_str()))
            .map(|error| error.message.clone());
        self.settle_stray_hand_off(
            &payload.request_id,
            &payload.checkout_id,
            refused.map_or(Ok(()), Err),
        );
        true
    }

    /// Where a pane's hand-off ended (B12, B13).
    pub fn ingest_stray_prompt(
        &mut self,
        pane_id: &str,
        request_id: &str,
        result: Result<(), String>,
    ) -> bool {
        let Some(checkout) = self
            .snapshot
            .status
            .stray_hand_offs
            .iter()
            .find(|receipt| receipt.request_id == request_id)
            .map(|receipt| receipt.checkout_id.clone())
        else {
            return false;
        };
        if result.is_ok() {
            // The words are the operator's own message to the session.
            self.note_delivery_reply(pane_id);
            self.record_operator_submit(pane_id);
        }
        self.settle_stray_hand_off(request_id, &checkout, result);
        true
    }

    fn settle_stray_hand_off(
        &mut self,
        request_id: &str,
        checkout_id: &str,
        result: Result<(), String>,
    ) {
        let state = match &result {
            Ok(()) => "sent",
            Err(_) => "failed",
        };
        crate::diagnostic!(serde_json::json!({
            "component": "project_status", "kind": "stray.hand_off",
            "request_id": request_id, "checkout": checkout_id, "state": state,
            "reason": result.err(),
        }));
        self.note_stray_hand_off(request_id, checkout_id, state);
    }

    fn note_stray_hand_off(&mut self, request_id: &str, checkout_id: &str, state: &'static str) {
        let receipts = &mut self.snapshot.status.stray_hand_offs;
        match receipts
            .iter_mut()
            .find(|receipt| receipt.request_id == request_id)
        {
            Some(receipt) => receipt.state = state,
            None => {
                receipts.push(StrayHandOffSnapshot {
                    request_id: request_id.to_owned(),
                    checkout_id: checkout_id.to_owned(),
                    state,
                });
                if receipts.len() > HAND_OFF_RECEIPTS {
                    receipts.remove(0);
                }
            }
        }
    }
}

/// Submits a 마무리 맡기기's words to the session's agent as the operator's
/// own message (PRD project-panel-results B11), through the input gate so a
/// sleeping agent's shell never takes them as a command. The request runs
/// on its own thread, never under the runtime mutex.
fn spawn_stray_prompt(
    route: crate::find::pane::PaneApiRoute,
    request_id: String,
    text: String,
    gate: Arc<crate::input_gate::PaneInputGate>,
) -> Result<(), String> {
    let core_pane_id = route.pane_id.clone();
    route.spawn(
        "herdr-core-stray-prompt",
        move |engine, herdr_pane_id| {
            let _admission = gate
                .admit(&core_pane_id)
                .map_err(|_| "The agent is asleep".to_owned())?;
            engine
                .prompt_agent(crate::live::CONTROL_WAIT, herdr_pane_id, &text)
                .map_err(crate::live::failed("agent.prompt"))
        },
        move |runtime, pane_id, result| runtime.ingest_stray_prompt(pane_id, &request_id, result),
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    fn stray(changed_files: u32, unpushed: u32, failed_pr: Option<u32>) -> Stray {
        Stray {
            checkout_id: "c".into(),
            branch: Some("work".into()),
            name: None,
            pane_id: None,
            agent: None,
            changed_files,
            unpushed,
            failed_pr,
            since_unix_ms: None,
            head: None,
        }
    }

    /// PRD project-panel-results B11: each kind's words, together when
    /// several are left; committing changes already pushes the branch.
    #[test]
    fn a_hand_off_says_what_is_left() {
        assert_eq!(hand_off_words(&stray(4, 0, None)), "커밋하고 PR 열어줘");
        assert_eq!(
            hand_off_words(&stray(0, 3, None)),
            "브랜치 푸시하고 PR 열어줘"
        );
        assert_eq!(
            hand_off_words(&stray(0, 0, Some(1815))),
            "#1815 CI 실패 원인 고쳐줘"
        );
        assert_eq!(
            hand_off_words(&stray(2, 3, Some(7))),
            "커밋하고 PR 열어줘. #7 CI 실패 원인 고쳐줘"
        );
    }
}
