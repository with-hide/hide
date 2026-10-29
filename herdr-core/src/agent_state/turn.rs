//! What the operator needs to do, derived from the five axes.
use super::axes::*;
use super::tally::RowMark;

/// What one agent is doing as far as removing its checkout is concerned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentUse {
    /// Stopped: finished a turn or idle. Its pane may close.
    Quiet,
    Working,
    /// Waiting on the operator: a question, an approval or a blocked prompt.
    Waiting,
    Unknown,
}

impl AgentUse {
    /// From the status model's axes (`docs/status-model.md`): a demand or a
    /// blocked prompt is waiting whatever else is reported, and an activity
    /// other than `working` or `stopped` is not claimed to be idle.
    pub(crate) fn of(demand: &str, blocked: bool, activity: &str) -> Self {
        if demand != "none" || blocked {
            Self::Waiting
        } else {
            match activity {
                "working" => Self::Working,
                "stopped" => Self::Quiet,
                _ => Self::Unknown,
            }
        }
    }
}

/// Semantic tone; a shell chooses its existing color token and opacity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Tone {
    pub kind: &'static str,
    pub read: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TabState {
    pub mark_tone: Tone,
}

/// A row's second line. `mode` says why it is on screen and how the shell
/// words it: `request` (an ask or a block's cause, until it is resolved),
/// `news` (unread), `waiting` (an in-progress row's sentence, or, with no
/// text, its proven cause the shell words), `kept` (the work left or the
/// unproven wait of a read stop, at rest and subdued), `quiet` (shown only on
/// the selected or hovered row), `vanished` (the awaited work died with its
/// process), and `checks_passed` or `checks_failed` (the checks the row waited
/// on ended). The shell words the empty-text modes in the operator's language.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RowLine {
    pub text: String,
    pub mode: &'static str,
    pub tone: Tone,
    /// The pull request a `checks` wait or a checks end names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u32>,
    /// A request the core asks for the agent, which the shell words in
    /// place of `text` (`AskSubject`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<crate::model::AskSubject>,
}

impl RowLine {
    fn new(text: &str, mode: &'static str, tone: Tone) -> Self {
        Self {
            text: text.to_owned(),
            mode,
            tone,
            pull_request: None,
            subject: None,
        }
    }
}

/// The decisions formerly repeated by each row, search, graph and board.
/// Surface differences are intentional: a waiting root has a working mark
/// but its chip still follows its own axes, and every demand remains visible.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RowState {
    pub session: super::sessions::Row,
    pub attention: bool,
    pub needs_you: bool,
    pub root: bool,
    pub title_emphasized: bool,
    pub selection_emphasizes_title: bool,
    pub asking: bool,
    pub working: bool,
    pub waits_on_children: bool,
    pub chip_tone: Tone,
    pub mark_tone: Tone,
    pub line: Option<RowLine>,
    pub branch_badge: Option<String>,
    pub bucket: &'static str,
    pub attention_rank: u8,
    pub search_tone: &'static str,
    pub subtree: &'static str,
    pub link: &'static str,
    pub link_rank: u8,
    pub verb: RequestVerb,
    pub request_todo: bool,
    pub descendant_asking: u32,
    pub request_since: Option<u64>,
    /// What a Needs You row asks the operator (PRD B2): its own demand, else
    /// its lead raised descendant; absent outside Needs You.
    pub ask: Option<Ask>,
    /// Where the row stands among its siblings in a tree (PRD B11): asking or
    /// raised (itself or below it), working, finished unread, then the rest.
    pub tree_rank: u8,
    /// The row's own PRs (PRD D-18, D-39).
    pub pr: Option<super::sessions::PrSummary>,
}

/// A Needs You row's second line: a verb and what to do, nothing else (PRD
/// D-37). `what` is absent when no label line or letter says it; the shell
/// then words the verb's own fallback. `more` counts the other asks under
/// the same root, which only its pane band names.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Ask {
    pub verb: super::escalation::Verb,
    pub what: Option<String>,
    pub more: usize,
    /// A question the core asks for the agent, which the shell words in
    /// place of `what`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<crate::model::AskSubject>,
}

/// The verb of the row's own demand. A native unanswered question also holds
/// the pane blocked until a reply; only that proven native wait overrides
/// the approval of a blocked menu. An AI question the operator has read is
/// no longer a demand (docs/status-model.md, The Sessions tool); a native one
/// holds until it is answered.
pub(crate) fn demand_verb(agent: &SidebarAgentSnapshot) -> Option<super::escalation::Verb> {
    use super::escalation::Verb;
    let native_question = agent.demand == "question"
        && agent
            .user_turn
            .as_ref()
            .is_some_and(|turn| turn.kind == hide_session::turns::UserTurnKind::Question);
    if (agent.blocked || agent.demand == "approval") && !native_question {
        Some(Verb::Approval)
    } else if (agent.demand == "error" && agent.unread)
        || (agent.demand == "question" && (native_question || agent.unread))
    {
        Some(Verb::Answer)
    } else {
        None
    }
}

/// When the row's own ask began: the core's question laid on it, else the
/// request the label read.
fn own_ask_since(agent: &SidebarAgentSnapshot) -> Option<u64> {
    agent.laid_since_unix_ms.or_else(|| {
        agent
            .request
            .as_ref()
            .map(|request| request.verb_since_unix_ms)
    })
}

/// Whether a root's lead raised descendant waited longer than the row's own
/// ask, so the row leads with it (needs-you-cleanup D-13). Without its own
/// ask the raise leads; an undated one never outranks a dated own ask.
fn raised_leads(agent: &SidebarAgentSnapshot) -> bool {
    let Some(lead) = agent.raised.first() else {
        return false;
    };
    if demand_verb(agent).is_none() {
        return true;
    }
    match (lead.since_unix_ms, own_ask_since(agent)) {
        (Some(raised), Some(own)) => raised < own,
        _ => false,
    }
}

fn ask_of(agent: &SidebarAgentSnapshot, needs_you: bool) -> Option<Ask> {
    // A block is not asked of the operator in a verb: its row draws the
    // cause instead (agent-blocked-state B1).
    if !needs_you || (agent.demand == "error" && agent.unread) {
        return None;
    }
    let own = demand_verb(agent);
    // Everything else that waits on the operator under this root is counted
    // behind the lead, `외 N건` (D-13).
    let total = agent.raised.len() + usize::from(own.is_some()) + agent.laid_more;
    if let Some(verb) = own.filter(|_| !raised_leads(agent)) {
        return Some(Ask {
            verb,
            what: agent.detail.clone(),
            more: total - 1,
            subject: agent.ask_subject.clone(),
        });
    }
    agent.raised.first().map(|lead| Ask {
        verb: lead.verb,
        what: lead.what.clone(),
        more: total - 1,
        subject: lead.subject.clone(),
    })
}

