//! What one analysis is asked and when a pane owes one.
//!
//! The context carries the operator's requests only (PRD overview-request-view
//! D-09): the first three and last eight or, once a goal exists, the new ones
//! and the latest exchange; each section is masked and cut on its own so the
//! requests survive a long exchange, within 4000 characters. A turn
//! costs at most two requests, one at its start and one at its end, and one
//! more only when the end's timed out (D-33); a `goal_update: keep` answer
//! keeps the goal verbatim.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;
use std::time::Duration;

use hide_ai::AiError;
use hide_session::label_transcript::{LabelEvent, LabelEventKind};
use regex::Regex;

/// Upper bound on the analysis context. This also guards against one
/// enormous turn while the initial or rolling sections are being assembled.
pub(crate) const MAX_ANALYSIS_CONTEXT_CHARS: usize = 4_000;
/// The assigned work's evidence survives terse follow-ups, without growing
/// the request or persisting an unbounded conversation.
pub(crate) const MAX_GOAL_EVIDENCE_CHARS: usize = 800;
/// The shortest goal that names a result rather than a word.
pub(crate) const MIN_GOAL_CHARS: usize = 8;

/// How long a goal and a `line` may be in the language they are written in
/// (PRD ai-output-language D-08): a character of Korean, Chinese or Japanese
/// says about what two of English do. Every surface draws a line in one
/// line; the prompt asks for these lengths and the parser cuts whatever
/// came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LabelLimits {
    pub(crate) goal: usize,
    pub(crate) line: usize,
}

impl LabelLimits {
    pub(crate) fn of(language: hide_ai::OutputLanguage) -> Self {
        use hide_ai::OutputLanguage::*;
        match language {
            English => Self { goal: 60, line: 80 },
            Korean | SimplifiedChinese | Japanese => Self { goal: 30, line: 40 },
        }
    }
}
/// The initial view includes a small head and a bounded tail of human turns.
pub(crate) const INITIAL_FIRST_USER_TURNS: usize = 3;
/// How many of the session's latest human turns feed the initial task
/// context; unbounded, a long session would grow every request.
pub(crate) const MAX_USER_REQUEST_TURNS: usize = 8;
/// The most events a pane keeps between reads besides the human turns the
/// context needs.
const MAX_RETAINED_EVENTS: usize = 512;
/// How long a pane waits before asking again after the provider layer said
/// no provider can answer now (not logged in, usage limit, none connected).
/// Without it the exhausted state was rediscovered on every event, which
/// once wrote 42828 identical lines in a day.
pub(crate) const PROVIDER_RECOVERY_INTERVAL: Duration = Duration::from_secs(600);

/// One provider verdict, already judged usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Analysis {
    /// What the session is to have made when it ends (D-38), absent until
    /// the operator has asked for work.
    pub(crate) goal: Option<String>,
    pub(crate) goal_update: GoalUpdate,
    /// This turn's work while it runs, its result once over, or what the
    /// operator is to answer or do when it ended on a question.
    pub(crate) line: String,
    pub(crate) end: LabelEnd,
    pub(crate) work: Option<crate::links::ledger::TurnJudgment>,
}

/// A clearer name for the same assignment is distinct from a new assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GoalUpdate {
    Keep,
    Refine,
    Replace,
}

/// How the turn stands by the agent's last word (D-08). Only a question, a
/// block, an unfinished turn and a wait on something other than a pull
/// request move a row's verb; the rest come from Herdr and GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelEnd {
    Working,
    /// The agent asks the operator something specific.
    Question,
    /// It stopped before the request was done and named the cause (a full
    /// disk, a login that lapsed, a failing tool). This is the label's word;
    /// Herdr's `blocked` lifecycle is an approval prompt and is another thing.
    Blocked,
    /// The agent reported the request finished.
    Done,
    /// It waits on something other than a pull request: a build, another
    /// agent.
    Waiting,
    /// It stopped before the request was done, without asking.
    Unfinished,
}

/// Why one analysis produced no verdict.
#[derive(Debug)]
pub(crate) enum AnalysisFailure {
    /// The provider layer gave up after its own retries, or none answers.
    Provider(AiError),
    /// A well-formed answer whose content the feature refuses.
    Invalid(String),
    /// The analysis thread itself failed; a bug, reported rather than lost.
    Worker(String),
    /// The daemon is stopping and cancelled the request. Nothing is
    /// recorded, so the next process asks again.
    Stopped,
}

impl AnalysisFailure {
    /// A wait means the same context can succeed once the environment
    /// changes (a login, a usage window). `None` is a failure the input
    /// settles, and the turn is parked with it rather than asked again.
    pub(crate) fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Provider(AiError::UsageLimited { retry_after }) => {
                Some(retry_after.unwrap_or(PROVIDER_RECOVERY_INTERVAL))
            }
            Self::Provider(
                AiError::NotAuthenticated
                | AiError::NoProvider(_)
                | AiError::ProviderUnavailable(_)
                | AiError::OverBudget { .. }
                // Hide AI turned off between the check and the request: the
                // turn waits for it to be turned on instead of being parked.
                | AiError::Disabled,
            ) => Some(PROVIDER_RECOVERY_INTERVAL),
            Self::Provider(_) | Self::Invalid(_) | Self::Worker(_) | Self::Stopped => None,
        }
    }

    /// A provider that ran out of time may answer the same context when
    /// asked again; a turn's end gets that one more request (D-33).
    pub(crate) fn timed_out(&self) -> bool {
        matches!(self, Self::Provider(AiError::Timeout))
    }

    /// A class for the diagnostic log: an error class or a reason code,
    /// never provider output.
    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Provider(error) => error.to_string(),
            Self::Invalid(reason) | Self::Worker(reason) => reason.clone(),
            Self::Stopped => "stopped".to_owned(),
        }
    }
}

