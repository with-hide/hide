//! What one agent hook gets from Project Memory (PRD core-host-node-move
//! D-12, B14): the core answers it from the store it owns, for the Project of
//! the checkout the hook's pane is attested to, so the store is read on the
//! core's machine whichever machine's agent asks. Bounded by the caller's
//! expiry and failing open: any missing part answers with no Memory.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Injection, InjectionKind, InjectionOutcome, MemoryStore, RetrievalQuery};

/// The most of a prompt a hook sends the core to rank Memory by: its start,
/// which names the task, and few enough bytes that the request stays one
/// small message whichever machine the core runs on.
pub const PROMPT_LIMIT_BYTES: usize = 8 * 1024;
/// Estimated rendered tokens, including escaping, wrappers and receipts.
/// This is a local estimate, not provider billing.
pub const RENDERED_TOKEN_LIMIT: usize = 1_200;

pub fn estimated_tokens(text: &str) -> usize {
    // Per character class, never per byte, with a tenth on top so the
    // estimate stays at or above what current tokenizers spend:
    // - ASCII letters and spaces about four characters a token (never fewer
    //   tokens than words);
    // - ASCII digits and symbols, as in ids, hashes, paths and escapes, about
    //   two characters a token;
    // - a Hangul syllable, a Han or kana character, or any other character
    //   of the Basic Multilingual Plane, one token;
    // - a character beyond it (an emoji, a rare Han), two tokens.
    let (mut prose, mut symbols, mut wide) = (0, 0, 0);
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() || ch.is_ascii_whitespace() {
            prose += 1;
        } else if ch.is_ascii() {
            symbols += 1;
        } else if u32::from(ch) > 0xFFFF {
            wide += 2;
        } else {
            wide += 1;
        }
    }
    let words = text
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .count();
    let tokens = words.max(usize::div_ceil(prose, 4)) + usize::div_ceil(symbols, 2) + wide;
    tokens + tokens.div_ceil(10)
}

/// `prompt`'s first [`PROMPT_LIMIT_BYTES`] bytes at most, cut on a character
/// boundary.
pub fn prompt_prefix(prompt: &str) -> &str {
    prefix(prompt, PROMPT_LIMIT_BYTES)
}

/// `text`'s first `bytes` bytes at most, cut on a character boundary.
pub fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The two events Memory answers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
}

impl HookEvent {
    /// The event's name as receipts carry it.
    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
        }
    }
}

/// How a hook's Memory read ended, for the hook's own record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum HookOutcome {
    Provided { count: usize },
    Empty,
    Disabled,
    Unavailable,
    Deadline,
    ProjectUnresolved,
}

/// What one hook asks for.
pub struct HookRequest<'a> {
    /// The provider id Memory keys sessions and receipts by: `claude`,
    /// `codex`, `opencode`, `pi` or `omp`.
    pub runtime_id: &'a str,
    pub event: HookEvent,
    pub session_id: &'a str,
    pub prompt: Option<&'a str>,
    /// The confirmed task of the caller's checkout, which a brief prompt
    /// with no Memory of its own, such as "go on", is ranked with.
    pub task_context: Option<&'a str>,
}

/// The longest prompt read as continuing its task rather than naming one.
pub const CONTINUATION_PROMPT_CHARS: usize = 80;

/// The Memory context for one hook, with its receipt, or none.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HookContext {
    pub context: Option<String>,
    #[serde(flatten)]
    pub outcome: HookOutcome,
}

impl HookContext {
    fn without(outcome: HookOutcome) -> Self {
        Self {
            context: None,
            outcome,
        }
    }
}