pub(crate) fn row_state(agent: &SidebarAgentSnapshot) -> RowState {
    let demand = agent.demand.as_str();
    let activity = agent.activity.as_str();
    let group = agent.group.as_str();
    let asking = matches!(demand, "error" | "question" | "approval");
    let needs_you = group == "needs_you";
    let attention = needs_you || agent.unread;
    // A raised root waits on its children too, but the raise is the news.
    let waiting = agent.wait.is_some() && agent.raised.is_empty();
    // The mark's colour is the mark's own; a chip, a search hit and a
    // request line take the same one. A delegated child's request draws no
    // mark of the operator's (B24) but its line keeps the warning colour,
    // dimmed like a read one.
    let chip_kind = match agent.mark {
        RowMark::Turn | RowMark::Look => "warning",
        RowMark::Going | RowMark::Waiting => "working",
        RowMark::Done => "success",
        RowMark::Unread => "news",
        RowMark::Quiet if asking => "warning",
        RowMark::Quiet | RowMark::Unknown | RowMark::Asleep => "subtle",
    };
    let bright = needs_you || agent.escalation.is_some();
    let chip_tone = Tone {
        kind: chip_kind,
        read: asking && !bright && !agent.emphasized,
    };
    let mark_tone = Tone {
        kind: if matches!(
            agent.mark,
            RowMark::Quiet | RowMark::Unknown | RowMark::Asleep
        ) {
            "subtle"
        } else {
            chip_kind
        },
        read: agent.mark == RowMark::Turn && chip_tone.read,
    };
    let subtle = Tone {
        kind: "subtle",
        read: false,
    };
    let news = Tone {
        kind: "news",
        read: false,
    };
    let mut line = agent
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| {
            if asking {
                RowLine::new(text, "request", chip_tone)
            } else if waiting {
                // What a waiting row waits for stays on its second line while
                // it waits, read or not (B12).
                RowLine::new(text, "waiting", subtle)
            } else if agent.unread {
                RowLine::new(text, "news", news)
            } else if stopped_unfinished(agent) {
                // Work left, or a wait nothing proves, stays said once read (B7).
                RowLine::new(text, "kept", subtle)
            } else {
                RowLine::new(text, "quiet", subtle)
            }
        });
    if let Some(subject) = agent.ask_subject.clone().filter(|_| asking) {
        // The core's own question: the shell words it (hide-plan B14).
        line = Some(RowLine {
            subject: Some(subject),
            ..RowLine::new("", "request", chip_tone)
        });
    }
    if line.is_none() && waiting {
        // No sentence: the shell words the proven cause (B10).
        line = Some(RowLine::new("", "waiting", subtle));
    }
    if let Some(line) = line.as_mut().filter(|line| line.mode == "waiting") {
        line.pull_request = agent
            .checks_wait
            .filter(|_| agent.wait == Some(crate::model::AgentWait::Checks));
    }
    if super::axes::wake_vanished(agent) {
        // The sentence is the web's, in the operator's language: the core
        // says only that the work it waited for is gone.
        line = Some(RowLine::new("", "vanished", subtle));
    }
    if let Some(ended) = agent
        .checks_ended
        .filter(|_| agent.unread && !asking && !waiting)
    {
        // The checks it waited on ended: that is the news (D-08, B13).
        line = Some(RowLine {
            pull_request: Some(ended.number),
            ..if ended.passed {
                RowLine::new("", "checks_passed", news)
            } else {
                RowLine::new(
                    "",
                    "checks_failed",
                    Tone {
                        kind: "warning",
                        read: false,
                    },
                )
            }
        });
    }
    let bucket = if needs_you || group == "done" {
        "turn"
    } else if waiting {
        "delegating"
    } else if group == "working" {
        "working"
    } else {
        "resting"
    };
    let counts = agent.descendant_counts;
    let waits_on_children =
        waiting || counts.working + counts.question + counts.approval + counts.error > 0;
    let working = group == "working" || waiting;
    let verb = agent
        .request
        .as_ref()
        .map(|request| request.verb)
        .unwrap_or(if group == "working" {
            RequestVerb::Working
        } else {
            RequestVerb::Idle
        });
    let request_todo = matches!(
        verb,
        RequestVerb::Answer
            | RequestVerb::Blocked
            | RequestVerb::Fix
            | RequestVerb::Review
            | RequestVerb::Stopped
            | RequestVerb::Result
    );
    RowState {
        session: super::sessions::row(agent),
        attention,
        needs_you,
        root: !agent.delegated,
        title_emphasized: !(agent.delegated && !attention) && (attention || agent.emphasized),
        selection_emphasizes_title: !agent.delegated || attention,
        asking,
        working,
        waits_on_children,
        chip_tone,
        mark_tone,
        line,
        branch_badge: agent
            .lineage_worktree_badge
            .as_deref()
            .filter(|_| agent.delegated)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        bucket,
        attention_rank: match group {
            "needs_you" if demand == "error" => 0,
            "needs_you" => 1,
            "done" => 2,
            "working" => 3,
            _ => 4,
        },
        search_tone: match chip_kind {
            "warning" => "attention",
            "working" => "working",
            "success" => "done",
            _ => "muted",
        },
        subtree: if agent.requires_close_status_check {
            "unknown"
        } else if demand != "none" && !demand.is_empty() {
            "waiting"
        } else if activity == "working" {
            "working"
        } else if matches!(RowMark::awake(agent), RowMark::Done | RowMark::Unread) {
            "unread"
        } else {
            "quiet"
        },
        link: if asking {
            "question"
        } else if working {
            "working"
        } else {
            "idle"
        },
        link_rank: if asking {
            0
        } else if working {
            1
        } else {
            2
        },
        verb,
        request_todo,
        descendant_asking: agent.descendant_counts.question + agent.descendant_counts.approval,
        request_since: if needs_you
            && raised_leads(agent)
            && let Some(lead) = agent.raised.first()
        {
            lead.since_unix_ms
        } else if needs_you && agent.laid_since_unix_ms.is_some() {
            agent.laid_since_unix_ms
        } else if request_todo {
            agent
                .request
                .as_ref()
                .map(|request| request.verb_since_unix_ms)
        } else {
            agent.changed_at_unix_ms
        },
        ask: ask_of(agent, needs_you),
        tree_rank: if asking
            || agent.escalation.is_some()
            || !agent.raised.is_empty()
            || agent
                .descendant_mark
                .is_some_and(|mark| mark.kind == super::escalation::MarkKind::Raised)
        {
            0
        } else if activity == "working" || waiting {
            1
        } else if matches!(RowMark::awake(agent), RowMark::Done | RowMark::Unread) {
            2
        } else {
            3
        },
        pr: super::sessions::own_prs(agent),
    }
}

/// Runtime gates deliberately distinguish a running process from a root
/// waiting on work it delegated. Both prevent sleep and reopening.
pub(crate) fn is_running(agent: &SidebarAgentSnapshot) -> bool {
    agent.activity == "working"
}

pub(crate) fn has_work_in_progress(agent: &SidebarAgentSnapshot) -> bool {
    is_running(agent) || agent.group == "working"
}

pub(crate) fn is_waiting_for_operator(agent: &SidebarAgentSnapshot) -> bool {
    agent.demand != "none" || agent.blocked
}

