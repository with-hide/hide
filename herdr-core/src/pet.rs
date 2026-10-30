//! Pet poses and their idle sequence. Agent totals and the attention queue
//! come from `agent_state::tally`; the pose reads those established values.

#[cfg(test)]
use crate::agent_state::{
    AgentGroup, attention_order, observe_unseen, subagents_active, summarize,
};
#[cfg(test)]
use crate::model::SidebarAgentSnapshot;
#[cfg(test)]
use std::collections::BTreeMap;

pub const SLEEP_IDLE_MS: u64 = 60_000;
pub const SLEEP_PHASE_MS: u64 = 4_000;
pub const FREE_ROAM_IDLE_MS: u64 = 8_000;

/// The pet counts the sidebar's four groups, plus the agents whose server
/// stopped answering, so the badge row and the sidebar can never disagree
/// about what is waiting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PetSummary {
    /// The pet's "act now" count: exactly the Needs You group.
    pub needs_you: usize,
    pub done: usize,
    pub working: usize,
    pub seen: usize,
    pub disconnected: usize,
    /// How many of `needs_you` are errors rather than questions or approvals.
    /// This is not a group of its own and nothing draws a count from it; the
    /// pose ladder puts `error` one rung above `notification`, so the ladder
    /// needs to know an error is in there.
    pub error: usize,
}

impl PetSummary {
    pub fn total(&self) -> usize {
        self.needs_you + self.working + self.done + self.seen + self.disconnected
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SleepPhase {
    #[default]
    Awake,
    Yawning,
    Dozing,
    Collapsing,
    Sleeping,
}

impl SleepPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Awake => "awake",
            Self::Yawning => "yawning",
            Self::Dozing => "dozing",
            Self::Collapsing => "collapsing",
            Self::Sleeping => "sleeping",
        }
    }
}

/// The clawd-compatible sleep sequence starts after one minute of inactivity.
/// Each transitional pose lasts four seconds; once asleep it stays asleep
/// until activity wakes it.
pub fn sleep_phase_for_idle_ms(idle_ms: u64) -> SleepPhase {
    if idle_ms < SLEEP_IDLE_MS {
        return SleepPhase::Awake;
    }
    match (idle_ms - SLEEP_IDLE_MS) / SLEEP_PHASE_MS {
        0 => SleepPhase::Yawning,
        1 => SleepPhase::Dozing,
        2 => SleepPhase::Collapsing,
        _ => SleepPhase::Sleeping,
    }
}

/// The documented pose ladder:
/// `error > notification > sweeping > attention > carrying/juggling > working
/// > thinking > idle/roam > sleeping`.
///
/// Herdr has no separate sweeping or thinking token, so those rungs stay
/// reserved and fall through to the surrounding states.
pub fn expanded_state(summary: PetSummary, idle_ms: u64, waking: bool) -> &'static str {
    if summary.error > 0 {
        "error"
    } else if summary.needs_you > 0 {
        "notification"
    } else if summary.working >= 2 {
        "juggling"
    } else if summary.working == 1 {
        "carrying"
    } else if waking {
        "waking"
    } else if sleep_phase_for_idle_ms(idle_ms) == SleepPhase::Sleeping {
        "sleeping"
    } else if idle_ms >= FREE_ROAM_IDLE_MS {
        "roam"
    } else if summary.working > 0 {
        "working"
    } else {
        "idle"
    }
}

pub fn is_roam_allowed(summary: PetSummary, idle_ms: u64, dragging: bool) -> bool {
    !dragging
        && summary.needs_you == 0
        && summary.working == 0
        && summary.seen > 0
        && (FREE_ROAM_IDLE_MS..SLEEP_IDLE_MS).contains(&idle_ms)
}

