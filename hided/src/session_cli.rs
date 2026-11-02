//! `hide session list|record|read` (PRD agent-session-read): the core
//! answers one JSON value (`herdr_core::links::session_command`); this draws
//! it as text for an agent to read, or prints it with `--json`. Both forms
//! carry the same sessions, request numbers, source marks and omission
//! counts (B19): a record past [`RECORD_TEXT_BYTES`] has its oldest flows
//! folded before either form is printed (B7).

use crate::env::Env;
use herdr_core::links::session_command::SessionCommand;
use serde_json::{Value, json};
use std::fmt::Write as _;

pub const USAGE: &str = "hide session list [--since <n>m|h|d] [--limit <n>] [--json]\nhide session record <id> [--json]\nhide session read <id> --request <n> | --from <n> [--events <n>] [--json]";

/// The most a record's text takes; past it the oldest flows fold (D-12).
pub const RECORD_TEXT_BYTES: usize = 64 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub struct Cli {
    pub command: SessionCommand,
    pub json: bool,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<Cli, String> {
    let verb = args.next().map(String::as_str).ok_or(USAGE)?;
    let mut json = false;
    let mut words = Vec::new();
    let mut options: Vec<(&str, &str)> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" if !json => json = true,
            name @ ("--since" | "--limit" | "--request" | "--from" | "--events") => {
                if options.iter().any(|(seen, _)| *seen == name) {
                    return Err(USAGE.to_owned());
                }
                let value = args.next().ok_or(USAGE)?;
                options.push((name, value.as_str()));
            }
            other if other.starts_with('-') => return Err(USAGE.to_owned()),
            other => words.push(other),
        }
    }
    let option = |name: &str| {
        options
            .iter()
            .find(|(seen, _)| *seen == name)
            .map(|(_, value)| *value)
    };
    let number = |name: &str| -> Result<Option<u32>, String> {
        option(name)
            .map(|value| value.parse::<u32>().map_err(|_| USAGE.to_owned()))
            .transpose()
    };
    let allowed = |names: &[&str]| options.iter().all(|(name, _)| names.contains(name));
    let command = match (verb, words.as_slice()) {
        ("list", []) if allowed(&["--since", "--limit"]) => SessionCommand::List {
            since_ms: option("--since").map(period).transpose()?,
            limit: number("--limit")?,
        },
        ("record", [id]) if allowed(&[]) => SessionCommand::Record {
            id: (*id).to_owned(),
        },
        ("read", [id]) if allowed(&["--request", "--from", "--events"]) => {
            let (request, from) = (number("--request")?, number("--from")?);
            if request.is_some() == from.is_some() {
                return Err(USAGE.to_owned());
            }
            SessionCommand::Read {
                id: (*id).to_owned(),
                request,
                from,
                events: number("--events")?,
            }
        }
        _ => return Err(USAGE.to_owned()),
    };
    Ok(Cli { command, json })
}

/// A period as `<n>m`, `<n>h` or `<n>d`, in milliseconds.
fn period(value: &str) -> Result<u64, String> {
    let (digits, minutes) = [("m", 1), ("h", 60), ("d", 24 * 60)]
        .into_iter()
        .find_map(|(unit, minutes)| Some((value.strip_suffix(unit)?, minutes)))
        .ok_or_else(|| USAGE.to_owned())?;
    digits
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(minutes * 60_000))
        .ok_or_else(|| USAGE.to_owned())
}

pub fn run(env: &Env, cli: &Cli) -> Result<(), String> {
    let refusal = |reason: &str, next_action: &str| {
        println!(
            "{}",
            json!({"ok": false, "reason": reason, "next_action": next_action})
        );
        Err(reason.to_owned())
    };
    let mut credential = match crate::workspace_cli::Credential::acquire(env) {
        Ok(credential) => credential,
        Err(reason) => return refusal(&reason, crate::cli::bootstrap_next_action(&reason)),
    };
    let answer = match crate::workspace_cli::request_session(&mut credential, &cli.command) {
        Ok(answer) => answer,
        Err(reason) => return refusal(&reason, crate::cli::request_next_action(&reason)),
    };
    if answer["ok"] != true {
        let reason = answer["reason"].as_str().unwrap_or("session_unavailable");
        let next = answer["next_action"].as_str().unwrap_or(reason);
        return refusal(reason, next);
    }
    let mut value = answer["result"].clone();
    if value["verb"] == "record" {
        fold_record(&mut value, RECORD_TEXT_BYTES);
    }
    if cli.json {
        println!("{}", json!({"ok": true, "value": value}));
    } else {
        print!("{}", draw(&value));
    }
    Ok(())
}