/// Cut a goal or a line to `max` characters without splitting a word, ending
/// in `…` when cut; a head shorter than half of `max` (a language written
/// with few spaces) is cut on a character instead.
pub(crate) fn truncate_phrase(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let budget: String = text.chars().take(max - 1).collect();
    let head = budget
        .rsplit_once(char::is_whitespace)
        .map(|(head, _)| head)
        .filter(|head| head.chars().count() * 2 >= max)
        .unwrap_or(&budget);
    format!("{}…", head.trim_end())
}

pub(crate) fn normalize_goal(raw: &str, max: usize) -> Option<String> {
    let candidate = raw
        .rsplit("</think>")
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches(|character| matches!(character, '`' | '*' | '"' | '#'))
        .trim();
    if candidate.chars().count() < MIN_GOAL_CHARS || candidate.chars().any(char::is_control) {
        return None;
    }
    Some(truncate_phrase(candidate, max))
}

/// Normalize a one-line non-title field without silently accepting a
/// multiline answer. An empty `line` is valid when the boundary has nothing
/// more to say.
pub(crate) fn normalize_text_field(raw: &str, allow_empty: bool) -> Option<String> {
    let candidate = raw.trim();
    if (!allow_empty && candidate.is_empty()) || candidate.chars().any(char::is_control) {
        return None;
    }
    Some(candidate.to_owned())
}

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)sk-[a-z0-9_-]{8,}|(?:api[_-]?key|token|password|secret)\s*[=:]\s*[^\s,;]+")
        .expect("valid secret expression")
});
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").expect("valid email expression")
});
static HOME_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    // Account names are private; the remaining folders and file name identify
    // a delegated task. Consume a quoted path through its matching quote so
    // another quote in the account or a home-looking tail cannot split it.
    // An unquoted Windows account with spaces must end
    // at a separator without trailing whitespace or invalid name characters,
    // so a bare home cannot consume prose through a later path's separator.
    // A path component cannot start another home prefix.
    let root = r"(?:(?:/Users|/home)/|(?i:[a-z]:[\\/]Users[\\/]))";
    let quoted = [
        ('"', "double_root"),
        ('\'', "single_root"),
        ('`', "backtick_root"),
    ]
    .map(|(quote, name)| format!(r"{quote}(?i:file:///?)?(?P<{name}>{root})[^{quote}\r\n]*{quote}"))
    .join("|");
    Regex::new(&format!(
        r#"(?x)
        (?P<quoted>{quoted})
        |
        (?P<boundary>(?:^|[^\w./\\~])(?i:file:///?)?)
        (?P<home>
            (?:/Users|/home)/[^/\\\s"]+/?
            |
            (?i:[a-z]:[\\/]Users[\\/])
            (?:
                [^\\/\s"<>:*?|](?:[^\\/\x00-\x1f"<>:*?|]*[^\\/\s"<>:*?|])?[\\/]
                |[^\\/\s"]+
            )
        )"#,
    ))
    .expect("valid home prefix expression")
});

fn conversation(events: &[LabelEvent]) -> Vec<&LabelEvent> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                LabelEventKind::Human | LabelEventKind::Assistant
            )
        })
        .collect()
}