/// The single pose the renderer draws.
///
/// A lost connection outranks everything: an unreachable server cannot say
/// anything true about agent state, so the pet says so rather than holding a
/// stale working pose. Otherwise this is [`expanded_state`] with the sleep
/// sequence's transitional poses made visible between 60s and 72s of idle,
/// which `expanded_state` itself reports as `roam`.
pub fn pose(summary: PetSummary, idle_ms: u64, waking: bool, connected: bool) -> &'static str {
    if !connected {
        return "disconnected";
    }
    let state = expanded_state(summary, idle_ms, waking);
    if state != "roam" {
        return state;
    }
    match sleep_phase_for_idle_ms(idle_ms) {
        SleepPhase::Awake => "roam",
        phase => phase.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_hooks::PaneHookTokens;

    /// A projected row named by the group it landed in. `needs_you` rows carry
    /// a question unless the name says error, which is the one demand the pet
    /// counts separately.
    fn agent(pane_id: &str, group: &str) -> SidebarAgentSnapshot {
        let (group, demand) = match group {
            "error" => ("needs_you", "error"),
            "needs_you" => ("needs_you", "question"),
            other => (other, "none"),
        };
        SidebarAgentSnapshot {
            record_connection: None,
            record_summary: None,
            record_metadata: None,
            telemetry_stale: false,
            start: Default::default(),
            close_session: None,
            state: Default::default(),
            resolved: None,
            resolved_recent: false,
            revisit: None,
            escalation: None,
            letters_waiting: None,
            raised: Vec::new(),
            pulls: None,
            descendant_mark: None,
            descendant_line: None,
            id: pane_id.to_owned(),
            herdr_name: None,
            started_name: None,
            declared_fork: None,
            forked_from: None,
            pane_id: pane_id.to_owned(),
            workspace_label: "Fixture".to_owned(),
            identity_label: "Fixture".to_owned(),
            checkout_label: None,
            agent_kind: "codex".to_owned(),
            demand: demand.to_owned(),
            activity: if group == "working" {
                "working"
            } else {
                "stopped"
            }
            .to_owned(),
            completed: group == "done",
            unread: group != "seen",
            blocked: false,
            group: group.to_owned(),
            mark: Default::default(),
            emphasized: false,
            status_code: crate::model::AgentStatusCode::Idle,
            requires_close_confirmation: false,
            requires_close_status_check: false,
            progress: None,
            expected_reply: None,
            detail: None,
            message: None,
            user_turn: None,
            ask_subject: None,
            laid_since_unix_ms: None,
            laid_more: 0,
            status_word_visible: true,
            changed_at_unix_ms: None,
            last_prompt_at_unix_ms: None,
            last_conversation_at_unix_ms: None,
            last_activity: "0000000000001".to_owned(),
            state_change_seq: None,
            session_id: None,
            own_find: false,
            spawned_from_pane_id: None,
            declared_parent_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: None,
            server_mark: None,
            delegated: false,
            descendant_counts: crate::model::DescendantCountsSnapshot::default(),
            direct_child_counts: crate::model::DescendantCountsSnapshot::default(),
            wait: None,
            checks_wait: None,
            background_left: 0,
            checks_ended: None,
            descendant_signals: std::collections::BTreeSet::new(),
            lineage_parent_pane_id: None,
            lineage_path_pane_ids: Vec::new(),
            lineage_sibling_pane_ids: Vec::new(),
            lineage_depth: 0,
            lineage_child_pane_ids: Vec::new(),
            close_descendant_pane_ids: Vec::new(),
            lineage_root_checkout_id: None,
            lineage_worktree_badge: None,
            lineage_orphan: false,
            lineage_hint: None,
            raised_hint: None,
            spawn_origin_pane_id: None,
            lineage_collapsed: false,
            sleep: None,
            row_facts: None,
            awaited_replies: Vec::new(),
            hide_ai: None,
            request: None,
            unseen_views: Vec::new(),
            look: None,
        }
    }

    fn idle_summary() -> PetSummary {
        PetSummary {
            seen: 1,
            ..Default::default()
        }
    }

    /// sidebar-agent-status Risks: a root waiting on its children moved
    /// from Done to Working, and the pet's badges follow the group the
    /// projection decided rather than the root's own completion.
    #[test]
    fn a_root_waiting_on_its_children_counts_as_working_not_done() {
        let mut waiting = agent("root", "done");
        waiting.wait = Some(crate::model::AgentWait::Children);
        let summary = summarize(&[waiting, agent("other", "done")], true);
        assert_eq!((summary.working, summary.done), (1, 1));
    }

    #[test]
    fn sleep_sequence_has_four_second_transitions() {
        assert_eq!(sleep_phase_for_idle_ms(59_999), SleepPhase::Awake);
        assert_eq!(sleep_phase_for_idle_ms(60_000), SleepPhase::Yawning);
        assert_eq!(sleep_phase_for_idle_ms(64_000), SleepPhase::Dozing);
        assert_eq!(sleep_phase_for_idle_ms(68_000), SleepPhase::Collapsing);
        assert_eq!(sleep_phase_for_idle_ms(72_000), SleepPhase::Sleeping);
    }

    #[test]
    fn expanded_priority_keeps_urgent_states_above_activity() {
        let mut summary = idle_summary();
        summary.working = 2;
        assert_eq!(expanded_state(summary, 0, false), "juggling");
        summary.working = 1;
        assert_eq!(expanded_state(summary, 0, false), "carrying");
        summary.working = 2;
        summary.needs_you = 1;
        assert_eq!(expanded_state(summary, 90_000, false), "notification");
        summary.error = 1;
        assert_eq!(expanded_state(summary, 90_000, false), "error");
    }

    #[test]
    fn roam_is_cancelled_by_work_or_sleep() {
        assert!(is_roam_allowed(idle_summary(), 8_000, false));
        assert!(!is_roam_allowed(idle_summary(), 7_999, false));
        assert!(!is_roam_allowed(idle_summary(), 60_000, false));
        assert!(!is_roam_allowed(idle_summary(), 8_000, true));
        let mut working = idle_summary();
        working.working = 1;
        assert!(!is_roam_allowed(working, 8_000, false));
    }

    #[test]
    fn pose_walks_idle_through_roam_and_the_full_sleep_sequence() {
        let summary = idle_summary();
        assert_eq!(pose(summary, 0, false, true), "idle");
        assert_eq!(pose(summary, 7_999, false, true), "idle");
        assert_eq!(pose(summary, 8_000, false, true), "roam");
        assert_eq!(pose(summary, 60_000, false, true), "yawning");
        assert_eq!(pose(summary, 64_000, false, true), "dozing");
        assert_eq!(pose(summary, 68_000, false, true), "collapsing");
        assert_eq!(pose(summary, 72_000, false, true), "sleeping");
        assert_eq!(pose(summary, 90_000, true, true), "waking");
    }

    #[test]
    fn a_lost_connection_outranks_every_other_pose() {
        let mut summary = idle_summary();
        summary.needs_you = 3;
        summary.error = 3;
        summary.working = 2;
        assert_eq!(pose(summary, 0, false, true), "error");
        assert_eq!(pose(summary, 0, false, false), "disconnected");
    }

    /// R6, AC9. The pet counts the sidebar's four groups and nothing else, so
    /// its act-now number is the Needs You count and its done number is the
    /// Done count. Before this the pet kept its own five buckets and the yellow
    /// badge held Needs You minus the errors.
    #[test]
    fn pet_groups_are_counted_as_the_sidebar_groups() {
        let agents = [
            agent("a", "error"),
            agent("b", "needs_you"),
            agent("c", "needs_you"),
            agent("d", "needs_you"),
            agent("e", "working"),
            agent("f", "done"),
            agent("g", "seen"),
            agent("h", "seen"),
        ];
        let summary = summarize(&agents, true);
        let groups = |group: AgentGroup| {
            agents
                .iter()
                .filter(|agent| crate::agent_state::group_of(agent) == group)
                .count()
        };
        assert_eq!(
            summary.needs_you,
            groups(AgentGroup::NeedsYou),
            "act now is the Needs You group, errors included"
        );
        assert_eq!(summary.done, groups(AgentGroup::Done));
        assert_eq!(summary.working, groups(AgentGroup::Working));
        assert_eq!(summary.seen, groups(AgentGroup::Seen));
        assert_eq!(
            summary.error, 1,
            "the pose ladder still knows an error is among them"
        );
        assert_eq!(summary.total(), agents.len());
    }

    /// AC9. A server that stopped answering cannot say anything true about what
    /// is waiting, so every count goes to zero and the retained rows are counted
    /// as disconnected instead.
    #[test]
    fn pet_groups_go_quiet_when_the_server_stops_answering() {
        let agents = [agent("a", "needs_you"), agent("b", "working")];
        let summary = summarize(&agents, false);
        assert_eq!(summary.disconnected, 2, "the last valid list is retained");
        assert_eq!(summary.needs_you, 0);
        assert_eq!(summary.done, 0);
        assert_eq!(summary.working, 0);
        assert_eq!(summary.seen, 0);
        assert_eq!(pose(summary, 0, false, false), "disconnected");
    }

    #[test]
    fn subagent_counts_sum_the_hook_tokens_of_listed_agents_and_go_quiet_while_disconnected() {
        let working = |count: Option<u32>| PaneHookTokens {
            version: Some(1),
            working: count,
            done: None,
            blocked: None,
        };
        let tokens = BTreeMap::from([
            ("a".to_owned(), working(Some(2))),
            ("b".to_owned(), working(Some(1))),
            // An instrumented pane whose count is unknown adds nothing.
            ("c".to_owned(), working(None)),
            // A token left on a pane whose agent is gone is not counted.
            ("gone".to_owned(), working(Some(9))),
        ]);
        let agents = [
            agent("a", "working"),
            agent("b", "working"),
            agent("c", "idle"),
        ];

        assert_eq!(subagents_active(&agents, &tokens, true), 3);
        assert_eq!(subagents_active(&agents, &tokens, false), 0);
        assert_eq!(subagents_active(&[agent("d", "seen")], &tokens, true), 0);
    }

    #[test]
    fn the_oldest_observed_unseen_pane_is_selected_before_later_ones() {
        let agents = [
            agent("later", "needs_you"),
            agent("working", "working"),
            agent("earlier", "error"),
        ];
        let observed = BTreeMap::from([("later".to_owned(), 2_000), ("earlier".to_owned(), 1_000)]);
        assert_eq!(attention_order(&agents, &observed), ["earlier", "later"]);
    }

    #[test]
    fn without_observations_snapshot_order_decides_and_seen_panes_never_qualify() {
        let agents = [
            agent("first", "needs_you"),
            agent("second", "error"),
            agent("acknowledged", "seen"),
            agent("done", "done"),
        ];
        assert_eq!(
            attention_order(&agents, &BTreeMap::new()),
            ["first", "second"]
        );
    }

    #[test]
    fn observing_the_same_snapshot_twice_keeps_the_first_moment() {
        let agents = [agent("a", "needs_you"), agent("b", "seen")];
        let mut observed = BTreeMap::new();
        observe_unseen(&mut observed, &agents, 1_000);
        observe_unseen(&mut observed, &agents, 5_000);
        assert_eq!(observed.get("a"), Some(&1_000));
        assert!(!observed.contains_key("b"));

        // Once the user acknowledges it, the pane is forgotten and a later
        // unseen state starts a fresh observation.
        observe_unseen(&mut observed, &[agent("a", "seen")], 6_000);
        assert!(observed.is_empty());
        observe_unseen(&mut observed, &agents, 7_000);
        assert_eq!(observed.get("a"), Some(&7_000));
    }
}