/// The answer as text.
pub fn draw(value: &Value) -> String {
    match value["verb"].as_str() {
        Some("list") => draw_list(&one_line(value)),
        Some("record") => draw_record(&one_line(value)),
        Some("read") => draw_read(value),
        _ => format!("{value}\n"),
    }
}

/// `value` with every line break and control character in its strings made
/// a space: a list or record line is one line, so a title cannot draw a
/// line of its own, such as a forged `[AI 요약]` mark. A read's events keep
/// their lines; they carry no marks.
fn one_line(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(
            text.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(one_line).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), one_line(field)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Folds the oldest past flows to their result line, then, while the text is
/// still past `limit`, the oldest requests of the current flow, and counts
/// both in the answer.
pub fn fold_record(value: &mut Value, limit: usize) {
    let mut folded = 0u64;
    let mut cut = 0u64;
    loop {
        if draw_record(value).len() <= limit {
            break;
        }
        let Some(flows) = value["flows"].as_array_mut() else {
            break;
        };
        if let Some(flow) = flows
            .iter_mut()
            .find(|flow| flow["current"] != true && flow["folded"] != true)
        {
            flow["requests"] = json!([]);
            flow["folded"] = json!(true);
            folded += 1;
            continue;
        }
        let Some(requests) = flows
            .iter_mut()
            .find(|flow| flow["current"] == true)
            .and_then(|flow| flow["requests"].as_array_mut())
            .filter(|requests| !requests.is_empty())
        else {
            break;
        };
        // A tenth at a time, so a long flow folds in a few passes.
        let drop = (requests.len() / 10).max(1);
        requests.drain(..drop);
        cut += drop as u64;
    }
    value["folded_flows"] = json!(folded);
    value["folded_requests"] = json!(cut);
}

fn tag(source: &Value) -> &'static str {
    if source == "ai_summary" {
        "[AI 요약]"
    } else {
        "[원문]"
    }
}

fn age(now: u64, at: &Value) -> String {
    let Some(at) = at.as_u64() else {
        return "-".to_owned();
    };
    let minutes = now.saturating_sub(at) / 60_000;
    match minutes {
        0 => "방금".to_owned(),
        1..=59 => format!("{minutes}분 전"),
        60..=1439 => format!("{}시간 전", minutes / 60),
        _ => format!("{}일 전", minutes / 1440),
    }
}

fn period_words(ms: u64) -> String {
    let minutes = ms / 60_000;
    if minutes.is_multiple_of(1440) {
        format!("최근 {}일", minutes / 1440)
    } else if minutes.is_multiple_of(60) {
        format!("최근 {}시간", minutes / 60)
    } else {
        format!("최근 {minutes}분")
    }
}

fn state_word(kind: &Value) -> &'static str {
    match kind.as_str() {
        Some("working") => "작업 중",
        Some("needs_you") => "답 기다림",
        Some("done") => "끝",
        Some("question") => "질문",
        Some("blocked") => "막힘",
        Some("waiting") => "기다림",
        Some("unfinished") => "멈춤",
        Some("limited") => "한도",
        Some("merged") => "머지됨",
        Some("closed") => "닫힘",
        Some("closing") => "닫은 줄",
        Some("moved_on") => "넘어감",
        _ => "-",
    }
}

