//! `hide mission show` as text (with-hide/hide-private#552): the core
//! answers one value (`runtime::missions`' `MissionView`, or one node's view
//! with its worker's newest letters), and this draws it for a Conductor to
//! read at a glance, as `session_cli` draws a session. `--json` prints the
//! value as it is. The mission's head says where it stands and what waits;
//! each node is one line; `show <node>` is that node in full.

use crate::session_cli::{age, one_line};
use hide_core::missions::{AttentionTo, WaitingOn};
use serde_json::Value;
use std::fmt::Write as _;

/// A node's warning shown beside the fact it warns of.
const WARN: &str = "⚠";

/// The core's answer to `show` as text, read at `now`; refused when a
/// node's `waiting_on` is not one this build reads.
pub fn draw(result: &Value, now: u64) -> Result<String, String> {
    let result = shown(result);
    if result["node"].is_object() {
        draw_node(&result, now)
    } else {
        draw_mission(&result["mission"], now)
    }
}

/// `value` with every control character a space (`one_line`), but for the
/// line breaks of an `evidence` text: a checker's findings, which its
/// worker reads here, keep the lines they were written in
/// (`evidence_words`).
fn shown(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| {
                    let field = match field {
                        Value::String(text) if key == "evidence" => Value::String(
                            text.lines()
                                .map(|line| one_line(&Value::from(line)))
                                .filter_map(|line| line.as_str().map(str::to_owned))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        other => shown(other),
                    };
                    (key.clone(), field)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(shown).collect()),
        other => one_line(other),
    }
}

/// A line's or a checker's evidence, as the words on the line it answers
/// and the lines under it: one line stays on that line; several, a
/// checker's findings, go each on its own line under it, indented by
/// `indent`, so a whole review reads as it was written.
fn evidence_words(evidence: &str, indent: &str) -> (String, String) {
    if !evidence.contains('\n') {
        return (format!(" · evidence {evidence}"), String::new());
    }
    let mut under = String::new();
    for line in evidence.lines() {
        under.push('\n');
        if !line.trim().is_empty() {
            let _ = write!(under, "{indent}{line}");
        }
    }
    (" · evidence".to_owned(), under)
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("-")
}

fn mission_status(status: &Value) -> &'static str {
    match status.as_str() {
        Some("planning") => "planning",
        Some("running") => "running",
        Some("merge_wait") => "waiting for merge",
        Some("done") => "done",
        Some("cancelled") => "cancelled",
        _ => "-",
    }
}

fn node_status(node: &Value) -> String {
    let word = match node["status"].as_str() {
        Some("planned") => "planned",
        Some("working") => "working",
        Some("done") => "done",
        Some("dropped") => "dropped",
        _ => "-",
    };
    match (node["status"].as_str(), node["agent_runtime"].as_str()) {
        (Some("working"), Some("asleep")) => format!("{word} (asleep)"),
        (Some("working"), Some("ended")) => format!("{word} (worker ended)"),
        _ => word.to_owned(),
    }
}

/// A node's kind in the operator's words (PRD node-presets D-10, D-13): a
/// code node, a general node with its preset's name from the core's one
/// table, or the waiting node that waits on work outside the mission.
fn kind_words(node: &Value) -> String {
    match text(node, "kind") {
        "change" => "code".to_owned(),
        "external" => "watching".to_owned(),
        "general" => {
            let row = hide_core::missions::PRESETS
                .iter()
                .find(|row| row.preset.is_some() && Some(row.key) == node["preset"].as_str())
                .unwrap_or_else(|| hide_core::missions::preset_row(None));
            match row.preset {
                Some(_) => format!("general · {}", row.key),
                None => "general".to_owned(),
            }
        }
        other => other.to_owned(),
    }
}

fn pr_word(state: &Value) -> &'static str {
    match state.as_str() {
        Some("failed") => "failed",
        Some("pending") => "pending",
        Some("mergeable") => "mergeable",
        Some("draft") => "draft",
        Some("merged") => "merged",
        Some("closed") => "closed",
        _ => "unread",
    }
}

fn line_word(state: &Value) -> &'static str {
    match state.as_str() {
        Some("pass") => "pass",
        Some("fail") => "fail",
        Some("skipped") => "skipped",
        _ => "open",
    }
}

fn reason_word(reason: &Value) -> &'static str {
    match reason.as_str() {
        Some("merged_unproven") => "merged with a finish line not passed",
        Some("worker_ended") => "worker ended with work left",
        Some("pr_closed") => "PR closed without a merge",
        Some("unplanned_pr") => "PR not in the plan",
        Some("conductor_failed") => "Conductor recorded a failure",
        Some("wake_refused") => "could not wake the sleeping worker",
        Some("close_refused") => {
            "work done but a check record is missing, so the mission cannot close"
        }
        Some("human_gate") => "a person's check",
        Some("not_started") => "could start, not started",
        Some("warning_stood") => "worker did not move after the warning",
        Some("wait_stood") => "waited long on CI, a review or the merge",
        Some("ci_failed") => "the awaited CI failed",
        Some("mission_cancelled") => "the awaited mission was cancelled",
        Some("node_dropped") => "the awaited node was dropped",
        Some("wait_expired") => "the wait passed its deadline",
        _ => "needs a look",
    }
}

fn warning(node: &Value, kind: &str) -> Option<Value> {
    node["warnings"]
        .as_array()?
        .iter()
        .find(|warning| warning["kind"] == kind)
        .cloned()
}

fn marked(warned: bool, words: String) -> String {
    if warned {
        format!("{WARN}{words}")
    } else {
        words
    }
}

fn line_of<'a>(node: &'a Value, kind: &str) -> Option<&'a Value> {
    node["criteria"]
        .as_array()?
        .iter()
        .find(|line| line["kind"] == kind)
}

/// The node's review or verification rounds against the mission's limit
/// (`review_rounds`), marked when the round at the limit failed and warns.
fn review_words(node: &Value) -> Option<String> {
    Some(marked(
        warning(node, "review_fails").is_some(),
        rounds_words(node)?,
    ))
}

/// `review 2/3 rounds`: the rounds its checkers answered of those allowed.
fn rounds_words(node: &Value) -> Option<String> {
    let rounds = &node["review_rounds"];
    let line = if rounds["line"] == "verify" {
        "verify"
    } else {
        "review"
    };
    Some(format!(
        "{line} {}/{} rounds",
        rounds["rounds"].as_u64()?,
        rounds["limit"].as_u64()?
    ))
}