/// The context `store_path`'s Memory gives `request` for `project`, or none
/// with why; `expired` bounds every step.
pub fn context_for<E>(
    store_path: &Path,
    project: &hide_project::ProjectIdentity,
    request: HookRequest<'_>,
    expired: E,
) -> HookContext
where
    E: Fn() -> bool + Clone + Send + 'static,
{
    let started = std::time::Instant::now();
    let caller_expired = expired;
    let expired = move || {
        caller_expired()
            || started.elapsed() >= std::time::Duration::from_millis(crate::HOOK_DEADLINE_MS)
    };
    context_until(store_path, project, request, expired)
}

/// One absolute invocation expiry owns opening, SQL work and rendering.
fn context_until<E>(
    store_path: &Path,
    project: &hide_project::ProjectIdentity,
    request: HookRequest<'_>,
    expired: E,
) -> HookContext
where
    E: Fn() -> bool + Clone + Send + 'static,
{
    let base = || HookContext::without(HookOutcome::Unavailable);
    let deadline = || HookContext::without(HookOutcome::Deadline);
    let Ok(store) = MemoryStore::open_hook_read_only_with_expiry(store_path, expired.clone())
    else {
        if expired() {
            return deadline();
        }
        return base();
    };
    if expired() {
        return deadline();
    }
    if request.session_id.is_empty() {
        return base();
    }
    let mut query = match request.event {
        HookEvent::SessionStart => RetrievalQuery::session_start(&project.id),
        HookEvent::UserPromptSubmit => {
            let excluded = match store.session_given_ids(
                &project.id,
                request.runtime_id,
                request.session_id,
            ) {
                Ok(Some(items)) => items,
                // A new session while the Project's receipts are full: nothing
                // could record what it is given, so it gets none (B26).
                Ok(None) | Err(_) => return base(),
            };
            RetrievalQuery::prompt(&project.id, request.prompt.unwrap_or_default(), excluded)
        }
    };
    // The public entry already composed the caller and 100 ms budgets before
    // opening. Keep that absolute expiry through retrieval rather than starting
    // a second wall clock; tests can own the same boundary with a logical clock.
    query.deadline = std::time::Duration::MAX;
    if expired() {
        return deadline();
    }
    let Ok(mut injection) = store.retrieve_with_expiry(&query, expired.clone()) else {
        if expired() {
            return deadline();
        }
        return base();
    };
    // Native hooks give text, not a continuation flag: a prompt that matches
    // any card, even one the session was already given, has its own topic.
    // Only a brief one that matches none is ranked again, on the same open
    // store, with its task. No language-specific words decide it.
    let prompt = request.prompt.unwrap_or_default();
    let continues = request.event == HookEvent::UserPromptSubmit
        && injection.matched == 0
        && !prompt.trim().is_empty()
        && prompt.chars().count() <= CONTINUATION_PROMPT_CHARS;
    if let Some(task) = request.task_context.filter(|_| continues) {
        query.text = prompt_prefix(&format!("{prompt}\n{task}")).to_owned();
        let Ok(continued) = store.retrieve_with_expiry(&query, expired.clone()) else {
            if expired() {
                return deadline();
            }
            return base();
        };
        injection = continued;
    }
    let Ok(generation) =
        store.context_generation(&project.id, request.runtime_id, request.session_id)
    else {
        return base();
    };
    // Budget the exact escaped envelope and a fixed-width authentication tag
    // before signing; removed items can never appear in its receipt.
    while !injection.items.is_empty() {
        let preview = render(
            request.event,
            injection.clone(),
            (injection.kind == InjectionKind::Bodies)
                .then_some("0000000000000000000000000000000000000000000000000000000000000000"),
            generation,
            &expired,
        );
        if preview
            .context
            .as_deref()
            .is_some_and(|text| estimated_tokens(text) <= RENDERED_TOKEN_LIMIT)
        {
            break;
        }
        injection.items.pop();
    }
    if injection.items.is_empty() && injection.outcome == InjectionOutcome::Provided {
        injection.outcome = InjectionOutcome::Empty;
    }
    // The index is not a delivery and leaves no receipt; only cards given in
    // full are recorded, so a session's prompts do not repeat them (B12).
    let receipt_auth = match injection.kind {
        InjectionKind::Index => None,
        InjectionKind::Bodies => match store.receipt_auth_tag(
            &project.id,
            request.runtime_id,
            request.session_id,
            &MemoryStore::context_receipt_event(request.event.name(), generation),
            &injection.item_key(),
        ) {
            Ok(tag) => Some(tag),
            Err(_) => return base(),
        },
    };
    render(
        request.event,
        injection,
        receipt_auth.as_deref(),
        generation,
        &expired,
    )
}