fn device_words(now: u64, device: &Value) -> String {
    let name = if device["local"] == true {
        "이 기기".to_owned()
    } else {
        device["label"]
            .as_str()
            .or_else(|| device["id"].as_str())
            .unwrap_or("-")
            .to_owned()
    };
    match device["disconnected_since_unix_ms"].as_u64() {
        Some(0) => format!("{name} · 장치 끊김 · 마지막 값"),
        Some(since) => format!("{name} · 장치 끊김 · {} 값", age(now, &json!(since))),
        None => name,
    }
}

fn session_name(session: &Value) -> String {
    session["id"]
        .as_str()
        .or_else(|| session["session_id"].as_str())
        .unwrap_or("-")
        .to_owned()
}

fn draw_list(value: &Value) -> String {
    let now = value["now_unix_ms"].as_u64().unwrap_or(0);
    let period = period_words(value["since_ms"].as_u64().unwrap_or(0));
    let sessions = value["sessions"].as_array().cloned().unwrap_or_default();
    let mut text = String::new();
    if sessions.is_empty() {
        let _ = writeln!(text, "{period} 세션 없음");
        return text;
    }
    let _ = writeln!(
        text,
        "{} · {period} · {}개",
        value["project"]["label"].as_str().unwrap_or("-"),
        sessions.len()
    );
    for session in &sessions {
        let mut line = format!(
            "{} · {} · {} · {} {}",
            session_name(&session["session"]),
            session["agent_name"].as_str().unwrap_or("-"),
            state_word(&session["state"]["kind"]),
            tag(&session["title_source"]),
            session["title"].as_str().unwrap_or("-"),
        );
        let prs = session["prs"].as_array().cloned().unwrap_or_default();
        if !prs.is_empty() {
            let numbers: Vec<String> = prs.iter().map(|n| format!("#{n}")).collect();
            let _ = write!(line, " · PR {}", numbers.join(" "));
        }
        let _ = write!(
            line,
            " · 요청 {} · {} · {}",
            session["requests"],
            device_words(now, &session["device"]),
            age(now, &session["last_activity_unix_ms"]),
        );
        if session["archived_at_unix_ms"].is_u64() {
            let _ = write!(
                line,
                " · 보관됨 · {}",
                age(now, &session["archived_at_unix_ms"])
            );
        }
        let _ = writeln!(text, "{line}");
    }
    if let Some(more) = value["more"].as_u64().filter(|more| *more > 0) {
        let _ = writeln!(text, "{more}개 더 있음");
    }
    text
}

fn draw_state(now: u64, state: &Value) -> String {
    let mut line = state_word(&state["kind"]).to_owned();
    if let Some(prs) = state["prs"].as_array().filter(|prs| !prs.is_empty()) {
        let numbers: Vec<String> = prs.iter().map(|n| format!("#{n}")).collect();
        let _ = write!(line, " {}", numbers.join(" "));
    }
    if let Some(said) = state["line"].as_str() {
        let _ = write!(line, " · {} {said}", tag(&state["line_source"]));
    }
    if let Some(limited) = state["limited"].as_str() {
        let _ = write!(line, " · 한도 {limited}");
    }
    if state["at_unix_ms"].is_u64() {
        let _ = write!(line, " · {}", age(now, &state["at_unix_ms"]));
    }
    line
}

