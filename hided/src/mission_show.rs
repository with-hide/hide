//! `hide mission show` as text (with-hide/hide-private#552): the core
//! answers one value (`runtime::missions`' `MissionView`, or one node's view
//! with its worker's newest letters), and this draws it for a Conductor to
//! read at a glance, as `session_cli` draws a session. `--json` prints the
//! value as it is. The mission's head says where it stands and what waits;
//! each node is one line; `show <node>` is that node in full.

use crate::session_cli::{age, one_line};
use serde_json::Value;
use std::fmt::Write as _;

/// A node's warning shown beside the fact it warns of.
const WARN: &str = "⚠";

/// The core's answer to `show` as text, read at `now`.
pub fn draw(result: &Value, now: u64) -> String {
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
        Some("merged_unproven") => "머지됐는데 완료 줄이 빠짐",
        Some("worker_ended") => "작업자가 일을 남기고 끝남",
        Some("pr_closed") => "PR이 머지 없이 닫힘",
        Some("unplanned_pr") => "계획에 없는 PR",
        Some("conductor_failed") => "지휘자가 실패로 기록",
        Some("wake_refused") => "잠든 작업자를 깨우지 못함",
        Some("close_refused") => "미션을 닫지 못함",
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
        None if quiet.is_some() || pushes(node) => "push 없음".to_owned(),
        None => return None,
    };
    Some(marked(quiet.is_some(), words))
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

