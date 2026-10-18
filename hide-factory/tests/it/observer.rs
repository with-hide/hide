//! The Observer, the mode table, the daily cap, quiet and vanished workers,
//! worker candidates and a Factory's pause, through the engine's commands
//! and ticks over the fake world (factory-observer PRD).

use crate::support::*;
use hide_factory::adapters::{Failure, MainCheck, OutsideEvent, PreMerge, WorkerStatus};
use hide_factory::command::{CardInput, Command};
use hide_factory::judgment::{JudgmentInput, WorkerTextSource};
use hide_factory::model::*;
use serde_json::{Value, json};

fn config(h: &mut Bench, key: &str, value: &str) -> Value {
    h.op(Command::Config {
        project: Some(PROJECT.into()),
        set: vec![(key.into(), value.into())],
    })
}

fn mode(h: &mut Bench, value: &str) {
    let answer = config(h, "observer_mode", value);
    assert_eq!(answer["ok"], true, "{answer}");
}

fn ask(h: &mut Bench, f: &str, t: &str, question: &str, choices: &[&str]) -> Value {
    h.as_worker(
        f,
        t,
        Command::Ask {
            text: question.into(),
            suggestion: "sqlite".into(),
            default_action: "sqlite".into(),
            deadline_hours: Some(24),
            letter: None,
            choices: choices.iter().map(|c| (*c).to_owned()).collect(),
        },
    )
}

fn block(h: &mut Bench, f: &str, t: &str, question: &str) -> Value {
    h.as_worker(
        f,
        t,
        Command::Block {
            text: question.into(),
            suggestion: "approve".into(),
            deadline_hours: Some(24),
            letter: None,
            choices: vec!["approve".into(), "deny".into()],
        },
    )
}

fn inbox(h: &mut Bench) -> Value {
    h.op(Command::Inbox)
}

fn groups(inbox: &Value) -> Vec<String> {
    inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["group"].as_str().unwrap().to_owned())
        .collect()
}

fn letters_to(h: &Bench, task: &str) -> Vec<String> {
    h.world()
        .messages
        .iter()
        .filter(|(to, _)| to == task)
        .map(|(_, body)| body.clone())
        .collect()
}

fn observer_calls(h: &Bench) -> usize {
    h.world()
        .judged
        .iter()
        .filter(|j| j.feature_id() == hide_factory::judgment::OBSERVER)
        .count()
}

// ---------------------------------------------------------- decision requests

#[test]
fn a_request_carries_at_most_five_choices_of_at_most_120_characters() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Asks", &[]);
    let six = ["a", "b", "c", "d", "e", "f"];
    let refused = ask(&mut h, &f, &t, "Which?", &six);
    assert_eq!(refused["reason"], "too_many_choices", "{refused}");
    assert_eq!(refused["detail"]["limit"], 5);
    let long = "가".repeat(121);
    let refused = ask(&mut h, &f, &t, "Which?", &[&long]);
    assert_eq!(refused["reason"], "choice_too_long", "{refused}");
    assert_eq!(refused["detail"]["limit"], 120);
    assert!(h.task(&f, &t).questions.is_empty(), "no request was made");
    let kept = ask(&mut h, &f, &t, "Which?", &["sqlite", "postgres"]);
    assert_eq!(kept["ok"], true, "{kept}");
    assert_eq!(h.task(&f, &t).questions[0].choices, ["sqlite", "postgres"]);
}

#[test]
fn a_technical_choice_in_assist_is_answered_by_the_ai_on_the_record_not_my_turn() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Store", &[]);
    h.world().observer.push_back(classified("B", "postgres"));
    ask(&mut h, &f, &t, "Which database?", &["sqlite", "postgres"]);
    // While the Observer sorts it, it is nobody's turn yet.
    assert_eq!(inbox(&mut h)["count"], 0);
    h.engine.tick();
    let task = h.task(&f, &t);
    let question = &task.questions[0];
    let answer = question.answer.as_ref().expect("answered");
    assert_eq!(
        (answer.relayed_by.as_str(), answer.text.as_str()),
        (OBSERVER, "postgres")
    );
    let decision = task.decisions.last().unwrap();
    assert_eq!(decision.by, OBSERVER);
    assert_eq!(decision.kind, Some(DecisionKind::B));
    assert_eq!(decision.reason.as_deref(), Some("B 판단"));
    // It differs from the default action, so the worker hears it (B11).
    assert!(
        letters_to(&h, &t)
            .iter()
            .any(|body| body.contains("Factory AI가") && body.contains("postgres"))
    );
    let inbox = inbox(&mut h);
    assert!(groups(&inbox).is_empty(), "not a person's (B21): {inbox}");
    assert_eq!(decision.source, Some(DecisionSource::Answer));
    assert_eq!(decision.question.as_deref(), Some(question.id.as_str()));
    assert!(decision.overridable());
    assert!(task.activity.iter().any(|entry| matches!(
        &entry.event,
        ActivityEvent::AiDecision { text } if text.ends_with("postgres")
    )));
}

#[test]
fn the_mode_decides_who_answers_a_technical_choice() {
    for (value, by_ai) in [("manual", false), ("autonomous", true)] {
        let mut h = Bench::new(false);
        let f = h.factory(true);
        mode(&mut h, value);
        let t = h.ready("Store", &[]);
        h.world().observer.push_back(classified("B", "postgres"));
        ask(&mut h, &f, &t, "Which database?", &[]);
        h.engine.tick();
        let question = h.task(&f, &t).questions[0].clone();
        assert_eq!(question.answer.is_some(), by_ai, "{value}");
        assert_eq!(h.task(&f, &t).ai_decisions() == 1, by_ai, "{value}");
        if !by_ai {
            let inbox = inbox(&mut h);
            assert_eq!(inbox["count"], 1);
            assert_eq!(inbox["items"][0]["decision_kind"], "B");
        }
    }
}

#[test]
fn a_permission_or_an_unsure_kind_always_goes_to_a_person() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    let t = h.ready("Deploy", &[]);
    let mut permission = classified("B", "yes");
    permission["permission_signal"] = json!(true);
    h.world().observer.push_back(permission);
    block(&mut h, &f, &t, "Delete the production bucket?");
    h.engine.tick();
    let first = inbox(&mut h);
    assert_eq!(first["count"], 1, "{first}");
    assert_eq!(first["items"][0]["fallback"], "permission", "{first}");
    assert!(h.task(&f, &t).questions[0].open());
    let mut unsure = classified("B", "yes");
    unsure["ambiguous"] = json!(true);
    h.world().observer.push_back(unsure);
    let u = h.ready("Region", &[]);
    block(&mut h, &f, &u, "Which region should the bucket live in?");
    h.engine.tick();
    let inbox = inbox(&mut h);
    assert_eq!(inbox["count"], 2, "{inbox}");
    assert!(
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["fallback"] == "unsure"),
        "{inbox}"
    );
}

#[test]
fn factory_ai_words_that_are_no_listed_choice_leave_a_closed_question_to_a_person() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    // A failed review asks a closed question, which goes to Factory AI first.
    h.world().judgment_failure = Some("transient".into());
    let id = h.add("Unreadable", &[])["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.engine.tick();
    h.world().judgment_failure = None;
    h.world()
        .observer
        .push_back(classified("A", "Retry the review"));
    h.engine.tick();
    h.engine.tick();
    let task = h.task(&f, &id);
    let open: Vec<_> = task.open_questions().collect();
    assert_eq!(open.len(), 1, "{:?}", task.questions);
    assert!(open[0].choices.iter().any(|c| c == "retry-review"));
    assert_eq!(inbox(&mut h)["count"], 1);
    assert_eq!(
        open[0].routing.as_ref().and_then(|r| r.fallback),
        Some(Fallback::NoChoice),
        "the mode would have let Factory AI decide, so it is not why"
    );
}