fn draw_record(value: &Value) -> String {
    let now = value["now_unix_ms"].as_u64().unwrap_or(0);
    let summaries = value["summaries"] == true;
    let head = &value["head"];
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# {} {} ({})",
        tag(&head["title_source"]),
        head["title"].as_str().unwrap_or("-"),
        session_name(&value["session"]),
    );
    let _ = writeln!(
        text,
        "{} · {} · {} · ⌂ {}",
        device_words(now, &head["device"]),
        head["agent_name"].as_str().unwrap_or("-"),
        head["project"]["label"].as_str().unwrap_or("-"),
        head["checkout"].as_str().unwrap_or("-"),
    );
    if value["state"]["source"] == "none" {
        let _ = writeln!(
            text,
            "상태: {}",
            value["native_state"].as_str().unwrap_or("-")
        );
    } else {
        let _ = writeln!(text, "상태: {}", draw_state(now, &value["state"]));
    }
    if value["archived_at_unix_ms"].is_u64() {
        let _ = writeln!(text, "보관됨 · {}", age(now, &value["archived_at_unix_ms"]));
    }
    if !summaries {
        let _ = writeln!(text, "AI 요약 꺼짐");
    }
    if let Some(now_block) = value["now"].as_object() {
        let request = now_block["request"].as_str();
        let said = now_block["now"].as_str();
        let _ = writeln!(
            text,
            "지금: {}{}{}",
            request.map_or(String::new(), |request| format!("[원문] {request}")),
            if request.is_some() && said.is_some() {
                " → "
            } else {
                ""
            },
            said.map_or(String::new(), |said| format!("[원문] {said}")),
        );
    }
    if let Some(asks) = value["asks"].as_array().filter(|asks| !asks.is_empty()) {
        let _ = writeln!(text, "\n## 나를 기다리는 것");
        for ask in asks {
            let _ = writeln!(
                text,
                "- {} {}",
                tag(&ask["source"]),
                ask["text"].as_str().unwrap_or("")
            );
        }
    }
    let flows = value["flows"].as_array().cloned().unwrap_or_default();
    if !flows.is_empty() {
        let _ = writeln!(text, "\n## 흐름");
    }
    for flow in &flows {
        if summaries {
            let when = if flow["current"] == true {
                "지금".to_owned()
            } else {
                format!("요청 {}", flow["request_count"])
            };
            let _ = writeln!(
                text,
                "### {} {} · {} · {}{}",
                tag(&flow["name_source"]),
                flow["name"].as_str().unwrap_or("-"),
                age(now, &flow["at_unix_ms"]),
                when,
                if flow["folded"] == true {
                    " · 접음"
                } else {
                    ""
                },
            );
            if flow["result"].is_object() {
                let result = &flow["result"];
                let mut line = format!("결과: {}", state_word(&result["kind"]));
                if let Some(prs) = result["prs"].as_array() {
                    let numbers: Vec<String> = prs.iter().map(|n| format!("#{n}")).collect();
                    let _ = write!(line, " {}", numbers.join(" "));
                }
                if let Some(said) = result["line"].as_str() {
                    let _ = write!(line, " · {} {said}", tag(&result["source"]));
                }
                let _ = writeln!(text, "{line}");
            }
        }
        for request in flow["requests"].as_array().into_iter().flatten() {
            let mut line = match request["number"].as_u64() {
                Some(number) => format!("  {number}. "),
                None => format!(
                    "  ✉ {} · ",
                    request["letter_from"].as_str().unwrap_or("편지")
                ),
            };
            let _ = write!(line, "{} · ", age(now, &request["at_unix_ms"]));
            if let Some(answers) = request["answers"].as_str() {
                let _ = write!(line, "↩ {answers}: ");
            }
            let _ = write!(
                line,
                "{} {}",
                tag(&request["source"]),
                request["text"].as_str().unwrap_or("")
            );
            if request["count"].as_u64().is_some_and(|count| count > 1) {
                let _ = write!(line, " ×{}", request["count"]);
            }
            if request["images"].as_u64().is_some_and(|images| images > 0) {
                let _ = write!(line, " · 이미지 {}장", request["images"]);
            }
            let _ = writeln!(text, "{line}");
        }
    }
    if let Some(folded) = value["folded_flows"].as_u64().filter(|n| *n > 0) {
        let _ = writeln!(text, "오래된 흐름 {folded}개 접음");
    }
    if let Some(cut) = value["folded_requests"].as_u64().filter(|n| *n > 0) {
        let _ = writeln!(text, "지금 흐름의 오래된 요청 {cut}개 접음");
    }
    if let Some(omitted) = value["omitted_requests"].as_u64().filter(|n| *n > 0) {
        let _ = writeln!(text, "오래된 요청 {omitted}개는 기록에 없음");
    }
    let groups = value["pr_groups"].as_array().cloned().unwrap_or_default();
    if groups.iter().any(|group| {
        group["pulls"]
            .as_array()
            .is_some_and(|pulls| !pulls.is_empty())
    }) {
        let _ = writeln!(text, "\n## PR");
        for group in &groups {
            if group["child"].is_object() {
                let _ = writeln!(text, "{}:", group["name"].as_str().unwrap_or("-"));
            }
            for pull in group["pulls"].as_array().into_iter().flatten() {
                let _ = writeln!(
                    text,
                    "- #{} {} ({})",
                    pull["number"],
                    pull["title"].as_str().unwrap_or(""),
                    pull["state"].as_str().unwrap_or("-"),
                );
            }
        }
        if let Some(omitted) = value["pr_omitted"].as_u64().filter(|n| *n > 0) {
            let _ = writeln!(text, "PR {omitted}개 더 있음");
        }
    }
    if let Some(artifacts) = value["artifacts"]
        .as_array()
        .filter(|artifacts| !artifacts.is_empty())
    {
        let _ = writeln!(text, "\n## 결과물");
        for artifact in artifacts {
            let _ = writeln!(text, "- [AI 요약] {}", artifact.as_str().unwrap_or(""));
        }
    }
    if let Some(children) = value["children"]
        .as_array()
        .filter(|children| !children.is_empty())
    {
        let _ = writeln!(text, "\n## 함께 진행한 세션");
        for child in children {
            let _ = writeln!(
                text,
                "- {} · {} · {} {} · PR {} · {}",
                session_name(&child["session"]),
                state_word(&child["state"]["kind"]),
                tag(&child["title_source"]),
                child["title"].as_str().unwrap_or("-"),
                child["prs"],
                device_words(now, &child["device"]),
            );
        }
        if let Some(omitted) = value["children_omitted"].as_u64().filter(|n| *n > 0) {
            let _ = writeln!(text, "세션 {omitted}개 더 있음");
        }
    }
    text
}