/// A session plan approval Enter starts the person's next request, unlike
/// an Enter at Herdr's ordinary blocked prompt. Preserve the observed status
/// when present and the existing row fallback when it is not.
pub(crate) fn submit_answers_prompt(
    agent: &SidebarAgentSnapshot,
    observed_status: Option<&str>,
) -> bool {
    match observed_status {
        Some(status) => status == "blocked",
        None => agent.blocked && agent.activity != "stopped",
    }
}

pub(crate) fn is_seen(agent: &SidebarAgentSnapshot) -> bool {
    agent.group == "seen"
}

pub(crate) fn rest_refusal(
    agent: &SidebarAgentSnapshot,
) -> Option<crate::model::AgentActionRefusal> {
    use crate::model::AgentActionRefusal;
    if has_work_in_progress(agent) {
        Some(AgentActionRefusal::Working)
    } else if is_waiting_for_operator(agent) {
        Some(AgentActionRefusal::WaitingForYou)
    } else if agent.activity != "stopped" {
        Some(AgentActionRefusal::UnknownActivity)
    } else {
        None
    }
}
use crate::model::{AgentStatusCode, PullRequestChecks, SidebarAgentSnapshot};
use crate::request_view::AgentPullRequestSnapshot;
use serde::{Deserialize, Serialize};

/// The four groups the sidebar reads top to bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentGroup {
    NeedsYou,
    Done,
    Working,
    Seen,
}

impl AgentGroup {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs_you",
            Self::Done => "done",
            Self::Working => "working",
            Self::Seen => "seen",
        }
    }

    /// Where the group sits in the sidebar, read top to bottom: what is
    /// waiting on the operator, what finished while they were away, what is
    /// still running, then everything already dealt with.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::NeedsYou => 0,
            Self::Done => 1,
            Self::Working => 2,
            Self::Seen => 3,
        }
    }
}

/// The group a row belongs to, given who owns it.
///
/// `waiting` is the answer for a row that is quiet itself while something it
/// started is still going: a working descendant, a background job, a
/// request whose reply is still being made. Such a row has not finished, so
/// it sits in Working, and it reaches Done only once nothing is left to wait on
/// (sidebar-agent-status D-01). Its own unread demand or a blocked prompt
/// still wins, because the flag is only ever set on a row with no demand of
/// its own.
#[allow(clippy::too_many_arguments)] // one row's axes, each its own input to the group
pub fn agent_group_for(
    demand: AgentDemand,
    activity: AgentActivity,
    completed: bool,
    unread: bool,
    blocked: bool,
    ownership: Ownership,
    waiting: bool,
    look: bool,
) -> AgentGroup {
    if ownership == Ownership::Delegated {
        // The row keeps its own mark and status word; only its claim on the
        // operator's attention is withheld.
        return if activity == AgentActivity::Working || waiting {
            AgentGroup::Working
        } else {
            AgentGroup::Seen
        };
    }
    if let Ownership::HideAi { operator_turn } = ownership {
        // Its home shows the work's progress and result; the operator is
        // called out only for what the operator has to do: the agent's own
        // question or block, read like any row's, or the home's turn.
        return if operator_turn || blocked || (demand != AgentDemand::None && unread) {
            AgentGroup::NeedsYou
        } else {
            AgentGroup::Seen
        };
    }
    // An agent's open the operator has not looked at is theirs to look at
    // (PRD unseen-agent-views D-05, D-08), whatever the agent does meanwhile.
    if blocked || (demand != AgentDemand::None && unread) || look {
        AgentGroup::NeedsYou
    } else if waiting {
        AgentGroup::Working
    } else if demand == AgentDemand::None
        && activity == AgentActivity::Stopped
        && completed
        && unread
    {
        AgentGroup::Done
    } else if activity == AgentActivity::Working {
        AgentGroup::Working
    } else {
        AgentGroup::Seen
    }
}

/// An intentionally closed execution has no live parent or unread demand.
/// Only an explicit sleep-id wake action can create its next execution.
pub(crate) fn dormant_group() -> AgentGroup {
    agent_group_for(
        AgentDemand::None,
        AgentActivity::Stopped,
        false,
        false,
        false,
        Ownership::Operator,
        false,
        false,
    )
}

/// The one short word a row shows. A reported completion the operator has not
/// read is `Done` when the label read the turn as finished and `Stopped` when
/// nothing judged it, so no word claims a finish nobody confirmed (D-03,
/// D-13); an ordinary stopped pane and a read completion are `Idle`. No view
/// ever shows an axis value, so nothing underscored can reach the screen. A
/// waiting row (`Waiting`) and one the label read as not finished
/// (`Unfinished`) are decided by the caller, which knows the rest of the row.
pub(crate) fn agent_status_code(
    demand: AgentDemand,
    activity: AgentActivity,
    start: AgentStart,
    completed: bool,
    unread: bool,
    finished: bool,
) -> AgentStatusCode {
    match demand {
        AgentDemand::Error => AgentStatusCode::Error,
        AgentDemand::Question => AgentStatusCode::Question,
        AgentDemand::Approval => AgentStatusCode::Approval,
        AgentDemand::None => match activity {
            AgentActivity::Working => AgentStatusCode::Working,
            AgentActivity::Stopped if completed && unread && finished => AgentStatusCode::Done,
            AgentActivity::Stopped if completed && unread => AgentStatusCode::Stopped,
            AgentActivity::Stopped => AgentStatusCode::Idle,
            AgentActivity::Unknown if start != AgentStart::Settled => AgentStatusCode::Starting,
            AgentActivity::Unknown => AgentStatusCode::Unknown,
        },
    }
}

/// Closing this pane would interrupt running work or discard an unresolved
/// demand. Read state remains a presentation axis, so an unread completion or
/// an ordinary stopped pane cannot create a close prompt by itself. An agent
/// still starting with the operator's first prompt would drop that prompt.
pub(crate) fn agent_requires_close_confirmation(
    activity: AgentActivity,
    demand: AgentDemand,
    blocked: bool,
    start: AgentStart,
) -> bool {
    activity == AgentActivity::Working
        || demand != AgentDemand::None
        || blocked
        || start == AgentStart::Starting { first_prompt: true }
}

/// An activity-less pane cannot safely be described as idle in a destructive
/// confirmation. The caller must obtain a fresh status before it can close.
/// An agent still starting has reported nothing yet; what it may lose is
/// decided by its start alone (`agent_requires_close_confirmation`).
pub(crate) fn agent_requires_close_status_check(
    activity: AgentActivity,
    demand: AgentDemand,
    blocked: bool,
    start: AgentStart,
) -> bool {
    activity == AgentActivity::Unknown
        && demand == AgentDemand::None
        && !blocked
        && start == AgentStart::Settled
}