#[test]
fn factory_ai_s_choice_on_a_closed_question_already_ran_and_cannot_be_changed() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().judgment_failure = Some("transient".into());
    let id = h.add("Retried", &[])["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.engine.tick();
    h.world().judgment_failure = None;
    h.world().observer.push_back(classified("A", "start-as-is"));
    h.engine.tick();
    h.engine.tick();
    let task = h.task(&f, &id);
    let at = task
        .decisions
        .iter()
        .position(|d| d.by == OBSERVER)
        .expect("Factory AI chose");
    assert!(!task.decision_changeable(&task.decisions[at]));
    let changed = h.op(Command::Answer {
        task: id.clone(),
        question: None,
        choice: None,
        text: Some("cancel".into()),
        change: false,
        decision: Some(format!("R{}", at + 1)),
    });
    assert_eq!(changed["reason"], "decision_not_changeable", "{changed}");
}

#[test]
fn a_worker_s_question_reaches_the_person_in_factory_ai_s_words_and_keeps_its_own_on_the_record() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Store", &[]);
    let mut verdict = classified("C", "");
    verdict["person_text"] = json!("Should the sort order be remembered between visits?");
    h.world().observer.push_back(verdict);
    ask(
        &mut h,
        &f,
        &t,
        "persist sortKey in ui_state.rs or localStorage?",
        &[],
    );
    h.engine.tick();
    let inbox = inbox(&mut h);
    assert_eq!(
        inbox["items"][0]["text"], "Should the sort order be remembered between visits?",
        "{inbox}"
    );
    assert_eq!(
        h.task(&f, &t).questions[0].text,
        "persist sortKey in ui_state.rs or localStorage?"
    );
}

#[test]
fn a_person_answering_first_leaves_the_late_verdict_without_effect() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Race", &[]);
    h.world().observer.push_back(classified("A", "postgres"));
    ask(&mut h, &f, &t, "Which database?", &[]);
    let question = h.task(&f, &t).questions[0].id.clone();
    let answered = h.op(Command::Answer {
        task: t.clone(),
        question: Some(question),
        choice: None,
        text: Some("sqlite".into()),
        change: false,
        decision: None,
    });
    assert_eq!(answered["ok"], true, "{answered}");
    h.engine.tick();
    let task = h.task(&f, &t);
    assert_eq!(task.questions[0].answer.as_ref().unwrap().text, "sqlite");
    assert_eq!(
        task.decisions.iter().filter(|d| d.by == OBSERVER).count(),
        0
    );
    assert!(
        !task
            .activity
            .iter()
            .any(|entry| matches!(entry.event, ActivityEvent::AiDecision { .. }))
    );
}

#[test]
fn an_observer_failure_sends_the_request_to_a_person_and_logs_why() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Fails", &[]);
    h.world().judgment_failure = Some("timeout".into());
    ask(&mut h, &f, &t, "Which?", &[]);
    h.engine.tick();
    let inbox = inbox(&mut h);
    assert_eq!(inbox["count"], 1, "{inbox}");
    let routing = h.task(&f, &t).questions[0].routing.clone().unwrap();
    assert_eq!(routing.to, RouteTo::Person);
    assert_eq!(routing.fallback, Some(Fallback::Failed));
    let events = h.engine.events(&f, Some(&t), 50);
    assert!(
        events
            .iter()
            .any(|e| e.kind == "observer.failed" && e.detail["reason"] == "timeout"),
        "{events:?}"
    );
}

#[test]
fn the_daily_cap_sends_the_rest_of_the_day_to_a_person_with_one_line() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let answer = config(&mut h, "observer_daily_limit", "1");
    assert_eq!(answer["ok"], true, "{answer}");
    let t = h.ready("Capped", &[]);
    h.world().observer.push_back(classified("B", "postgres"));
    ask(&mut h, &f, &t, "First?", &[]);
    h.engine.tick();
    ask(&mut h, &f, &t, "Second?", &[]);
    ask(&mut h, &f, &t, "Third?", &[]);
    h.engine.tick();
    assert_eq!(observer_calls(&h), 1, "past the cap nothing is sent");
    let task = h.task(&f, &t);
    let fallbacks: Vec<_> = task
        .questions
        .iter()
        .filter_map(|q| q.routing.as_ref().and_then(|r| r.fallback))
        .collect();
    assert_eq!(fallbacks, [Fallback::DailyLimit, Fallback::DailyLimit]);
    let caps = h
        .engine
        .factories()
        .next()
        .unwrap()
        .activity
        .iter()
        .filter(|entry| matches!(entry.event, ActivityEvent::DailyLimit { limit: 1 }))
        .count();
    assert_eq!(caps, 1, "once a day");
    let view = &h.op(Command::Status { project: None })["factories"][0];
    assert_eq!(view["observer_capped"], true, "the header marks it (B21)");
    assert_eq!(
        (
            view["observer_today"].clone(),
            view["observer_limit"].clone()
        ),
        (json!(1), json!(1))
    );
    // The next local day counts from zero.
    h.advance(DAY_MS);
    h.world().observer.push_back(classified("B", "postgres"));
    ask(&mut h, &f, &t, "Tomorrow?", &[]);
    h.engine.tick();
    assert_eq!(observer_calls(&h), 2);
}

#[test]
fn a_call_hide_ai_never_sent_is_not_counted() {
    // Hide AI's classes for a request no provider took, and ones that may
    // follow a model turn (D-34): a provider past its restart cap is
    // reported unavailable after its turn answered.
    for (reason, counted) in [
        ("disabled", 0),
        ("no_provider", 0),
        ("not_authenticated", 0),
        ("usage_limited", 0),
        ("provider_unavailable", 1),
        ("transient", 1),
        ("over_budget", 1),
        ("timeout", 1),
    ] {
        let mut h = Bench::new(false);
        let f = h.factory(true);
        let t = h.ready("Off", &[]);
        h.world().judgment_failure = Some(reason.into());
        ask(&mut h, &f, &t, "Which?", &[]);
        h.engine.tick();
        let view = &h.op(Command::Status { project: None })["factories"][0];
        assert_eq!(view["observer_today"], counted, "{reason}");
        assert_eq!(inbox(&mut h)["count"], 1, "a person decides: {reason}");
    }
    // A judgment the engine could not queue was never sent either.
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Queue full", &[]);
    h.world().judge_down = true;
    ask(&mut h, &f, &t, "Which?", &[]);
    h.engine.tick();
    let view = &h.op(Command::Status { project: None })["factories"][0];
    assert_eq!(view["observer_today"], 0);
    assert_eq!(inbox(&mut h)["count"], 1);
}