fn render(events: &[&LabelEvent]) -> String {
    events
        .iter()
        .map(|event| format!("{}: {}", event.kind.role(), event.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn latest_exchange(events: &[LabelEvent]) -> String {
    let conversation = conversation(events);
    if conversation.is_empty() {
        return String::new();
    }
    let last = conversation
        .iter()
        .rposition(|event| event.kind == LabelEventKind::Human)
        .unwrap_or(0);
    let start = conversation[..last]
        .iter()
        .rposition(|event| event.kind == LabelEventKind::Human)
        .unwrap_or(last);
    render(&conversation[start..])
}

fn latest_turn_exchange(events: &[LabelEvent]) -> String {
    let conversation = conversation(events);
    let Some(start) = conversation
        .iter()
        .rposition(|event| event.kind == LabelEventKind::Human)
    else {
        return String::new();
    };
    render(&conversation[start..])
}

/// The operator's first three and last eight requests with an explicit
/// omission marker, joined, or `None` when there is none.
fn initial_operator_requests(
    events: &[LabelEvent],
    operator: &dyn Fn(&LabelEvent) -> bool,
) -> Option<String> {
    let requests = events
        .iter()
        .filter(|event| event.kind == LabelEventKind::Human && operator(event))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return None;
    }
    let recent_start = requests.len().saturating_sub(MAX_USER_REQUEST_TURNS);
    let first_count = requests.len().min(INITIAL_FIRST_USER_TURNS);
    let omitted = recent_start.saturating_sub(first_count);
    let mut lines = requests[..first_count]
        .iter()
        .map(|event| format!("user: {}", event.text))
        .collect::<Vec<_>>();
    lines.push(format!(
        "<omitted-requests>{omitted}개 요청 생략</omitted-requests>"
    ));
    lines.extend(
        requests[recent_start..]
            .iter()
            .skip(first_count.saturating_sub(recent_start))
            .map(|event| format!("user: {}", event.text)),
    );
    Some(lines.join("\n---\n"))
}

/// The initial text handed to the provider: the first three and last eight
/// of the operator's requests (`operator` says which person's messages are)
/// with an explicit omission marker, and the latest exchange.
pub(crate) fn analysis_context(
    events: &[LabelEvent],
    operator: &dyn Fn(&LabelEvent) -> bool,
) -> String {
    let Some(requests) = initial_operator_requests(events, operator) else {
        return String::new();
    };
    bounded_context(
        "<initial-operator-requests>\n",
        &requests,
        "\n</initial-operator-requests>",
        &latest_exchange(events),
    )
}

/// The rolling text for a goal that already exists: the operator's new
/// requests as goal evidence, and the latest exchange for the line and the
/// end.
pub(crate) fn rolling_analysis_context(
    previous_goal: &str,
    goal_evidence: &str,
    new_requests: &[LabelEvent],
    events: &[LabelEvent],
) -> String {
    let delta = new_requests
        .iter()
        .filter(|event| event.kind == LabelEventKind::Human)
        .map(|event| format!("user: {}", event.text))
        .collect::<Vec<_>>()
        .join("\n---\n");
    let delta = if delta.is_empty() {
        "<none/>".to_owned()
    } else {
        delta
    };
    bounded_context(
        &format!(
            "<previous-goal>{}</previous-goal>\n<goal-evidence>{}</goal-evidence>\n<new-operator-requests>\n",
            // A goal stored by an earlier build is not bounded by this one's
            // limits; it may not crowd the new requests out.
            head(
                &mask(&strip_code_fences(previous_goal)),
                MAX_REQUEST_SECTION_CHARS / 4
            ),
            head(goal_evidence, MAX_GOAL_EVIDENCE_CHARS),
        ),
        &delta,
        "\n</new-operator-requests>",
        &latest_turn_exchange(events),
    )
}

/// Attributed requests establish work; the first three replies can clarify a
/// path-only assignment without a later sub-task replacing its subject. Mask
/// before retaining either; device records never save this text to disk.
pub(crate) fn goal_evidence(
    events: &[LabelEvent],
    operator: &dyn Fn(&LabelEvent) -> bool,
) -> Option<String> {
    let requests = initial_operator_requests(events, operator)?;
    let first = events
        .iter()
        .position(|event| event.kind == LabelEventKind::Human && operator(event))?;
    let replies = events[first + 1..]
        .iter()
        .take_while(|event| event.kind != LabelEventKind::Human)
        .filter(|event| event.kind == LabelEventKind::Assistant && event.sender.is_none())
        .take(3)
        .map(|event| format!("assistant: {}", event.text))
        .collect::<Vec<_>>()
        .join("\n");
    if replies.is_empty() {
        return Some(head(
            &mask(&strip_code_fences(&requests)),
            MAX_GOAL_EVIDENCE_CHARS,
        ));
    }
    const OPEN: &str = "\n<assignment-replies>\n";
    const CLOSE: &str = "\n</assignment-replies>";
    let request_room = MAX_GOAL_EVIDENCE_CHARS / 2;
    Some(format!(
        "{}{}{}{}",
        head(&mask(&strip_code_fences(&requests)), request_room),
        OPEN,
        head(
            &mask(&strip_code_fences(&replies)),
            MAX_GOAL_EVIDENCE_CHARS - request_room - chars(OPEN) - chars(CLOSE)
        ),
        CLOSE,
    ))
}

/// The share of [`MAX_ANALYSIS_CONTEXT_CHARS`] the requests section may take,
/// tags included (PRD label-goal-command-context D-09).
const MAX_REQUEST_SECTION_CHARS: usize = MAX_ANALYSIS_CONTEXT_CHARS / 2;
/// Where a section was cut.
const CUT_MARK: char = '…';

/// The requests section, then the latest exchange, each masked on its own
/// and cut on its own (D-08): the requests keep their head within
/// [`MAX_REQUEST_SECTION_CHARS`], so the first request always stays, and the
/// exchange keeps its tail in whatever the whole bound leaves. Cutting the
/// joined text instead dropped the requests whenever the exchange was long,
/// as Codex's 6,400-character goal reminder made it.
fn bounded_context(open: &str, requests: &str, close: &str, exchange: &str) -> String {
    let requests = format!(
        "{open}{}{close}",
        head(
            &mask(&strip_code_fences(requests)),
            MAX_REQUEST_SECTION_CHARS.saturating_sub(chars(open) + chars(close)),
        )
    );
    const EXCHANGE_OPEN: &str = "\n<latest-exchange>\n";
    const EXCHANGE_CLOSE: &str = "\n</latest-exchange>";
    let room = MAX_ANALYSIS_CONTEXT_CHARS
        .saturating_sub(chars(&requests) + chars(EXCHANGE_OPEN) + chars(EXCHANGE_CLOSE));
    format!(
        "{requests}{EXCHANGE_OPEN}{}{EXCHANGE_CLOSE}",
        tail(&mask(&strip_code_fences(exchange)), room)
    )
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

/// The first `max` characters of `text`, the last of them [`CUT_MARK`] when
/// any were left out.
fn head(text: &str, max: usize) -> String {
    if chars(text) <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut kept = text.chars().take(max.saturating_sub(1)).collect::<String>();
    kept.push(CUT_MARK);
    kept
}

/// The last `max` characters of `text`, the first of them [`CUT_MARK`] when
/// any were left out.
fn tail(text: &str, max: usize) -> String {
    if chars(text) <= max {
        return text.to_owned();
    }
    if max <= 1 {
        return head(&CUT_MARK.to_string(), max);
    }
    let start = text
        .char_indices()
        .rev()
        .nth(max.saturating_sub(2))
        .map_or(0, |(index, _)| index);
    format!("{CUT_MARK}{}", &text[start..])
}

/// Which of a turn's two boundaries an analysis is answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisPhase {
    /// The user has just spoken; naming the task here is what lets a
    /// working pane say what it is working on.
    TurnStart,
    /// The agent has answered and stopped; whether it waits on the user can
    /// only be judged once its last word is in.
    TurnEnd,
}

impl AnalysisPhase {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::TurnStart => "start",
            Self::TurnEnd => "end",
        }
    }
}