/// What a row's second line says, decided by the row's group (PRD D-06).
///
/// Rows that still concern the operator - the Needs You group and an unread
/// Done - keep their status word and add what the operator is being asked
/// for (`expected_reply`), or what happened (`progress`) when nothing is
/// asked. A working row shows only its progress: the mark already says it is
/// working. A row the operator has read, and a row whose activity is unknown,
/// say nothing more than their name, unless it still holds a request. With no sentence at all, a row that
/// would have shown one keeps the status word alone, so a pane with no label
/// plugin behind it still reads as it did before (PRD B6, B13).
///
/// A native question waiting now (`question`, see [`native_question_line`])
/// is what is being asked, in the agent's own words: it stands in for an
/// `expected_reply` the label has not made for this turn, and before a
/// `progress` that describes the work around the question rather than the
/// question itself. An `expected_reply` still wins, as the shorter "what to
/// do"; the worker clears it whenever the agent runs again, so one present
/// beside a waiting question was made for this turn.
///
/// A read stop the label found unfinished, or waiting on what nothing
/// proves (`kept`), keeps its progress in Seen: the work is still left (B7).
///
/// Returns whether the status word is drawn and the sentence beside it.
pub(crate) fn agent_second_line(
    group: AgentGroup,
    demand: AgentDemand,
    kept: bool,
    expected_reply: Option<&str>,
    question: Option<&str>,
    progress: Option<&str>,
) -> (bool, Option<String>) {
    let expected_reply = expected_reply.or(question);
    // An unresolved demand keeps its request whatever group the row sits in:
    // a read question in Seen and a delegated child's approval still say what
    // they are asking, so a view can keep that line until the request is
    // answered rather than until it is looked at (sidebar-agent-status B7).
    // Only a group that wanted a sentence falls back to the status word.
    let request = (demand != AgentDemand::None)
        .then(|| expected_reply.or(progress))
        .flatten();
    let sentence = match group {
        AgentGroup::NeedsYou | AgentGroup::Done => expected_reply.or(progress),
        AgentGroup::Working => request.or(progress),
        AgentGroup::Seen => {
            return (
                false,
                request.or(progress.filter(|_| kept)).map(str::to_owned),
            );
        }
    };
    // The word is the sentence's stand-in, never its prefix: the mark and the
    // group heading already say Question or Done, and the word beside a
    // sentence took the width the sentence needed (2026-09-18).
    match sentence {
        Some(sentence) => (false, Some(sentence.to_owned())),
        None => (true, None),
    }
}

/// The second line of a native question waiting now: the first line of
/// its text, one line and cut on a character boundary to the width of an
/// `expected_reply`, ending in `…` when cut. A plan waiting for approval has
/// none: its text is a document whose first line is whatever heading the
/// agent chose, not what is asked, so its row keeps the status word unless
/// the label says more. Content the read did not carry stays absent.
pub(crate) fn native_question_line(
    turn: Option<&hide_session::turns::UserTurnFact>,
) -> Option<String> {
    let turn = turn.filter(|turn| turn.kind == hide_session::turns::UserTurnKind::Question)?;
    first_line_cut(&turn.content.as_ref()?.text())
}

/// The first line of `text`, one line and cut on a character boundary to
/// the width of an `expected_reply`, ending in `…` when cut.
fn first_line_cut(text: &str) -> Option<String> {
    use crate::sidebar::MAX_EXPECTED_REPLY_CHARS;
    let line = text
        .lines()
        .find_map(|line| crate::display_text::one_line(line, MAX_EXPECTED_REPLY_CHARS + 1))?;
    if line.chars().count() <= MAX_EXPECTED_REPLY_CHARS {
        return Some(line);
    }
    let mut cut: String = line.chars().take(MAX_EXPECTED_REPLY_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    Some(cut)
}

/// The one ordering every agent surface reads: the two sidebar views, the pet
/// dashboard, and the agent switcher's candidate list.
///
/// Group order first, then most recent activity descending, then the order
/// Herdr sent the rows in. The sort is stable, so that third key costs
/// nothing, and re-running it on an already ordered list is the identity
/// (engineering rule 11).
///
/// It runs after the read axis, never inside `project_agents`: a row's group
/// depends on whether the operator has read it, and a projection that has not
/// met the read record ledger treats every completion as unread. Ordinary idle
/// rows still stay Seen. The order Hide shows is Hide's own.
pub(crate) fn sort_agents(agents: &mut [SidebarAgentSnapshot]) {
    agents.sort_by(|left, right| {
        group_of(left)
            .rank()
            .cmp(&group_of(right).rank())
            .then_with(|| right.last_activity.cmp(&left.last_activity))
    });
}

pub fn group_of(agent: &SidebarAgentSnapshot) -> AgentGroup {
    let (demand, activity, unread) = axes_of(agent);
    agent_group_for(
        demand,
        activity,
        agent.completed,
        unread,
        agent.blocked,
        ownership_of(agent),
        agent.wait.is_some(),
        !agent.unseen_views.is_empty(),
    )
}

/// Fills in every value the shell draws from the lifecycle axes.
///
/// Called again whenever the read axis moves, so the derived values can never
/// describe a different read state than the row they sit on.
pub(crate) fn derive_from_axes(agent: &mut SidebarAgentSnapshot) {
    let (demand, activity, unread) = axes_of(agent);
    let waiting = agent.wait.is_some();
    let group = agent_group_for(
        demand,
        activity,
        agent.completed,
        unread,
        agent.blocked,
        ownership_of(agent),
        waiting,
        !agent.unseen_views.is_empty(),
    );
    let group = if !agent.raised.is_empty() {
        AgentGroup::NeedsYou
    } else {
        group
    };
    agent.group = group.name().to_owned();
    agent.look = look_of(agent, |agent| {
        // Where the row would stand with nothing to look at: what a phone or
        // desktop notification follows, which a View never sends (PRD
        // unseen-agent-views D-13, B12).
        if !agent.raised.is_empty() {
            return AgentGroup::NeedsYou;
        }
        agent_group_for(
            demand,
            activity,
            agent.completed,
            unread,
            agent.blocked,
            ownership_of(agent),
            waiting,
            false,
        )
    });
    agent.mark = RowMark::of(agent);
    // A row the operator still has to deal with is drawn bright; everything
    // already read or merely running is subdued.
    agent.emphasized = matches!(group, AgentGroup::NeedsYou | AgentGroup::Done);
    agent.status_code = if waiting && agent.raised.is_empty() {
        AgentStatusCode::Waiting
    } else {
        match stop_verdict(agent) {
            // The work it waited for died with its process: the row keeps
            // today's word for that, Stopped (status-marks B6).
            Some(StopVerdict::Unfinished) if super::axes::wake_vanished(agent) => {
                AgentStatusCode::Stopped
            }
            Some(StopVerdict::Unfinished) => AgentStatusCode::Unfinished,
            verdict => agent_status_code(
                demand,
                activity,
                agent.start,
                agent.completed,
                unread,
                verdict == Some(StopVerdict::Finished),
            ),
        }
    };
    let question = native_question_line(agent.user_turn.as_ref());
    // A question the core asks for the agent is said by its subject, never
    // by the label's sentence about the work around it.
    let (status_word_visible, detail) = if let Some(subject) = &agent.ask_subject {
        // A mission decision's line is also its sentence, which a surface
        // that does not know the subject draws instead (D-12).
        let line = match subject {
            crate::model::AskSubject::MissionDecision { line } => first_line_cut(line),
            crate::model::AskSubject::PlanReview { .. } => None,
        };
        (true, line)
    } else {
        agent_second_line(
            group,
            demand,
            stopped_unfinished(agent),
            agent.expected_reply.as_deref(),
            question.as_deref(),
            agent.progress.as_deref(),
        )
    };
    agent.status_word_visible = status_word_visible;
    agent.detail = detail;
    // A finished turn's leftover tasks are said on its line (status-marks D-19).
    agent.background_left = if finished_verdict(agent) {
        super::axes::background_left(agent)
    } else {
        0
    };
    agent.requires_close_confirmation =
        agent_requires_close_confirmation(activity, demand, agent.blocked, agent.start);
    agent.requires_close_status_check =
        agent_requires_close_status_check(activity, demand, agent.blocked, agent.start);
    // A sleeping agent keeps its group, order and read axis; only its mark
    // and word say that its process is gone until it is opened (PRD B10).
    if let Some(sleep) = &agent.sleep {
        agent.status_code = crate::agent_sleep::status_code(sleep);
    }
    agent.state = row_state(agent);
}

/// What an operator's own row says about the Views it opened that they have
/// not seen (PRD unseen-agent-views D-05, D-14). A delegated row's go up as
/// its ancestors' signals instead (D-10), and a home's row has none (D-18).
fn look_of(
    agent: &SidebarAgentSnapshot,
    otherwise: impl FnOnce(&SidebarAgentSnapshot) -> AgentGroup,
) -> Option<crate::model::LookSnapshot> {
    if ownership_of(agent) != Ownership::Operator {
        return None;
    }
    let newest = agent.unseen_views.first()?;
    Some(crate::model::LookSnapshot {
        group: otherwise(agent).name().to_owned(),
        count: u32::try_from(agent.unseen_views.len()).unwrap_or(u32::MAX),
        view_id: newest.view_id.clone(),
        label: newest.label.clone(),
        workspace_id: newest.workspace_id.clone(),
        checkout_id: newest.checkout_id.clone(),
    })
}

/// Re-derives every drawn value of a row whose sleep mark was just set.
pub fn rederive(agent: &mut SidebarAgentSnapshot) {
    derive_from_axes(agent);
}

/// What a row asks of the operator now, in the order the view draws its
/// groups (D-06).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestVerb {
    /// A question or an approval waits on the operator.
    Answer,
    /// The agent stopped and named what stopped it; clearing it is the
    /// operator's. It follows `Answer` (D-07).
    Blocked,
    /// A pull request's checks failed.
    Fix,
    /// A pull request is ready to review or merge.
    Review,
    /// The agent stopped before its work was done and nothing is left to
    /// wake it: an AI `unfinished`, a `waiting` the session cannot prove, or
    /// a block the operator has already read.
    Stopped,
    /// A finished turn, or a pull request settled since the last request,
    /// the operator has not looked at.
    Result,
    Working,
    /// Its pull request's checks, its descendants, or something it named.
    Waiting,
    #[default]
    Idle,
}