fn render(
    event: HookEvent,
    injection: Injection,
    receipt_auth: Option<&str>,
    generation: u64,
    expired: &impl Fn() -> bool,
) -> HookContext {
    if expired() {
        return HookContext::without(HookOutcome::Deadline);
    }
    let outcome = match injection.outcome {
        InjectionOutcome::Provided => HookOutcome::Provided {
            count: injection.items.len(),
        },
        InjectionOutcome::Empty => HookOutcome::Empty,
        InjectionOutcome::Disabled => HookOutcome::Disabled,
        InjectionOutcome::Deadline => HookOutcome::Deadline,
        InjectionOutcome::Unavailable | InjectionOutcome::Stale => HookOutcome::Unavailable,
    };
    let Some(mut context) = injection.context() else {
        return HookContext::without(outcome);
    };
    if let Some(auth) = receipt_auth {
        let generation_attribute = if generation == 0 {
            String::new()
        } else {
            format!(" generation=\"{generation}\"")
        };
        context.push_str(&format!(
            "<hide-memory-receipt event=\"{}\" count=\"{}\" items=\"{}\" auth=\"{}\"{} />\n",
            event.name(),
            injection.items.len(),
            injection.item_key(),
            auth,
            generation_attribute,
        ));
    }
    HookContext {
        context: Some(context),
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Policy checks own a logical invocation clock. The real SQL interruption
    // test below explicitly exercises the public wall-clock entry instead.
    use super::context_until as context_for;
    use crate::{AnalysisBatch, Candidate, CandidateKind, CandidateRelation};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const NODE: &str = "node-a";

    /// A clock that moves only when the test moves it, ending at the store's
    /// own hook deadline.
    fn invocation_clock() -> (Arc<AtomicU64>, impl Fn() -> bool + Clone + Send + 'static) {
        let anchor = Instant::now();
        let end = anchor + Duration::from_millis(crate::HOOK_DEADLINE_MS);
        let elapsed = Arc::new(AtomicU64::new(0));
        let clock = Arc::clone(&elapsed);
        let expired = move || anchor + Duration::from_millis(clock.load(Ordering::SeqCst)) >= end;
        (elapsed, expired)
    }

    fn frozen() -> impl Fn() -> bool + Clone + Send + 'static {
        invocation_clock().1
    }

    fn rule(text: &str, salience: f64) -> Candidate {
        Candidate {
            text: text.into(),
            title: String::new(),
            aliases: Vec::new(),
            kind: CandidateKind::Rule,
            confidence: 0.9,
            salience,
            source_offsets: vec![1],
            human_offsets: vec![1],
            source_times: Default::default(),
            cited_paths: Vec::new(),
            relation: CandidateRelation::New,
        }
    }

    /// A store at `root` holding the enabled Project at `checkout`, with
    /// `cards`.
    fn enabled_store(
        root: &Path,
        checkout: &Path,
        cards: &[Candidate],
    ) -> (PathBuf, hide_project::ProjectIdentity) {
        fs::create_dir_all(checkout).unwrap();
        let project = hide_project::resolve(checkout, NODE).unwrap();
        let path = crate::database_path(root);
        let mut store = MemoryStore::open(&path).unwrap();
        store
            .ensure_project(&project.id, &project.root, NODE)
            .unwrap();
        store.set_enabled(&project.id, true, true).unwrap();
        if !cards.is_empty() {
            store
                .apply_candidates(
                    &AnalysisBatch {
                        id: "fixture-batch".into(),
                        project_id: project.id.clone(),
                        provider: "codex".into(),
                        analysis_provider: "codex".into(),
                        session_id: "source-session".into(),
                        content_hash: "fixture-hash".into(),
                        created_at_unix_ms: 1,
                    },
                    cards,
                )
                .unwrap();
        }
        (path, project)
    }

    fn ask<'a>(event: HookEvent, session: &'a str, prompt: Option<&'a str>) -> HookRequest<'a> {
        HookRequest {
            runtime_id: "codex",
            event,
            session_id: session,
            prompt,
            task_context: None,
        }
    }

    #[test]
    fn multilingual_render_estimates_do_not_discount_unspaced_cjk_text() {
        for text in ["한글".repeat(700), "漢字".repeat(700), "かな".repeat(700)] {
            assert!(estimated_tokens(&text) >= text.chars().count());
        }
    }

    #[test]
    fn a_realistic_korean_card_is_delivered_within_the_budget() {
        // The operator writes mostly Korean: a card of 350 to 600 Hangul
        // characters, a few paragraphs of a rule, is one prompt's Memory.
        let sentence = "훅 응답은 백 밀리초 안에 끝나야 하고 느린 작업은 워커 스레드로 넘긴다. ";
        for sentences in [9, 12, 14] {
            let body = sentence.repeat(sentences);
            let characters = body.trim().chars().count();
            assert!((350..=600).contains(&characters), "{characters}");
            let temp = tempfile::tempdir().unwrap();
            let (path, project) = enabled_store(
                temp.path(),
                &temp.path().join("project"),
                &[rule(body.trim(), 0.9)],
            );
            let answer = context_for(
                &path,
                &project,
                ask(
                    HookEvent::UserPromptSubmit,
                    "s",
                    Some("훅 응답 시간을 줄여줘"),
                ),
                frozen(),
            );
            assert_eq!(
                answer.outcome,
                HookOutcome::Provided { count: 1 },
                "{characters} Hangul characters"
            );
        }
    }

    #[test]
    fn non_ascii_punctuation_in_english_costs_a_token_per_character_not_per_byte() {
        let plain = "Don't merge before \"the tests\" pass - ask first.";
        let typographic = "Don’t merge before “the tests” pass — ask first.";
        let extra = typographic.chars().filter(|ch| !ch.is_ascii()).count();
        assert_eq!(extra, 4);
        assert!(
            estimated_tokens(typographic) <= estimated_tokens(plain) + extra,
            "{} against {}",
            estimated_tokens(typographic),
            estimated_tokens(plain)
        );
    }

    #[test]
    fn ids_paths_escapes_and_emoji_are_not_counted_as_prose() {
        // Lower bounds a BPE tokenizer meets or exceeds: a hex digest
        // splits into pieces of about two or three characters, an emoji is
        // at least two tokens, an escape is at least two.
        let digest = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert!(estimated_tokens(digest) >= digest.len() / 3);
        let emoji = "🚀✅".repeat(50);
        assert!(estimated_tokens(&emoji) >= 2 * 50 + 50);
        let escaped = "&lt;hide-memory&gt; &amp; ".repeat(20);
        assert!(estimated_tokens(&escaped) >= escaped.len() / 3);
        let card = format!(
            "{} 경로는 `hide-memory/src/store/progress.rs:182`, 커밋 {digest} 🚀",
            "입력 실패는 하루 안에서만 연속으로 센다.".repeat(10)
        );
        let lower = card.chars().filter(|ch| !ch.is_ascii()).count()
            + card
                .chars()
                .filter(|ch| ch.is_ascii() && !ch.is_ascii_alphabetic() && *ch != ' ')
                .count()
                / 3;
        assert!(estimated_tokens(&card) >= lower);
    }

    #[test]
    fn a_prompt_sharing_no_word_or_alias_gets_no_memory() {
        // B12
        let temp = tempfile::tempdir().unwrap();
        let (path, project) = enabled_store(
            temp.path(),
            &temp.path().join("project-memory"),
            &[rule(
                "Project Memory provider work uses a bounded queue",
                0.9,
            )],
        );
        let prompt = |text| {
            context_for(
                &path,
                &project,
                ask(HookEvent::UserPromptSubmit, "unrelated-session", Some(text)),
                frozen(),
            )
        };

        let result = prompt("오늘 점심 메뉴 추천해줘");
        assert_eq!(result.outcome, HookOutcome::Empty);
        assert!(result.context.is_none());
        let result = prompt("the provider queue is stuck");
        assert_eq!(result.outcome, HookOutcome::Provided { count: 1 });
    }

    #[test]
    fn a_prompt_omits_cards_the_session_was_already_given_in_full() {
        // B12: a card given at an earlier prompt is not given again.
        let temp = tempfile::tempdir().unwrap();
        let cards = (0..6)
            .map(|index| rule(&format!("Rule {index} about durable hooks"), 0.9))
            .collect::<Vec<_>>();
        let (path, project) = enabled_store(temp.path(), &temp.path().join("project"), &cards);
        MemoryStore::open(&path)
            .unwrap()
            .record_receipt(
                &project.id,
                "codex",
                "fixture-session",
                "event:9",
                "m1@1",
                2,
            )
            .unwrap();

        let result = context_for(
            &path,
            &project,
            ask(
                HookEvent::UserPromptSubmit,
                "fixture-session",
                Some("durable hooks"),
            ),
            frozen(),
        );

        assert_eq!(result.outcome, HookOutcome::Provided { count: 3 });
        let context = result.context.unwrap();
        assert!(!context.contains("Rule 0 "), "{context}");
        assert!(
            context.contains("m2@1"),
            "the receipt names cards by number: {context}"
        );
    }

    #[test]
    fn session_start_gives_the_index_without_a_receipt_and_the_first_prompt_gets_memory() {
        // B10, B13: the index is not a delivery, so it records nothing and
        // the first prompt does not wait for it.
        let temp = tempfile::tempdir().unwrap();
        let mut cards = (0..22)
            .map(|index| {
                rule(
                    &format!("Race rule {index} about durable hooks"),
                    1.0 - f64::from(index) / 100.0,
                )
            })
            .collect::<Vec<_>>();
        cards.push(Candidate {
            kind: CandidateKind::Fact,
            ..rule("A fact about durable hooks", 1.0)
        });
        let (path, project) = enabled_store(temp.path(), &temp.path().join("project"), &cards);
        let session = "immediate-session";

        let start = context_for(
            &path,
            &project,
            ask(HookEvent::SessionStart, session, None),
            frozen(),
        );
        assert_eq!(start.outcome, HookOutcome::Provided { count: 20 });
        let start = start.context.unwrap();
        assert!(start.contains("hide memory show"));
        assert!(start.contains("m1 Race rule 0 about durable hooks"));
        assert!(!start.contains("A fact about"));
        assert!(!start.contains("Race rule 21 "));
        assert!(!start.contains("hide-memory-receipt"));

        let prompt = context_for(
            &path,
            &project,
            ask(HookEvent::UserPromptSubmit, session, Some("durable hooks")),
            frozen(),
        );
        assert_eq!(prompt.outcome, HookOutcome::Provided { count: 3 });
        assert!(prompt.context.unwrap().contains("hide-memory-receipt"));
    }

    #[test]
    fn a_linked_worktree_checkout_reaches_its_main_checkouts_memory() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        fs::create_dir_all(&main).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&main)
                    .status()
                    .unwrap()
                    .success(),
                "git {args:?}"
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        fs::write(main.join("README.md"), "fixture\n").unwrap();
        git(&["add", "."]);
        git(&["-c", "commit.gpgsign=false", "commit", "-qm", "init"]);
        git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ]);

        // The pane's checkout names the Project, whose root is the main one.
        let (path, project) = enabled_store(
            temp.path(),
            &linked,
            &[rule("Keep the local projection bounded", 0.9)],
        );
        assert_eq!(project.checkout_root, fs::canonicalize(&linked).unwrap());
        assert_eq!(project.root, fs::canonicalize(&main).unwrap());

        let result = context_for(
            &path,
            &project,
            ask(
                HookEvent::UserPromptSubmit,
                "linked-session",
                Some("projection 정리"),
            ),
            frozen(),
        );

        assert_eq!(result.outcome, HookOutcome::Provided { count: 1 });
        assert!(
            result
                .context
                .unwrap()
                .contains("Keep the local projection bounded")
        );
    }

    #[test]
    fn receipt_suppression_follows_revisions_and_native_context_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let (path, project) = enabled_store(
            temp.path(),
            &temp.path().join("project"),
            &[rule("Keep durable hooks bounded", 0.9)],
        );
        let mut writer = MemoryStore::open(&path).unwrap();
        let provided = || {
            context_for(
                &path,
                &project,
                ask(HookEvent::UserPromptSubmit, "s", Some("durable hooks")),
                frozen(),
            )
        };
        assert_eq!(provided().outcome, HookOutcome::Provided { count: 1 });
        writer
            .record_receipt(&project.id, "codex", "s", "event:1", "m1@1", 2)
            .unwrap();
        assert_eq!(provided().outcome, HookOutcome::Empty);
        let item = writer.item_by_number(&project.id, 1).unwrap().unwrap();
        writer
            .apply_candidates(
                &AnalysisBatch {
                    id: "revision".into(),
                    project_id: project.id.clone(),
                    provider: "codex".into(),
                    analysis_provider: "codex".into(),
                    session_id: "source-session".into(),
                    content_hash: "revision".into(),
                    created_at_unix_ms: 3,
                },
                &[Candidate {
                    relation: CandidateRelation::Supersedes { target_id: item.id },
                    ..rule("Keep durable hooks bounded after restart", 0.9)
                }],
            )
            .unwrap();
        assert_eq!(provided().outcome, HookOutcome::Provided { count: 1 });
        writer
            .record_receipt(&project.id, "codex", "s", "event:2", "m1@2", 4)
            .unwrap();
        assert_eq!(provided().outcome, HookOutcome::Empty);
        writer
            .observe_context_reset(&project.id, "codex", "s", 50)
            .unwrap();
        writer
            .observe_context_reset(&project.id, "codex", "s", 10)
            .unwrap();
        assert_eq!(
            writer
                .context_generation(&project.id, "codex", "s")
                .unwrap(),
            51
        );
        let context = provided().context.unwrap();
        assert!(context.contains("generation=\"51\""));
        let auth = context
            .split("auth=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(
            writer
                .verify_receipt_auth(
                    &project.id,
                    "codex",
                    "s",
                    &MemoryStore::context_receipt_event("UserPromptSubmit", 51),
                    "m1@2",
                    auth
                )
                .unwrap()
        );
        assert!(
            !writer
                .verify_receipt_auth(&project.id, "codex", "s", "UserPromptSubmit", "m1@2", auth)
                .unwrap()
        );
        writer
            .record_receipt(&project.id, "codex", "s", "event:52", "m1@2", 5)
            .unwrap();
        writer
            .record_receipt_context(&project.id, "codex", "s", "event:52", 51)
            .unwrap();
        assert_eq!(provided().outcome, HookOutcome::Empty);
        assert_eq!(
            context_for(
                &path,
                &project,
                ask(HookEvent::UserPromptSubmit, "other", Some("durable hooks")),
                frozen()
            )
            .outcome,
            HookOutcome::Provided { count: 1 }
        );
    }

    #[test]
    fn escaped_index_and_bodies_include_the_envelope_in_the_shared_budget() {
        let temp = tempfile::tempdir().unwrap();
        let cards = (0..22)
            .map(|index| Candidate {
                // Escapes cost about a token per two symbols, so an
                // escape-heavy card is sized to fit the budget alone.
                title: format!("{index} {}", "<durable&hooks>".repeat(8)),
                ..rule(&format!("durable hooks {}", "<&>".repeat(60)), 0.9)
            })
            .collect::<Vec<_>>();
        let (path, project) = enabled_store(temp.path(), &temp.path().join("project"), &cards);
        for event in [HookEvent::SessionStart, HookEvent::UserPromptSubmit] {
            let answer = context_for(
                &path,
                &project,
                ask(event, "s", Some("durable hooks")),
                frozen(),
            );
            let text = answer.context.expect("actual Provided context");
            assert!(
                estimated_tokens(&text) <= RENDERED_TOKEN_LIMIT,
                "{}",
                estimated_tokens(&text)
            );
            assert!(matches!(answer.outcome,HookOutcome::Provided{count} if count>0));
        }
    }

    #[test]
    fn unspaced_korean_and_cjk_bodies_share_the_exact_rendered_budget() {
        let temp = tempfile::tempdir().unwrap();
        let cards = (0..12)
            .map(|index| {
                rule(
                    &format!("durable hooks {index} {}", "한글漢字かな".repeat(25)),
                    0.9,
                )
            })
            .collect::<Vec<_>>();
        let (path, project) = enabled_store(temp.path(), &temp.path().join("project"), &cards);
        for event in [HookEvent::SessionStart, HookEvent::UserPromptSubmit] {
            let answer = context_for(
                &path,
                &project,
                ask(event, "s", Some("durable hooks")),
                frozen(),
            );
            let text = answer.context.expect("a bounded multilingual capsule");
            assert!(estimated_tokens(&text) <= RENDERED_TOKEN_LIMIT);
            assert!(text.chars().filter(|ch| !ch.is_ascii()).count() <= RENDERED_TOKEN_LIMIT);
        }
    }

    #[test]
    fn an_empty_session_start_adds_nothing() {
        // B10: no card for the index, no index.
        let result = render(
            HookEvent::SessionStart,
            Injection {
                outcome: InjectionOutcome::Empty,
                kind: InjectionKind::Index,
                items: Vec::new(),
                token_count: 0,
                matched: 0,
            },
            None,
            0,
            &frozen(),
        );

        assert_eq!(result.outcome, HookOutcome::Empty);
        assert_eq!(result.context, None);
    }

    #[test]
    fn an_expired_ask_answers_deadline_before_opening_a_missing_store() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("project");
        fs::create_dir_all(&checkout).unwrap();
        let project = hide_project::resolve(&checkout, NODE).unwrap();
        let path = crate::database_path(temp.path());
        let (elapsed, expired) = invocation_clock();
        elapsed.fetch_add(100, Ordering::SeqCst);

        let result = context_for(
            &path,
            &project,
            ask(HookEvent::SessionStart, "s", None),
            expired,
        );

        assert_eq!(result.outcome, HookOutcome::Deadline);
        assert!(result.context.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn a_render_at_the_deadline_carries_neither_the_capsule_nor_its_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let body = "Keep the invocation deadline through rendering";
        let (path, project) = enabled_store(
            temp.path(),
            &temp.path().join("project"),
            &[rule(body, 0.9)],
        );

        let (elapsed, expired) = invocation_clock();
        let reader = MemoryStore::open_hook_read_only_with_expiry(&path, expired.clone()).unwrap();
        elapsed.fetch_add(99, Ordering::SeqCst);
        let injection = reader
            .retrieve_with_expiry(
                &RetrievalQuery::prompt(&project.id, "invocation deadline", Vec::new()),
                &expired,
            )
            .unwrap();
        assert_eq!(injection.outcome, InjectionOutcome::Provided);
        assert_eq!(injection.items[0].text, body);
        let item_key = injection.item_key();
        let auth = reader
            .receipt_auth_tag(
                &project.id,
                "codex",
                "render-session",
                "UserPromptSubmit",
                &item_key,
            )
            .unwrap();
        assert!(
            reader
                .verify_receipt_auth(
                    &project.id,
                    "codex",
                    "render-session",
                    "UserPromptSubmit",
                    &item_key,
                    &auth
                )
                .unwrap()
        );
        elapsed.fetch_add(1, Ordering::SeqCst);

        let result = render(
            HookEvent::UserPromptSubmit,
            injection,
            Some(&auth),
            0,
            &expired,
        );

        assert_eq!(result.outcome, HookOutcome::Deadline);
        assert_eq!(result.context, None);
    }

    #[test]
    fn a_real_sql_query_is_interrupted_by_the_inner_budget_before_the_outer_expiry() {
        let temp = tempfile::tempdir().unwrap();
        let (path, project) = enabled_store(
            temp.path(),
            &temp.path().join("project"),
            &[rule("Use bounded hooks", 0.9)],
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("ALTER TABLE projects RENAME TO projects_base; CREATE VIEW projects AS SELECT * FROM projects_base WHERE (WITH RECURSIVE slow(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM slow WHERE n<100000000) SELECT sum(n) FROM slow)>0;").unwrap();
        drop(connection);
        let outer = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = super::context_for(
            &path,
            &project,
            ask(HookEvent::SessionStart, "slow-sql", None),
            move || std::time::Instant::now() >= outer,
        );
        assert!(matches!(result.outcome, HookOutcome::Deadline));
        assert!(result.context.is_none());
        assert!(
            std::time::Instant::now() < outer,
            "the inner SQL interrupt precedes the outer expiry"
        );
    }

    #[test]
    fn a_missing_or_corrupt_store_answers_unavailable_for_both_events() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("project");
        fs::create_dir_all(&checkout).unwrap();
        let project = hide_project::resolve(&checkout, NODE).unwrap();
        let path = crate::database_path(temp.path());
        for event in [HookEvent::SessionStart, HookEvent::UserPromptSubmit] {
            let missing = context_for(
                &path,
                &project,
                ask(event, "s", Some("keep going")),
                frozen(),
            );
            assert_eq!(missing.outcome, HookOutcome::Unavailable, "{event:?}");
            assert!(missing.context.is_none());
        }
        fs::write(&path, b"not a sqlite database").unwrap();
        let corrupt = context_for(
            &path,
            &project,
            ask(HookEvent::UserPromptSubmit, "s", Some("keep going")),
            frozen(),
        );
        assert_eq!(corrupt.outcome, HookOutcome::Unavailable);
        assert!(corrupt.context.is_none());
    }

    #[test]
    fn a_locked_store_answers_unavailable_without_waiting_for_the_writer() {
        let temp = tempfile::tempdir().unwrap();
        let (path, project) = enabled_store(temp.path(), &temp.path().join("project"), &[]);

        let lock = rusqlite::Connection::open(&path).unwrap();
        lock.pragma_update(None, "journal_mode", "DELETE").unwrap();
        lock.pragma_update(None, "locking_mode", "EXCLUSIVE")
            .unwrap();
        lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let started = Instant::now();
        // The lock decides the outcome; the frozen clock never expires, and
        // the elapsed bound below catches a wait for the writer.
        let result = context_for(
            &path,
            &project,
            ask(HookEvent::UserPromptSubmit, "s", Some("keep going")),
            frozen(),
        );
        assert_eq!(result.outcome, HookOutcome::Unavailable);
        assert!(result.context.is_none());
        assert!(started.elapsed() < Duration::from_millis(crate::HOOK_DEADLINE_MS));
        lock.execute_batch("ROLLBACK").unwrap();
    }
}
