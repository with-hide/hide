//! The first-run wizard's progress (PRD first-run-onboarding D-07, B6): the
//! step it opens on, when its first step's apply has settled, the desktop
//! notification switch it and Settings > Notifications share (D-21), and the
//! one-time introduction of the Factory screen (B43). The shell
//! draws the steps; each move is one event here, saved with the UI state.

use super::Runtime;
use crate::model::{AgentOnboarding, FIRST_RUN_VERSION, FirstRun, FirstRunStep, ScreenIntro};

/// The first step's Continue, waiting for the kit pass it queued on this Mac.
pub(super) struct FirstRunApply {
    /// The agents the step showed on.
    agents: Vec<String>,
    /// A browser tab's wizard ends with this step (B26).
    finish: bool,
    /// This Continue answered the first-run question (the record waited);
    /// a pass that did not run leaves that question unanswered again.
    answer: bool,
}

impl Runtime {
    /// Discovery proves saved configuration only. Unknown and failed reads
    /// keep a step with loading/retry; only an actual empty result skips it.
    pub(super) fn first_run_has_devices(&self) -> bool {
        let profiles = &self.snapshot.status.herdr_profiles;
        profiles.state != "ready" || !profiles.profiles.is_empty()
    }

    pub(super) fn settle_first_run_devices(&mut self) -> bool {
        if self.first_run_step() == Some(FirstRunStep::Devices) && !self.first_run_has_devices() {
            return self.move_first_run(FirstRunStep::Devices, FirstRunStep::Notifications);
        }
        false
    }

    /// Navigation has no installation, permission or reconnect side effect.
    pub(super) fn back_first_run(&mut self, from: FirstRunStep) -> bool {
        if self.first_run_step() != Some(from) {
            return false;
        }
        let to = match from {
            FirstRunStep::Devices => FirstRunStep::Agents,
            FirstRunStep::Notifications if self.first_run_has_devices() => FirstRunStep::Devices,
            FirstRunStep::Notifications => FirstRunStep::Agents,
            FirstRunStep::Project => FirstRunStep::Notifications,
            FirstRunStep::Herdr | FirstRunStep::Agents | FirstRunStep::Done => return false,
        };
        self.move_first_run(from, to)
    }