pub(crate) fn verb_of(
    row: &SidebarAgentSnapshot,
    pull_requests: &[AgentPullRequestSnapshot],
    result_opened: Option<u64>,
) -> RequestVerb {
    if row.blocked || (row.demand == "question" && row.unread) {
        return RequestVerb::Answer;
    }
    if row.demand == "error" && row.unread {
        return RequestVerb::Blocked;
    }
    if row.activity == "working" {
        return RequestVerb::Working;
    }
    let duty = || {
        pull_requests
            .iter()
            .filter(|pull_request| pull_request.live && pull_request.duty)
    };
    let open = |checks: &[PullRequestChecks]| {
        duty().any(|pull_request| {
            !pull_request.badge.is_settled() && checks.contains(&pull_request.checks)
        })
    };
    if open(&[PullRequestChecks::Failed]) {
        return RequestVerb::Fix;
    }
    if open(&[
        PullRequestChecks::Passing,
        PullRequestChecks::None,
        PullRequestChecks::Unknown,
    ]) {
        return RequestVerb::Review;
    }
    // Only the label analysis reads a turn as not finished (D-33); a row
    // without one never stops here. A block the operator has read stops here
    // too, keeping its cause.
    if stopped_unfinished(row) || row.demand == "error" {
        return RequestVerb::Stopped;
    }
    if open(&[PullRequestChecks::Pending]) {
        return RequestVerb::Waiting;
    }
    // A settled pull request is a result until the operator opens it.
    let unseen = |pull_request: &AgentPullRequestSnapshot| match result_opened {
        None => true,
        Some(opened) => pull_request
            .settled_at_unix_ms
            .is_some_and(|settled| settled > opened),
    };
    if (row.completed && row.unread)
        || duty().any(|pull_request| pull_request.badge.is_settled() && unseen(pull_request))
    {
        return RequestVerb::Result;
    }
    if row.wait.is_some() {
        return RequestVerb::Waiting;
    }
    RequestVerb::Idle
}