fn hash_event(event: &LabelEvent) -> u64 {
    let mut hasher = DefaultHasher::new();
    event.at_unix_ms.hash(&mut hasher);
    event.text.hash(&mut hasher);
    hasher.finish()
}

/// Identity of the turn a transcript is in, taken from the user's own last
/// message, its time and its words: fixed for the whole turn and changing
/// exactly once, at the boundary, even when the new message repeats the last
/// one word for word. Hashing the growing context instead once spent a day's
/// request budget by midday. `None` means there is no turn to analyze yet.
pub(crate) fn turn_key(events: &[LabelEvent]) -> Option<u64> {
    events
        .iter()
        .rfind(|event| event.kind == LabelEventKind::Human)
        .map(hash_event)
}

/// The last human event that fed task analysis. Unlike the byte cursor it is
/// about the semantic input boundary, and survives a restart.
pub(crate) fn task_input_cursor(events: &[LabelEvent]) -> Option<u64> {
    events
        .iter()
        .rev()
        .find(|event| event.kind == LabelEventKind::Human)
        .map(hash_event)
}

pub(crate) fn new_human_turns(
    events: &[LabelEvent],
    previous_cursor: Option<u64>,
) -> Vec<LabelEvent> {
    let humans = events
        .iter()
        .filter(|event| event.kind == LabelEventKind::Human)
        .cloned()
        .collect::<Vec<_>>();
    let Some(previous_cursor) = previous_cursor else {
        return humans;
    };
    let Some(previous_index) = humans
        .iter()
        .rposition(|event| hash_event(event) == previous_cursor)
    else {
        // The bounded retention may have dropped the old boundary; the
        // retained human history is the available delta rather than a
        // silent claim that no new request arrived.
        return humans;
    };
    humans.into_iter().skip(previous_index + 1).collect()
}

/// The boundary this pane sits on, or `None` when the turn's two calls are
/// spent. Level-triggered: the condition holds until the call lands, so a
/// call deferred by a provider wait is made later instead of lost.
pub(crate) fn analysis_phase(
    newest_user_is_last: bool,
    working: bool,
    start_done: bool,
    end_done: bool,
) -> Option<AnalysisPhase> {
    if newest_user_is_last || working {
        return (!start_done).then_some(AnalysisPhase::TurnStart);
    }
    (!end_done).then_some(AnalysisPhase::TurnEnd)
}

/// Whether the newest human, interruption or assistant event is the human.
pub(crate) fn newest_user_is_last(events: &[LabelEvent]) -> bool {
    events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event.kind,
                LabelEventKind::Human | LabelEventKind::Interrupted | LabelEventKind::Assistant
            )
        })
        .is_some_and(|event| event.kind == LabelEventKind::Human)
}

/// The user tore the current turn down: the newest human-side event is an
/// interruption.
pub(crate) fn interrupted(events: &[LabelEvent]) -> bool {
    events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event.kind,
                LabelEventKind::Human | LabelEventKind::Interrupted
            )
        })
        .is_some_and(|event| event.kind == LabelEventKind::Interrupted)
}