/// Where the node's time went (`ready_at_unix_ms`, `started_at_unix_ms`,
/// `worked_at_unix_ms`, `own_met_at_unix_ms`, `finished_at_unix_ms`): work
/// from when it last went to work (its start, or back from a finish) until
/// its worker's lines stood met, review·CI from then until it finished, and
/// waited from when it could have started until it did; a span still
/// running counts to `now`. A part the record cannot say (no start yet, a
/// book from before the stamps) is left out, never shown empty, a wait
/// under a minute says nothing, and a node that started says nothing when
/// neither work nor review·CI can be said.
fn time_words(node: &Value, now: u64) -> Option<String> {
    let at = |field: &str| node[field].as_u64();
    let started = at("started_at_unix_ms");
    let from = at("worked_at_unix_ms").or(started);
    let finished = at("finished_at_unix_ms");
    let met = at("own_met_at_unix_ms");
    let end = finished.unwrap_or(now);
    let work = match (from, met) {
        (Some(from), Some(met)) => Some(met.saturating_sub(from)),
        (Some(from), None) if finished.is_none() => Some(now.saturating_sub(from)),
        _ => None,
    };
    let review = met.map(|met| end.saturating_sub(met));
    let wait = at("ready_at_unix_ms")
        .and_then(|ready| match started {
            Some(start) => Some(start.saturating_sub(ready)),
            None if node["status"] == "planned" => Some(now.saturating_sub(ready)),
            None => None,
        })
        .filter(|wait| *wait >= MINUTE_MS);
    // A wait alone never stands for the time of a node that started.
    if work.is_none() && review.is_none() && started.is_some() {
        return None;
    }
    let parts: Vec<String> = [("work", work), ("review·CI", review), ("waited", wait)]
        .into_iter()
        .filter_map(|(word, span)| Some(format!("{word} {}", span_words(span?))))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

const MINUTE_MS: u64 = 60_000;

/// A span as the node's time reads it: whole minutes under an hour, else
/// hours to one decimal (`40m`, `1.5h`, `9h`).
fn span_words(ms: u64) -> String {
    let minutes = ms / MINUTE_MS;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let tenths = (ms + 3 * MINUTE_MS) / (6 * MINUTE_MS);
    match tenths % 10 {
        0 => format!("{}h", tenths / 10),
        tenth => format!("{}.{tenth}h", tenths / 10),
    }
}

/// Its last push as a fact, when Hide read one: `push 12m ago`.
fn push_words(node: &Value, now: u64) -> Option<String> {
    let at = node["pushed_at_unix_ms"].as_u64()?;
    Some(format!("push {}", minutes_ago(now, at)))
}

/// What the node waits on that its Conductor set (`Mission::wait_of`), as
/// the map says it: `waiting · <why> · <left> left`, the time gone once
/// its end has passed and Hide has not lifted it yet.
fn wait_words(node: &Value, now: u64) -> Option<String> {
    let wait = node["wait"].as_object()?;
    let why = wait.get("why")?.as_str()?;
    let until = wait.get("until_unix_ms")?.as_u64()?;
    Some(if until > now {
        format!("waiting · {why} · {} left", span_words(until - now))
    } else {
        format!("waiting · {why}")
    })
}

/// Why a stopped worker stopped (`StopCause`): a usage limit or an API
/// error with the agent's own line, or no report.
fn cause_words(cause: &Value) -> String {
    let line = cause["line"].as_str().unwrap_or_default();
    match cause["kind"].as_str() {
        Some("limit") => format!("usage limit · {line}"),
        Some("error") => format!("API error · {line}"),
        _ => "no report".to_owned(),
    }
}

/// A node whose worker stopped with nothing to wait on (`work` `stopped`):
/// since when and why, marked when its warning stands.
fn stop_words(node: &Value, now: u64) -> Option<String> {
    let work = &node["work"];
    if work["state"] != "stopped" {
        return None;
    }
    let since = work["since_unix_ms"].as_u64()?;
    Some(marked(
        warning(node, "stopped").is_some(),
        format!(
            "stopped {} · {}",
            minutes_ago(now, since),
            cause_words(&work["cause"])
        ),
    ))
}

/// How long ago `at` was in minutes up to a day.
fn minutes_ago(now: u64, at: u64) -> String {
    match now.saturating_sub(at) / 60_000 {
        0 => "just now".to_owned(),
        minutes @ 1..=1439 => format!("{minutes}m ago"),
        minutes => format!("{}d ago", minutes / 1440),
    }
}

/// A change request's number as its host writes it (`#12`, `!12`): the label
/// its carried reference has.
///
/// The fallbacks here and in [`change_request_name`] and the stack line
/// serve one pair of builds: this `hide` answered by a daemon of an earlier
/// release (1.2.11 and before carry no reference and no stack words), which
/// meets it while the app on disk was updated and its daemon, which quitting
/// the app leaves running, waits for the app's next start to replace it.
/// The reverse pair needs nothing here: an older `hide` ignores the fields.
fn number_label(number: u64, reference: &Value) -> String {
    reference["label"]
        .as_str()
        .map_or_else(|| format!("#{number}"), str::to_owned)
}

/// A change request as a text names it (`PR #12`, `MR !12`), by the unit of
/// the host its carried reference names; `PR #N` with no reference, from a
/// daemon of an earlier release (see [`number_label`]).
fn change_request_name(number: u64, reference: &Value) -> String {
    match (
        reference["repository"]["host"].as_str(),
        u32::try_from(number),
    ) {
        (Some(host), Ok(number)) => {
            hide_core::change_request_name(host, number, &number_label(number.into(), reference))
        }
        _ => format!("PR #{number}"),
    }
}

fn pr_words(node: &Value) -> Option<String> {
    let number = node["pr"].as_u64()?;
    let mut words = format!(
        "{} {}",
        number_label(number, &node["pr_reference"]),
        pr_word(&node["pr_state"])
    );
    let more = node["prs"].as_array().map_or(0, Vec::len).saturating_sub(1);
    if more > 0 {
        let _ = write!(words, " +{more}");
    }
    if node["pr_state"] != "merged"
        && let Some(ci) = line_of(node, "ci")
    {
        let _ = write!(
            words,
            " · CI {}",
            match (ci["state"].as_str(), node["pr_state"].is_null()) {
                (Some("pass"), _) => "pass",
                (Some("fail"), _) => "fail",
                (Some("skipped"), _) => "skipped",
                // The pull request itself is not read: nothing is running that
                // Hide knows of.
                (_, true) => "unread",
                _ => "running",
            }
        );
    }
    Some(words)
}

/// A node whose plan gives the operator its merge (`merge.why`, PRD
/// mission-destination D-10), and where that merge stands; none for every
/// other node, a scope `pr` mission's included, which reads as before.
fn merge_words(node: &Value) -> Option<&'static str> {
    let merge = &node["merge"];
    merge["why"].as_str()?;
    Some(match merge["state"].as_str() {
        Some("due") => "👤 yours to merge",
        Some("merged") => "👤 merged by the operator",
        _ => "👤 the operator merges",
    })
}