/// The notification state machine. Transport and subscription policy stay
/// in hided; the status transition and descendant raise live with agent state.
pub mod push {
    use super::super::tally::phone::{AgentKey, PhoneAgent, Projection};
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;
    /// What a root agent's notification says, or would say.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Effective {
        NeedsYou,
        Done,
        Working,
        Seen,
        Other,
    }

    impl Effective {
        /// The state a notification announces for this one, if any.
        fn announced(self) -> Option<NoticeState> {
            match self {
                Self::NeedsYou => Some(NoticeState::NeedsYou),
                Self::Done => Some(NoticeState::Done),
                Self::Working | Self::Seen | Self::Other => None,
            }
        }

        fn from_group(group: &str) -> Self {
            match group {
                "needs_you" => Self::NeedsYou,
                "done" => Self::Done,
                "working" => Self::Working,
                // 다시 볼 것 is read: it clears a phone notice as 확인함 does.
                "seen" | "revisit" => Self::Seen,
                _ => Self::Other,
            }
        }
    }

    /// The states a notification announces; the wire value is also the key of
    /// the phone's translated word for it. The last two are the human delivery
    /// causes (`herdr_core::delivery::worker::HumanNoticeKind`).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum NoticeState {
        NeedsYou,
        Done,
        ObserverUnconfirmed,
        LetterUndelivered,
    }

    impl NoticeState {
        pub fn as_str(self) -> &'static str {
            match self {
                Self::NeedsYou => "needs_you",
                Self::Done => "done",
                Self::ObserverUnconfirmed => "observer_unconfirmed",
                Self::LetterUndelivered => "letter_undelivered",
            }
        }
    }

    /// One notification to send to every subscribed phone. It carries data only:
    /// the title and the place are the operator's own words, the state is a key
    /// the phone words in its own language.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Notice {
        pub key: AgentKey,
        pub title: String,
        pub state: NoticeState,
        /// The row's own status code and already-chosen second-line sentence.
        /// Delivery notices carry neither; no transport generates a sentence.
        pub status_code: Option<String>,
        pub detail: Option<String>,
        /// What the core asks on the row for the agent, which the receiver
        /// words in its own language.
        pub subject: Option<crate::model::AskSubject>,
        /// The project (`project`, or the part of `project · branch` before the
        /// branch separator); empty when the agent has no place.
        pub place: String,
    }

    /// Root rows use their core group, which a raised descendant already moved
    /// to Needs You; a delegated row never notifies on its own (PRD B9).
    /// Existing human delivery notices own undelivered-letter and
    /// unconfirmed-watch notifications.
    fn effective(projection: &Projection) -> BTreeMap<AgentKey, (Effective, &PhoneAgent)> {
        let mut roots = BTreeMap::new();
        for agent in projection.agents() {
            let state = if agent.root_pane_id != agent.pane_id {
                // Its root carries any raise; clear a delegated row's previous
                // state without a disappearance grace.
                Effective::Seen
            } else if agent.human_notice {
                Effective::Other
            } else {
                Effective::from_group(&agent.group)
            };
            roots.insert(agent.key(), (state, agent));
        }
        roots
    }

    /// Follows root agents across projections and says which entered Needs You
    /// or Done, and which the desktop made Seen. The first projection it sees
    /// only seeds it: nothing that was already waiting is announced.
    #[derive(Default)]
    pub struct Transitions {
        last: Option<BTreeMap<AgentKey, Effective>>,
        /// Agents that left the list, with their state and when: a device that
        /// reconnects, or a list that was briefly empty, brings them back in the
        /// state they had, which is no transition (and no second notice).
        vanished: BTreeMap<AgentKey, (Effective, std::time::Instant)>,
    }

    /// How long a vanished agent's state is kept for its return.
    pub const VANISHED_TTL: Duration = Duration::from_secs(10 * 60);

    impl Transitions {
        pub fn reset(&mut self) {
            self.last = None;
            self.vanished.clear();
        }

        /// Whether a projection has seeded it since it was made or reset.
        pub fn seeded(&self) -> bool {
            self.last.is_some()
        }

        pub fn observe(&mut self, projection: &Projection) -> (Vec<Notice>, BTreeSet<AgentKey>) {
            self.observe_at(projection, std::time::Instant::now())
        }

        pub fn observe_at(
            &mut self,
            projection: &Projection,
            at: std::time::Instant,
        ) -> (Vec<Notice>, BTreeSet<AgentKey>) {
            let now = effective(projection);
            let states: BTreeMap<AgentKey, Effective> = now
                .iter()
                .map(|(key, (state, ..))| (key.clone(), *state))
                .collect();
            let Some(last) = self.last.replace(states) else {
                return (Vec::new(), BTreeSet::new());
            };
            let mut notices = Vec::new();
            let mut seen = BTreeSet::new();
            for (key, (state, agent)) in &now {
                let before = last
                    .get(key)
                    .copied()
                    .or_else(|| self.vanished.remove(key).map(|(state, _)| state))
                    .unwrap_or(Effective::Other);
                if before == *state {
                    continue;
                }
                if let Some(announced) = state.announced() {
                    notices.push(Notice {
                        key: key.clone(),
                        title: agent.title.clone(),
                        state: announced,
                        status_code: Some(agent.status_code.clone()),
                        detail: agent.detail.clone(),
                        subject: agent.subject.clone(),
                        place: agent
                            .place
                            .as_deref()
                            .map(|place| place.split(" · ").next().unwrap_or(place).to_owned())
                            .unwrap_or_default(),
                    });
                } else if *state == Effective::Seen
                    && matches!(before, Effective::NeedsYou | Effective::Done)
                {
                    seen.insert(key.clone());
                }
            }
            for (key, before) in &last {
                if !now.contains_key(key) {
                    self.vanished.insert(key.clone(), (*before, at));
                }
            }
            // An agent gone for good: its notification is closed on the next push.
            self.vanished.retain(|key, (before, since)| {
                let expired = at.duration_since(*since) >= VANISHED_TTL;
                if expired && matches!(before, Effective::NeedsYou | Effective::Done) {
                    seen.insert(key.clone());
                }
                !expired
            });
            (notices, seen)
        }
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;
    use crate::sidebar::{SessionSnapshotPayload, project_agents};
    use serde_json::json;

    fn row() -> SidebarAgentSnapshot {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"pane_id": "root", "agent": "claude", "agent_status": "idle", "state_change_seq": 1}]
        })).unwrap();
        project_agents(payload).agents.remove(0)
    }

    // A root whose own AI question was read, with a raised descendant, asks
    // the descendant's ask, not its read question; a native question holds
    // until answered.
    #[test]
    fn a_read_ai_question_is_no_demand_while_a_native_one_still_is() {
        use super::super::escalation::Verb;
        let mut agent = row();
        agent.demand = "question".into();
        agent.unread = true;
        assert_eq!(demand_verb(&agent), Some(Verb::Answer));
        agent.unread = false;
        assert_eq!(demand_verb(&agent), None);
        agent.user_turn = Some(hide_session::turns::UserTurnFact {
            kind: hide_session::turns::UserTurnKind::Question,
            content: None,
        });
        assert_eq!(demand_verb(&agent), Some(Verb::Answer));
    }

    // agent-blocked-state B1 with agent-hierarchy-screens B14: a block in
    // Needs You is asked in no verb; its row and Sessions line carry the cause.
    #[test]
    fn a_block_in_needs_you_has_no_ask_and_its_cause_is_the_line() {
        let mut agent = row();
        agent.demand = "error".into();
        agent.group = "needs_you".into();
        agent.unread = true;
        agent.detail = Some("디스크가 가득 차 멈췄어요".into());
        let state = row_state(&agent);
        assert_eq!(state.ask, None);
        assert_eq!(
            state.line.map(|line| (line.text, line.mode)),
            Some(("디스크가 가득 차 멈췄어요".to_owned(), "request"))
        );
        assert_eq!(
            super::super::sessions::row(&agent).line.as_deref(),
            Some("디스크가 가득 차 멈췄어요")
        );
    }

    #[test]
    fn read_question_keeps_its_request_line_and_hue_without_operator_attention() {
        let mut agent = row();
        agent.demand = "question".into();
        agent.activity = "stopped".into();
        agent.group = "seen".into();
        agent.emphasized = false;
        agent.unread = false;
        agent.detail = Some("  진행할까요?  ".into());
        agent.mark = RowMark::Turn;
        let state = row_state(&agent);
        assert!(!state.attention);
        assert_eq!(state.bucket, "resting");
        assert_eq!(state.subtree, "waiting");
        assert_eq!(state.search_tone, "attention");
        assert_eq!(
            state.line,
            Some(RowLine {
                text: "진행할까요?".into(),
                mode: "request",
                tone: Tone {
                    kind: "warning",
                    read: true
                },
                pull_request: None,
                subject: None,
            })
        );
        assert_eq!(
            state.mark_tone,
            Tone {
                kind: "warning",
                read: true
            }
        );
        // A delegated child's question is its parent's to answer: no mark of
        // the operator's, but its request keeps the warning hue, dimmed (B24).
        agent.delegated = true;
        agent.lineage_depth = 1;
        agent.lineage_worktree_badge = Some("  feature  ".into());
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Quiet);
        let child = row_state(&agent);
        assert!(!child.root);
        assert!(!child.selection_emphasizes_title);
        assert_eq!(child.branch_badge.as_deref(), Some("feature"));
        assert_eq!(child.mark_tone.kind, "subtle");
        assert_eq!(child.chip_tone, state.chip_tone);
    }

    /// One meaning across surfaces (D-12): a waiting root is in progress on
    /// its mark, its chip and its search hit alike.
    #[test]
    fn a_waiting_root_is_in_progress_on_every_surface() {
        let mut agent = row();
        agent.group = "working".into();
        agent.activity = "stopped".into();
        agent.emphasized = false;
        agent.unread = false;
        agent.wait = Some(crate::model::AgentWait::Children);
        agent.mark = RowMark::of(&agent);
        assert_eq!(agent.mark, RowMark::Waiting);
        let state = row_state(&agent);
        assert_eq!(state.mark_tone.kind, "working");
        assert_eq!(state.chip_tone.kind, "working");
        assert_eq!(state.search_tone, "working");
        assert_eq!(state.bucket, "delegating");
        assert_eq!(state.link, "working");
        assert!(state.working);
        assert!(!state.title_emphasized);
        assert!(state.selection_emphasizes_title);
    }

    /// B8: a block the operator has read leaves Needs You and keeps its turn
    /// mark and cause dimmed; it still counts as the operator's turn.
    #[test]
    fn a_read_block_keeps_its_mark_and_cause_dimmed_in_seen() {
        let mut agent = row();
        agent.demand = "error".into();
        agent.activity = "stopped".into();
        agent.progress = Some("디스크 여유가 없어 검증을 못 함".into());
        agent.unread = true;
        derive_from_axes(&mut agent);
        assert_eq!(agent.group, "needs_you");
        assert_eq!(agent.mark, RowMark::Turn);
        assert!(!agent.state.mark_tone.read);
        assert_eq!(agent.state.attention_rank, 0);
        assert!(!agent.state.chip_tone.read);

        agent.unread = false;
        derive_from_axes(&mut agent);
        assert_eq!(agent.group, "seen");
        assert_eq!(agent.mark, RowMark::Turn);
        assert!(agent.state.mark_tone.read);
        assert_eq!(agent.status_code, AgentStatusCode::Error);
        assert_eq!(
            agent.detail.as_deref(),
            Some("디스크 여유가 없어 검증을 못 함")
        );
        assert_eq!(
            agent.state.chip_tone,
            Tone {
                kind: "warning",
                read: true
            }
        );
        let mut counts = crate::model::MarkCountsSnapshot::default();
        RowMark::of(&agent).count_into(&mut counts);
        assert_eq!((counts.turn, counts.unread), (1, 0));
    }

    /// D-10, B15: the operator's turn outranks progress, progress outranks a
    /// stop to read, and a stop's dot is green only on a finished verdict.
    #[test]
    fn precedence_is_the_turn_then_progress_then_a_stop_to_read() {
        use crate::labels::analysis::LabelEnd;
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.completed = true;
        agent.unread = true;
        agent.row_facts = Some(crate::request_view::RowFacts {
            end: Some(LabelEnd::Unfinished),
            ..Default::default()
        });
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.group.as_str(), agent.mark),
            ("done", RowMark::Unread)
        );
        assert_eq!(agent.status_code, AgentStatusCode::Unfinished);

        agent.wait = Some(crate::model::AgentWait::Children);
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.group.as_str(), agent.mark),
            ("working", RowMark::Waiting)
        );
        assert_eq!(agent.status_code, AgentStatusCode::Waiting);

        agent.demand = "error".into();
        agent.wait = None;
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.group.as_str(), agent.mark),
            ("needs_you", RowMark::Turn)
        );
    }

    /// D-03, D-13, B4, B18: the dot to read is green only when the label read
    /// the turn as finished; work left, a wait nothing proves and a stop no
    /// label judged are plain, and none of them says Done.
    #[test]
    fn a_dot_to_read_is_green_only_on_a_finished_verdict() {
        use crate::labels::analysis::LabelEnd;
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.completed = true;
        agent.unread = true;
        let cases = [
            (Some(LabelEnd::Done), RowMark::Done, AgentStatusCode::Done),
            (
                Some(LabelEnd::Unfinished),
                RowMark::Unread,
                AgentStatusCode::Unfinished,
            ),
            // A wait nothing proves reads as work left (status-marks D-17).
            (
                Some(LabelEnd::Waiting),
                RowMark::Unread,
                AgentStatusCode::Unfinished,
            ),
            (None, RowMark::Unread, AgentStatusCode::Stopped),
        ];
        for (end, mark, code) in cases {
            agent.row_facts = end.map(|end| crate::request_view::RowFacts {
                end: Some(end),
                ..Default::default()
            });
            derive_from_axes(&mut agent);
            assert_eq!(
                (agent.group.as_str(), agent.mark, agent.status_code),
                ("done", mark, code),
                "{end:?}"
            );
        }
        // Read, the dot goes; work left keeps its word and its line (B7).
        agent.unread = false;
        agent.progress = Some("테스트 2개 남음".into());
        agent.row_facts = Some(crate::request_view::RowFacts {
            end: Some(LabelEnd::Unfinished),
            ..Default::default()
        });
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.mark, agent.status_code),
            (RowMark::Quiet, AgentStatusCode::Unfinished)
        );
        assert_eq!(
            agent.state.line.as_ref().map(|line| line.mode),
            Some("kept")
        );
    }

    /// status-marks D-16, B27, B28: a turn the label read as finished is to
    /// read as done, the tasks it left counted on its line; any other verdict,
    /// or none, keeps the row in progress on them.
    #[test]
    fn a_finished_turn_is_not_hidden_by_the_background_tasks_it_left() {
        use crate::labels::analysis::LabelEnd;
        let facts = |end| {
            Some(crate::request_view::RowFacts {
                end,
                wake_devices: 2,
                ..Default::default()
            })
        };
        let mut rows = vec![row()];
        rows[0].activity = "stopped".into();
        rows[0].completed = true;
        rows[0].unread = true;
        rows[0].row_facts = facts(Some(LabelEnd::Done));
        crate::agent_state::apply_lineage(&mut rows, &[], &[]);
        let agent = &rows[0];
        assert_eq!(
            (
                agent.group.as_str(),
                agent.mark,
                agent.status_code,
                agent.wait,
                agent.background_left
            ),
            ("done", RowMark::Done, AgentStatusCode::Done, None, 2)
        );

        for end in [Some(LabelEnd::Waiting), Some(LabelEnd::Unfinished), None] {
            rows[0].row_facts = facts(end);
            crate::agent_state::apply_lineage(&mut rows, &[], &[]);
            let agent = &rows[0];
            assert_eq!(
                (
                    agent.group.as_str(),
                    agent.mark,
                    agent.status_code,
                    agent.wait,
                    agent.background_left
                ),
                (
                    "working",
                    RowMark::Waiting,
                    AgentStatusCode::Waiting,
                    Some(crate::model::AgentWait::Background),
                    0
                ),
                "{end:?}"
            );
            let line = agent.state.line.as_ref().expect("a waiting line");
            assert_eq!(line.mode, "waiting", "{end:?}");
        }
    }

    /// status-marks D-25, B36: a row that waits on proven work draws the
    /// dashed ring and the agent working again draws the solid one; both
    /// count as one in-progress mark.
    #[test]
    fn a_waiting_row_draws_the_dashed_ring_and_counts_as_in_progress() {
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.completed = true;
        agent.wait = Some(crate::model::AgentWait::Background);
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Waiting);
        assert_eq!(serde_json::to_value(agent.mark).unwrap(), "waiting");
        let mut counts = crate::model::MarkCountsSnapshot::default();
        agent.mark.count_into(&mut counts);
        assert_eq!(counts.going, 1);
        assert_eq!(row_state(&agent).mark_tone.kind, "working");

        agent.activity = "working".into();
        agent.completed = false;
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Going);
    }

    /// The phone draws a sleeping pane as the desktop does and refuses its
    /// input, so the core's own sleep state, its failure reason included,
    /// reaches the projection whole; an awake row carries none.
    #[test]
    fn the_phone_projection_carries_the_sleep_the_core_published() {
        let mut agent = row();
        let rest = json!({"navigator": {"agents": [&agent], "workspaces": []}});
        let phone = super::super::phone::project(&rest, "local");
        assert_eq!(phone.agents().next().expect("the awake row").sleep, None);

        agent.sleep = Some(crate::model::AgentSleepSnapshot {
            state: "failed".into(),
            reason: Some("The conversation file is gone".into()),
            since_unix_ms: 7,
            progress: Some("파서를 나눴어요".into()),
        });
        derive_from_axes(&mut agent);
        let rest = json!({"navigator": {"agents": [&agent], "workspaces": []}});
        let phone = super::super::phone::project(&rest, "local");
        let sleep = phone
            .agents()
            .next()
            .and_then(|listed| listed.sleep.clone())
            .expect("the failed wake on the phone");
        assert_eq!(
            (
                sleep.state.as_str(),
                sleep.reason.as_deref(),
                sleep.since_unix_ms,
                sleep.progress.as_deref(),
            ),
            (
                "failed",
                Some("The conversation file is gone"),
                7,
                Some("파서를 나눴어요"),
            )
        );
    }

    /// status-marks D-21, B31: a sleeping agent keeps a moon and is counted on
    /// no badge, whatever its turn was.
    #[test]
    fn a_sleeping_row_draws_asleep_and_counts_on_no_badge() {
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.completed = true;
        agent.unread = true;
        agent.sleep = Some(crate::model::AgentSleepSnapshot {
            state: "sleeping".into(),
            reason: None,
            since_unix_ms: 1,
            progress: None,
        });
        agent.progress = Some("파서를 나눴어요".into());
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Asleep);
        let mut counts = crate::model::MarkCountsSnapshot::default();
        agent.mark.count_into(&mut counts);
        assert_eq!(counts, crate::model::MarkCountsSnapshot::default());
        // It keeps its place in a tree and its class in a close list.
        assert_eq!((agent.state.tree_rank, agent.state.subtree), (2, "unread"));
        // The phone draws no line under it, as the sidebar does not.
        let rest = json!({"navigator": {"agents": [&agent], "workspaces": []}});
        let phone = super::super::phone::project(&rest, "local");
        let listed = phone
            .agents()
            .next()
            .expect("the sleeping row on the phone");
        assert_eq!(listed.line, None);

        // The operator's turn outranks the moon (D-10), and the phone draws
        // its ask as it would on any other row.
        agent.demand = "question".into();
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Turn);
        let rest = json!({"navigator": {"agents": [&agent], "workspaces": []}});
        let phone = super::super::phone::project(&rest, "local");
        let line = phone
            .agents()
            .next()
            .and_then(|listed| listed.line.clone())
            .expect("the ask's line on the phone");
        assert_eq!((line.mode.as_str(), line.tone), ("request", "warning"));
    }

    /// sidebar-quiet-mark-indent D-04, B5: an agent whose activity Hide cannot
    /// read draws the unknown mark, not the resting one, and counts on no
    /// badge; one that also asks the operator draws their turn.
    #[test]
    fn an_unreadable_activity_draws_unknown_and_a_resting_agent_quiet() {
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.unread = false;
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.mark, agent.status_code),
            (RowMark::Quiet, AgentStatusCode::Idle)
        );

        agent.activity = "unknown".into();
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.mark, agent.status_code),
            (RowMark::Unknown, AgentStatusCode::Unknown)
        );
        let mut counts = crate::model::MarkCountsSnapshot::default();
        agent.mark.count_into(&mut counts);
        assert_eq!(counts, crate::model::MarkCountsSnapshot::default());

        agent.demand = "question".into();
        agent.unread = true;
        derive_from_axes(&mut agent);
        assert_eq!(agent.mark, RowMark::Turn);
    }

    /// status-marks B16, B31: a waking, a failed and an unknown row keep a
    /// line on the phone that says their state, as the sidebar's does.
    #[test]
    fn a_waking_failed_or_unknown_row_keeps_a_state_line_on_the_phone() {
        for code in [
            AgentStatusCode::Waking,
            AgentStatusCode::SleepFailed,
            AgentStatusCode::Unknown,
        ] {
            let mut agent = row();
            agent.progress = Some("파서를 나눴어요".into());
            derive_from_axes(&mut agent);
            agent.status_code = code;
            let rest = json!({"navigator": {"agents": [&agent], "workspaces": []}});
            let phone = super::super::phone::project(&rest, "local");
            let line = phone
                .agents()
                .next()
                .and_then(|listed| listed.line.clone())
                .unwrap_or_else(|| panic!("{code:?} keeps a line"));
            assert_eq!(
                (line.mode.as_str(), line.text.as_str(), line.tone),
                ("kept", "", "subtle"),
                "{code:?}"
            );
        }
    }

    /// D-07, D-08: a quiet row waiting on its pull request's checks is in
    /// progress and names it; the end of those checks is the news its line says.
    #[test]
    fn a_checks_wait_is_in_progress_and_its_end_is_news() {
        let mut agent = row();
        agent.activity = "stopped".into();
        agent.completed = true;
        agent.unread = true;
        agent.checks_wait = Some(12);
        agent.wait = Some(crate::model::AgentWait::Checks);
        derive_from_axes(&mut agent);
        assert_eq!(
            (agent.group.as_str(), agent.mark),
            ("working", RowMark::Waiting)
        );
        let line = agent.state.line.clone().expect("a cause line");
        assert_eq!(
            (line.mode, line.text.as_str(), line.pull_request),
            ("waiting", "", Some(12))
        );

        agent.wait = None;
        agent.checks_wait = None;
        agent.checks_ended = Some(crate::model::ChecksEnded {
            number: 12,
            passed: false,
        });
        derive_from_axes(&mut agent);
        assert_eq!(agent.group, "done");
        let line = agent.state.line.clone().expect("the checks' end");
        assert_eq!(
            (line.mode, line.pull_request, line.tone.kind),
            ("checks_failed", Some(12), "warning")
        );
    }
}
