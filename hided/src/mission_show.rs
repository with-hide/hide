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
    let result = one_line(result);
    if result["node"].is_object() {
        draw_node(&result, now)
    } else {
        draw_mission(&result["mission"], now)
    }
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("-")
}

fn mission_status(status: &Value) -> &'static str {
    match status.as_str() {
        Some("planning") => "계획 중",
        Some("running") => "진행 중",
        Some("merge_wait") => "머지 기다림",
        Some("done") => "끝",
        Some("cancelled") => "취소",
        _ => "-",
    }
}

fn node_status(node: &Value) -> String {
    let word = match node["status"].as_str() {
        Some("planned") => "계획",
        Some("working") => "작업 중",
        Some("done") => "끝",
        Some("dropped") => "빠짐",
        _ => "-",
    };
    match (node["status"].as_str(), node["agent_runtime"].as_str()) {
        (Some("working"), Some("asleep")) => format!("{word} (잠듦)"),
        (Some("working"), Some("ended")) => format!("{word} (작업자 끝남)"),
        _ => word.to_owned(),
    }
}

fn pr_word(state: &Value) -> &'static str {
    match state.as_str() {
        Some("failed") => "실패",
        Some("pending") => "대기",
        Some("mergeable") => "머지 가능",
        Some("draft") => "초안",
        Some("merged") => "머지됨",
        Some("closed") => "닫힘",
        _ => "읽지 못함",
    }
}

fn line_word(state: &Value) -> &'static str {
    match state.as_str() {
        Some("pass") => "통과",
        Some("fail") => "실패",
        Some("skipped") => "건너뜀",
        _ => "열림",
    }
}

fn reason_word(reason: &Value) -> &'static str {
    match reason.as_str() {
        Some("merged_unproven") => "머지됐는데 통과 안 된 완료 줄이 있음",
        Some("worker_ended") => "작업자가 일을 남기고 끝남",
        Some("pr_closed") => "PR이 머지 없이 닫힘",
        Some("unplanned_pr") => "계획에 없는 PR",
        Some("conductor_failed") => "지휘자가 실패로 기록",
        Some("wake_refused") => "잠든 작업자를 깨우지 못함",
        Some("close_refused") => "작업은 끝났는데 확인 기록이 빠져 미션을 닫지 못함",
        Some("human_gate") => "사람 검토 지점",
        Some("not_started") => "시작할 수 있는데 시작 안 함",
        Some("warning_stood") => "경고가 풀리지 않고 그대로",
        Some("wait_stood") => "CI·리뷰·머지를 오래 기다림",
        _ => "확인 필요",
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

/// The node's failed reviews in a row, marked when they warn.
fn review_words(node: &Value) -> Option<String> {
    let fails = node["review_fails"].as_u64().filter(|fails| *fails > 0)?;
    Some(marked(
        warning(node, "review_fails").is_some(),
        format!("리뷰 FAIL {fails}연속"),
    ))
}

/// Its last push, or that a change at work has none yet, marked when the
/// quiet warns.
fn push_words(node: &Value, now: u64) -> Option<String> {
    let quiet = warning(node, "no_push");
    let words = match node["pushed_at_unix_ms"].as_u64() {
        Some(at) => format!("push {}", minutes_ago(now, at)),
        None if quiet.is_some() || node["asks_push"] == true => "push 없음".to_owned(),
        None => return None,
    };
    Some(marked(quiet.is_some(), words))
}

/// Why a stopped worker stopped (`StopCause`): a usage limit or an API
/// error with the agent's own line, or no report.
fn cause_words(cause: &Value) -> String {
    let line = cause["line"].as_str().unwrap_or_default();
    match cause["kind"].as_str() {
        Some("limit") => format!("사용 한도 · {line}"),
        Some("error") => format!("API 오류 · {line}"),
        _ => "보고 없음".to_owned(),
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
            "멈춤 {} · {}",
            minutes_ago(now, since),
            cause_words(&work["cause"])
        ),
    ))
}