    /// Bounded draft data goes through the existing coalesced UI save owner.
    /// A pending agent apply owns its selection until that pass settles.
    pub(super) fn update_first_run_draft(
        &mut self,
        payload: super::events::FirstRunDraftPayload,
    ) -> bool {
        let ids_fit = |ids: &[String]| {
            ids.len() <= 128 && ids.iter().all(|id| !id.is_empty() && id.len() <= 256)
        };
        if payload.agents.as_ref().is_some_and(|ids| !ids_fit(ids))
            || payload.devices.as_ref().is_some_and(|ids| !ids_fit(ids))
            || payload
                .project_device
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 256)
            || payload
                .project_path
                .as_ref()
                .is_some_and(|path| path.len() > 4096)
            || payload.project_form.as_ref().is_some_and(|form| {
                !["", "add", "clone", "create"].contains(&form.view.as_str())
                    || [&form.url, &form.parent, &form.name]
                        .iter()
                        .any(|value| value.len() > 4096)
            })
        {
            self.set_error(
                "first_run.draft_invalid",
                "The setup choice could not be saved. Choose again.",
                false,
            );
            return true;
        }
        let before = self.snapshot.ui_state.first_run_choices.clone();
        let pending = self.first_run_apply_in_flight();
        let draft = &mut self.snapshot.ui_state.first_run_choices;
        if let Some(mut agents) = payload.agents.filter(|_| !pending) {
            agents.sort();
            agents.dedup();
            draft.agents = Some(agents);
        }
        if let Some(mut devices) = payload.devices {
            devices.sort();
            devices.dedup();
            draft.devices = devices;
        }
        if let Some(device) = payload.project_device {
            draft.project_device = Some(device);
        }
        if let Some(path) = payload.project_path {
            draft.project_path = path;
        }
        if let Some(form) = payload.project_form {
            draft.project_form = form;
        }
        if before == *draft {
            return false;
        }
        self.persist_ui_state();
        true
    }
    /// The step the wizard opens on, or `None` when it does not open: a Mac
    /// that was never asked, or progress saved by another flow version.
    pub(super) fn first_run_step(&self) -> Option<FirstRunStep> {
        self.snapshot
            .ui_state
            .first_run
            .filter(|run| run.version == FIRST_RUN_VERSION)
            .map(|run| run.step)
    }

    /// Starts the wizard on a Mac whose kit record waits for the first-run
    /// agent choice; any progress already saved stands. It opens on moving
    /// the operator's older Herdr when there is one to move or the look at
    /// the server has not answered yet (PRD herdr-takeover B1, D-09), and
    /// stays closed after its move was left unanswered (B9). The caller
    /// saves.
    pub(super) fn open_first_run(&mut self) -> bool {
        if self.snapshot.ui_state.first_run.is_some() || self.first_run_closed_for_herdr() {
            return false;
        }
        let step = if self.first_run_herdr_pending() {
            FirstRunStep::Herdr
        } else {
            FirstRunStep::Agents
        };
        self.snapshot.ui_state.first_run = Some(FirstRun {
            version: FIRST_RUN_VERSION,
            step,
        });
        crate::diagnostic!(serde_json::json!({
            "component": "first_run",
            "kind": "first_run.opened",
            "version": FIRST_RUN_VERSION,
            "step": step.as_str(),
        }));
        true
    }

    pub(super) fn move_first_run(&mut self, from: FirstRunStep, to: FirstRunStep) -> bool {
        if to == FirstRunStep::Done {
            let choices = &mut self.snapshot.ui_state.first_run_choices;
            choices.project_device = None;
            choices.project_path.clear();
            choices.project_form = Default::default();
        }
        self.snapshot.ui_state.first_run = Some(FirstRun {
            version: FIRST_RUN_VERSION,
            step: to,
        });
        self.persist_ui_state();
        crate::diagnostic!(serde_json::json!({
            "component": "first_run",
            "kind": "first_run.step",
            "from": from.as_str(),
            "to": to.as_str(),
        }));
        true
    }

    /// The operator left the step `from` (Continue, Not now, Skip for now, a
    /// project added). Only the step on screen moves, so a second press or a
    /// window still showing an earlier step changes nothing (engineering rule
    /// 11). The first step ends through its apply, never through this event,
    /// unless this Mac's pass stopped before changing anything: then there is
    /// nothing to choose, and Not now closes the wizard with the question
    /// unanswered, so the next pass that waits for it opens it again.
    /// Leaving the notification step may set the desktop switch with the same
    /// press: Continue on a Mac that already allows Hide turns it on (B16),
    /// Not now leaves it off even after Turn on was pressed while macOS asks
    /// (B21); `None` leaves it as it is.
    pub(super) fn advance_first_run(
        &mut self,
        from: FirstRunStep,
        notifications: Option<bool>,
    ) -> bool {
        let to = match from {
            FirstRunStep::Devices => FirstRunStep::Notifications,
            FirstRunStep::Notifications => FirstRunStep::Project,
            FirstRunStep::Project => FirstRunStep::Done,
            FirstRunStep::Agents
                if self.first_run_step() == Some(FirstRunStep::Agents)
                    && self.kit_view(self.node.as_str()).refused.is_some() =>
            {
                self.snapshot.ui_state.first_run = None;
                self.first_run_apply = None;
                self.persist_ui_state();
                crate::diagnostic!(serde_json::json!({
                    "component": "first_run",
                    "kind": "first_run.closed_refused",
                }));
                return true;
            }
            // The Herdr step ends through its own event (`herdr_takeover`).
            FirstRunStep::Herdr | FirstRunStep::Agents | FirstRunStep::Done => {
                crate::diagnostic!(serde_json::json!({
                    "component": "first_run",
                    "kind": "first_run.advance_refused",
                    "from": from.as_str(),
                }));
                return false;
            }
        };
        if self.first_run_step() != Some(from) {
            return false;
        }
        if let (Some(enabled), FirstRunStep::Notifications) = (notifications, from) {
            self.set_desktop_notification_preferences(Some(enabled), (!enabled).then_some(false));
        }
        if from == FirstRunStep::Notifications {
            self.snapshot
                .ui_state
                .first_run_choices
                .notifications_answered = true;
        }
        self.move_first_run(from, to)
    }

    /// Whether the first step's Continue still waits for its kit pass; a
    /// second press meanwhile is the same intent (B10).
    pub(super) fn first_run_apply_in_flight(&self) -> bool {
        self.first_run_apply.is_some()
    }

    /// The first step's Continue queued its choice: the step ends once this
    /// Mac's kit pass says every chosen agent is in place (B12). With nothing
    /// queued, what the kit last said decides now.
    pub(super) fn wait_for_first_run_apply(
        &mut self,
        agents: Vec<String>,
        finish: bool,
        answer: bool,
    ) -> bool {
        // A browser tab's wizard is the agent step alone (PRD herdr-takeover
        // D-24): its Continue leaves a Herdr step the desktop app would have
        // shown first unanswered, and Overview offers the move instead.
        if finish && self.first_run_step() == Some(FirstRunStep::Herdr) {
            self.move_first_run(FirstRunStep::Herdr, FirstRunStep::Agents);
        }
        if self.first_run_step() != Some(FirstRunStep::Agents) {
            return false;
        }
        self.first_run_apply = Some(FirstRunApply {
            agents: agents.clone(),
            finish,
            answer,
        });
        self.snapshot.ui_state.first_run_choices.agents = Some(agents);
        self.persist_ui_state();
        self.settle_first_run_apply()
    }

    /// Decides the first step after this Mac's kit report was stored, or
    /// after its kit could not run. A report while work is still queued
    /// predates the pass Continue asked for and decides nothing. A pass that
    /// did not run, a record that did not save (B6) and a chosen agent whose
    /// piece failed (B12) keep the step on screen; otherwise the wizard moves
    /// on.
    pub(super) fn settle_first_run_apply(&mut self) -> bool {
        if self.local_kit_pending.is_some() || self.first_run_apply.is_none() {
            return false;
        }
        let Some(apply) = self.first_run_apply.take() else {
            return false;
        };
        if self.first_run_step() != Some(FirstRunStep::Agents) {
            return false;
        }
        if self.snapshot.ui_state.agent_onboarding == Some(AgentOnboarding::Pending) {
            crate::diagnostic!(serde_json::json!({
                "component": "first_run",
                "kind": "first_run.apply_unsaved",
            }));
            return false;
        }
        let node = self.node.clone();
        let view = self.kit_view(node.as_str());
        // The kit could not run here, or ran without listing an agent
        // (retirement refused, account lock): nothing was written, so a
        // first-run answer is unanswered again and Continue sends it anew,
        // and the agents are read again for the step to show.
        if view.unavailable.is_some() || view.agents.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "first_run",
                "kind": "first_run.apply_not_run",
                "unavailable": view.unavailable,
            }));
            if apply.answer {
                self.snapshot.ui_state.agent_onboarding = Some(AgentOnboarding::Pending);
                self.persist_ui_state();
            }
            self.local_kit_check_requested = true;
            return true;
        }
        let failed: Vec<String> = view
            .agents
            .iter()
            .filter(|row| apply.agents.contains(&row.id) && row.enabled)
            .filter(|row| {
                [Some(&row.skill), row.hook.as_ref(), row.herdr.as_ref()]
                    .into_iter()
                    .flatten()
                    // A piece refused because the agent's copy is too old is
                    // not a failure Try again fixes: the step shows the
                    // update and goes on, and the update installs it.
                    .any(|piece| {
                        piece.state == hide_kit::ComponentState::Failed
                            && piece
                                .why
                                .as_ref()
                                .is_none_or(|why| why.cause != hide_kit::PartCause::UpdateAgent)
                    })
            })
            .map(|row| row.id.clone())
            .collect();
        if !failed.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "first_run",
                "kind": "first_run.apply_failed",
                "agents": failed,
            }));
            return false;
        }
        let to = if apply.finish {
            FirstRunStep::Done
        } else {
            if self.first_run_has_devices() {
                FirstRunStep::Devices
            } else {
                FirstRunStep::Notifications
            }
        };
        self.move_first_run(FirstRunStep::Agents, to)
    }

    /// One user intent changes either choice, or turns both off in the
    /// wizard. Permission is the desktop host's to read. Saves use the
    /// existing coalesced UI-state writer, and unchanged choices publish nothing.
    pub(super) fn set_desktop_notification_preferences(
        &mut self,
        needs_you: Option<bool>,
        done: Option<bool>,
    ) -> bool {
        let state = &mut self.snapshot.ui_state;
        let needs_you = needs_you.unwrap_or(state.desktop_notifications);
        let done = done.unwrap_or(state.desktop_notifications_done);
        if state.desktop_notifications == needs_you && state.desktop_notifications_done == done {
            return false;
        }
        state.desktop_notifications = needs_you;
        state.desktop_notifications_done = done;
        self.persist_ui_state();
        crate::diagnostic!(serde_json::json!({
            "component": "first_run",
            "kind": "desktop_notifications.set",
            "needs_you": needs_you,
            "done": done,
        }));
        true
    }

    /// A screen's one-line introduction was shown; it is not shown again.
    pub(super) fn mark_screen_intro_seen(&mut self, screen: ScreenIntro) -> bool {
        if !self.snapshot.ui_state.screen_intros_seen.insert(screen) {
            return false;
        }
        self.persist_ui_state();
        true
    }
}