/// What the node waits on, as the core decided it (`waiting_on`); a
/// value this build cannot read is refused rather than drawn as nothing.
fn waits_on(node: &Value) -> Result<Option<String>, String> {
    let waiting: Option<WaitingOn> = serde_json::from_value(node["waiting_on"].clone())
        .map_err(|error| format!("node {} waiting_on: {error}", text(node, "key")))?;
    let Some(waiting) = waiting else {
        return Ok(None);
    };
    let words = match waiting {
        WaitingOn::Attention { to, reason } => {
            let who = match to {
                AttentionTo::Operator => "operator",
                AttentionTo::Conductor => "Conductor",
            };
            let reason = serde_json::to_value(reason).unwrap_or(Value::Null);
            format!("! {who}: {}", reason_word(&reason))
        }
        WaitingOn::Nodes { keys } => format!(
            "waits on {}",
            keys.iter()
                .map(|key| match &key.line {
                    Some(line) => format!("{} line {line}", key.node),
                    None => key.node.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        // A node that starts on a line of another stacks on that one's branch.
        WaitingOn::Start => match node["stack"]["on"].as_str() {
            Some(on) => format!("can start · stacked on {on}"),
            None => "not started".to_owned(),
        },
        WaitingOn::Below { node: below } => format!("waits on {below} merging first"),
        WaitingOn::Decision => "waits on the operator's decision".to_owned(),
        WaitingOn::Blocked => "blocked · a node before it was dropped".to_owned(),
        WaitingOn::Worker => "waits on the worker".to_owned(),
        WaitingOn::Review => "waits on review".to_owned(),
        WaitingOn::Verify => "waits on verify".to_owned(),
        WaitingOn::Ci => "waits on CI".to_owned(),
        WaitingOn::Merge => "waits on the merge".to_owned(),
        WaitingOn::Conductor => "waits on the Conductor".to_owned(),
    };
    Ok(Some(words))
}

fn warning_count(node: &Value) -> usize {
    node["warnings"].as_array().map_or(0, Vec::len)
}

fn draw_mission(mission: &Value, now: u64) -> Result<String, String> {
    let nodes = mission["nodes"].as_array().cloned().unwrap_or_default();
    let count = |status: &str| nodes.iter().filter(|node| node["status"] == status).count();
    let dropped = count("dropped");
    let mut head = format!(
        "{} · {} · {} · done {}/{}",
        text(mission, "id"),
        text(mission, "title"),
        mission_status(&mission["status"]),
        count("done"),
        nodes.len() - dropped,
    );
    if dropped > 0 {
        let _ = write!(head, " · dropped {dropped}");
    }
    let _ = write!(
        head,
        " · open decision {} · warnings {}",
        usize::from(mission["decision"].is_string()),
        nodes.iter().map(warning_count).sum::<usize>(),
    );
    let mut out = format!("{head}\n");
    // Where the results go; a mission from before destinations names none
    // and reads as it did.
    if let Some(destination) = mission["destination"].as_str() {
        let _ = writeln!(out, "destination · {destination}");
    }
    // The scope's line, said only when the plan gave the operator a merge
    // the scope does not (PRD mission-destination B18).
    let mine: Vec<&str> = nodes
        .iter()
        .filter(|node| node["merge"]["why"].is_string())
        .map(|node| text(node, "key"))
        .collect();
    if !mine.is_empty() {
        let _ = writeln!(
            out,
            "through the merge · 👤 the operator merges {}",
            mine.join(", ")
        );
    }
    if let Some(decision) = mission["decision"].as_str() {
        let _ = writeln!(out, "decision · {decision}");
    }
    if mission["conductor"]["present"] == false && mission["status"] != "done" {
        let _ = writeln!(out, "no Conductor · take it over with hide mission take");
    }
    if mission["stale"] == true {
        let _ = writeln!(out, "the mission's device does not answer · last values");
    }
    if mission["github_stale"] == true {
        let _ = writeln!(
            out,
            "GitHub read failed · values from {}",
            age(now, &mission["github_read_at_unix_ms"])
        );
    }
    // The operator's one card for the raises the Conductor has not settled.
    if let Some(net) = mission["safety_net"].as_object() {
        let held = net["nodes"].as_array().cloned().unwrap_or_default();
        let keys: Vec<&str> = held.iter().map(|node| text(node, "key")).collect();
        let _ = writeln!(
            out,
            "safety net · {} held · since {} · shown to the operator · {}",
            held.len(),
            age(now, &net["since_unix_ms"]),
            keys.join(" "),
        );
    }
    for node in &nodes {
        let mut fields = vec![text(node, "key").to_owned(), node_status(node)];
        fields.extend(time_words(node, now));
        fields.extend(review_words(node));
        fields.extend(push_words(node, now));
        fields.extend(wait_words(node, now));
        fields.extend(stop_words(node, now));
        fields.extend(pr_words(node));
        fields.extend(merge_words(node).map(str::to_owned));
        fields.extend(waits_on(node)?);
        let mark = if warning_count(node) > 0 { WARN } else { " " };
        let _ = writeln!(out, "{mark} {}", fields.join(" · "));
    }
    // What the Conductor settled on its own, as the map's Decided by the Conductor.
    let alone = mission["skipped_alone"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !alone.is_empty() {
        let _ = writeln!(out, "decided by the Conductor {}", alone.len());
        for skip in &alone {
            let follow = skip["follow_up"]
                .as_str()
                .map(|node| format!(" · follow-up {node}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  ⤳ {} · {} skipped · {}{follow} · {} · {}",
                text(skip, "node"),
                text(skip, "line"),
                text(skip, "reason"),
                text(skip, "by"),
                age(now, &skip["at_unix_ms"]),
            );
        }
    }
    Ok(out)
}

fn worker_words(node: &Value) -> Option<String> {
    // What the worker was proven launched with, else what the plan asked.
    let worker = [&node["launched"], &node["worker"]]
        .into_iter()
        .find(|worker| worker.is_object())?;
    // The item as the core names it (`Claude · work`, PRD agent-profiles-phase2
    // B12): the agent with its profile's name now, none for the built-in
    // default; a planned profile the machine no longer lists says so.
    let mut words = match worker["item"].as_str() {
        Some(item) if worker["profile_gone"] == true => {
            format!("{} · profile gone", item)
        }
        Some(item) => item.to_owned(),
        None => text(worker, "agent").to_owned(),
    };
    // The model by the name its item's list shows for it (a gateway's
    // `deepseek/…` for `opus`), else as the start passes it.
    let model = worker["model_label"].as_str().or(worker["model"].as_str());
    for part in [model, worker["effort"].as_str()].into_iter().flatten() {
        let _ = write!(words, " {part}");
    }
    if let Some(agent) = node["agent_id"].as_str() {
        let _ = write!(words, " · {agent}");
    }
    if let Some(pane) = node["pane_id"].as_str() {
        let _ = write!(words, " · pane {pane}");
    }
    match node["agent_runtime"].as_str() {
        Some("asleep") => words.push_str(" · asleep"),
        Some("ended") => words.push_str(" · ended"),
        Some("running") => words.push_str(" · awake"),
        _ => {}
    }
    Some(words)
}

fn recorded_words(by: &Value, at: &Value, now: u64) -> String {
    let name = by["name"].as_str().unwrap_or("-");
    format!("{name} {}", age(now, at))
}

fn draw_line(out: &mut String, line: &Value, now: u64) {
    let mut words = format!("  {} {}", text(line, "key"), line_word(&line["state"]));
    // A start gate asks before the node's worker is bound (PRD node-presets
    // D-06); an end gate, the one before, carries no `when`.
    if line["when"] == "start" {
        words.push_str(" · before start");
    }
    if let Some(said) = line["text"].as_str() {
        let _ = write!(words, " · {said}");
    }
    if let Some(rounds) = line["rounds"].as_u64().filter(|rounds| *rounds > 0) {
        let _ = write!(words, " · {rounds} rounds");
    }
    if let Some(commit) = line["commit"].as_str() {
        let _ = write!(words, " · {}", commit.get(..7).unwrap_or(commit));
    }
    if let Some(skip) = line["skip"]["reason"].as_str() {
        let _ = write!(words, " · skipped because {skip}");
    }
    if let Some(node) = line["skip"]["follow_up"].as_str() {
        let _ = write!(words, " · follow-up {node}");
    }
    if let Some(summary) = line["summary"].as_str() {
        let _ = write!(words, " · {summary}");
    }
    let mut under = String::new();
    if let Some(evidence) = line["evidence"].as_str() {
        let (on, below) = evidence_words(evidence, "      ");
        words.push_str(&on);
        under = below;
    }
    if line["by"].is_object() {
        let _ = write!(
            words,
            " · {}",
            recorded_words(&line["by"], &line["by"]["at_unix_ms"], now)
        );
    }
    let _ = writeln!(out, "{words}{under}");
    for checker in line["checkers"].as_array().into_iter().flatten() {
        let mut words = format!(
            "    {} {}",
            text(&checker["worker"], "agent"),
            line_word(&checker["state"])
        );
        if let Some(agent) = checker["agent_id"].as_str() {
            let _ = write!(words, " · {agent}");
        } else {
            words.push_str(" · not bound yet");
        }
        if checker["at_unix_ms"].is_u64() {
            let _ = write!(words, " · {}", age(now, &checker["at_unix_ms"]));
        }
        if let Some(summary) = checker["summary"].as_str() {
            let _ = write!(words, " · {summary}");
        }
        if let Some(evidence) = checker["evidence"].as_str() {
            let (on, below) = evidence_words(evidence, "        ");
            words.push_str(&on);
            words.push_str(&below);
        }
        let _ = writeln!(out, "{words}");
    }
}

fn draw_node(result: &Value, now: u64) -> Result<String, String> {
    let node = &result["node"];
    let mut out = format!(
        "{} · {} · {}\n",
        text(result, "mission_id"),
        text(node, "key"),
        text(node, "title")
    );
    let mut state = vec![node_status(node), kind_words(node)];
    if let Some(delivery) = node["delivery"].as_str() {
        state.push(match delivery {
            "pr" => "by PR".to_owned(),
            "direct" => format!(
                "directly into {}",
                result["destination"].as_str().unwrap_or("main")
            ),
            other => other.to_owned(),
        });
    }
    // Its attention has a line of its own.
    if node["waiting_on"]["kind"] != "attention" {
        state.extend(waits_on(node)?);
    }
    let _ = writeln!(out, "state {}", state.join(" · "));
    // A change's pull request takes this branch as its base, unless it is
    // stacked on another.
    if let (Some(destination), "change") = (result["destination"].as_str(), text(node, "kind")) {
        let _ = writeln!(out, "destination {destination}");
    }
    // A node started on a line of another stacks on that one's branch until
    // its work reaches the destination.
    if let Some(on) = node["stack"]["on"].as_str() {
        let mut words = format!("stacked on {on}");
        // The change request below and the command that opens one on its
        // branch, in the project's host's words, which the core writes.
        // A daemon of an earlier release sends neither and names a stack
        // in GitHub's words, the only host it read (see `number_label`).
        if let Some(pr) = result["stack_pr"].as_str() {
            let _ = write!(words, " · {pr}");
        } else if let Some(pr) = node["stack"]["pr"].as_u64() {
            let _ = write!(words, " · PR #{pr}");
        }
        match node["stack"]["branch"].as_str() {
            Some(branch) => {
                let create = result["stack_create"]
                    .as_str()
                    .map_or_else(|| format!("gh pr create --base {branch}"), str::to_owned);
                let _ = write!(
                    words,
                    " · start from branch {branch} · {create} · merges after {on}"
                );
            }
            None => {
                let _ = write!(words, " · {on} has no branch yet · merges after {on}");
            }
        }
        let _ = writeln!(out, "{words}");
    }
    // Its merge is the operator's: never the Conductor's to make.
    if let (Some(words), Some(why)) = (merge_words(node), node["merge"]["why"].as_str()) {
        let _ = writeln!(out, "{words} · {why}");
    }
    if let Some(worker) = worker_words(node) {
        let _ = writeln!(out, "worker {worker}");
    }
    for pr in node["prs"].as_array().into_iter().flatten() {
        let name = pr["number"].as_u64().map_or_else(
            || format!("PR #{}", pr["number"]),
            |number| change_request_name(number, &pr["reference"]),
        );
        let mut words = format!("{name} {}", pr_word(&pr["state"]));
        if let Some(url) = pr["url"].as_str() {
            let _ = write!(words, " · {url}");
        }
        let _ = writeln!(out, "{words}");
    }
    if let Some(branch) = node["branch"].as_str() {
        let _ = writeln!(out, "branch {branch}");
    }
    if let Some(reason) = result["unread_reason"].as_str() {
        let _ = writeln!(out, "unread · {reason}");
    }
    if let Some(time) = time_words(node, now) {
        let _ = writeln!(out, "time {time}");
    }
    if let Some(push) = push_words(node, now) {
        let _ = writeln!(out, "{push}");
    }
    if let Some(wait) = wait_words(node, now) {
        let _ = writeln!(out, "{wait}");
    }
    if let Some(stop) = stop_words(node, now) {
        let _ = writeln!(out, "{stop}");
    }
    for warning in node["warnings"].as_array().into_iter().flatten() {
        let what = match warning["kind"].as_str() {
            Some("review_fails") => format!(
                "{} · limit reached, ordinary and minor findings go to follow-up issues",
                rounds_words(node).unwrap_or_else(|| "review".to_owned())
            ),
            Some("stopped") => format!(
                "stopped for {}m · {}",
                now.saturating_sub(warning["from_unix_ms"].as_u64().unwrap_or(now)) / 60_000,
                cause_words(&warning["cause"])
            ),
            _ => "needs a look".to_owned(),
        };
        // A full letter queue leaves it untold until the next evaluation.
        let told = if warning["told_unix_ms"].is_u64() {
            format!("Conductor told {}", age(now, &warning["told_unix_ms"]))
        } else {
            "Conductor not told yet".to_owned()
        };
        let _ = writeln!(out, "warning {WARN} {what} · {told}");
    }
    if node["attention"].is_object() {
        let attention = &node["attention"];
        let who = if attention["to"] == "operator" {
            "operator"
        } else {
            "Conductor"
        };
        let _ = writeln!(
            out,
            "attention ! {who} · {} · {}",
            reason_word(&attention["reason"]),
            age(now, &attention["since_unix_ms"])
        );
    }
    if let Some(said) = node["line"].as_str() {
        let _ = writeln!(out, "Conductor note {said}");
    }
    let lines = node["criteria"].as_array().cloned().unwrap_or_default();
    if !lines.is_empty() {
        let _ = writeln!(
            out,
            "finish lines {}/{}",
            node["criteria_met"].as_u64().unwrap_or(0),
            lines.len()
        );
        for line in &lines {
            draw_line(&mut out, line, now);
        }
    }
    draw_letters(&mut out, "recent letters", &result["letters"], now);
    // Absent from a core before the checkers' letters were listed.
    if result.get("checker_letters").is_some() {
        draw_letters(&mut out, "checker letters", &result["checker_letters"], now);
    }
    Ok(out)
}

/// A list of a node's letters under `head`, each with the id
/// `hide letter show` reads it by.
fn draw_letters(out: &mut String, head: &str, letters: &Value, now: u64) {
    match letters.as_array() {
        None => {
            let _ = writeln!(out, "{head} · unread");
        }
        Some(letters) if letters.is_empty() => {
            let _ = writeln!(out, "{head} none");
        }
        Some(letters) => {
            let _ = writeln!(out, "{head}");
            for letter in letters {
                let _ = writeln!(
                    out,
                    "  {} · {} · {} → {} · {} · {}",
                    text(letter, "id"),
                    age(now, &letter["at_unix_ms"]),
                    text(letter, "from"),
                    text(letter, "to"),
                    text(letter, "kind"),
                    text(letter, "line"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINUTE: u64 = 60_000;
    const NOW: u64 = 1_000 * MINUTE;

    /// A running mission as the core answers it: `show` stuck in review and
    /// quiet, `ci` waiting on its review, `docs` done, `later` waiting on
    /// `show`, `old` dropped, and a decision open.
    fn mission() -> Value {
        json!({"mission": {
        "id": "mission-33", "title": "Show summary", "status": "running",
        "decision": "Merge order?", "question": {"key": 1, "node": "ci", "asked_at_unix_ms": 1},
        "conductor": {"present": true}, "stale": false, "github_stale": false,
        "nodes": [
            {"key": "show", "kind": "change", "delivery": "pr", "status": "working",
             "agent_runtime": "running", "pr": 7, "prs": [{"number": 7, "state": "failed"}],
             "pr_state": "failed", "review_rounds": {"line": "review", "rounds": 3, "limit": 3},
             "ready_at_unix_ms": NOW - 780 * MINUTE, "started_at_unix_ms": NOW - 600 * MINUTE,
             "own_met_at_unix_ms": NOW - 510 * MINUTE, "pushed_at_unix_ms": NOW - 95 * MINUTE,
             "waiting_on": {"kind": "worker"},
             "criteria": [
                {"key": "c1", "kind": "own", "state": "pass"},
                {"key": "review", "kind": "review", "state": "fail", "rounds": 3},
                {"key": "ci", "kind": "ci", "state": "fail"}],
             "warnings": [
                {"kind": "review_fails", "from_unix_ms": NOW - 20 * MINUTE, "since_unix_ms": NOW - 20 * MINUTE,
                 "told_unix_ms": NOW - 20 * MINUTE}]},
            {"key": "ci", "kind": "change", "delivery": "pr", "status": "working",
             "pr": 8, "prs": [{"number": 8, "state": "pending"}], "pr_state": "pending",
             "pushed_at_unix_ms": NOW - 12 * MINUTE, "warnings": [],
             "waiting_on": {"kind": "decision"},
             "criteria": [{"key": "c1", "kind": "own", "state": "pass"},
                          {"key": "review", "kind": "review", "state": "open"},
                          {"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "docs", "kind": "change", "delivery": "pr", "status": "done",
             "pr": 6, "prs": [{"number": 5, "state": "closed"}, {"number": 6, "state": "merged"}],
             "pr_state": "merged", "warnings": [],
             "criteria": [{"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "later", "kind": "general", "preset": "research", "status": "planned", "depends_on": ["show", "docs"],
             "waiting_on": {"kind": "nodes", "keys": ["show"]},
             "wait": {"why": "disk", "until_unix_ms": NOW + 40 * MINUTE, "mission": false},
             "warnings": [], "criteria": []},
            {"key": "old", "kind": "change", "delivery": "pr", "status": "dropped",
             "warnings": [], "criteria": []},
        ]}})
    }

    /// with-hide/hide-private#552 c1: the head says the title, the state,
    /// how many nodes finished of those left in the plan, the open decision
    /// and the warnings; each node is one line with its state, where its
    /// time went (mission-82), its review rounds of the limit, last push,
    /// the wait its Conductor set, pull request and CI, what it waits on, and `⚠` on the line and on
    /// each fact that warns. A done node of a book from before the time's
    /// stamp says no time rather than an empty one.
    #[test]
    fn a_mission_reads_as_its_head_and_one_line_per_node() {
        assert_eq!(
            draw(&mission(), NOW).unwrap(),
            "mission-33 · Show summary · running · done 1/4 · dropped 1 · open decision 1 · warnings 1\n\
             decision · Merge order?\n\
             ⚠ show · working · work 1.5h · review·CI 8.5h · waited 3h · ⚠review 3/3 rounds · push 95m ago · #7 failed · CI fail · waits on the worker\n\
             \x20 ci · working · push 12m ago · #8 pending · CI pass · waits on the operator's decision\n\
             \x20 docs · done · #6 merged +1\n\
             \x20 later · planned · waiting · disk · 40m left · waits on show\n\
             \x20 old · dropped\n"
        );
    }

    /// mission-82 c2: a node's time splits at its start and where its
    /// worker's lines first all stood met, each part counting to `now`
    /// while it runs: work alone before the stamp, the wait of a planned
    /// node that could start, nothing for a wait under a minute, and no
    /// work or review for a finished node of a book from before the stamp,
    /// whose `started` and `finished` alone cannot say where review began.
    #[test]
    fn a_nodes_time_splits_into_work_review_and_wait_and_leaves_out_what_it_cannot_say() {
        let at = |minutes: u64| json!(NOW - minutes * MINUTE);
        for (node, words) in [
            (
                json!({"status": "working", "started_at_unix_ms": at(40), "ready_at_unix_ms": at(40)}),
                Some("work 40m"),
            ),
            (
                json!({"status": "done", "started_at_unix_ms": at(200), "own_met_at_unix_ms": at(110),
                       "finished_at_unix_ms": at(20), "ready_at_unix_ms": at(203)}),
                Some("work 1.5h · review·CI 1.5h · waited 3m"),
            ),
            (
                json!({"status": "planned", "ready_at_unix_ms": at(25)}),
                Some("waited 25m"),
            ),
            (
                json!({"status": "done", "started_at_unix_ms": at(300), "finished_at_unix_ms": at(10)}),
                None,
            ),
            // A node finished before the stamp keeps its total, not its wait.
            (
                json!({"status": "done", "started_at_unix_ms": at(300), "finished_at_unix_ms": at(10),
                       "ready_at_unix_ms": at(480)}),
                None,
            ),
            // Back at work from a finish hours later: the split counts that run.
            (
                json!({"status": "working", "started_at_unix_ms": at(400), "worked_at_unix_ms": at(60),
                       "own_met_at_unix_ms": at(20), "ready_at_unix_ms": at(400)}),
                Some("work 40m · review·CI 20m"),
            ),
            (json!({"status": "planned"}), None),
        ] {
            assert_eq!(time_words(&node, NOW).as_deref(), words, "{node}");
        }
        assert_eq!(
            [0, 59, 60, 87, 90, 540, 544, 545].map(|minutes| span_words(minutes * MINUTE)),
            ["0m", "59m", "1h", "1.5h", "1.5h", "9h", "9.1h", "9.1h"]
        );
    }

    /// The safety net is one line under the head naming the held nodes and
    /// since when they stand, and the lines the Conductor skipped on its own
    /// follow the nodes, each with its reason, who and when, as the map's
    /// `decided by the Conductor` lists them.
    #[test]
    fn the_safety_net_and_the_conductors_own_skips_are_shown() {
        let mut value = mission();
        value["mission"]["safety_net"] = json!({
        "conductor_present": true, "since_unix_ms": NOW - 25 * MINUTE,
        "nodes": [
            {"key": "show", "title": "Show", "reason": "worker_ended", "pr": null, "since_unix_ms": NOW - 25 * MINUTE},
            {"key": "ci", "title": "CI", "reason": "wait_stood", "pr": 8, "since_unix_ms": NOW - 70 * MINUTE},
        ]});
        value["mission"]["skipped_alone"] = json!([
            {"node": "docs", "title": "Docs", "line": "check", "kind": "check",
             "reason": "main merged, CI passed", "by": "lead", "at_unix_ms": NOW - 40 * MINUTE},
            {"node": "show", "title": "Show", "line": "review", "kind": "review",
             "reason": "minor findings left", "follow_up": "show-rest", "by": "lead", "at_unix_ms": NOW - 30 * MINUTE},
        ]);
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains(
                "safety net · 2 held · since 25m ago · shown to the operator · show ci\n"
            ),
            "{text}"
        );
        assert!(
            text.ends_with(
                "decided by the Conductor 2\n  ⤳ docs · check skipped · main merged, CI passed · lead · 40m ago\n  ⤳ show · review skipped · minor findings left · follow-up show-rest · lead · 30m ago\n"
            ),
            "{text}"
        );
        assert!(!draw(&mission(), NOW).unwrap().contains("safety net"));
    }

    /// Each line says what the core decided its node waits on, in words for
    /// every kind the core decides, and the wait its Conductor set loses its
    /// time once its end passed.
    #[test]
    fn a_node_line_says_what_the_core_decided_it_waits_on() {
        for (waiting, words) in [
            (
                json!({"kind": "attention", "to": "conductor", "reason": "worker_ended"}),
                "! Conductor: worker ended with work left",
            ),
            (
                json!({"kind": "attention", "to": "operator", "reason": "pr_closed"}),
                "! operator: PR closed without a merge",
            ),
            (
                json!({"kind": "decision"}),
                "waits on the operator's decision",
            ),
            (
                json!({"kind": "blocked"}),
                "blocked · a node before it was dropped",
            ),
            (
                json!({"kind": "nodes", "keys": ["show", "docs:c1"]}),
                "waits on show, docs line c1",
            ),
            (
                json!({"kind": "below", "node": "show"}),
                "waits on show merging first",
            ),
            (json!({"kind": "start"}), "not started"),
            (json!({"kind": "worker"}), "waits on the worker"),
            (json!({"kind": "review"}), "waits on review"),
            (json!({"kind": "verify"}), "waits on verify"),
            (json!({"kind": "ci"}), "waits on CI"),
            (json!({"kind": "merge"}), "waits on the merge"),
            (json!({"kind": "conductor"}), "waits on the Conductor"),
        ] {
            let mut value = mission();
            value["mission"]["nodes"][1]["waiting_on"] = waiting;
            let text = draw(&value, NOW).unwrap();
            assert!(text.contains(&format!(" · CI pass · {words}\n")), "{text}");
        }
        // A node that could start on a line of another says where it stacks.
        let mut value = mission();
        value["mission"]["nodes"][1]["waiting_on"] = json!({"kind": "start"});
        value["mission"]["nodes"][1]["stack"] = json!({"on": "show", "branch": "m1/show", "pr": 7});
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains(" · CI pass · can start · stacked on show\n"),
            "{text}"
        );
        let mut value = mission();
        value["mission"]["nodes"][3]["wait"]["until_unix_ms"] = json!(NOW - MINUTE);
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains("  later · planned · waiting · disk · waits on show\n"),
            "{text}"
        );
        // A node Hide read no push of says nothing of pushes: there is no
        // missing-push mark (D-32).
        value["mission"]["nodes"][1]["pushed_at_unix_ms"] = Value::Null;
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains(
                "  ci · working · #8 pending · CI pass · waits on the operator's decision\n"
            ),
            "{text}"
        );
    }

    /// mission-56: a worker stopped with nothing to wait on reads on its
    /// node's line and in full as since when and why, marked once it warns,
    /// and its warning says how long and why.
    #[test]
    fn a_stopped_worker_reads_as_since_when_and_why() {
        let mut value = mission();
        let cause = json!({"kind": "limit", "line": "resets 4pm"});
        let node = &mut value["mission"]["nodes"][1];
        node["work"] =
            json!({"state": "stopped", "since_unix_ms": NOW - 6 * MINUTE, "cause": cause});
        node["warnings"] = json!([{"kind": "stopped", "from_unix_ms": NOW - 6 * MINUTE,
            "since_unix_ms": NOW - MINUTE, "told_unix_ms": NOW - MINUTE, "cause": cause}]);
        let drawn = draw(&value, NOW).unwrap();
        assert!(
            drawn.contains(
                "⚠ ci · working · push 12m ago · ⚠stopped 6m ago · usage limit · resets 4pm · #8 pending"
            ),
            "{drawn}"
        );
        let answer = json!({"mission_id": "mission-33", "node": value["mission"]["nodes"][1].clone(),
            "letters": [], "checker_letters": []});
        let full = draw(&answer, NOW).unwrap();
        assert!(
            full.contains("\n⚠stopped 6m ago · usage limit · resets 4pm\n"),
            "{full}"
        );
        assert!(
            full.contains(
                "warning ⚠ stopped for 6m · usage limit · resets 4pm · Conductor told 1m ago"
            ),
            "{full}"
        );
        value["mission"]["nodes"][1]["work"]["cause"] = json!(null);
        value["mission"]["nodes"][1]["warnings"] = json!([]);
        assert!(
            draw(&value, NOW)
                .unwrap()
                .contains(" ci · working · push 12m ago · stopped 6m ago · no report · #8")
        );
    }

    /// A title cannot draw a line of its own.
    #[test]
    fn a_title_or_a_line_cannot_draw_a_line_of_its_own() {
        let mut value = mission();
        value["mission"]["title"] = json!("Show\n⚠ forged · done");
        let text = draw(&value, NOW).unwrap();
        assert_eq!(text.lines().count(), 7, "{text}");
    }

    /// A node's change request reads by the unit of the host its carried
    /// reference names: `MR !12` on gitlab.com, `PR #12` on GitHub, and the
    /// carried label alone for a host this build does not know.
    #[test]
    fn a_nodes_change_request_is_named_by_its_hosts_unit() {
        let line = |host: &str, label: &str| {
            let mut node = mission()["mission"]["nodes"][0].clone();
            node["prs"] = json!([{"number": 12, "state": "pending", "reference": {
                "repository": {"host": host, "authority": "example.test", "path": "example-user/project"},
                "number": 12, "label": label}}]);
            let answer = json!({"mission_id": "mission-33", "node": node, "letters": []});
            let text = draw(&answer, NOW).unwrap();
            text.lines()
                .find(|line| line.contains("12 "))
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("{text}"))
        };
        assert!(line("gitlab", "!12").starts_with("MR !12 "));
        assert!(line("github", "#12").starts_with("PR #12 "));
        assert!(line("forge", "~12").starts_with("~12 "));
    }

    /// with-hide/hide-private#552 c1: one node in full: its state, worker,
    /// pull requests, last push, warnings, every line with its evidence and each
    /// checker's answer, its worker's newest letters, and the letters its
    /// checkers wrote to the Conductor, each with the id `hide letter show`
    /// reads it by (with-hide/hide-private#668).
    #[test]
    fn a_node_reads_in_full_with_its_lines_reviews_and_letters() {
        let mut node = mission()["mission"]["nodes"][0].clone();
        node["title"] = json!("Summarize show");
        node["branch"] = json!("feat/show");
        node["worker"] = json!({"agent": "claude", "model": "opus"});
        node["agent_id"] = json!("agent-2");
        node["pane_id"] = json!("w1:p2");
        node["prs"][0]["url"] = json!("https://github.com/acme/hide/pull/7");
        node["criteria_met"] = json!(1);
        node["criteria"][0]["text"] = json!("show reads as text");
        node["criteria"][0]["evidence"] = json!("unit test");
        node["criteria"][0]["by"] =
            json!({"role": "worker", "name": "w", "at_unix_ms": NOW - 30 * MINUTE});
        node["criteria"][1]["checkers"] = json!([
            {"worker": {"agent": "codex"}, "agent_id": "agent-3", "state": "fail",
             "summary": "input lost", "evidence": "repro in test", "at_unix_ms": NOW - 20 * MINUTE},
            {"worker": {"agent": "claude"}, "state": "open"}]);
        let answer = json!({"mission_id": "mission-33", "node": node, "letters": [
            {"id": "letter-9", "at_unix_ms": NOW - 3 * MINUTE, "from": "w", "to": "lead", "kind": "progress", "line": "pushed the fix"}],
            "checker_letters": [
            {"id": "letter-7", "at_unix_ms": NOW - 20 * MINUTE, "from": "r", "to": "lead", "kind": "report", "line": "FAIL: input lost"}]});
        assert_eq!(
            draw(&answer, NOW).unwrap(),
            "mission-33 · show · Summarize show\n\
             state working · code · by PR · waits on the worker\n\
             worker claude opus · agent-2 · pane w1:p2 · awake\n\
             PR #7 failed · https://github.com/acme/hide/pull/7\n\
             branch feat/show\n\
             time work 1.5h · review·CI 8.5h · waited 3h\n\
             push 95m ago\n\
             warning ⚠ review 3/3 rounds · limit reached, ordinary and minor findings go to follow-up issues · Conductor told 20m ago\n\
             finish lines 1/3\n\
             \x20 c1 pass · show reads as text · evidence unit test · w 30m ago\n\
             \x20 review fail · 3 rounds\n\
             \x20   codex fail · agent-3 · 20m ago · input lost · evidence repro in test\n\
             \x20   claude open · not bound yet\n\
             \x20 ci fail\n\
             recent letters\n\
             \x20 letter-9 · 3m ago · w → lead · progress · pushed the fix\n\
             checker letters\n\
             \x20 letter-7 · 20m ago · r → lead · report · FAIL: input lost\n"
        );
        let unread = json!({"mission_id": "mission-33", "node": {"key": "x", "status": "planned"}, "letters": null});
        assert!(
            draw(&unread, NOW)
                .unwrap()
                .ends_with("recent letters · unread\n")
        );
    }

    /// A checker's record holds every finding, one per line, and its worker
    /// reads them here: `show` prints them whole, each on its own line under
    /// the checker's, a control character still a space; a line's one-line
    /// evidence stays on its line.
    #[test]
    fn a_checkers_findings_read_as_written_under_its_line() {
        let mut node = mission()["mission"]["nodes"][0].clone();
        let tail = "x".repeat(4000);
        let findings =
            format!("S1 serious: input lost\u{1b}[31m at a.rs:7\r\n\nO1 ordinary: {tail}\n");
        node["criteria"][0]["evidence"] = json!("unit test");
        node["criteria"][1]["checkers"] = json!([
            {"worker": {"agent": "codex"}, "agent_id": "agent-3", "state": "fail",
             "evidence": findings, "at_unix_ms": NOW - 20 * MINUTE}]);
        let text = draw(
            &json!({"mission_id": "mission-33", "node": node, "letters": []}),
            NOW,
        )
        .unwrap();
        let expected = format!(
            "\x20   codex fail · agent-3 · 20m ago · evidence\n\x20       S1 serious: input lost [31m at a.rs:7\n\n\x20       O1 ordinary: {tail}\n"
        );
        assert!(text.contains(&expected), "{text}");
        assert!(
            text.contains("\x20 c1 pass · evidence unit test\n"),
            "{text}"
        );
    }

    /// A worker launched with a profile names it beside its agent, and one
    /// on the built-in default reads as before (PRD agent-profiles-phase2
    /// B1, B12).
    #[test]
    fn a_workers_line_names_the_profile_it_was_launched_with() {
        let mut node = mission()["mission"]["nodes"][0].clone();
        node["agent_id"] = json!("agent-2");
        let worker = |node: &Value| {
            draw(
                &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                NOW,
            )
            .unwrap()
            .lines()
            .find(|line| line.starts_with("worker "))
            .map(str::to_owned)
        };
        node["launched"] =
            json!({"agent_id": "agent-2", "agent": "claude", "item": "Claude", "model": "opus"});
        assert_eq!(
            worker(&node).as_deref(),
            Some("worker Claude opus · agent-2 · awake")
        );
        node["launched"]["profile"] = json!({"id": "p-work", "name": "work"});
        node["launched"]["item"] = json!("Claude · work");
        node["launched"]["model_label"] = json!("deepseek/deepseek-v4.1-flash");
        assert_eq!(
            worker(&node).as_deref(),
            Some("worker Claude · work deepseek/deepseek-v4.1-flash · agent-2 · awake")
        );
    }

    /// A planned worker reads as its item, a profile the machine no longer
    /// lists saying so, before any worker launched.
    #[test]
    fn a_planned_workers_line_names_its_item() {
        let mut node = mission()["mission"]["nodes"][0].clone();
        node["launched"] = Value::Null;
        node["agent_id"] = Value::Null;
        node["agent_runtime"] = Value::Null;
        node["worker"] = json!({"agent": "claude", "profile": "p-gw", "item": "Claude · gw", "model": "opus", "model_label": "glm-5", "effort": "high"});
        let line = |node: &Value| {
            draw(
                &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                NOW,
            )
            .unwrap()
            .lines()
            .find(|line| line.starts_with("worker "))
            .map(str::to_owned)
        };
        assert_eq!(
            line(&node).as_deref(),
            Some("worker Claude · gw glm-5 high")
        );
        node["worker"] = json!({"agent": "claude", "profile": "p-gone", "item": "Claude", "profile_gone": true, "model": "opus"});
        assert_eq!(
            line(&node).as_deref(),
            Some("worker Claude · profile gone opus")
        );
    }

    /// A node read alone says what the core decided it waits on, as its
    /// line in the list does: the nodes before a planned one, and the
    /// operator's decision that stopped one.
    #[test]
    fn a_node_read_alone_waits_on_the_nodes_before_it_and_the_decision() {
        let mut planned = mission()["mission"]["nodes"][3].clone();
        planned["title"] = json!("Later");
        let alone = |node: &Value| {
            draw(
                &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                NOW,
            )
            .unwrap()
        };
        let text = alone(&planned);
        assert!(
            text.contains("state planned · general · research · waits on show\n"),
            "{text}"
        );
        let decided = mission()["mission"]["nodes"][1].clone();
        let text = alone(&decided);
        assert!(
            text.contains("state working · code · by PR · waits on the operator's decision\n"),
            "{text}"
        );
    }

    /// PRD node-presets D-10, D-13, B8: a node's kind is said in the
    /// operator's words, a general node's with its preset's name and one
    /// with none as general, and an outside node as the waiting node it is.
    #[test]
    fn a_nodes_kind_is_said_by_its_preset_and_an_outside_node_waits() {
        let said = |kind: Value| {
            let mut node = mission()["mission"]["nodes"][3].clone();
            for (name, value) in kind.as_object().unwrap() {
                node[name] = value.clone();
            }
            node.as_object_mut().unwrap().remove("waiting_on");
            let text = draw(
                &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                NOW,
            )
            .unwrap();
            text.lines()
                .find_map(|line| line.strip_prefix("state planned · "))
                .unwrap()
                .to_owned()
        };
        assert_eq!(said(json!({"preset": "verify"})), "general · verify");
        assert_eq!(said(json!({"preset": "deploy"})), "general · deploy");
        assert_eq!(said(json!({"preset": null})), "general");
        assert_eq!(
            said(json!({"kind": "external", "preset": null})),
            "watching"
        );
    }

    /// PRD node-presets D-06: a start gate's line says it asks before the
    /// start; an end gate's reads as before.
    #[test]
    fn a_start_gate_line_says_it_asks_before_the_start() {
        let gate_line = |gate: Value| {
            let mut node = mission()["mission"]["nodes"][3].clone();
            node["criteria"] = json!([gate]);
            let text = draw(
                &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                NOW,
            )
            .unwrap();
            text.lines()
                .find(|line| line.starts_with("  gate "))
                .unwrap()
                .to_owned()
        };
        let start = gate_line(json!({"key": "gate", "kind": "human", "state": "open",
            "text": "배포해도 될지", "when": "start"}));
        assert!(start.contains("before start · 배포해도 될지"), "{start}");
        let end = gate_line(json!({"key": "gate", "kind": "human", "state": "open",
            "text": "배포된 화면"}));
        assert!(!end.contains("before start"), "{end}");
    }

    /// PRD mission-destination B1, B15: a mission that records where its
    /// results go says so under its head, and a change node says it too,
    /// where its worker reads the base of its pull request; a mission from
    /// before destinations reads line for line as it did.
    #[test]
    fn a_destination_is_named_under_the_head_and_on_a_change_node() {
        let mut value = mission();
        let before = draw(&value, NOW).unwrap();
        value["mission"]["destination"] = json!("feat/login");
        let text = draw(&value, NOW).unwrap();
        let (head, rest) = before.split_once('\n').unwrap();
        assert_eq!(text, format!("{head}\ndestination · feat/login\n{rest}"));

        let alone = |node: &Value, destination: Option<&str>| {
            let mut answer = json!({"mission_id": "mission-33", "node": node, "letters": []});
            if let Some(destination) = destination {
                answer["destination"] = json!(destination);
            }
            draw(&answer, NOW).unwrap()
        };
        let mut direct = mission()["mission"]["nodes"][1].clone();
        direct["delivery"] = json!("direct");
        let old = alone(&direct, None);
        assert!(old.contains("· directly into main ·"), "{old}");
        assert!(!old.contains("destination"), "{old}");
        let text = alone(&direct, Some("feat/login"));
        assert!(
            text.contains("· directly into feat/login ·")
                && text.contains("\ndestination feat/login\n"),
            "{text}"
        );
        // A node stacked on another starts from that one's branch and bases
        // its pull request there, and merges after it.
        // With no words from the daemon the line is GitHub's, as before; a
        // gitlab.com project's daemon writes the change request below and
        // the create command in gitlab.com's.
        let mut stacked = mission()["mission"]["nodes"][1].clone();
        stacked["stack"] = json!({"on": "core", "branch": "m82/core", "pr": 12});
        let text = alone(&stacked, Some("main"));
        assert!(
            text.contains(
                "\ndestination main\nstacked on core · PR #12 · start from branch m82/core · gh pr create --base m82/core · merges after core\n"
            ),
            "{text}"
        );
        let mut answer = json!({"mission_id": "mission-33", "node": stacked.clone(), "letters": [],
            "destination": "main", "stack_pr": "MR !12",
            "stack_create": "glab mr create --target-branch m82/core"});
        answer["node"]["stack"] = json!({"on": "core", "branch": "m82/core", "pr": 12});
        let text = draw(&answer, NOW).unwrap();
        assert!(
            text.contains(
                "\nstacked on core · MR !12 · start from branch m82/core · glab mr create --target-branch m82/core · merges after core\n"
            ),
            "{text}"
        );
        stacked["stack"] = json!({"on": "core", "branch": null, "pr": null});
        assert!(
            alone(&stacked, Some("main"))
                .contains("\nstacked on core · core has no branch yet · merges after core\n")
        );
        // A node that changes no code goes to no branch.
        let research = mission()["mission"]["nodes"][3].clone();
        assert!(!alone(&research, Some("feat/login")).contains("destination"));

        // B11: a branch Hide could not read says why, which names a
        // destination that is gone.
        let mut answer = json!({"mission_id": "mission-33", "node": direct, "letters": [],
            "destination": "feat/login",
            "unread_reason": "Neither feat/login nor origin/feat/login could be read"});
        answer["node"]["branch"] = json!("feat/fix");
        let text = draw(&answer, NOW).unwrap();
        assert!(
            text.contains(
                "branch feat/fix\nunread · Neither feat/login nor origin/feat/login could be read\n"
            ),
            "{text}"
        );
    }

    /// PRD mission-destination B18: a node the plan gives the operator to
    /// merge is named in the scope's line under the head and marked on its
    /// line and in full with why; a node the scope gives the operator (scope
    /// `pr`) or the Conductor reads as before.
    #[test]
    fn a_node_the_operator_merges_is_named_under_the_head_and_on_its_line() {
        let mut value = mission();
        let before = draw(&value, NOW).unwrap();
        value["mission"]["nodes"][2]["merge"] = json!({"by": "operator", "state": "merged"});
        value["mission"]["nodes"][1]["merge"] = json!({"by": "conductor", "state": "ahead"});
        assert_eq!(
            draw(&value, NOW).unwrap(),
            before,
            "the scope decides: as before"
        );

        value["mission"]["nodes"][1]["merge"] =
            json!({"by": "operator", "why": "DB는 직접 보고 머지", "state": "due"});
        let text = draw(&value, NOW).unwrap();
        let (head, rest) = before.split_once('\n').unwrap();
        let rest = rest.replace(
            "#8 pending · CI pass ·",
            "#8 pending · CI pass · 👤 yours to merge ·",
        );
        assert_eq!(
            text,
            format!("{head}\nthrough the merge · 👤 the operator merges ci\n{rest}")
        );

        let alone = draw(
            &json!({"mission_id": "mission-33", "node": value["mission"]["nodes"][1], "letters": []}),
            NOW,
        )
        .unwrap();
        assert!(
            alone.contains("\n👤 yours to merge · DB는 직접 보고 머지\n"),
            "{alone}"
        );
    }

    /// with-hide/hide#2463: a node whose pull request Hide cannot read holds
    /// its CI line open; the text says the CI is not read, not that it runs.
    #[test]
    fn an_open_ci_line_of_a_pull_request_not_read_is_not_a_running_ci() {
        let mut value = mission();
        let node = &mut value["mission"]["nodes"][1];
        node["pr_state"] = Value::Null;
        node["criteria"][2]["state"] = json!("open");
        let text = draw(&value, NOW).unwrap();
        assert!(text.contains("#8 unread · CI unread ·"), "{text}");
        let node = &mut value["mission"]["nodes"][1];
        node["pr_state"] = json!("pending");
        let text = draw(&value, NOW).unwrap();
        assert!(text.contains("#8 pending · CI running ·"), "{text}");
    }

    /// A `waiting_on` this build does not read refuses the text, so the
    /// command prints the answer as JSON instead of drawing the node short.
    #[test]
    fn a_wait_this_build_cannot_read_refuses_the_text() {
        let mut value = mission();
        value["mission"]["nodes"][1]["waiting_on"] = json!({"kind": "landing"});
        let refused = draw(&value, NOW).unwrap_err();
        assert!(refused.contains("node ci waiting_on"), "{refused}");
    }

    /// A gitlab.com project's change requests read as gitlab.com writes
    /// them from the references the daemon carries, and a GitHub project's
    /// as before: a node's own in its line and its detail, and the one a
    /// stacked node waits on in the daemon's own words. An outside node's
    /// number is never printed, only its title.
    #[test]
    fn a_projects_change_requests_read_as_its_host_writes_them() {
        let reference = |host: &str, number: u32| {
            let (authority, label) = if host == "gitlab" {
                ("gitlab.com", format!("!{number}"))
            } else {
                ("github.com", format!("#{number}"))
            };
            json!({"repository": {"host": host, "authority": authority, "path": "acme/app"},
                   "number": number, "key": format!("{authority}/acme/app#{number}"), "label": label,
                   "url": "", "checks_url": "", "files_url": ""})
        };
        for (host, line, detail, stack) in [
            (
                "gitlab",
                " · !45 pending · ",
                "\nMR !45 pending\n",
                "stacked on guard · MR !45 · ",
            ),
            (
                "github",
                " · #45 pending · ",
                "\nPR #45 pending\n",
                "stacked on guard · PR #45 · ",
            ),
        ] {
            let guard = json!({"key": "guard", "title": "Guard", "kind": "change", "delivery": "pr",
                "status": "working", "pr": 45, "pr_state": "pending", "pr_reference": reference(host, 45),
                "prs": [{"number": 45, "state": "pending", "reference": reference(host, 45)}],
                "waiting_on": {"kind": "worker"}, "warnings": [], "criteria": []});
            let moved = json!({"key": "moved", "title": "Moved", "kind": "external", "status": "working",
                "external": {"pr": 44}, "outside": {"base": "main", "state": "pending", "reference": reference(host, 44)},
                "prs": [], "waiting_on": {"kind": "worker"}, "warnings": [], "criteria": []});
            let mut value = mission();
            value["mission"]["nodes"] = json!([guard.clone(), moved.clone()]);
            let overview = draw(&value, NOW).unwrap();
            assert!(
                overview.contains(&format!("  guard · working{line}")),
                "{host}: {overview}"
            );
            assert!(!overview.contains("44"), "{host}: {overview}");
            let alone = |node: &Value| {
                draw(
                    &json!({"mission_id": "mission-33", "node": node, "letters": []}),
                    NOW,
                )
                .unwrap()
            };
            assert!(alone(&guard).contains(detail), "{host}");
            assert!(!alone(&moved).contains("44"), "{host}: {}", alone(&moved));
            let mut stacked = guard.clone();
            stacked["key"] = json!("top");
            stacked["stack"] = json!({"on": "guard", "branch": "m91/guard", "pr": 45});
            let words = if host == "gitlab" { "MR !45" } else { "PR #45" };
            let answer = json!({"mission_id": "mission-33", "node": stacked, "letters": [], "stack_pr": words});
            assert!(draw(&answer, NOW).unwrap().contains(stack), "{host}");
        }
    }
}