#[test]
fn a_person_overrides_the_ai_answer_and_the_worker_hears_it() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Override", &[]);
    h.world().observer.push_back(classified("B", "postgres"));
    ask(&mut h, &f, &t, "Which database?", &[]);
    h.engine.tick();
    let question = h.task(&f, &t).questions[0].id.clone();
    // A person who answered without "다른 답" lost the race: nothing moves.
    let letters = letters_to(&h, &t).len();
    let late = h.op(Command::Answer {
        task: t.clone(),
        question: Some(question.clone()),
        choice: None,
        text: Some("sqlite".into()),
        change: false,
        decision: None,
    });
    assert_eq!(late["reason"], "already_answered", "{late}");
    assert_eq!(letters_to(&h, &t).len(), letters, "no second letter");
    let changed = h.op(Command::Answer {
        task: t.clone(),
        question: Some(question.clone()),
        choice: None,
        text: Some("sqlite after all".into()),
        change: true,
        decision: None,
    });
    assert_eq!(changed["ok"], true, "{changed}");
    let task = h.task(&f, &t);
    assert!(task.questions[0].routing.as_ref().unwrap().overridden);
    // Factory AI's record becomes the person's, keeping what it had said.
    let record = task.decisions.last().unwrap();
    assert!(record.text.ends_with("sqlite after all"));
    assert_ne!(record.by, OBSERVER);
    assert!(record.changed.as_ref().unwrap().from.ends_with("postgres"));
    assert_eq!(task.ai_decisions(), 0);
    assert!(
        letters_to(&h, &t)
            .iter()
            .any(|body| body.contains("sqlite after all"))
    );
    // A second change is a person's own answer, not the AI's.
    let again = h.op(Command::Answer {
        task: t.clone(),
        question: Some(question),
        choice: None,
        text: Some("no".into()),
        change: true,
        decision: None,
    });
    assert_eq!(again["reason"], "already_answered", "{again}");
}

#[test]
fn a_finished_task_s_ai_answer_cannot_be_changed() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Finished", &[]);
    h.world().observer.push_back(classified("B", "postgres"));
    ask(&mut h, &f, &t, "Which database?", &[]);
    h.engine.tick();
    h.done(&f, &t);
    for _ in 0..10 {
        if h.state(&f, &t) == TaskState::Done {
            break;
        }
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::Done);
    let question = h.task(&f, &t).questions[0].id.clone();
    let refused = h.op(Command::Answer {
        task: t.clone(),
        question: Some(question),
        choice: None,
        text: Some("sqlite".into()),
        change: true,
        decision: None,
    });
    assert_eq!(refused["reason"], "task_finished", "{refused}");
}

fn board_card(h: &Bench, id: &str) -> Value {
    let summary = serde_json::to_value(h.engine.summary()).unwrap();
    summary["factories"][0]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|column| column["cards"].as_array().unwrap().iter())
        .find(|card| card["task"] == id)
        .cloned()
        .expect("card on the board")
}

#[test]
fn a_task_blocked_while_factory_ai_sorts_the_request_is_not_yet_a_person_s() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Sorting", &[]);
    h.world().hold_judgments = true;
    block(&mut h, &f, &t, "Which table?");
    h.engine.tick();
    assert_eq!(h.state(&f, &t), TaskState::Blocked);
    let sorting = board_card(&h, &t);
    assert_eq!(
        (
            sorting["needs_person"].clone(),
            sorting["waiting_group"].clone()
        ),
        (json!(false), json!("other")),
        "{sorting}"
    );
    h.world().hold_judgments = false;
    h.engine.tick();
    let routed = board_card(&h, &t);
    assert_eq!(
        (
            routed["needs_person"].clone(),
            routed["waiting_group"].clone()
        ),
        (json!(true), json!("person")),
        "{routed}"
    );
}

#[test]
fn a_verdict_that_lands_after_its_task_was_cancelled_leaves_nothing_waiting_on_revive() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Revived", &[]);
    h.world().hold_judgments = true;
    ask(&mut h, &f, &t, "Which table?", &[]);
    h.engine.tick();
    let cancelled = h.op(Command::Cancel { task: t.clone() });
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    h.world().hold_judgments = false;
    h.engine.tick();
    let revived = h.op(Command::Revive { task: t.clone() });
    assert_eq!(revived["ok"], true, "{revived}");
    h.engine.tick();
    // Cancelling answered the request; the revived worker asks again.
    assert_eq!(h.task(&f, &t).open_questions().count(), 0);
    assert_eq!(inbox(&mut h)["items"], json!([]));
}

#[test]
fn a_request_still_sorting_when_its_task_went_outside_is_a_person_s_on_revive() {
    let mut h = Bench::new(true);
    let f = h.factory(true);
    let t = h.ready("Taken outside", &[]);
    let issue = h.task(&f, &t).issue.unwrap();
    h.world().hold_judgments = true;
    block(&mut h, &f, &t, "Which table?");
    h.world().outside.push_back(OutsideEvent::ClosingPr {
        issue,
        pr: 55,
        url: "pr/55".into(),
        merged: false,
    });
    h.advance(3 * MINUTE_MS);
    h.engine.tick();
    assert_eq!(h.state(&f, &t), TaskState::Outside);
    h.world().hold_judgments = false;
    h.engine.tick();
    let revived = h.op(Command::Revive { task: t.clone() });
    assert_eq!(revived["ok"], true, "{revived}");
    h.engine.tick();
    let asked = inbox(&mut h);
    assert!(
        asked["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["task"] == t.as_str() && item["text"] == "Which table?"),
        "{asked}"
    );
}

#[test]
fn an_ai_answer_is_on_the_record_and_activity_and_a_person_can_change_it() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Records", &[]);
    for answer in ["postgres", "b"] {
        h.world().observer.push_back(classified("B", answer));
    }
    ask(&mut h, &f, &t, "Which database?", &[]);
    ask(&mut h, &f, &t, "Two?", &[]);
    h.engine.tick();
    // What Factory AI answered is not a person's item (B20, B21).
    let items = inbox(&mut h);
    assert_eq!(items["count"], 0, "{items}");
    let task = h.task(&f, &t);
    assert_eq!(task.ai_decisions(), 2);
    assert_eq!(
        task.activity
            .iter()
            .filter(|entry| matches!(entry.event, ActivityEvent::AiDecision { .. }))
            .count(),
        2
    );
    let shown = h.op(Command::Show { task: t.clone() });
    let decision = shown["task"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["text"].as_str().unwrap().contains("Which database?"))
        .cloned()
        .expect("the AI's decision");
    assert_eq!(decision["by"], "ai");
    assert_eq!(decision["overridable"], true);
    let id = decision["id"].as_str().unwrap().to_owned();

    let changed = h.op(Command::Answer {
        task: t.clone(),
        question: None,
        choice: None,
        text: Some("mysql".into()),
        change: false,
        decision: Some(id.clone()),
    });
    assert_eq!(changed["ok"], true, "{changed}");
    let shown = h.op(Command::Show { task: t.clone() });
    let after = shown["task"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(
        after["by"], "person",
        "the decision is now the person's (B28)"
    );
    assert!(after["text"].as_str().unwrap().ends_with("mysql"));
    assert!(
        after["changed"]["from"]
            .as_str()
            .unwrap()
            .ends_with("postgres")
    );
    assert!(
        letters_to(&h, &t).iter().any(|body| body.contains("mysql")),
        "the worker hears the new answer"
    );
    let again = h.op(Command::Answer {
        task: t.clone(),
        question: None,
        choice: None,
        text: Some("sqlite".into()),
        change: false,
        decision: Some(id),
    });
    assert_eq!(again["reason"], "decision_not_changeable", "{again}");
}

// ------------------------------------------------------------- wrong cards

fn fix(kind: &str) -> Value {
    let mut verdict = classified("E", "");
    verdict["proposal"] = json!({
        "type": kind,
        "title": "Store in postgres",
        "goal": "Keep the data in postgres",
        "criteria": ["postgres holds the rows"],
        "prerequisite": false,
    });
    verdict
}

#[test]
fn an_autonomous_card_fix_changes_the_card_and_waits_for_a_person_at_merge() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    let t = h.ready("Store", &[]);
    h.world().observer.push_back(fix("card_fix"));
    block(
        &mut h,
        &f,
        &t,
        "The card says sqlite but the PRD says postgres?",
    );
    h.engine.tick();
    let task = h.task(&f, &t);
    assert_eq!(task.card.title, "Store in postgres");
    assert!(task.scope_approved, "the merge waits for a person (B8)");
    let record = task.decisions.last().unwrap();
    assert_eq!(record.by, OBSERVER);
    assert!(
        !record.overridable(),
        "an applied fix is undone its own way (B29)"
    );
    assert!(
        task.activity
            .iter()
            .any(|entry| matches!(entry.event, ActivityEvent::AiDecision { .. }))
    );
    // The blocked worker gets the new card when it wakes.
    tick_until_state(&mut h, &f, &t, TaskState::Running);
    assert!(
        h.world()
            .wakes
            .iter()
            .any(|(task, body)| *task == t && body.contains("Keep the data in postgres"))
    );
}