fn strip_code_fences(text: &str) -> String {
    let mut inside = false;
    text.lines()
        .filter(|line| {
            if line.trim_start().starts_with("```") {
                inside = !inside;
                return false;
            }
            !inside
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The ledger's side of an END request: who the request is from, the tasks a
/// verdict may continue (open ones, and the newest finished ones, which a
/// request that asks for them again reopens) and the open missed items.
pub(crate) fn work_context(
    work_author: bool,
    ledger: &crate::links::ledger::LedgerContext,
) -> String {
    let tasks = |tasks: &[crate::links::ledger::WorkTask]| {
        serde_json::json!(
            tasks
                .iter()
                .map(|task| serde_json::json!({"id": task.id, "name": task.name}))
                .collect::<Vec<_>>()
        )
        .to_string()
    };
    let missed = serde_json::json!(
        ledger
            .open_missed
            .iter()
            .map(|missed| serde_json::json!({"id": missed.id, "kind": missed.kind, "text": missed.text}))
            .collect::<Vec<_>>()
    )
    .to_string();
    redact(&format!(
        "\n<work-author>{work_author}</work-author>\n<open-tasks>{}</open-tasks>\n<finished-tasks>{}</finished-tasks>\n<open-missed>{missed}</open-missed>",
        tasks(&ledger.open_tasks),
        tasks(&ledger.finished_tasks),
    ))
}

/// [`mask`], then its tail of the last [`MAX_ANALYSIS_CONTEXT_CHARS`] + 1
/// characters, for a caller that sends one text whole (a pull
/// request summary, the ledger's side of a request); the label context cuts
/// its sections itself (`bounded_context`).
pub(crate) fn redact(text: &str) -> String {
    let masked = mask(text);
    let start = masked
        .char_indices()
        .rev()
        .nth(MAX_ANALYSIS_CONTEXT_CHARS)
        .map_or(0, |(index, _)| index);
    masked[start..].to_owned()
}

/// `text` with secrets, email addresses and home accounts masked, trimmed.
fn mask(text: &str) -> String {
    let masked = SECRET.replace_all(text, "[redacted-secret]");
    let masked = EMAIL.replace_all(&masked, "[redacted-personal]");
    let masked = HOME_PREFIX.replace_all(&masked, |captures: &regex::Captures<'_>| {
        if let Some(quoted) = captures.name("quoted") {
            let root = ["double_root", "single_root", "backtick_root"]
                .into_iter()
                .find_map(|name| captures.name(name))
                .expect("quoted path has a home root");
            let path = quoted.as_str();
            let closing_quote = path.len() - 1;
            let account_and_tail = &path[root.end() - quoted.start()..closing_quote];
            if account_and_tail.is_empty() || account_and_tail.starts_with(['/', '\\']) {
                return path.to_owned();
            }
            let boundary = &path[..root.start() - quoted.start()];
            let ending = &path[closing_quote..];
            return match account_and_tail.find(['/', '\\']) {
                Some(separator) => {
                    format!("{boundary}~/{}{ending}", &account_and_tail[separator + 1..])
                }
                None => format!("{boundary}~{ending}"),
            };
        }
        let boundary = &captures["boundary"];
        let home = &captures["home"];
        if home.ends_with(['/', '\\']) {
            format!("{boundary}~/")
        } else {
            // A bare home may be followed by sentence punctuation rather
            // than a path tail. Keep that punctuation outside the account.
            let account_end = home.trim_end_matches(['.', ',', ';', ':', ')', '!', '?', ']', '}']);
            format!("{boundary}~{}", &home[account_end.len()..])
        }
    });
    masked.trim().to_owned()
}

pub(crate) fn context_fingerprint(context: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    context.hash(&mut hasher);
    hasher.finish()
}

/// Keeps what the context can still use: the first three human turns, the
/// last eight, and everything from the second-last human on, capped at
/// `MAX_RETAINED_EVENTS` so a long streamed answer cannot grow a pane.
pub(crate) fn retain_bounded(events: &mut VecDeque<LabelEvent>) {
    let human_positions = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| (event.kind == LabelEventKind::Human).then_some(index))
        .collect::<Vec<_>>();
    let second_last_human = human_positions.iter().rev().nth(1).copied();
    let mut retained = VecDeque::new();
    for (index, event) in events.drain(..).enumerate() {
        let keep = if event.kind == LabelEventKind::Human {
            // The first turns the initial context names, and the last ones.
            index
                <= human_positions
                    .get(INITIAL_FIRST_USER_TURNS.saturating_sub(1))
                    .copied()
                    .unwrap_or(usize::MAX)
                || human_positions
                    .iter()
                    .rev()
                    .take(MAX_USER_REQUEST_TURNS)
                    .any(|position| *position == index)
        } else {
            match human_positions.as_slice() {
                [] => false,
                [first_human] => index >= *first_human,
                _ => second_last_human.is_some_and(|boundary| index >= boundary),
            }
        };
        if keep {
            retained.push_back(event);
        }
    }
    *events = retained;
    while events.len() > MAX_RETAINED_EVENTS {
        let Some(index) = events
            .iter()
            .position(|event| event.kind != LabelEventKind::Human)
        else {
            break;
        };
        events.remove(index);
    }
}

#[cfg(test)]
pub(crate) fn human(text: &str, at: u64) -> LabelEvent {
    LabelEvent {
        kind: LabelEventKind::Human,
        at_unix_ms: at,
        text: text.to_owned(),
        offset: at,
        images: 0,
        sender: None,
    }
}

#[cfg(test)]
pub(crate) fn assistant(text: &str, at: u64) -> LabelEvent {
    LabelEvent {
        kind: LabelEventKind::Assistant,
        at_unix_ms: at,
        text: text.to_owned(),
        offset: at,
        images: 0,
        sender: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_truncates_on_a_word_boundary_and_rejects_short_or_multiline_titles() {
        assert_eq!(
            truncate_phrase(
                "Refactor the authentication middleware to use JWT tokens",
                30
            ),
            "Refactor the authentication…"
        );
        assert_eq!(normalize_goal("짧음", 30), None);
        assert_eq!(normalize_goal("two\nlines here ok", 30), None);
        assert_eq!(
            normalize_goal("**라벨 생성기 옮기기**", 30).as_deref(),
            Some("라벨 생성기 옮기기")
        );
    }

    #[test]
    fn initial_context_spans_the_first_three_and_last_eight_operator_requests() {
        let events = (0..15)
            .map(|index| human(&format!("turn {index}"), index))
            .collect::<Vec<_>>();
        let context = analysis_context(&events, &|_| true);
        for kept in [0, 1, 2, 7, 14] {
            assert!(
                context.contains(&format!("user: turn {kept}\n"))
                    || context.contains(&format!("user: turn {kept}"))
            );
        }
        assert!(!context.contains("user: turn 5\n"));
        assert!(context.contains("<omitted-requests>4개 요청 생략</omitted-requests>"));
    }

    #[test]
    fn another_agents_request_never_reaches_the_goal_evidence() {
        let events = vec![
            human("요청 보기 만들기", 1),
            assistant("착수", 2),
            human("테스트 결과 알려줘", 3),
            assistant("결과", 4),
        ];
        let context = analysis_context(&events, &|event| event.offset != 3);
        let (requests, exchange) = context
            .split_once("<latest-exchange>")
            .expect("the context has both sections");
        assert!(requests.contains("user: 요청 보기 만들기"));
        assert!(!requests.contains("테스트 결과"));
        // The exchange still carries it: the line and the end judge the turn.
        assert!(exchange.contains("테스트 결과 알려줘"));
    }

    #[test]
    fn rolling_context_carries_only_the_previous_goal_and_the_new_requests() {
        let events = vec![
            human("old request", 1),
            assistant("done", 2),
            human("new request", 3),
        ];
        let cursor = task_input_cursor(&events[..1]);
        let delta = new_human_turns(&events, cursor);
        let context = rolling_analysis_context("이전 작업 제목", "", &delta, &events);
        assert!(context.contains("<previous-goal>이전 작업 제목</previous-goal>"));
        assert!(context.contains("user: new request"));
        assert!(!context.contains("user: old request"));
    }

    #[test]
    fn path_only_assignments_keep_the_agents_initial_scope_not_a_later_subtask() {
        let events = vec![
            human("Read worker.md and carry out the assigned work", 1),
            assistant("지시를 읽겠습니다", 2),
            assistant("사용량의 모델별 집계와 기간 필터를 맡았습니다", 3),
            human("이제 문서 한 줄만 수정해", 4),
            assistant("문서 한 줄을 수정했습니다", 5),
        ];
        let evidence = goal_evidence(&events, &|_| true).unwrap();
        assert!(
            evidence.contains("사용량의 모델별 집계와 기간 필터"),
            "{evidence}"
        );
        assert!(!evidence.contains("assistant: 문서 한 줄"), "{evidence}");
        assert!(evidence.chars().count() <= MAX_GOAL_EVIDENCE_CHARS);
    }

    #[test]
    fn assignment_evidence_survives_a_terse_follow_up_within_the_context_bound() {
        let events = vec![
            human("에이전트 그래프의 필터와 프로젝트 접기를 개선해줘", 1),
            assistant("시안을 만들겠습니다", 2),
            human("그 방향으로 진행해", 3),
            assistant(&"큰 출력 ".repeat(2_000), 4),
        ];
        let evidence = goal_evidence(&events[..1], &|_| true).unwrap();
        let context =
            rolling_analysis_context("개선사항 구현 및 배포", &evidence, &events[2..3], &events);
        assert!(context.contains("에이전트 그래프의 필터와 프로젝트 접기를 개선해줘"));
        assert!(context.contains("그 방향으로 진행해"));
        assert_eq!(context.chars().count(), MAX_ANALYSIS_CONTEXT_CHARS);

        let evidence = goal_evidence(
            &[
                human(
                    "token=private-value /Users/example/project 그래프 필터 개선",
                    1,
                ),
                human(&"긴 요청 ".repeat(2_000), 2),
                human("다른 에이전트의 작업", 3),
            ],
            &|event| event.offset != 3,
        )
        .unwrap();
        assert!(evidence.chars().count() <= MAX_GOAL_EVIDENCE_CHARS);
        assert!(!evidence.contains("private-value"));
        assert!(!evidence.contains("/Users/example"));
        assert!(!evidence.contains("다른 에이전트의 작업"));
    }

    fn sections(context: &str) -> (&str, &str) {
        context
            .split_once("\n<latest-exchange>\n")
            .expect("the context has both sections")
    }

    #[test]
    fn a_long_exchange_never_pushes_the_operators_requests_out() {
        // Codex's goal reminder alone is about 6,400 characters.
        let reminder = format!("{}끝", "reminder ".repeat(800));
        let events = vec![
            human("첫 요청 문장", 1),
            assistant(&reminder, 2),
            human("두 번째 요청", 3),
            assistant(&reminder, 4),
        ];
        let initial = analysis_context(&events, &|_| true);
        let rolling = rolling_analysis_context("이전 작업 제목", "", &events[2..3], &events);
        for (context, kept) in [
            (&initial, "user: 첫 요청 문장"),
            (&rolling, "<previous-goal>이전 작업 제목</previous-goal>"),
        ] {
            assert_eq!(context.chars().count(), MAX_ANALYSIS_CONTEXT_CHARS);
            let (requests, exchange) = sections(context);
            assert!(requests.contains(kept), "{requests}");
            assert!(requests.contains("user: 두 번째 요청"), "{requests}");
            assert!(
                exchange.ends_with("끝\n</latest-exchange>"),
                "the exchange keeps its tail"
            );
        }
    }

    #[test]
    fn long_requests_keep_their_head_within_half_the_context() {
        let paste = format!("첫 요청 머리 {}", "붙여넣은 본문 ".repeat(600));
        let events = vec![
            human(&paste, 1),
            assistant("착수", 2),
            human("마지막 요청", 3),
            assistant("마지막 답", 4),
        ];
        for context in [
            analysis_context(&events, &|_| true),
            rolling_analysis_context("이전 작업 제목", "", &events, &events),
        ] {
            let (requests, exchange) = sections(&context);
            assert!(requests.chars().count() <= MAX_REQUEST_SECTION_CHARS);
            assert!(requests.contains("user: 첫 요청 머리"), "{requests}");
            assert!(
                requests.ends_with("…\n</initial-operator-requests>")
                    || requests.ends_with("…\n</new-operator-requests>"),
                "{requests}"
            );
            assert!(exchange.contains("assistant: 마지막 답"));
        }
    }

    #[test]
    fn a_long_stored_goal_cannot_break_the_bound() {
        let goal = "이전 빌드가 남긴 긴 목표 ".repeat(400);
        let answer = "긴 답 ".repeat(2_000);
        let events = vec![human("새 요청", 1), assistant(&answer, 2)];
        let context = rolling_analysis_context(&goal, "", &events[..1], &events);
        assert!(context.chars().count() <= MAX_ANALYSIS_CONTEXT_CHARS);
        assert!(sections(&context).0.contains("user: 새 요청"));
    }

    #[test]
    fn short_requests_leave_the_rest_of_the_bound_to_the_exchange() {
        let answer = "긴 답 ".repeat(2_000);
        let events = vec![human("짧은 요청", 1), assistant(&answer, 2)];
        let context = analysis_context(&events, &|_| true);
        let (requests, exchange) = sections(&context);
        assert!(requests.chars().count() < 200);
        assert_eq!(context.chars().count(), MAX_ANALYSIS_CONTEXT_CHARS);
        assert!(exchange.chars().count() > MAX_ANALYSIS_CONTEXT_CHARS - 200);
    }

    #[test]
    fn turn_identity_holds_while_output_grows_and_moves_only_at_a_boundary() {
        let mut events = vec![human("first", 1), assistant("a", 2)];
        let key = turn_key(&events);
        events.push(assistant("b", 3));
        assert_eq!(turn_key(&events), key);
        events.push(human("second", 4));
        assert_ne!(turn_key(&events), key);
        let key = turn_key(&events);
        events.extend([assistant("c", 5), human("second", 6)]);
        assert_ne!(
            turn_key(&events),
            key,
            "the same words again begin a new turn"
        );
        assert_eq!(turn_key(&[assistant("only output", 1)]), None);
    }

    #[test]
    fn the_phase_stays_offered_until_its_call_actually_lands() {
        assert_eq!(
            analysis_phase(true, false, false, false),
            Some(AnalysisPhase::TurnStart)
        );
        assert_eq!(analysis_phase(false, true, true, false), None);
        assert_eq!(
            analysis_phase(false, false, true, false),
            Some(AnalysisPhase::TurnEnd)
        );
        assert_eq!(analysis_phase(false, false, true, true), None);
    }

    #[test]
    fn masking_removes_secrets_emails_and_home_accounts_and_keeps_prose() {
        let context = analysis_context(
            &[human(
                "token=abc123 메일 me@example.com 파일 /Users/example/project 그대로 두기",
                1,
            )],
            &|_| true,
        );
        assert!(context.contains("[redacted-secret]"));
        assert!(context.contains("[redacted-personal]"));
        assert!(context.contains("~/project"));
        assert!(context.contains("그대로 두기"));
        assert!(!context.contains("abc123"));
        assert!(!context.contains("me@example.com"));
        assert!(!context.contains("/Users/example"));
    }

    #[test]
    fn home_accounts_are_hidden_and_path_tails_are_kept_on_every_system() {
        let spaced_windows_path = [
            "d:",
            "users",
            "Example Account",
            "worktrees/fix-search",
            "brief.md",
        ]
        .join("\\");
        let spaced_windows_home = ["d:", "users", "Example Account"].join("\\");
        let apostrophe_windows_home = ["C:", "Users", "Example'Account"].join("\\");
        let apostrophe_windows_path =
            [apostrophe_windows_home.as_str(), "rest", "brief.md"].join("\\");
        let mut failures = Vec::new();
        for (request, expected_request) in [
            (
                "/Users/example/worktrees/fix-search/agents/runs/fix-search/brief.md",
                "~/worktrees/fix-search/agents/runs/fix-search/brief.md",
            ),
            (
                "/home/example/worktrees/fix-search/agents/runs/fix-search/brief.md",
                "~/worktrees/fix-search/agents/runs/fix-search/brief.md",
            ),
            (
                r"C:\Users\example\worktrees\fix-search\agents\runs\fix-search\brief.md",
                r"~/worktrees\fix-search\agents\runs\fix-search\brief.md",
            ),
            (
                "C:/Users/example/worktrees/fix-search/agents/runs/fix-search/brief.md",
                "~/worktrees/fix-search/agents/runs/fix-search/brief.md",
            ),
            (
                spaced_windows_path.as_str(),
                r"~/worktrees/fix-search\brief.md",
            ),
            (spaced_windows_home.as_str(), "~"),
            (
                "/Users/example/한글 폴더/example/brief.md",
                "~/한글 폴더/example/brief.md",
            ),
            (apostrophe_windows_path.as_str(), r"~/rest\brief.md"),
            (apostrophe_windows_home.as_str(), "~"),
            (
                "/Users/example/notes /Users/alice/brief.md",
                "~/notes /Users/alice/brief.md",
            ),
        ]
        .map(|(path, expected)| {
            (
                format!("Read \"{path}\" and execute it."),
                format!("Read \"{expected}\" and execute it."),
            )
        })
        .into_iter()
        .chain(
            [
                ("cd /Users/example", "cd ~"),
                (concat!("홈은 /Users/example", "."), "홈은 ~."),
                (
                    "open file:///Users/example/x/brief.md",
                    "open file://~/x/brief.md",
                ),
                ("cwd=/home/alice", "cwd=~"),
                (r"cd C:\Users\example", "cd ~"),
                ("cwd=C:/Users/example", "cwd=~"),
                (concat!("홈은 /home/alice", "."), "홈은 ~."),
                (concat!(r"홈은 C:\Users\example", "."), "홈은 ~."),
                (
                    concat!(r"Compare C:\Users\example", ", /home/alice/report.md"),
                    "Compare ~, ~/report.md",
                ),
                (
                    concat!(r"Compare C:\Users\example", " and /tmp/report.md"),
                    "Compare ~ and /tmp/report.md",
                ),
                (
                    concat!(r"Compare C:\Users\example", r" and C:\Temp\report.md"),
                    r"Compare ~ and C:\Temp\report.md",
                ),
                (
                    "open file:///C:/Users/example/x/brief.md",
                    "open file:///~/x/brief.md",
                ),
                (
                    "Read '/home/alice/tasks/brief.md' and execute it.",
                    "Read '~/tasks/brief.md' and execute it.",
                ),
                (
                    "Read `C:/Users/example/tasks/brief.md` and execute it.",
                    "Read `~/tasks/brief.md` and execute it.",
                ),
            ]
            .map(|(request, expected)| (request.to_owned(), expected.to_owned())),
        )
        .chain([
            (
                format!("Read {spaced_windows_path} and execute it."),
                r"Read ~/worktrees/fix-search\brief.md and execute it.".to_owned(),
            ),
            (
                format!("Read {apostrophe_windows_path} and execute it."),
                r"Read ~/rest\brief.md and execute it.".to_owned(),
            ),
        ]) {
            let events = [human(&request, 1)];
            for context in [
                analysis_context(&events, &|_| true),
                rolling_analysis_context("기존 작업 결과물", "", &events, &events),
            ] {
                if !context.contains(&expected_request) || context.contains(&request) {
                    failures.push(format!(
                        "{request} -> {context}; expected {expected_request}"
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn paths_outside_account_homes_are_kept_as_written() {
        let request = concat!(
            "/tmp/example/project/brief.md /var/folders/example/brief.md ",
            "/etc/example/config.conf agents/runs/fix-search/brief.md ",
            "repo/Users/example/brief.md /tmp/Users/example/brief.md ",
            "file:///tmp/Users/example/brief.md repo/file:///Users/example/brief.md ",
            r"C:\Temp\example\brief.md repo\Users\example\brief.md"
        );
        let events = [human(request, 1)];
        for context in [
            analysis_context(&events, &|_| true),
            rolling_analysis_context("기존 작업 결과물", "", &events, &events),
        ] {
            assert!(context.contains(request), "{context}");
        }
    }

    #[test]
    fn retention_keeps_the_human_turns_the_context_needs() {
        let mut events = VecDeque::new();
        for index in 0..20 {
            events.push_back(human(&format!("h{index}"), index * 2));
            events.push_back(assistant(&format!("a{index}"), index * 2 + 1));
        }
        retain_bounded(&mut events);
        let humans = events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Human)
            .map(|event| event.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(humans.first().map(String::as_str), Some("h0"));
        assert_eq!(humans.last().map(String::as_str), Some("h19"));
        assert_eq!(humans.len(), 3 + 8);
        assert_eq!(events.back().map(|event| event.text.as_str()), Some("a19"));
    }

    #[test]
    fn a_request_refused_because_hide_ai_is_off_waits_instead_of_being_parked() {
        assert_eq!(
            AnalysisFailure::Provider(AiError::Disabled).retry_after(),
            Some(PROVIDER_RECOVERY_INTERVAL),
            "a turn is asked again once Hide AI is on; it is not given up on (B33)"
        );
    }
}
