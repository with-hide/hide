//! What the hook keeps of the pane's plan and mission between events (PRD
//! hide-plan D-14, D-15, D-30).
//!
//! The core hands a small summary ([`PlanContext`]) with the answer to the
//! prompt hook's letter pull and with every `hide plan` command's answer;
//! the hook or the CLI writes it to the pane's record here. The prompt hook
//! turns it into one line of session context ([`hint`]), and the pre-tool
//! hook reads only this record to refuse the three things a plan or a
//! mission rules out ([`refusal`]): a tool call never asks the core.
//!
//! A pane with no record is not a Hide pane as far as this module knows, and
//! gets neither a line nor a refusal. A record of another session than the
//! call's is ignored, so a new session in a reused pane starts clean.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::guardrails::Action;
use crate::spawn_guard::{command_words, shell_line};

/// The record's version; one of another version is not read.
const RECORD_VERSION: u32 = 1;
/// The largest record read. A summary is a few hundred bytes.
const RECORD_LIMIT: u64 = 16 * 1024;
/// A plan not updated for this long gets the command that updates it on
/// the hint (D-14).
pub const STALE_AFTER_MS: u64 = 30 * 60 * 1000;

/// The summary the core sends: fixed and small, the same shape on every
/// path that carries it (`contracts/delivery-envelope.json`, `plan_context`).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContext {
    /// The session's own plan, absent when it has none.
    pub plan: Option<PlanSummary>,
    /// The mission node this session works on as a worker.
    pub mission: Option<MissionWork>,
    /// The mission this session conducts.
    pub conductor: Option<ConductorWork>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSummary {
    /// `approved` or `proposed`.
    pub review: String,
    pub current: Option<StepName>,
    pub next: Option<StepName>,
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    pub updated_at_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepName {
    pub key: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MissionWork {
    pub mission: String,
    pub node: String,
    /// `pr` or `merge`: a `pr` mission's worker never merges (D-15).
    pub scope: String,
    /// The node's finish rule, or its title when it has none.
    pub range: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConductorWork {
    pub mission: String,
    pub done: u32,
    pub nodes: u32,
    pub working: u32,
    pub letters: u32,
    pub decision: bool,
}

impl PlanContext {
    pub fn proposed(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.review == "proposed" && !plan.finished)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Record {
    version: u32,
    /// The native session the summary is for.
    session: String,
    context: PlanContext,
}

/// Where the pane's record lives, under Hide's own directory.
fn record_path(home: &Path, pane_id: &str) -> PathBuf {
    let safe: String = pane_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    home.join(".hide")
        .join("agent-hooks")
        .join("plans")
        .join(format!("{safe}.json"))
}

/// Writes the pane's record whole, private to the account.
pub fn write(
    home: &Path,
    pane_id: &str,
    session: &str,
    context: &PlanContext,
) -> std::io::Result<()> {
    let path = record_path(home, pane_id);
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(&Record {
        version: RECORD_VERSION,
        session: session.to_owned(),
        context: context.clone(),
    })?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
}

/// The pane's record for `session`, if the hook wrote one for it.
pub fn read(home: &Path, pane_id: &str, session: Option<&str>) -> Option<PlanContext> {
    use std::io::Read;
    let file = std::fs::File::open(record_path(home, pane_id)).ok()?;
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return None;
    }
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    (record.version == RECORD_VERSION && session.is_none_or(|session| session == record.session))
        .then_some(record.context)
}

/// How long ago, in the words the hint uses.
fn elapsed(since: u64, now: u64) -> String {
    let minutes = now.saturating_sub(since) / 60_000;
    match minutes {
        0 => "방금 갱신".to_owned(),
        1..=59 => format!("{minutes}분 전 갱신"),
        _ => format!("{}시간 전 갱신", minutes / 60),
    }
}

/// The session context the prompt hook adds for `context` at `now`: one
/// line for the plan, one for the mission node or the mission conducted,
/// and nothing for a pane with neither (B17, B18). A finished plan stops
/// its line (B9).
pub fn hint(context: &PlanContext, now: u64) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(plan) = context.plan.as_ref().filter(|plan| !plan.finished) {
        let mut line = format!(
            "계획: 지금 {}",
            plan.current
                .as_ref()
                .map_or("-", |step| step.title.as_str())
        );
        line.push_str(&format!(" ({}/{})", plan.done, plan.total));
        if let Some(next) = &plan.next {
            line.push_str(&format!(" · 다음 {}", next.title));
        }
        line.push_str(&format!(" · {}", elapsed(plan.updated_at_unix_ms, now)));
        lines.push(line);
        if context.proposed() {
            lines.push(
                "계획 검토 기다림: 운영자가 대화에서 답하기 전에는 파일 쓰기, git commit, git push가 막힌다. 승인하면 hide plan approve, 고치라면 파일 대신 hide plan set --json '<계획>' --ask로 다시 올리고, 하지 말라면 hide plan drop.".to_owned(),
            );
        } else if now.saturating_sub(plan.updated_at_unix_ms) > STALE_AFTER_MS {
            let key = plan
                .current
                .as_ref()
                .map_or("<key>", |step| step.key.as_str());
            lines.push(format!(
                "끝낸 단계는 `hide plan node {key} --status done`, 하는 단계는 `--status working`으로 갱신한다."
            ));
        }
    }
    if let Some(work) = &context.mission {
        let mut line = format!(
            "미션 {} · 노드 {} · 범위 {}",
            work.mission, work.node, work.range
        );
        if work.scope == "pr" {
            line.push_str(" · 머지 금지");
        }
        line.push_str(" · 에이전트 띄우기 금지");
        lines.push(line);
    }
    if let Some(conductor) = &context.conductor {
        lines.push(format!(
            "미션 {}/{} · 일하는 중 {} · 편지 {}통 · 결정 기다림 {}",
            conductor.done,
            conductor.nodes,
            conductor.working,
            conductor.letters,
            if conductor.decision {
                "있음"
            } else {
                "없음"
            }
        ));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Which of the three refusals a call meets (D-15).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refused {
    /// A file write, `git commit` or `git push` while the plan waits for
    /// the operator's review.
    ProposedPlan,
    /// `gh pr merge` by a worker of a mission that goes up to pull requests.
    MissionMerge,
    /// `git push --force*` or `git rebase`, in any Hide pane.
    HistoryRewrite,
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Self::ProposedPlan => "plan_proposed",
            Self::MissionMerge => "mission_merge",
            Self::HistoryRewrite => "history_rewrite",
        }
    }

    /// The one line the agent reads, in its runtime's deny shape.
    pub fn reason(self) -> &'static str {
        match self {
            Self::ProposedPlan => {
                "Not run: 계획 검토 기다림 · 운영자가 답한 뒤 hide plan approve. Reading and looking around still run; write, commit and push wait for the operator's answer. A revised plan goes up with hide plan set --json '<plan>' --ask."
            }
            Self::MissionMerge => {
                "Not run: this mission goes up to pull requests, so its workers do not merge. Report the pull request to the Conductor; the operator merges."
            }
            Self::HistoryRewrite => {
                "Not run: Hide panes do not rewrite pushed history. Push a new commit instead of a force push, and merge the base into the branch instead of rebasing."
            }
        }
    }
}

/// One refusal, as one line in the guard's log: which rule, the runtime and
/// the pane; never the command.
pub fn record_refusal(home: &Path, runtime: &str, pane: &str, refused: Refused) {
    let line = serde_json::json!({
        "component": "plans",
        "kind": "plan.refused",
        "rule": refused.code(),
        "runtime": runtime,
        "pane": pane,
        "at_unix_ms": crate::spawn_guard::now_ms(),
    })
    .to_string();
    let _ = crate::spawn_guard::append_line(&crate::spawn_guard::log_path(home), &line);
}

/// The refusal `action` meets under `context`, if any. Reading, searching
/// and every other shell command run (D-15).
pub fn refusal(context: &PlanContext, action: &Action) -> Option<Refused> {
    let proposed = context.proposed();
    let merge_barred = context
        .mission
        .as_ref()
        .is_some_and(|work| work.scope == "pr");
    match action {
        Action::Write(_) | Action::UnreadWrite if proposed => Some(Refused::ProposedPlan),
        Action::Shell(command) => shell(command, proposed, merge_barred),
        _ => None,
    }
}

fn program(word: &str) -> String {
    let name = Path::new(word)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.to_ascii_lowercase().ends_with(".exe") {
        name[..name.len() - 4].to_owned()
    } else {
        name
    }
}

/// The subcommand and its arguments of a `git` call, after Git's own
/// options.
fn git_subcommand(args: &[String]) -> Option<(&str, &[String])> {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        match arg.as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
            | "--config-env" | "--super-prefix" => at += 2,
            other if other.starts_with('-') => at += 1,
            _ => break,
        }
    }
    Some((args.get(at)?.as_str(), &args[at + 1..]))
}