#[test]
fn an_assist_wrong_card_offers_the_fix_as_a_choice() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Store", &[]);
    h.world().observer.push_back(fix("new_task"));
    block(&mut h, &f, &t, "This needs a migration first?");
    h.engine.tick();
    let item = inbox(&mut h)["items"][0].clone();
    assert!(
        item["choices"]
            .as_array()
            .unwrap()
            .contains(&json!(PROPOSAL_CHOICE)),
        "{item}"
    );
    assert_eq!(h.engine.tasks_of(&f).count(), 1, "nothing applied yet");
    let chosen = h.op(Command::Answer {
        task: t.clone(),
        question: item["question"].as_str().map(str::to_owned),
        choice: Some(PROPOSAL_CHOICE.into()),
        text: None,
        change: false,
        decision: None,
    });
    assert_eq!(chosen["ok"], true, "{chosen}");
    let child = h
        .engine
        .tasks_of(&f)
        .find(|task| task.id != t)
        .expect("a new Task");
    assert_eq!(child.card.title, "Store in postgres");
    assert_eq!(child.proposed_by.as_deref(), Some(t.as_str()));
}

#[test]
fn factory_ai_makes_no_task_from_a_task_that_was_itself_proposed() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    let t = h.ready("Store", &[]);
    h.world().observer.push_back(fix("new_task"));
    block(&mut h, &f, &t, "This needs a migration first?");
    h.engine.tick();
    let child = h
        .engine
        .tasks_of(&f)
        .find(|task| task.id != t)
        .expect("Factory AI made a Task")
        .id
        .clone();
    tick_until_state(&mut h, &f, &child, TaskState::Running);
    // The Task Factory AI made asks for one more: a person decides (B30 depth rule).
    h.world().observer.push_back(fix("new_task"));
    block(&mut h, &f, &child, "And another migration?");
    h.engine.tick();
    assert_eq!(h.engine.tasks_of(&f).count(), 2, "no grandchild");
    assert!(
        inbox(&mut h)["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["task"] == child.as_str() && item["group"] == "answer"),
        "the request waits for a person"
    );
}

// ------------------------------------------------------------- risk merges

fn risk_only(h: &mut Bench, f: &str, title: &str) -> String {
    let t = h.ready(title, &[]);
    h.world().premerge.insert(
        t.clone(),
        [PreMerge::RiskPath {
            paths: vec!["infra/x".into()],
        }]
        .into(),
    );
    h.done(f, &t);
    t
}

#[test]
fn an_autonomous_factory_lets_the_observer_approve_a_lone_risk_path() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().risk_merge = Some(json!({"approve": true, "reason": "인프라 변경이 카드 범위"}));
    let t = risk_only(&mut h, &f, "Infra");
    for _ in 0..10 {
        h.engine.tick();
        if matches!(h.state(&f, &t), TaskState::Landed | TaskState::Done) {
            break;
        }
    }
    let task = h.task(&f, &t);
    assert!(
        matches!(task.state, TaskState::Landed | TaskState::Done),
        "{:?}",
        task.state
    );
    let decision = task.decisions.last().unwrap();
    assert_eq!(
        (decision.by.as_str(), decision.reason.as_deref()),
        (OBSERVER, Some("인프라 변경이 카드 범위"))
    );
    assert_eq!(decision.source, Some(DecisionSource::RiskMerge));
    assert!(!decision.overridable(), "merged: a record only (B29)");
    assert!(
        task.activity
            .iter()
            .any(|entry| matches!(entry.event, ActivityEvent::AiDecision { .. }))
    );
}

#[test]
fn a_risk_merge_approved_after_the_mode_left_autonomous_does_not_merge() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().risk_merge = Some(json!({"approve": true, "reason": "카드 범위"}));
    let t = risk_only(&mut h, &f, "Infra");
    tick_until_state(&mut h, &f, &t, TaskState::MergeWaiting);
    mode(&mut h, "assist");
    let attempts = h.world().merge_attempts;
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::MergeWaiting);
    assert_eq!(h.world().merge_attempts, attempts);
}

/// An outside push turns main red; the next main read finds it broken.
fn break_main(h: &mut Bench) {
    {
        let mut world = h.world();
        world.head = "outside-red".into();
        world.main_checks.insert(
            "outside-red".into(),
            MainCheck::Red {
                link: "run/9".into(),
            },
        );
    }
    h.advance(3 * MINUTE_MS);
    h.engine.tick();
    assert!(h.engine.factories().next().unwrap().main.broken);
}

fn merge_asks(h: &Bench) -> usize {
    h.world()
        .judged
        .iter()
        .filter(|j| matches!(j.input, JudgmentInput::ObserverMerge { .. }))
        .count()
}

#[test]
fn a_risk_merge_approved_after_main_broke_does_not_merge() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().risk_merge = Some(json!({"approve": true, "reason": "카드 범위"}));
    let t = risk_only(&mut h, &f, "Infra");
    tick_until_state(&mut h, &f, &t, TaskState::MergeWaiting);
    // The approval was asked with the move; it answers only when released.
    h.world().hold_judgments = true;
    let attempts = h.world().merge_attempts;
    break_main(&mut h);
    h.world().hold_judgments = false;
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(merge_asks(&h), 1);
    assert_eq!(h.state(&f, &t), TaskState::MergeWaiting);
    assert_eq!(h.world().merge_attempts, attempts, "nothing merges on red");
}

#[test]
fn resuming_on_a_red_main_asks_no_risk_merge_approval() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().risk_merge = Some(json!({"approve": true, "reason": "카드 범위"}));
    let t = risk_only(&mut h, &f, "Infra");
    tick_until_state(&mut h, &f, &t, TaskState::MergeWaiting);
    // The approval was asked with the move; it answers only when released.
    h.world().hold_judgments = true;
    // The approval lands while paused, so the resume would ask again.
    h.op(Command::PauseFactory { project: None });
    h.world().hold_judgments = false;
    h.engine.tick();
    break_main(&mut h);
    h.op(Command::ResumeFactory { project: None });
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(merge_asks(&h), 1, "no second approval asked on red");
    assert_eq!(h.state(&f, &t), TaskState::MergeWaiting);
}

#[test]
fn in_assist_a_risk_path_waits_for_a_person_and_no_merge_judgment_runs() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    h.world().risk_merge = Some(json!({"approve": true, "reason": "x"}));
    let t = risk_only(&mut h, &f, "Infra");
    for _ in 0..10 {
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::MergeWaiting);
    assert!(
        !h.world()
            .judged
            .iter()
            .any(|j| matches!(j.input, JudgmentInput::ObserverMerge { .. }))
    );
}

fn tick_until_state(h: &mut Bench, factory: &str, id: &str, state: TaskState) {
    for _ in 0..20 {
        if h.state(factory, id) == state {
            return;
        }
        h.engine.tick();
    }
    panic!(
        "{id} never reached {state:?}; it is {:?}",
        h.state(factory, id)
    );
}