fn draw_read(value: &Value) -> String {
    let mut text = String::new();
    let of = value["total"]
        .as_u64()
        .map_or_else(|| "more".to_owned(), |total| total.to_string());
    let next = value["next_from"].as_u64().map_or_else(
        || "마지막 이벤트까지 읽음".to_owned(),
        |next| format!("다음 --from {next}"),
    );
    let _ = writeln!(
        text,
        "events {}-{} of {of} · {next}",
        value["first"], value["last"]
    );
    for event in value["events"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "\n#{} {} ({}){}",
            event["number"],
            event["role"].as_str().unwrap_or("-"),
            event["kind"].as_str().unwrap_or("-"),
            if event["images"].as_u64().is_some_and(|n| n > 0) {
                format!(" · 이미지 {}장", event["images"])
            } else {
                String::new()
            },
        );
        let _ = writeln!(text, "[원문] {}", event["text"].as_str().unwrap_or(""));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn parsed(words: &[&str]) -> Result<Cli, String> {
        parse(args(words).iter())
    }

    #[test]
    fn the_three_verbs_parse_with_their_options() {
        assert_eq!(
            parsed(&["list", "--since", "3d", "--limit", "20", "--json"]).unwrap(),
            Cli {
                command: SessionCommand::List {
                    since_ms: Some(3 * 24 * 60 * 60 * 1000),
                    limit: Some(20),
                },
                json: true,
            }
        );
        assert_eq!(
            parsed(&["record", "agent-12"]).unwrap().command,
            SessionCommand::Record {
                id: "agent-12".into()
            }
        );
        assert_eq!(
            parsed(&["read", "abc", "--request", "3", "--events", "8"])
                .unwrap()
                .command,
            SessionCommand::Read {
                id: "abc".into(),
                request: Some(3),
                from: None,
                events: Some(8),
            }
        );
    }

    #[test]
    fn a_read_takes_one_of_request_or_from_and_options_stay_with_their_verb() {
        for words in [
            &["read", "abc"][..],
            &["read", "abc", "--request", "1", "--from", "2"],
            &["record", "abc", "--limit", "3"],
            &["list", "--since", "7w"],
            &["list", "--since", "7일"],
            &["list", "--since", "７d"],
            &["list", "--since", "d"],
            &["list", "extra"],
            &["list", "--limit", "many"],
        ] {
            assert!(parsed(words).is_err(), "{words:?}");
        }
        // A limit past the cap reaches the core, which refuses it as
        // invalid_request (B2).
        assert!(parsed(&["list", "--limit", "500"]).is_ok());
    }

    fn record_with(flows: usize, requests: usize, words: usize) -> Value {
        let flows: Vec<Value> = (0..flows)
            .map(|index| {
                json!({
                    "name": format!("flow {index}"), "name_source": "ai_summary", "at_unix_ms": 0,
                    "request_count": requests, "current": index + 1 == flows, "result": null, "folded": false,
                    "requests": (0..requests).map(|n| json!({
                        "number": n + 1, "at_unix_ms": 0, "text": "가".repeat(words), "source": "raw",
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({"verb": "record", "summaries": true, "now_unix_ms": 0, "head": {}, "session": {}, "state": {"source": "none"}, "flows": flows})
    }

    #[test]
    fn a_long_record_folds_its_oldest_flows_and_says_how_many() {
        let mut value = record_with(6, 40, 120);
        assert!(draw_record(&value).len() > RECORD_TEXT_BYTES);
        fold_record(&mut value, RECORD_TEXT_BYTES);
        let text = draw_record(&value);
        assert!(text.len() <= RECORD_TEXT_BYTES, "{}", text.len());
        let folded = value["folded_flows"].as_u64().unwrap();
        assert!(folded > 0);
        assert!(text.contains(&format!("오래된 흐름 {folded}개 접음")));
        assert_eq!(value["flows"][0]["folded"], true);
        assert_eq!(value["flows"][5]["folded"], false, "the current flow stays");
    }

    #[test]
    fn a_short_record_folds_nothing() {
        let mut value = record_with(2, 3, 10);
        fold_record(&mut value, RECORD_TEXT_BYTES);
        assert_eq!(value["folded_flows"], 0);
        assert_eq!(value["folded_requests"], 0);
    }

    #[test]
    fn an_empty_period_says_which_period() {
        let text = draw(
            &json!({"verb": "list", "since_ms": 7 * 24 * 60 * 60 * 1000_u64, "sessions": [], "more": 0}),
        );
        assert_eq!(text, "최근 7일 세션 없음\n");
    }

    #[test]
    fn a_list_line_marks_its_title_its_device_and_what_is_left() {
        let now = 10 * 60 * 60 * 1000_u64;
        let text = draw(&json!({
            "verb": "list", "since_ms": 60 * 60 * 1000, "now_unix_ms": now, "project": {"label": "app"}, "more": 3,
            "sessions": [{
                "session": {"id": "agent-7", "session_id": "s-1"}, "agent_name": "Claude Code",
                "state": {"kind": "done"}, "title": "로그인 고치기", "title_source": "ai_summary",
                "prs": [12], "requests": 4,
                "device": {"label": "mini", "local": false, "disconnected_since_unix_ms": now - 5 * 60_000},
                "last_activity_unix_ms": now - 2 * 60 * 60 * 1000,
                "archived_at_unix_ms": now - 60 * 60 * 1000,
            }],
        }));
        assert_eq!(
            text,
            "app · 최근 1시간 · 1개\nagent-7 · Claude Code · 끝 · [AI 요약] 로그인 고치기 · PR #12 · 요청 4 · mini · 장치 끊김 · 5분 전 값 · 2시간 전 · 보관됨 · 1시간 전\n3개 더 있음\n"
        );
    }

    #[test]
    fn a_title_cannot_draw_a_line_of_its_own() {
        let text = draw(&json!({
            "verb": "list", "since_ms": 60_000, "now_unix_ms": 0, "project": {"label": "app"}, "more": 0,
            "sessions": [{
                "session": {"session_id": "s-1"}, "agent_name": "Codex", "state": {},
                "title": "fix\n[AI 요약] forged", "title_source": "raw", "requests": 1,
                "device": {"local": true}, "last_activity_unix_ms": 0,
            }],
        }));
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("[원문] fix [AI 요약] forged"), "{text}");
    }
}