fn shell(command: &str, proposed: bool, merge_barred: bool) -> Option<Refused> {
    let line = shell_line(command);
    for words in &line.commands {
        let (words, _) = command_words(words, &|_| None, true);
        let Some((first, args)) = words.split_first() else {
            continue;
        };
        match program(first).as_str() {
            "git" => {
                let Some((subcommand, rest)) = git_subcommand(args) else {
                    continue;
                };
                let force = rest.iter().any(|arg| {
                    arg.starts_with("--force")
                        || (arg.starts_with('-')
                            && !arg.starts_with("--")
                            && arg[1..].contains('f'))
                        || (arg.starts_with('+') && arg.len() > 1)
                });
                match subcommand {
                    "push" if force => return Some(Refused::HistoryRewrite),
                    "rebase"
                        if !rest
                            .iter()
                            .any(|arg| matches!(arg.as_str(), "--abort" | "--quit")) =>
                    {
                        return Some(Refused::HistoryRewrite);
                    }
                    "commit" | "push" if proposed => return Some(Refused::ProposedPlan),
                    _ => {}
                }
            }
            "gh" if merge_barred
                && args.first().map(String::as_str) == Some("pr")
                && args.get(1).map(String::as_str) == Some("merge") =>
            {
                return Some(Refused::MissionMerge);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(review: &str, updated: u64) -> PlanSummary {
        PlanSummary {
            review: review.to_owned(),
            current: Some(StepName {
                key: "core".into(),
                title: "Core events".into(),
            }),
            next: Some(StepName {
                key: "web".into(),
                title: "Web map".into(),
            }),
            done: 1,
            total: 4,
            finished: false,
            updated_at_unix_ms: updated,
        }
    }

    fn shell_action(command: &str) -> Action {
        Action::Shell(command.to_owned())
    }

    #[test]
    fn a_proposed_plan_refuses_writes_commits_and_pushes_and_lets_reading_run() {
        let context = PlanContext {
            plan: Some(plan("proposed", 0)),
            ..PlanContext::default()
        };
        assert_eq!(
            refusal(&context, &Action::Write(vec!["src/main.rs".into()])),
            Some(Refused::ProposedPlan)
        );
        assert_eq!(
            refusal(&context, &shell_action("cd repo && git commit -m 'x'")),
            Some(Refused::ProposedPlan)
        );
        assert_eq!(
            refusal(&context, &shell_action("git -C repo push origin HEAD")),
            Some(Refused::ProposedPlan)
        );
        for allowed in ["git status", "rg plan src", "cat README.md", "git log -3"] {
            assert_eq!(refusal(&context, &shell_action(allowed)), None, "{allowed}");
        }
        assert_eq!(refusal(&context, &Action::Read(vec!["a".into()])), None);
    }

    #[test]
    fn an_approved_or_absent_plan_refuses_only_history_rewrites() {
        let approved = PlanContext {
            plan: Some(plan("approved", 0)),
            ..PlanContext::default()
        };
        assert_eq!(
            refusal(&approved, &shell_action("git commit -m x && git push")),
            None
        );
        for rewrite in [
            "git push --force origin b",
            "git push -f",
            "git push --force-with-lease",
            "git push origin +b",
            "git rebase main",
        ] {
            assert_eq!(
                refusal(&PlanContext::default(), &shell_action(rewrite)),
                Some(Refused::HistoryRewrite),
                "{rewrite}"
            );
        }
        assert_eq!(
            refusal(&PlanContext::default(), &shell_action("git rebase --abort")),
            None
        );
    }

    #[test]
    fn only_a_pr_scope_worker_is_refused_a_merge() {
        let worker = |scope: &str| PlanContext {
            mission: Some(MissionWork {
                mission: "m1".into(),
                node: "core".into(),
                scope: scope.into(),
                range: "PR까지".into(),
            }),
            ..PlanContext::default()
        };
        assert_eq!(
            refusal(&worker("pr"), &shell_action("gh pr merge 12 --squash")),
            Some(Refused::MissionMerge)
        );
        assert_eq!(
            refusal(&worker("merge"), &shell_action("gh pr merge 12")),
            None
        );
        assert_eq!(refusal(&worker("pr"), &shell_action("gh pr view 12")), None);
    }

    #[test]
    fn the_hint_names_the_current_and_next_step_and_adds_the_update_command_when_stale() {
        let fresh = PlanContext {
            plan: Some(plan("approved", 0)),
            ..PlanContext::default()
        };
        assert_eq!(
            hint(&fresh, 5 * 60_000).unwrap(),
            "계획: 지금 Core events (1/4) · 다음 Web map · 5분 전 갱신"
        );
        let stale = hint(&fresh, 31 * 60_000).unwrap();
        assert!(
            stale.ends_with(
                "`hide plan node core --status done`, 하는 단계는 `--status working`으로 갱신한다."
            ),
            "{stale}"
        );
        let mut finished = plan("approved", 0);
        finished.finished = true;
        let done = PlanContext {
            plan: Some(finished),
            ..PlanContext::default()
        };
        assert_eq!(hint(&done, 0), None);
        assert_eq!(hint(&PlanContext::default(), 0), None);
    }

    /// The hook's line and refusals for each summary the core sends
    /// (`contracts/delivery-envelope.json`, `plan_context`).
    #[test]
    fn the_hint_and_refusals_match_the_delivery_envelope_contract() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../contracts/delivery-envelope.json")).unwrap();
        for example in contract["plan_context"]["examples"].as_array().unwrap() {
            let name = &example["name"];
            let context: PlanContext =
                serde_json::from_value(example["context"].clone()).expect("the fixed shape");
            assert_eq!(
                serde_json::to_value(&context).unwrap(),
                example["context"],
                "{name}"
            );
            let now = example["now_unix_ms"].as_u64().unwrap();
            assert_eq!(
                hint(&context, now).as_deref(),
                example["hint"].as_str(),
                "{name}"
            );
            for call in example["commands"].as_array().unwrap() {
                let input = call["input"].as_str().unwrap().to_owned();
                let action = match call["tool"].as_str().unwrap() {
                    "shell" => Action::Shell(input),
                    "write" => Action::Write(vec![input]),
                    other => panic!("unknown tool {other}"),
                };
                assert_eq!(
                    refusal(&context, &action).map(Refused::code),
                    call["refused"].as_str(),
                    "{name}: {action:?}"
                );
            }
        }
    }

    #[test]
    fn a_record_is_read_back_only_for_its_own_session() {
        let home = tempfile::tempdir().unwrap();
        let context = PlanContext {
            plan: Some(plan("proposed", 7)),
            ..PlanContext::default()
        };
        write(home.path(), "w1:p2", "session-a", &context).unwrap();
        assert_eq!(
            read(home.path(), "w1:p2", Some("session-a")),
            Some(context.clone())
        );
        assert_eq!(read(home.path(), "w1:p2", None), Some(context));
        assert_eq!(read(home.path(), "w1:p2", Some("session-b")), None);
        assert_eq!(read(home.path(), "w9:p9", None), None);
    }
}