// ----------------------------------------------------------- quiet workers

#[test]
fn a_worker_that_asked_once_is_still_caught_resting_without_a_later_report() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Asked once", &[]);
    ask(&mut h, &f, &t, "Which?", &[]);
    h.engine.tick();
    // The turn that asked ends: reported.
    let first = h.world().now + MINUTE_MS;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since: first });
    h.advance(4 * MINUTE_MS);
    h.engine.tick();
    assert!(
        letters_to(&h, &t)
            .iter()
            .all(|b| !b.contains("done, ask, block"))
    );
    // A later turn ends with no report (D-24).
    let second = h.world().now + MINUTE_MS;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since: second });
    h.advance(4 * MINUTE_MS);
    h.engine.tick();
    assert!(
        letters_to(&h, &t)
            .iter()
            .any(|b| b.contains("done, ask, block"))
    );
}

#[test]
fn a_worker_working_again_leaves_its_rest_and_a_later_quiet_turn_is_still_caught() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Works again", &[]);
    ask(&mut h, &f, &t, "Which?", &[]);
    h.engine.tick();
    let first = h.world().now + MINUTE_MS;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since: first });
    h.advance(4 * MINUTE_MS);
    h.engine.tick();
    let resting_since =
        |h: &mut Bench| h.op(Command::Show { task: t.clone() })["task"]["resting_since"].clone();
    assert_eq!(resting_since(&mut h), json!(first));
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Working);
    h.advance(MINUTE_MS);
    h.engine.tick();
    assert_eq!(
        resting_since(&mut h),
        Value::Null,
        "the Task page stops counting the rest"
    );
    // The next turn ends with no report since the one that asked (D-24).
    let second = h.world().now + MINUTE_MS;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since: second });
    h.advance(4 * MINUTE_MS);
    h.engine.tick();
    assert_eq!(resting_since(&mut h), json!(second));
    assert!(
        letters_to(&h, &t)
            .iter()
            .any(|b| b.contains("done, ask, block"))
    );
}

#[test]
fn an_unknown_activity_is_never_counted_as_rest() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Unknown", &[]);
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Unknown);
    h.advance(60 * MINUTE_MS);
    h.engine.tick();
    assert_eq!(h.state(&f, &t), TaskState::Running);
    assert!(letters_to(&h, &t).is_empty());
}

#[test]
fn a_diagnosis_that_finds_a_question_raises_it_for_the_worker() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Asked on screen", &[]);
    let mut question = classified("C", "");
    question["text"] = json!("Blue or green?");
    question["suggestion"] = json!("blue");
    question["choices"] = json!(["blue", "green"]);
    h.world()
        .diagnosis
        .push_back(json!({"verdict": "question", "reason": "화면에서 물음", "question": question}));
    h.world().texts.user_turn = Some("Blue or green?".into());
    let since = h.world().now;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since });
    // Wake, then two quiet minutes, then the diagnosis.
    for _ in 0..3 {
        h.advance(2 * MINUTE_MS);
        h.engine.tick();
    }
    h.engine.tick();
    let task = h.task(&f, &t);
    assert_eq!(task.state, TaskState::Blocked);
    let raised = task.questions.last().unwrap();
    assert_eq!(raised.text, "Blue or green?");
    assert_eq!(raised.choices, ["blue", "green"]);
    // A product choice in assist is a person's.
    assert_eq!(inbox(&mut h)["count"], 1);
}

#[test]
fn a_diagnosed_stop_tells_the_person_what_factory_ai_read() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Quiet", &[]);
    h.world()
        .diagnosis
        .push_back(json!({"verdict": "stuck", "reason": "테스트 실행을 기다리다 멈춤"}));
    h.world().texts.screen = Some("cargo test 실행 중".into());
    let since = h.world().now;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since });
    for _ in 0..3 {
        h.advance(2 * MINUTE_MS);
        h.engine.tick();
    }
    h.engine.tick();
    assert_eq!(h.task(&f, &t).stop, Some(StopReason::NoReport));
    // The recovery schedule has it first; past 180 minutes it is a person's.
    assert_eq!(inbox(&mut h)["count"], 0);
    h.op(Command::Config {
        project: None,
        set: vec![("recovery".into(), "restart_worker=off".into())],
    });
    h.advance(181 * MINUTE_MS);
    h.engine.tick();
    let inbox = inbox(&mut h);
    assert_eq!(inbox["items"][0]["stop"], "no_report", "{inbox}");
    assert_eq!(
        inbox["items"][0]["observer_reason"],
        "테스트 실행을 기다리다 멈춤"
    );
    // The Task page keeps the wake and the diagnosis, and says which text it
    // read: the screen, as the worker had no turn or answer text.
    let detail = h.engine.show(&f, &t).unwrap();
    assert!(detail.woke_at.is_some() && detail.diagnosed_at.is_some());
    assert_eq!(detail.diagnosed_from, Some(WorkerTextSource::Screen));
}

#[test]
fn hide_ai_off_stops_a_quiet_worker_without_a_diagnosis() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Off", &[]);
    h.world().judgment_failure = Some("disabled".into());
    let since = h.world().now;
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Resting { since });
    for _ in 0..4 {
        h.advance(2 * MINUTE_MS);
        h.engine.tick();
    }
    let task = h.task(&f, &t);
    assert_eq!(
        (task.state, task.stop),
        (TaskState::Stopped, Some(StopReason::NoReport))
    );
    assert_eq!(task.diagnosis, None);
    assert!(
        letters_to(&h, &t)
            .iter()
            .any(|b| b.contains("done, ask, block")),
        "still woken once"
    );
}

// ------------------------------------------------------- vanished workers

#[test]
fn a_vanished_worker_restarts_once_then_stops_and_a_retry_gives_the_restart_back() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Vanishes", &[]);
    let gone = |h: &mut Bench| {
        h.world()
            .worker_status
            .insert(t.clone(), WorkerStatus::Gone);
        h.advance(5 * MINUTE_MS);
        h.engine.tick();
    };
    // While its agent's usage is used up the restart waits, unspent.
    let runtime = h.task(&f, &t).worker.unwrap().runtime;
    let until = h.world().now + HOUR_MS;
    h.world().usage_limits.insert(runtime, until);
    gone(&mut h);
    let task = h.task(&f, &t);
    assert_eq!((task.state, task.auto_restarts), (TaskState::Running, 0));
    h.advance(HOUR_MS);
    h.world().usage_limits.clear();
    gone(&mut h);
    let task = h.task(&f, &t);
    assert_eq!(task.state, TaskState::Running, "back in the same worktree");
    assert_eq!(task.auto_restarts, 1);
    let resumed = h.world().spawned.last().cloned().unwrap();
    assert!(resumed.resume.is_some(), "the same worktree and session");
    gone(&mut h);
    let task = h.task(&f, &t);
    assert_eq!(
        (task.state, task.stop),
        (TaskState::Stopped, Some(StopReason::WorkerGone))
    );
    h.op(Command::Retry { task: t.clone() });
    assert_eq!(h.task(&f, &t).auto_restarts, 0);
}

#[test]
fn a_vanished_worker_whose_restart_is_refused_stops_for_a_person_instead_of_asking_again() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Refused again", &[]);
    let asked = h.world().spawn_asks.len();
    h.world().spawn_failure = Some(Failure::task("worker.spawn", "refused"));
    for _ in 0..3 {
        h.world()
            .worker_status
            .insert(t.clone(), WorkerStatus::Gone);
        h.advance(5 * MINUTE_MS);
        h.engine.tick();
    }
    let task = h.task(&f, &t);
    assert_eq!(
        (task.state, task.stop),
        (TaskState::Stopped, Some(StopReason::WorkerGone))
    );
    assert_eq!(h.world().spawn_asks.len() - asked, 1, "one restart asked");
}