/// How long ago `at` was in minutes up to a day, so the quiet that warns
/// at ninety minutes reads as such.
fn minutes_ago(now: u64, at: u64) -> String {
    match now.saturating_sub(at) / 60_000 {
        0 => "방금".to_owned(),
        minutes @ 1..=1439 => format!("{minutes}분 전"),
        minutes => format!("{}일 전", minutes / 1440),
    }
}

fn pr_words(node: &Value) -> Option<String> {
    let number = node["pr"].as_u64()?;
    let mut words = format!("#{number} {}", pr_word(&node["pr_state"]));
    let more = node["prs"].as_array().map_or(0, Vec::len).saturating_sub(1);
    if more > 0 {
        let _ = write!(words, " 외 {more}");
    }
    if node["pr_state"] != "merged"
        && let Some(ci) = line_of(node, "ci")
    {
        let _ = write!(
            words,
            " · CI {}",
            match ci["state"].as_str() {
                Some("pass") => "통과",
                Some("fail") => "실패",
                Some("skipped") => "건너뜀",
                _ => "도는 중",
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
        Some("due") => "👤 머지할 차례",
        Some("merged") => "👤 머지함",
        _ => "👤 내가 머지",
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
                AttentionTo::Operator => "운영자",
                AttentionTo::Conductor => "지휘자",
            };
            let reason = serde_json::to_value(reason).unwrap_or(Value::Null);
            format!("! {who}: {}", reason_word(&reason))
        }
        WaitingOn::Nodes { keys } => format!("기다림 {}", keys.join(", ")),
        WaitingOn::Decision => "기다림 운영자 결정".to_owned(),
        WaitingOn::Blocked => "막힘 앞 노드 빠짐".to_owned(),
        WaitingOn::Start => "시작 전".to_owned(),
        WaitingOn::Worker => "기다림 작업자".to_owned(),
        WaitingOn::Review => "기다림 리뷰".to_owned(),
        WaitingOn::Verify => "기다림 검증".to_owned(),
        WaitingOn::Ci => "기다림 CI".to_owned(),
        WaitingOn::Merge => "기다림 머지".to_owned(),
        WaitingOn::Conductor => "기다림 지휘자".to_owned(),
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
        "{} · {} · {} · 끝 {}/{}",
        text(mission, "id"),
        text(mission, "title"),
        mission_status(&mission["status"]),
        count("done"),
        nodes.len() - dropped,
    );
    if dropped > 0 {
        let _ = write!(head, " · 빠짐 {dropped}");
    }
    let _ = write!(
        head,
        " · 열린 결정 {} · 경고 {}",
        usize::from(mission["decision"].is_string()),
        nodes.iter().map(warning_count).sum::<usize>(),
    );
    let mut out = format!("{head}\n");
    // Where the results go; a mission from before destinations names none
    // and reads as it did.
    if let Some(destination) = mission["destination"].as_str() {
        let _ = writeln!(out, "도착 · {destination}");
    }
    // The scope's line, said only when the plan gave the operator a merge
    // the scope does not (PRD mission-destination B18).
    let mine: Vec<&str> = nodes
        .iter()
        .filter(|node| node["merge"]["why"].is_string())
        .map(|node| text(node, "key"))
        .collect();
    if !mine.is_empty() {
        let _ = writeln!(out, "머지까지 · {}만 👤 내가 머지", mine.join(", "));
    }
    if let Some(decision) = mission["decision"].as_str() {
        let _ = writeln!(out, "결정 · {decision}");
    }
    if mission["conductor"]["present"] == false && mission["status"] != "done" {
        let _ = writeln!(out, "지휘자 없음 · hide mission take로 이어받기");
    }
    if mission["stale"] == true {
        let _ = writeln!(out, "미션 장치가 응답하지 않음 · 마지막 값");
    }
    if mission["github_stale"] == true {
        let _ = writeln!(
            out,
            "GitHub 읽기 실패 · {} 값",
            age(now, &mission["github_read_at_unix_ms"])
        );
    }
    // The operator's one card for the raises the Conductor has not settled.
    if let Some(net) = mission["safety_net"].as_object() {
        let held = net["nodes"].as_array().cloned().unwrap_or_default();
        let keys: Vec<&str> = held.iter().map(|node| text(node, "key")).collect();
        let _ = writeln!(
            out,
            "안전망 · 걸린 일 {}건 · {}부터 그대로 · 운영자에게 보임 · {}",
            held.len(),
            age(now, &net["since_unix_ms"]),
            keys.join(" "),
        );
    }
    for node in &nodes {
        let mut fields = vec![text(node, "key").to_owned(), node_status(node)];
        fields.extend(review_words(node));
        fields.extend(push_words(node, now));
        fields.extend(stop_words(node, now));
        fields.extend(pr_words(node));
        fields.extend(merge_words(node).map(str::to_owned));
        fields.extend(waits_on(node)?);
        let mark = if warning_count(node) > 0 { WARN } else { " " };
        let _ = writeln!(out, "{mark} {}", fields.join(" · "));
    }
    // What the Conductor settled on its own, as the map's `Conductor가 정한 것`.
    let alone = mission["skipped_alone"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !alone.is_empty() {
        let _ = writeln!(out, "Conductor가 정한 것 {}", alone.len());
        for skip in &alone {
            let _ = writeln!(
                out,
                "  ⤳ {} · {} 넘김 · {} · {} · {}",
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
            format!("{} · 없는 프로필", item)
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
        Some("asleep") => words.push_str(" · 잠듦"),
        Some("ended") => words.push_str(" · 끝남"),
        Some("running") => words.push_str(" · 깨어 있음"),
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
    if let Some(said) = line["text"].as_str() {
        let _ = write!(words, " · {said}");
    }
    if let Some(fails) = line["fails_in_a_row"].as_u64().filter(|fails| *fails > 0) {
        let _ = write!(words, " · FAIL {fails}연속");
    }
    if let Some(commit) = line["commit"].as_str() {
        let _ = write!(words, " · {}", commit.get(..7).unwrap_or(commit));
    }
    if let Some(skip) = line["skip"]["reason"].as_str() {
        let _ = write!(words, " · 건너뜀 이유 {skip}");
    }
    if let Some(summary) = line["summary"].as_str() {
        let _ = write!(words, " · {summary}");
    }
    if let Some(evidence) = line["evidence"].as_str() {
        let _ = write!(words, " · 근거 {evidence}");
    }
    if line["by"].is_object() {
        let _ = write!(
            words,
            " · {}",
            recorded_words(&line["by"], &line["by"]["at_unix_ms"], now)
        );
    }
    let _ = writeln!(out, "{words}");
    for checker in line["checkers"].as_array().into_iter().flatten() {
        let mut words = format!(
            "    {} {}",
            text(&checker["worker"], "agent"),
            line_word(&checker["state"])
        );
        if let Some(agent) = checker["agent_id"].as_str() {
            let _ = write!(words, " · {agent}");
        } else {
            words.push_str(" · 아직 안 붙음");
        }
        if checker["at_unix_ms"].is_u64() {
            let _ = write!(words, " · {}", age(now, &checker["at_unix_ms"]));
        }
        if let Some(summary) = checker["summary"].as_str() {
            let _ = write!(words, " · {summary}");
        }
        if let Some(evidence) = checker["evidence"].as_str() {
            let _ = write!(words, " · 근거 {evidence}");
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
    let mut state = vec![node_status(node), text(node, "kind").to_owned()];
    if let Some(delivery) = node["delivery"].as_str() {
        state.push(match delivery {
            "pr" => "PR로".to_owned(),
            "direct" => format!(
                "{}에 바로",
                result["destination"].as_str().unwrap_or("main")
            ),
            other => other.to_owned(),
        });
    }
    // Its attention has a line of its own.
    if node["waiting_on"]["kind"] != "attention" {
        state.extend(waits_on(node)?);
    }
    let _ = writeln!(out, "상태 {}", state.join(" · "));
    // A change's pull request takes this branch as its base, unless it is
    // stacked on another.
    if let (Some(destination), "change") = (result["destination"].as_str(), text(node, "kind")) {
        let _ = writeln!(out, "도착 {destination}");
    }
    // Its merge is the operator's: never the Conductor's to make.
    if let (Some(words), Some(why)) = (merge_words(node), node["merge"]["why"].as_str()) {
        let _ = writeln!(out, "{words} · {why}");
    }
    if let Some(worker) = worker_words(node) {
        let _ = writeln!(out, "작업자 {worker}");
    }
    for pr in node["prs"].as_array().into_iter().flatten() {
        let mut words = format!("PR #{} {}", pr["number"], pr_word(&pr["state"]));
        if let Some(url) = pr["url"].as_str() {
            let _ = write!(words, " · {url}");
        }
        let _ = writeln!(out, "{words}");
    }
    if let Some(branch) = node["branch"].as_str() {
        let _ = writeln!(out, "브랜치 {branch}");
    }
    if let Some(reason) = result["unread_reason"].as_str() {
        let _ = writeln!(out, "읽지 못함 · {reason}");
    }
    if let Some(push) = push_words(node, now) {
        let _ = writeln!(out, "{push}");
    }
    if let Some(stop) = stop_words(node, now) {
        let _ = writeln!(out, "{stop}");
    }
    for warning in node["warnings"].as_array().into_iter().flatten() {
        let what = match warning["kind"].as_str() {
            Some("review_fails") => format!(
                "리뷰 FAIL {}연속",
                node["review_fails"].as_u64().unwrap_or(0)
            ),
            Some("no_push") => format!(
                "push 없이 {}분",
                now.saturating_sub(warning["from_unix_ms"].as_u64().unwrap_or(now)) / 60_000
            ),
            Some("stopped") => format!(
                "멈춘 지 {}분 · {}",
                now.saturating_sub(warning["from_unix_ms"].as_u64().unwrap_or(now)) / 60_000,
                cause_words(&warning["cause"])
            ),
            _ => "확인 필요".to_owned(),
        };
        // A full letter queue leaves it untold until the next evaluation.
        let told = if warning["told_unix_ms"].is_u64() {
            format!("지휘자에게 알림 {}", age(now, &warning["told_unix_ms"]))
        } else {
            "지휘자에게 알릴 차례".to_owned()
        };
        let _ = writeln!(out, "경고 {WARN} {what} · {told}");
    }
    if node["attention"].is_object() {
        let attention = &node["attention"];
        let who = if attention["to"] == "operator" {
            "운영자"
        } else {
            "지휘자"
        };
        let _ = writeln!(
            out,
            "주의 ! {who} · {} · {}",
            reason_word(&attention["reason"]),
            age(now, &attention["since_unix_ms"])
        );
    }
    if let Some(said) = node["line"].as_str() {
        let _ = writeln!(out, "지휘자 메모 {said}");
    }
    let lines = node["criteria"].as_array().cloned().unwrap_or_default();
    if !lines.is_empty() {
        let _ = writeln!(
            out,
            "완료 줄 {}/{}",
            node["criteria_met"].as_u64().unwrap_or(0),
            lines.len()
        );
        for line in &lines {
            draw_line(&mut out, line, now);
        }
    }
    draw_letters(&mut out, "최근 편지", &result["letters"], now);
    // Absent from a core before the checkers' letters were listed.
    if result.get("checker_letters").is_some() {
        draw_letters(&mut out, "검토자 편지", &result["checker_letters"], now);
    }
    Ok(out)
}

/// A list of a node's letters under `head`, each with the id
/// `hide request show` reads it by.
fn draw_letters(out: &mut String, head: &str, letters: &Value, now: u64) {
    match letters.as_array() {
        None => {
            let _ = writeln!(out, "{head} · 읽지 못함");
        }
        Some(letters) if letters.is_empty() => {
            let _ = writeln!(out, "{head} 없음");
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
             "pr_state": "failed", "review_fails": 2, "pushed_at_unix_ms": NOW - 95 * MINUTE,
             "asks_push": true, "waiting_on": {"kind": "worker"},
             "criteria": [
                {"key": "c1", "kind": "own", "state": "pass"},
                {"key": "review", "kind": "review", "state": "fail", "fails_in_a_row": 2},
                {"key": "ci", "kind": "ci", "state": "fail"}],
             "warnings": [
                {"kind": "review_fails", "from_unix_ms": NOW - 20 * MINUTE, "since_unix_ms": NOW - 20 * MINUTE,
                 "told_unix_ms": NOW - 20 * MINUTE},
                {"kind": "no_push", "from_unix_ms": NOW - 95 * MINUTE, "since_unix_ms": NOW - 5 * MINUTE}]},
            {"key": "ci", "kind": "change", "delivery": "pr", "status": "working",
             "pr": 8, "prs": [{"number": 8, "state": "pending"}], "pr_state": "pending",
             "review_fails": 0, "pushed_at_unix_ms": NOW - 12 * MINUTE, "warnings": [],
             "asks_push": true, "waiting_on": {"kind": "decision"},
             "criteria": [{"key": "c1", "kind": "own", "state": "pass"},
                          {"key": "review", "kind": "review", "state": "open"},
                          {"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "docs", "kind": "change", "delivery": "pr", "status": "done",
             "pr": 6, "prs": [{"number": 5, "state": "closed"}, {"number": 6, "state": "merged"}],
             "pr_state": "merged", "review_fails": 0, "warnings": [],
             "criteria": [{"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "later", "kind": "research", "status": "planned", "depends_on": ["show", "docs"],
             "waiting_on": {"kind": "nodes", "keys": ["show"]},
             "review_fails": 0, "warnings": [], "criteria": []},
            {"key": "old", "kind": "change", "delivery": "pr", "status": "dropped",
             "review_fails": 0, "warnings": [], "criteria": []},
        ]}})
    }

    /// with-hide/hide-private#552 c1: the head says the title, the state,
    /// how many nodes finished of those left in the plan, the open decision
    /// and the warnings; each node is one line with its state, failed
    /// reviews in a row, last push, pull request and CI, what it waits on,
    /// and `⚠` on the line and on each fact that warns.
    #[test]
    fn a_mission_reads_as_its_head_and_one_line_per_node() {
        assert_eq!(
            draw(&mission(), NOW).unwrap(),
            "mission-33 · Show summary · 진행 중 · 끝 1/4 · 빠짐 1 · 열린 결정 1 · 경고 2\n\
             결정 · Merge order?\n\
             ⚠ show · 작업 중 · ⚠리뷰 FAIL 2연속 · ⚠push 95분 전 · #7 실패 · CI 실패 · 기다림 작업자\n\
             \x20 ci · 작업 중 · push 12분 전 · #8 대기 · CI 통과 · 기다림 운영자 결정\n\
             \x20 docs · 끝 · #6 머지됨 외 1\n\
             \x20 later · 계획 · 기다림 show\n\
             \x20 old · 빠짐\n"
        );
    }

    /// The safety net is one line under the head naming the held nodes and
    /// since when they stand, and the lines the Conductor skipped on its own
    /// follow the nodes, each with its reason, who and when, as the map's
    /// `Conductor가 정한 것` lists them.
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
        ]);
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains(
                "안전망 · 걸린 일 2건 · 25분 전부터 그대로 · 운영자에게 보임 · show ci\n"
            ),
            "{text}"
        );
        assert!(
            text.ends_with(
                "Conductor가 정한 것 1\n  ⤳ docs · check 넘김 · main merged, CI passed · lead · 40분 전\n"
            ),
            "{text}"
        );
        assert!(!draw(&mission(), NOW).unwrap().contains("안전망"));
    }

    /// Each line says what the core decided its node waits on, in words for
    /// every kind the core decides, and a change a push is asked of with no
    /// push Hide has read says so.
    #[test]
    fn a_node_line_says_what_the_core_decided_it_waits_on() {
        for (waiting, words) in [
            (
                json!({"kind": "attention", "to": "conductor", "reason": "worker_ended"}),
                "! 지휘자: 작업자가 일을 남기고 끝남",
            ),
            (
                json!({"kind": "attention", "to": "operator", "reason": "pr_closed"}),
                "! 운영자: PR이 머지 없이 닫힘",
            ),
            (json!({"kind": "decision"}), "기다림 운영자 결정"),
            (json!({"kind": "blocked"}), "막힘 앞 노드 빠짐"),
            (
                json!({"kind": "nodes", "keys": ["show", "docs"]}),
                "기다림 show, docs",
            ),
            (json!({"kind": "start"}), "시작 전"),
            (json!({"kind": "worker"}), "기다림 작업자"),
            (json!({"kind": "review"}), "기다림 리뷰"),
            (json!({"kind": "verify"}), "기다림 검증"),
            (json!({"kind": "ci"}), "기다림 CI"),
            (json!({"kind": "merge"}), "기다림 머지"),
            (json!({"kind": "conductor"}), "기다림 지휘자"),
        ] {
            let mut value = mission();
            value["mission"]["nodes"][1]["waiting_on"] = waiting;
            let text = draw(&value, NOW).unwrap();
            assert!(text.contains(&format!(" · CI 통과 · {words}\n")), "{text}");
        }
        let mut value = mission();
        let node = &mut value["mission"]["nodes"][1];
        node["pushed_at_unix_ms"] = Value::Null;
        node["pr"] = Value::Null;
        let text = draw(&value, NOW).unwrap();
        assert!(
            text.contains("  ci · 작업 중 · push 없음 · 기다림 운영자 결정\n"),
            "{text}"
        );
        value["mission"]["nodes"][1]["asks_push"] = json!(false);
        assert!(
            draw(&value, NOW)
                .unwrap()
                .contains("  ci · 작업 중 · 기다림 운영자 결정\n")
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
                "⚠ ci · 작업 중 · push 12분 전 · ⚠멈춤 6분 전 · 사용 한도 · resets 4pm · #8 대기"
            ),
            "{drawn}"
        );
        let answer = json!({"mission_id": "mission-33", "node": value["mission"]["nodes"][1].clone(),
            "letters": [], "checker_letters": []});
        let full = draw(&answer, NOW).unwrap();
        assert!(
            full.contains("\n⚠멈춤 6분 전 · 사용 한도 · resets 4pm\n"),
            "{full}"
        );
        assert!(
            full.contains("경고 ⚠ 멈춘 지 6분 · 사용 한도 · resets 4pm · 지휘자에게 알림 1분 전"),
            "{full}"
        );
        value["mission"]["nodes"][1]["work"]["cause"] = json!(null);
        value["mission"]["nodes"][1]["warnings"] = json!([]);
        assert!(
            draw(&value, NOW)
                .unwrap()
                .contains(" ci · 작업 중 · push 12분 전 · 멈춤 6분 전 · 보고 없음 · #8")
        );
    }

    /// A title cannot draw a line of its own.
    #[test]
    fn a_title_or_a_line_cannot_draw_a_line_of_its_own() {
        let mut value = mission();
        value["mission"]["title"] = json!("Show\n⚠ forged · 끝");
        let text = draw(&value, NOW).unwrap();
        assert_eq!(text.lines().count(), 7, "{text}");
    }

    /// with-hide/hide-private#552 c1: one node in full: its state, worker,
    /// pull requests, push, warnings, every line with its evidence and each
    /// checker's answer, its worker's newest letters, and the letters its
    /// checkers wrote to the Conductor, each with the id `hide request show`
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
             상태 작업 중 · change · PR로 · 기다림 작업자\n\
             작업자 claude opus · agent-2 · pane w1:p2 · 깨어 있음\n\
             PR #7 실패 · https://github.com/acme/hide/pull/7\n\
             브랜치 feat/show\n\
             ⚠push 95분 전\n\
             경고 ⚠ 리뷰 FAIL 2연속 · 지휘자에게 알림 20분 전\n\
             경고 ⚠ push 없이 95분 · 지휘자에게 알릴 차례\n\
             완료 줄 1/3\n\
             \x20 c1 통과 · show reads as text · 근거 unit test · w 30분 전\n\
             \x20 review 실패 · FAIL 2연속\n\
             \x20   codex 실패 · agent-3 · 20분 전 · input lost · 근거 repro in test\n\
             \x20   claude 열림 · 아직 안 붙음\n\
             \x20 ci 실패\n\
             최근 편지\n\
             \x20 letter-9 · 3분 전 · w → lead · progress · pushed the fix\n\
             검토자 편지\n\
             \x20 letter-7 · 20분 전 · r → lead · report · FAIL: input lost\n"
        );
        let unread = json!({"mission_id": "mission-33", "node": {"key": "x", "status": "planned"}, "letters": null});
        assert!(
            draw(&unread, NOW)
                .unwrap()
                .ends_with("최근 편지 · 읽지 못함\n")
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
            .find(|line| line.starts_with("작업자 "))
            .map(str::to_owned)
        };
        node["launched"] =
            json!({"agent_id": "agent-2", "agent": "claude", "item": "Claude", "model": "opus"});
        assert_eq!(
            worker(&node).as_deref(),
            Some("작업자 Claude opus · agent-2 · 깨어 있음")
        );
        node["launched"]["profile"] = json!({"id": "p-work", "name": "work"});
        node["launched"]["item"] = json!("Claude · work");
        node["launched"]["model_label"] = json!("deepseek/deepseek-v4.1-flash");
        assert_eq!(
            worker(&node).as_deref(),
            Some("작업자 Claude · work deepseek/deepseek-v4.1-flash · agent-2 · 깨어 있음")
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
            .find(|line| line.starts_with("작업자 "))
            .map(str::to_owned)
        };
        assert_eq!(
            line(&node).as_deref(),
            Some("작업자 Claude · gw glm-5 high")
        );
        node["worker"] = json!({"agent": "claude", "profile": "p-gone", "item": "Claude", "profile_gone": true, "model": "opus"});
        assert_eq!(
            line(&node).as_deref(),
            Some("작업자 Claude · 없는 프로필 opus")
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
            text.contains("상태 계획 · research · 기다림 show\n"),
            "{text}"
        );
        let decided = mission()["mission"]["nodes"][1].clone();
        let text = alone(&decided);
        assert!(
            text.contains("상태 작업 중 · change · PR로 · 기다림 운영자 결정\n"),
            "{text}"
        );
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
        assert_eq!(text, format!("{head}\n도착 · feat/login\n{rest}"));

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
        assert!(old.contains("· main에 바로 ·"), "{old}");
        assert!(!old.contains("도착"), "{old}");
        let text = alone(&direct, Some("feat/login"));
        assert!(
            text.contains("· feat/login에 바로 ·") && text.contains("\n도착 feat/login\n"),
            "{text}"
        );
        // A node that changes no code goes to no branch.
        let research = mission()["mission"]["nodes"][3].clone();
        assert!(!alone(&research, Some("feat/login")).contains("도착"));

        // B11: a branch Hide could not read says why, which names a
        // destination that is gone.
        let mut answer = json!({"mission_id": "mission-33", "node": direct, "letters": [],
            "destination": "feat/login",
            "unread_reason": "Neither feat/login nor origin/feat/login could be read"});
        answer["node"]["branch"] = json!("feat/fix");
        let text = draw(&answer, NOW).unwrap();
        assert!(
            text.contains(
                "브랜치 feat/fix\n읽지 못함 · Neither feat/login nor origin/feat/login could be read\n"
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
            "#8 대기 · CI 통과 ·",
            "#8 대기 · CI 통과 · 👤 머지할 차례 ·",
        );
        assert_eq!(
            text,
            format!("{head}\n머지까지 · ci만 👤 내가 머지\n{rest}")
        );

        let alone = draw(
            &json!({"mission_id": "mission-33", "node": value["mission"]["nodes"][1], "letters": []}),
            NOW,
        )
        .unwrap();
        assert!(
            alone.contains("\n👤 머지할 차례 · DB는 직접 보고 머지\n"),
            "{alone}"
        );
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
}
