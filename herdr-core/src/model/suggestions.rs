//! The `suggestions` snapshot section (PRD hide-ai-suggestions B1-B3,
//! B10-B19, D-01, D-07, D-37, D-44): the Hide AI tool's 제안 tab for the
//! Project the tool opened. The core decides every group, state and count
//! here; the shell words them and dispatches the actions.

use crate::suggestions::ledger::{ImprovementKind, Signal};
use crate::suggestions::notebook::Outcome;
use crate::suggestions::policy::Quantity;
use crate::suggestions::settings::ApplyMode;
use serde::Serialize;

/// The section: absent until the tool opens a Project, then the opened
/// Project's suggestions, or `None` once the tool closes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SuggestionsSnapshot {
    pub project: Option<ProjectSuggestionsSnapshot>,
}

/// One Project's suggestion tab, opened by the same `memory_open` that opens
/// its Memory tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProjectSuggestionsSnapshot {
    /// The catalog Project the shell named.
    pub workspace_id: String,
    /// The first read has not answered yet.
    pub loading: bool,
    pub unavailable: Option<SuggestionsUnavailable>,
    /// The tool head's switch for the whole of Hide AI here (B3).
    pub enabled: bool,
    /// The one-time `내 구독 사용량을 씀` line was shown (B3).
    pub intro_seen: bool,
    pub status: SuggestionsStatus,
    pub ai: SuggestionsAi,
    pub github: SuggestionsGithub,
    pub settings: SuggestionSettingsSnapshot,
    /// The tab's rows in drawing order: the 고를 것 group, then 반영 중,
    /// then the folded 반영됨.
    pub rows: Vec<SuggestionRowSnapshot>,
    pub counts: SuggestionCountsSnapshot,
}

/// Why a Project's suggestions cannot be read; the reason is in the
/// diagnostic log.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionsUnavailable {
    Unregistered,
    Unreadable,
}

/// The tab's one line (D-44).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionsStatus {
    /// On, never investigated: `다음에 Hide를 열 때 첫 조사`.
    #[default]
    FirstRunPending,
    /// A run is going: `조사 중` beside the owl, the rows as they were.
    Investigating,
    /// Today's run ended and nothing is there to choose: `오늘은 제안 없음`.
    NoneToday,
    /// The last run could not investigate, so nothing new rose from it:
    /// `조사 못 함` beside the owl, quietly, until a run investigates (D-35).
    NotInvestigated,
    /// Nothing to say.
    Idle,
    /// The head switch or `제안만 끄기` is off.
    Off,
}

/// Whether the Hide AI agent can investigate (B19).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionsAi {
    #[default]
    Ready,
    /// Hide AI is off or signed out; Settings › Hide AI is the place.
    NotReady,
    /// The chosen agent cannot promise to only read (OpenCode, D-39).
    CannotInvestigate,
}

/// The GitHub connection the CI and review signals and the pull request
/// watch rest on (D-41).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionsGithub {
    #[default]
    Connected,
    /// No GitHub repository, no `gh`, signed out or no permission.
    Missing,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SuggestionSettingsSnapshot {
    pub suggestions_enabled: bool,
    pub quantity: Quantity,
    pub apply_mode: ApplyMode,
    /// Every signal in a fixed order, with its switch.
    pub signals: Vec<SuggestionSignalSnapshot>,
    /// Lines the operator skipped with `넘어가기`, which ⚙ restores (D-43).
    pub dismissed: Vec<SuggestionDismissedSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionSignalSnapshot {
    pub signal: Signal,
    pub enabled: bool,
    /// Why the switch is off and cannot be turned on here (B5, D-23).
    pub unavailable: Option<SignalUnavailable>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalUnavailable {
    /// The Project is not a GitHub repository or GitHub does not answer.
    NoGithub,
    /// Memory is off in this Project, so corrections are not learned.
    MemoryOff,
    /// Hide AI or its turn-end analysis (Settings › Hide AI's agent
    /// summaries) is off, so no improvement is found (D-57).
    AnalysisOff,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SuggestionDismissedSnapshot {
    pub id: String,
    pub discomfort: Option<String>,
    pub basis: Option<SuggestionBasisSnapshot>,
    /// The skip may still bring the line back once when its evidence
    /// doubles; false once it came back and was skipped again, which is for
    /// good (D-43).
    pub may_return: bool,
}

/// The signal and tag a line the deterministic grouping wrote rests on; the
/// shell words a row that has no text from it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionBasisSnapshot {
    pub signal: Signal,
    pub tag: String,
}

/// The tab's number and the rail's (D-01).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SuggestionCountsSnapshot {
    /// What the operator has to do in the 제안 tab: new suggestions and
    /// rows that are the operator's turn.
    pub suggestions: usize,
    /// The 제안 tab's number plus the Memory tab's own; none while the head
    /// switch is off.
    pub rail: usize,
}

/// Which of the tab's three groups a row is in (D-07).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionGroup {
    /// 고를 것.
    Choose,
    /// 반영 중.
    Applying,
    /// The folded 반영됨.
    Applied,
}