#[test]
fn closing_a_worker_pane_in_hide_pauses_its_task_for_a_person() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Closed", &[]);
    let pane = h.task(&f, &t).worker.unwrap().pane.unwrap();
    h.engine.worker_closed(&pane);
    let task = h.task(&f, &t);
    assert_eq!(
        (task.state, task.pause_reason),
        (TaskState::Paused, Some(PauseReason::PaneClosed))
    );
    h.world()
        .worker_status
        .insert(t.clone(), WorkerStatus::Gone);
    h.advance(10 * MINUTE_MS);
    h.engine.tick();
    assert_eq!(
        h.state(&f, &t),
        TaskState::Paused,
        "not started again on its own"
    );
    let inbox = inbox(&mut h);
    assert_eq!(inbox["items"][0]["result_code"], "resume_worker", "{inbox}");
    assert_eq!(inbox["count"], 1);
    h.op(Command::Resume { task: t.clone() });
    tick_until_state(&mut h, &f, &t, TaskState::Running);
    assert!(h.world().spawned.last().unwrap().resume.is_some());
}

#[test]
fn a_worker_that_woke_in_a_fresh_pane_stays_its_tasks_worker_there() {
    let mut h = Bench::new(false);
    let f = h.factory(false);
    let claude = h.ready("Keeps its pane", &[]);
    assert_eq!(config(&mut h, "workers", r#"[{"agent":"pi"}]"#)["ok"], true);
    let t = h.ready("Moves", &[]);
    let before = h.task(&f, &t).worker.expect("worker");
    let (pane, agent) = (before.pane.clone().unwrap(), before.agent.clone());
    // Only an agent that closes its pane to sleep has a pane to follow.
    let sleepers: Vec<_> = h
        .engine
        .sleeper_panes()
        .map(|(task, worker)| (task.id.clone(), worker.pane.clone()))
        .collect();
    assert_eq!(sleepers, [(t.clone(), before.pane.clone())], "not {claude}");

    let old = (pane.as_str(), agent.as_deref());
    let moved = h
        .engine
        .worker_rebound(&f, old, "pane-woken", "agent-woken")
        .expect("the Task holds the worker");
    assert_eq!(
        (moved.pane.as_deref(), moved.agent.as_deref()),
        (Some("pane-woken"), Some("agent-woken"))
    );
    assert_eq!(
        WorkerRef {
            pane: before.pane.clone(),
            agent: before.agent.clone(),
            ..moved.clone()
        },
        before,
        "nothing else of the worker moved"
    );
    assert_eq!(
        h.engine.role_for(Some("pane-woken"), None),
        Some((f.clone(), t.clone())),
        "its reports are the Task's from the new pane"
    );
    assert_eq!(
        h.engine.role_for(Some(&pane), None),
        None,
        "the closed pane is no worker"
    );
    // Asked again after it was applied, and from a pane no Task holds.
    assert_eq!(
        h.engine
            .worker_rebound(&f, old, "pane-woken", "agent-woken"),
        Some(moved)
    );
    assert!(
        h.engine
            .worker_rebound(&f, ("pane-else", None), "pane-x", "agent-x")
            .is_none()
    );
}

#[test]
fn a_wake_the_core_could_not_finish_stops_the_task_once_for_a_person() {
    let mut h = Bench::new(false);
    let f = h.factory(false);
    assert_eq!(
        config(&mut h, "workers", r#"[{"agent":"omp"}]"#)["ok"],
        true
    );
    let t = h.ready("Cannot wake", &[]);
    let worker = h.task(&f, &t).worker.expect("worker");
    let old = (worker.pane.as_deref().unwrap(), worker.agent.as_deref());
    assert!(h.engine.worker_wake_failed(&f, old, "wake_failed"));
    let task = h.task(&f, &t);
    assert_eq!(
        (task.state, task.stop, task.stop_detail.as_deref()),
        (
            TaskState::Stopped,
            Some(StopReason::WorkerStart),
            Some("wake_failed")
        )
    );
    assert!(
        !h.engine.worker_wake_failed(&f, old, "wake_failed"),
        "the same failure read again stops nothing"
    );
}

// ---------------------------------------------------------- Factory pause

#[test]
fn a_paused_factory_starts_nothing_asks_no_ai_and_delivers_answers_on_resume() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let running = h.ready("Running", &[]);
    block(&mut h, &f, &running, "Which?");
    h.engine.tick();
    let paused = h.op(Command::PauseFactory { project: None });
    assert_eq!(paused["ok"], true, "{paused}");
    let waiting = h.add("Arrives while paused", &[]);
    let waiting = waiting["task"]["id"].as_str().unwrap().to_owned();
    let calls = h.world().judged.len();
    for _ in 0..3 {
        h.engine.tick();
    }
    assert_eq!(h.world().judged.len(), calls, "no AI judgment while paused");
    assert_eq!(
        h.state(&f, &waiting),
        TaskState::Drafting,
        "reviewed on resume"
    );
    // A person still answers; the worker hears it on resume.
    let question = h.task(&f, &running).questions[0].id.clone();
    h.op(Command::Answer {
        task: running.clone(),
        question: Some(question),
        choice: Some("approve".into()),
        text: None,
        change: false,
        decision: None,
    });
    for _ in 0..3 {
        h.engine.tick();
    }
    assert_ne!(
        h.state(&f, &running),
        TaskState::Running,
        "no start while paused"
    );
    // It survives a daemon restart.
    let mut h = h.restart();
    assert_eq!(
        h.op(Command::Status { project: None })["factories"][0]["paused"],
        true
    );
    h.op(Command::ResumeFactory { project: None });
    tick_until_state(&mut h, &f, &running, TaskState::Running);
    assert!(
        h.world()
            .wakes
            .iter()
            .any(|(task, body)| *task == running && body.contains("approve"))
    );
    tick_until_state(&mut h, &f, &waiting, TaskState::Running);
}

#[test]
fn a_task_reported_while_its_factory_is_paused_is_checked_on_resume_and_merges_only_then() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Reported while paused", &[]);
    h.op(Command::PauseFactory { project: None });
    // The worker's turn was still running: it reports after the pause.
    h.done(&f, &t);
    let premerge = h.world().premerge_calls;
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(
        h.world().premerge_calls,
        premerge,
        "no merge read while paused"
    );
    let task = h.task(&f, &t);
    assert!(!task.gates.contains(&Gate::CheckFailed), "{:?}", task.gates);
    assert_eq!(task.state, TaskState::Verifying);
    let drift = |h: &Bench| {
        h.world().judged.iter().any(|j| {
            matches!(j.input, JudgmentInput::Drift { .. }) && j.task.as_deref() == Some(t.as_str())
        })
    };
    assert!(!drift(&h), "no check is asked while paused");
    h.op(Command::ResumeFactory { project: None });
    for _ in 0..10 {
        h.engine.tick();
        if matches!(h.state(&f, &t), TaskState::Landed | TaskState::Done) {
            break;
        }
    }
    assert!(drift(&h));
    let task = h.task(&f, &t);
    assert!(
        matches!(task.state, TaskState::Landed | TaskState::Done),
        "{:?} {:?}",
        task.state,
        task.gates
    );
}

#[test]
fn a_task_verified_while_its_factory_is_paused_goes_to_a_person_s_merge() {
    let mut h = Bench::new(false);
    let f = h.factory(false);
    let t = h.ready("Verified while paused", &[]);
    // Its checks are asked before the pause and answer after it.
    h.world().hold_judgments = true;
    h.done(&f, &t);
    h.op(Command::PauseFactory { project: None });
    h.world().hold_judgments = false;
    let premerge = h.world().premerge_calls;
    for _ in 0..3 {
        h.engine.tick();
    }
    assert_eq!(
        h.state(&f, &t),
        TaskState::MergeWaiting,
        "a person can merge (B41)"
    );
    assert_eq!(
        h.world().premerge_calls,
        premerge + 1,
        "its merge check runs once, not per tick"
    );
    let merged = h.op(Command::Merge { task: t.clone() });
    assert_eq!(merged["ok"], true, "{merged}");
    assert!(matches!(
        h.state(&f, &t),
        TaskState::Landed | TaskState::Done
    ));
}

#[test]
fn a_task_its_merge_check_sends_back_during_a_pause_reaches_its_worker_only_on_resume() {
    for (answer, letter) in [
        (
            PreMerge::Conflict {
                files: vec!["src/lib.rs".into()],
            },
            "rebase",
        ),
        (
            PreMerge::QuickCheckFailed {
                check: "cargo check".into(),
            },
            "검증 실패",
        ),
    ] {
        let mut h = Bench::new(false);
        let f = h.factory(false);
        let t = h.ready("Sent back while paused", &[]);
        h.world().hold_judgments = true;
        h.done(&f, &t);
        h.op(Command::PauseFactory { project: None });
        h.world()
            .premerge
            .insert(t.clone(), [answer].into_iter().collect());
        h.world().hold_judgments = false;
        let wakes = h.world().wakes.len();
        let letters = letters_to(&h, &t).len();
        for _ in 0..3 {
            h.engine.tick();
        }
        assert_eq!(h.state(&f, &t), TaskState::Running, "{letter}");
        assert_eq!(h.world().wakes.len(), wakes, "no turn starts: {letter}");
        assert_eq!(letters_to(&h, &t).len(), letters, "{letter}");
        assert!(h.task(&f, &t).worker.expect("worker").asleep, "{letter}");
        h.op(Command::ResumeFactory { project: None });
        let woken: Vec<String> = h
            .world()
            .wakes
            .iter()
            .filter(|(to, _)| to == &t)
            .map(|(_, body)| body.clone())
            .collect();
        assert_eq!(woken.len(), 1, "{woken:?}");
        assert!(woken[0].contains(letter), "{woken:?}");
    }
}

#[test]
fn a_worker_whose_agent_cannot_sleep_is_never_counted_asleep_and_hears_a_pause_s_send_back_on_resume()
 {
    let mut h = Bench::new(false);
    let f = h.factory(false);
    let sleepless = Runtime::parse("opencode").expect("opencode declares a start");
    h.world().sleepless.push(sleepless);
    assert_eq!(
        config(&mut h, "workers", r#"[{"agent":"opencode"}]"#)["ok"],
        true
    );
    let t = h.ready("Sleepless", &[]);
    assert_eq!(h.task(&f, &t).worker.expect("worker").runtime, sleepless);
    h.world().hold_judgments = true;
    h.done(&f, &t);
    h.op(Command::PauseFactory { project: None });
    h.world().premerge.insert(
        t.clone(),
        [PreMerge::Conflict {
            files: vec!["src/lib.rs".into()],
        }]
        .into_iter()
        .collect(),
    );
    h.world().hold_judgments = false;
    let letters = letters_to(&h, &t).len();
    for _ in 0..3 {
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::Running);
    assert!(
        !h.task(&f, &t).worker.expect("worker").asleep,
        "its agent kept working"
    );
    assert_eq!(letters_to(&h, &t).len(), letters, "held while paused");
    h.op(Command::ResumeFactory { project: None });
    let sent: Vec<String> = letters_to(&h, &t).split_off(letters);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("rebase"), "{sent:?}");
}

#[test]
fn a_pause_names_a_worker_whose_agent_cannot_sleep_and_a_cancel_leaves_it_awake() {
    let mut h = Bench::new(false);
    let f = h.factory(false);
    let sleepless = Runtime::parse("opencode").expect("opencode declares a start");
    h.world().sleepless.push(sleepless);
    assert_eq!(
        config(&mut h, "workers", r#"[{"agent":"opencode"}]"#)["ok"],
        true
    );
    let t = h.ready("Keeps working", &[]);
    assert_eq!(h.state(&f, &t), TaskState::Running);
    let paused = h.op(Command::PauseFactory { project: None });
    assert_eq!(paused["awake"], json!([t]), "{paused}");
    assert!(!h.task(&f, &t).worker.expect("worker").asleep);
    h.op(Command::ResumeFactory { project: None });
    assert_eq!(h.op(Command::Cancel { task: t.clone() })["ok"], true);
    assert!(
        !h.task(&f, &t).worker.expect("worker").asleep,
        "its agent stays awake in its pane"
    );
}

#[test]
fn a_task_only_an_automatic_merge_would_take_waits_out_a_pause_without_reading_anything() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Auto while paused", &[]);
    h.world().hold_judgments = true;
    h.done(&f, &t);
    h.op(Command::PauseFactory { project: None });
    h.world().hold_judgments = false;
    for _ in 0..3 {
        h.engine.tick();
    }
    let premerge = h.world().premerge_calls;
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::Verifying);
    assert_eq!(h.world().premerge_calls, premerge, "nothing read per tick");
    h.op(Command::ResumeFactory { project: None });
    for _ in 0..10 {
        h.engine.tick();
        if matches!(h.state(&f, &t), TaskState::Landed | TaskState::Done) {
            break;
        }
    }
    assert!(matches!(
        h.state(&f, &t),
        TaskState::Landed | TaskState::Done
    ));
}