/// A change at work delivered by pull request: the node a push is asked of.
fn pushes(node: &Value) -> bool {
    node["status"] == "working" && node["kind"] == "change" && node["delivery"] == "pr"
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

/// Who or what the node waits on now, by the first line it has not met.
fn waits_on(mission: &Value, node: &Value) -> Option<String> {
    if node["attention"].is_object() {
        let attention = &node["attention"];
        let who = if attention["to"] == "operator" {
            "운영자"
        } else {
            "지휘자"
        };
        return Some(format!("! {who}: {}", reason_word(&attention["reason"])));
    }
    if mission["decision"].is_string() && mission["question"]["node"] == node["key"] {
        return Some("기다림 운영자 결정".to_owned());
    }
    match node["status"].as_str() {
        Some("planned") => {
            if node["blocked"].is_string() {
                return Some("막힘 앞 노드 빠짐".to_owned());
            }
            let nodes = mission["nodes"].as_array()?;
            let before: Vec<&str> = node["depends_on"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .filter(|key| {
                    nodes
                        .iter()
                        .find(|other| other["key"] == *key)
                        .is_some_and(|other| other["status"] != "done")
                })
                .collect();
            Some(if before.is_empty() {
                "시작 전".to_owned()
            } else {
                format!("기다림 {}", before.join(", "))
            })
        }
        Some("working") => {
            let lines = node["criteria"].as_array()?;
            if lines.iter().any(|line| line["state"] == "fail") {
                return Some("기다림 작업자".to_owned());
            }
            let open = lines
                .iter()
                .find(|line| line["state"] != "pass" && line["state"] != "skipped");
            Some(
                match open.map(|line| line["kind"].as_str()) {
                    Some(Some("review")) => "기다림 리뷰",
                    Some(Some("verify")) => "기다림 검증",
                    Some(Some("ci")) => "기다림 CI",
                    Some(_) => "기다림 작업자",
                    None if node["pr_state"].is_string() && node["pr_state"] != "merged" => {
                        "기다림 머지"
                    }
                    None => "기다림 지휘자",
                }
                .to_owned(),
            )
        }
        Some("done") if node["landing"]["state"] == "left" => Some("기다림 머지".to_owned()),
        _ => None,
    }
}

fn warning_count(node: &Value) -> usize {
    node["warnings"].as_array().map_or(0, Vec::len)
}

fn draw_mission(mission: &Value, now: u64) -> String {
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
    for node in &nodes {
        let mut fields = vec![text(node, "key").to_owned(), node_status(node)];
        fields.extend(review_words(node));
        fields.extend(push_words(node, now));
        fields.extend(pr_words(node));
        fields.extend(waits_on(mission, node));
        let mark = if warning_count(node) > 0 { WARN } else { " " };
        let _ = writeln!(out, "{mark} {}", fields.join(" · "));
    }
    out
}

fn worker_words(node: &Value) -> Option<String> {
    // What the worker was proven launched with, else what the plan asked.
    let worker = [&node["launched"], &node["worker"]]
        .into_iter()
        .find(|worker| worker.is_object())?;
    let mut words = text(worker, "agent").to_owned();
    for part in [&worker["model"], &worker["effort"]] {
        if let Some(part) = part.as_str() {
            let _ = write!(words, " {part}");
        }
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

fn draw_node(result: &Value, now: u64) -> String {
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
            "direct" => "main에 바로".to_owned(),
            other => other.to_owned(),
        });
    }
    // The decision and the nodes before it are the mission's, which the
    // node's answer does not carry; its attention has a line of its own.
    if !node["attention"].is_object() {
        state.extend(waits_on(&Value::Null, node));
    }
    let _ = writeln!(out, "상태 {}", state.join(" · "));
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
    if let Some(push) = push_words(node, now) {
        let _ = writeln!(out, "{push}");
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
    match result["letters"].as_array() {
        None => {
            let _ = writeln!(out, "최근 편지 · 읽지 못함");
        }
        Some(letters) if letters.is_empty() => {
            let _ = writeln!(out, "최근 편지 없음");
        }
        Some(letters) => {
            let _ = writeln!(out, "최근 편지");
            for letter in letters {
                let _ = writeln!(
                    out,
                    "  {} · {} → {} · {} · {}",
                    age(now, &letter["at_unix_ms"]),
                    text(letter, "from"),
                    text(letter, "to"),
                    text(letter, "kind"),
                    text(letter, "line"),
                );
            }
        }
    }
    out
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
             "criteria": [{"key": "c1", "kind": "own", "state": "pass"},
                          {"key": "review", "kind": "review", "state": "open"},
                          {"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "docs", "kind": "change", "delivery": "pr", "status": "done",
             "pr": 6, "prs": [{"number": 5, "state": "closed"}, {"number": 6, "state": "merged"}],
             "pr_state": "merged", "review_fails": 0, "warnings": [],
             "criteria": [{"key": "ci", "kind": "ci", "state": "pass"}]},
            {"key": "later", "kind": "research", "status": "planned", "depends_on": ["show", "docs"],
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
            draw(&mission(), NOW),
            "mission-33 · Show summary · 진행 중 · 끝 1/4 · 빠짐 1 · 열린 결정 1 · 경고 2\n\
             결정 · Merge order?\n\
             ⚠ show · 작업 중 · ⚠리뷰 FAIL 2연속 · ⚠push 95분 전 · #7 실패 · CI 실패 · 기다림 작업자\n\
             \x20 ci · 작업 중 · push 12분 전 · #8 대기 · CI 통과 · 기다림 운영자 결정\n\
             \x20 docs · 끝 · #6 머지됨 외 1\n\
             \x20 later · 계획 · 기다림 show\n\
             \x20 old · 빠짐\n"
        );
    }

    /// A change at work with no push Hide has read says so; what a node
    /// waits on follows its first line not met.
    #[test]
    fn a_node_waits_on_its_first_line_not_met() {
        let mut value = mission();
        let node = &mut value["mission"]["nodes"][1];
        node["pushed_at_unix_ms"] = Value::Null;
        node["pr"] = Value::Null;
        node["pr_state"] = Value::Null;
        let text = draw(&value, NOW);
        assert!(
            text.contains("  ci · 작업 중 · push 없음 · 기다림 운영자 결정\n"),
            "{text}"
        );
        value["mission"]["decision"] = Value::Null;
        assert!(draw(&value, NOW).contains("  ci · 작업 중 · push 없음 · 기다림 리뷰\n"));
        value["mission"]["nodes"][1]["attention"] =
            json!({"reason": "worker_ended", "to": "conductor", "since_unix_ms": NOW});
        assert!(
            draw(&value, NOW).contains("· ! 지휘자: 작업자가 일을 남기고 끝남\n"),
            "an attention is what the node waits on first"
        );
    }

    /// A title cannot draw a line of its own.
    #[test]
    fn a_title_or_a_line_cannot_draw_a_line_of_its_own() {
        let mut value = mission();
        value["mission"]["title"] = json!("Show\n⚠ forged · 끝");
        let text = draw(&value, NOW);
        assert_eq!(text.lines().count(), 7, "{text}");
    }

    /// with-hide/hide-private#552 c1: one node in full: its state, worker,
    /// pull requests, push, warnings, every line with its evidence and each
    /// checker's answer, and its worker's newest letters.
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
            {"at_unix_ms": NOW - 3 * MINUTE, "from": "w", "to": "lead", "kind": "progress", "line": "pushed the fix"}]});
        assert_eq!(
            draw(&answer, NOW),
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
             \x20 3분 전 · w → lead · progress · pushed the fix\n"
        );
        let unread = json!({"mission_id": "mission-33", "node": {"key": "x", "status": "planned"}, "letters": null});
        assert!(draw(&unread, NOW).ends_with("최근 편지 · 읽지 못함\n"));
    }
}