/// A row's mark (D-37).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionState {
    /// 새 제안: the green open circle.
    New,
    /// 내 차례: the yellow dot, also in Needs You.
    MyTurn,
    /// Hide AI가 하는 중: the blue turning ring and dimmed text.
    HideAiWorking,
    /// 지켜보는 중: a quiet line inside 반영됨.
    Watching,
    /// 효과 있었음.
    Effective,
    /// 반영됨, nothing more to say.
    Applied,
}

/// Where a row comes from: a notebook line, or a Memory card's way into
/// the repository's documents, which this tab took over (D-02).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionSource {
    Notebook,
    Memory,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SuggestionRowSnapshot {
    /// `s12` for a notebook line, `m72` for a Memory card; every
    /// `suggestion_action` names the row by it.
    pub id: String,
    pub source: SuggestionSource,
    pub group: SuggestionGroup,
    pub state: SuggestionState,
    /// 불편, in a person's words.
    pub discomfort: Option<String>,
    /// 놓친 것, one sentence.
    pub cause: Option<String>,
    /// 고칠 것.
    pub fix: Option<String>,
    /// A pull request (`PR 만들기`) or an issue (`이슈 만들기`).
    pub outcome: Outcome,
    pub basis: Option<SuggestionBasisSnapshot>,
    /// Hide applied it by itself (`자동`, D-29).
    pub auto: bool,
    /// Why a new suggestion waits instead of being applied.
    pub hold: Option<SuggestionHold>,
    /// A new suggestion that rose again after the operator skipped it
    /// (`넘어갔지만 다시 생겼어요`, D-43); skipping it now is for good.
    pub returned_after_skip: bool,
    /// The problem came back after its fix merged (`문서로는 안 됐어요`).
    pub escalated: Option<SuggestionEscalationSnapshot>,
    pub evidence: Vec<SuggestionEvidenceSnapshot>,
    pub attempt: Option<SuggestionAttemptSnapshot>,
    pub watch: Option<SuggestionWatchSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionHold {
    /// `GitHub 연결 필요` stands where `PR 만들기` or `이슈 만들기` would:
    /// a pull request and an issue both need GitHub (D-41).
    GithubNeeded,
    /// An automatic pull request nobody reviewed yet holds the next one
    /// (D-29).
    AutoPrUnreviewed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionEscalationSnapshot {
    /// The fix level now suggested: 1 a script or a check, 2 a hook.
    pub level: u8,
    pub previous_pr_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionEvidenceKind {
    Session,
    Pr,
}

/// One piece of evidence a row links (B10); a session whose record is gone
/// is drawn dimmed and cannot be opened, and a pull request opens by its
/// `url`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionEvidenceSnapshot {
    pub kind: SuggestionEvidenceKind,
    pub signal: Signal,
    /// An improvement's kind, which the row shows as a chip (B25).
    pub improvement: Option<ImprovementKind>,
    /// The session's agent and native id, for a session.
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub pr_number: Option<u32>,
    pub url: Option<String>,
    pub at_unix_ms: u64,
    pub gone: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionAttemptPhase {
    /// The agent is being started.
    Starting,
    /// The agent works on the pull request or the issue.
    Working,
    /// The pull request is open and waits for the operator's review.
    PrOpen,
    /// Its checks failed and the agent is not working (B12).
    CiFailed,
    /// The agent ended with no pull request (B12): `다시 시도` and `넘어가기`.
    Failed,
    /// The issue is open (B14).
    IssueOpen,
    Merged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionAttemptSnapshot {
    pub phase: SuggestionAttemptPhase,
    pub started_at_unix_ms: u64,
    pub pr_number: Option<u32>,
    pub url: Option<String>,
    /// The agent's registration id while it lives, which opens its pane.
    pub agent_id: Option<String>,
    /// Why a failed attempt failed, as a code the shell words.
    pub failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SuggestionWatchSnapshot {
    /// Sessions in a checkout holding the merge, so far.
    pub sessions_seen: usize,
    pub sessions_needed: usize,
}