#[test]
fn a_check_that_fails_during_a_pause_holds_the_task_for_a_person_with_its_gate() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Check fails while paused", &[]);
    h.world().hold_judgments = true;
    h.done(&f, &t);
    h.op(Command::PauseFactory { project: None });
    h.world().judgment_failure = Some("timeout".into());
    h.world().hold_judgments = false;
    tick_until_state(&mut h, &f, &t, TaskState::MergeWaiting);
    assert!(
        h.task(&f, &t).gates.contains(&Gate::CheckFailed),
        "{:?}",
        h.task(&f, &t).gates
    );
    let merged = h.op(Command::Merge { task: t.clone() });
    assert_eq!(merged["ok"], true, "{merged}");
}

#[test]
fn a_risk_merge_approved_after_the_factory_paused_waits_for_the_resume() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    mode(&mut h, "autonomous");
    h.world().risk_merge = Some(json!({"approve": true, "reason": "카드 범위"}));
    let t = risk_only(&mut h, &f, "Infra");
    // The merge judgment is asked; the Factory pauses before its answer.
    tick_until_state(&mut h, &f, &t, TaskState::MergeWaiting);
    h.op(Command::PauseFactory { project: None });
    let attempts = h.world().merge_attempts;
    for _ in 0..5 {
        h.engine.tick();
    }
    assert_eq!(h.state(&f, &t), TaskState::MergeWaiting);
    assert_eq!(
        h.world().merge_attempts,
        attempts,
        "nothing merges while paused"
    );
    h.op(Command::ResumeFactory { project: None });
    for _ in 0..10 {
        h.engine.tick();
        if matches!(h.state(&f, &t), TaskState::Landed | TaskState::Done) {
            break;
        }
    }
    assert!(matches!(
        h.state(&f, &t),
        TaskState::Landed | TaskState::Done
    ));
}

#[test]
fn pausing_puts_running_workers_to_sleep() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    let t = h.ready("Sleeps", &[]);
    let paused = h.op(Command::PauseFactory { project: None });
    assert_eq!(paused["awake"], json!([]), "{paused}");
    assert!(h.task(&f, &t).worker.unwrap().asleep);
    assert!(h.world().sleeps.contains(&t));
    h.op(Command::ResumeFactory { project: None });
    assert!(!h.task(&f, &t).worker.unwrap().asleep);
}

// ------------------------------------------------------- worker candidates

fn candidates(h: &mut Bench) {
    let answer = config(
        h,
        "workers",
        r#"[{"agent":"claude","description":"작은 일"},{"agent":"codex","model":"gpt-5.5","effort":"high","description":"큰 리팩터링"}]"#,
    );
    assert_eq!(answer["ok"], true, "{answer}");
}

#[test]
fn the_review_picks_a_candidate_and_the_worker_starts_with_its_model_and_effort() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    candidates(&mut h);
    h.world().intake.insert(
        "Refactor".into(),
        json!({"questions": [], "dependencies": [], "flags": [], "split": [], "fits_scope": null, "worker": 1, "worker_reason": "큰 변경"}),
    );
    let t = h.ready("Refactor", &[]);
    // The pick shows before the worker starts, so a person can change it.
    let detail = h.engine.show(&f, &t).unwrap();
    assert_eq!(
        (detail.ai_picked_worker, detail.ai_pick_reason.as_deref()),
        (Some(2), Some("큰 변경"))
    );
    let review = h
        .world()
        .judged
        .iter()
        .find_map(|j| match &j.input {
            JudgmentInput::IntakeReview { workers, .. } => Some(workers.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(review.len(), 2);
    assert_eq!(review[1].description, "큰 리팩터링");
    tick_until_state(&mut h, &f, &t, TaskState::Running);
    let spawn = h
        .world()
        .spawned
        .iter()
        .find(|s| s.task == t)
        .cloned()
        .unwrap();
    assert_eq!(spawn.runtime, Runtime::CODEX);
    // The choice is the spawn's own, so a restart can carry it; it never
    // rides in the configured arguments.
    assert_eq!(
        (spawn.model.as_deref(), spawn.effort.as_deref()),
        (Some("gpt-5.5"), Some("high"))
    );
    assert!(
        !spawn
            .args
            .iter()
            .any(|arg| arg == "gpt-5.5" || arg.contains("model_reasoning_effort"))
    );
    let detail = h.engine.show(&f, &t).unwrap();
    let worker = detail.worker.unwrap();
    assert_eq!(worker.picked.as_deref(), Some("큰 리팩터링"));
    assert_eq!(worker.pick_reason.as_deref(), Some("큰 변경"));
    // The review's pick is not an Observer call.
    let view = &h.op(Command::Status { project: None })["factories"][0];
    assert_eq!(view["observer_today"], 0);
}

#[test]
fn a_person_s_pin_wins_over_the_review_and_a_failed_review_starts_the_first() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    candidates(&mut h);
    let pinned = h.add_card(CardInput {
        worker: Some(1),
        ..card("Pinned", &[])
    });
    let pinned = pinned["task"]["id"].as_str().unwrap().to_owned();
    h.world().intake.insert(
        "Pinned".into(),
        json!({"questions": [], "dependencies": [], "flags": [], "split": [], "fits_scope": null, "worker": 1, "worker_reason": "x"}),
    );
    tick_until_state(&mut h, &f, &pinned, TaskState::Running);
    let spawn = h
        .world()
        .spawned
        .iter()
        .find(|s| s.task == pinned)
        .cloned()
        .unwrap();
    assert_eq!(
        spawn.runtime,
        Runtime::CLAUDE,
        "--worker 1 is the first candidate"
    );
    let refused = h.add_card(CardInput {
        worker: Some(3),
        ..card("Out of range", &[])
    });
    assert_eq!(refused["reason"], "worker_out_of_range", "{refused}");
}

#[test]
fn worker_and_observer_settings_are_checked_and_old_values_keep_reading() {
    let mut h = Bench::new(false);
    let f = h.factory(true);
    // A Factory from before candidates reads its default_runtime as one.
    assert_eq!(
        h.op(Command::Status { project: None })["factories"][0]["workers"],
        json!([{"agent": "claude", "description": ""}])
    );
    let bad = config(&mut h, "observer_mode", "auto");
    assert_eq!(bad["reason"], "config_invalid");
    assert_eq!(
        bad["detail"]["allowed"],
        json!(["manual", "assist", "autonomous"])
    );
    let bad = config(&mut h, "observer_daily_limit", "1001");
    assert_eq!(
        (bad["reason"].clone(), bad["detail"]["max"].clone()),
        (json!("out_of_range"), json!(1000))
    );
    let six = format!("[{}]", [r#"{"agent":"claude"}"#; 6].join(","));
    assert_eq!(config(&mut h, "workers", &six)["reason"], "out_of_range");
    let long = json!([{"agent": "claude", "description": "가".repeat(201)}]).to_string();
    let refused = config(&mut h, "workers", &long);
    assert_eq!(
        (refused["reason"].clone(), refused["detail"]["max"].clone()),
        (json!("worker_description_too_long"), json!(200))
    );
    h.world().missing_agents.push(Runtime::CODEX);
    let missing = config(&mut h, "default_runtime", "codex");
    assert_eq!(missing["reason"], "agent_not_installed", "{missing}");
    let unknown = config(&mut h, "default_runtime", "future-agent");
    assert_eq!(unknown["reason"], "agent_not_startable", "{unknown}");
    let effort = config(
        &mut h,
        "workers",
        r#"[{"agent":"claude","effort":"ultra"}]"#,
    );
    assert_eq!(effort["reason"], "config_invalid", "{effort}");
    // Every agent whose adapter declares a start can work (B28); one that
    // declares no launch model refuses a candidate naming one.
    let model = config(
        &mut h,
        "workers",
        r#"[{"agent":"opencode","model":"fixture-model"}]"#,
    );
    assert_eq!(model["reason"], "config_invalid", "{model}");
    h.world().missing_agents.clear();
    assert_eq!(config(&mut h, "default_runtime", "codex")["ok"], true);
    let factory = h.engine.factories().next().unwrap().clone();
    assert_eq!(factory.config.default_runtime, Runtime::CODEX);
    assert!(
        factory.config.workers.is_empty(),
        "written only when the workers change"
    );
    h.world().refused_ai.push("codex".into());
    let refused = config(&mut h, "factory_ai", "codex");
    assert_eq!(refused["reason"], "factory_ai_unavailable", "{refused}");
    assert_eq!(h.engine.factories().next().unwrap().config.factory_ai, None);
    let fits = json!([{"agent": "claude", "description": "가".repeat(200)}]).to_string();
    assert_eq!(config(&mut h, "workers", &fits)["ok"], true);
    let _ = f;
}

#[test]
fn every_factory_judgment_runs_on_the_factory_ai() {
    let mut h = Bench::new(false);
    let _f = h.factory(true);
    for (key, value) in [
        ("factory_ai", "codex"),
        ("factory_ai_model", "gpt-5.5"),
        ("factory_ai_effort", "high"),
    ] {
        let answer = config(&mut h, key, value);
        assert_eq!(answer["ok"], true, "{answer}");
    }
    h.ready("Judged", &[]);
    let ai = h.world().judged[0].ai.clone().unwrap();
    assert_eq!(
        (
            ai.provider.as_str(),
            ai.model.as_deref(),
            ai.effort.as_deref()
        ),
        ("codex", Some("gpt-5.5"), Some("high"))
    );
}
