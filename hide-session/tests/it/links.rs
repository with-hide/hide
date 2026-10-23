//! What a session file says about links (PRD link-graph D-15, D-27, D-28,
//! D-43), read through the shared line parser.

use hide_session::links::{self, ReadRequest};
use hide_session::{Agent, parse_events};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/adapters")
        .join(relative)
}

const CLAUDE: &str = "claude-2.1.288/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl";
const CLAUDE_SUBAGENT: &str = "claude-2.1.288/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001/subagents/agent-a1.jsonl";

#[test]
fn a_claude_session_names_its_branch_its_pull_request_and_the_request_before_it() {
    let contents = fs::read_to_string(fixture(CLAUDE)).unwrap();
    let facts = parse_events(Agent::Claude, &contents).links;

    assert_eq!(
        facts.session_id.as_deref(),
        Some("a1b2c3d4-0000-4000-8000-000000000001")
    );
    assert_eq!(facts.cwd.as_deref(), Some("/work/app"));
    assert_eq!(facts.interactive, Some(true));
    assert!(!facts.subagent);
    let branches: Vec<_> = facts.spans.iter().map(|s| s.branch.as_deref()).collect();
    assert_eq!(branches, vec![Some("main")]);
    // The tool output and the agent's own `pr-link` record name the same
    // pull request once; the address in a reply is a mention, not a link.
    assert_eq!(facts.prs.len(), 1);
    let pr = &facts.prs[0];
    assert_eq!((pr.repository.as_str(), pr.number), ("acme/app", 12));
    assert_eq!(
        pr.request.as_deref(),
        Some("요청 보기를 만들어줘\n긴 요청의 둘째 줄")
    );
    // A Hide letter and the compaction summary are not the operator's.
    let (_, last) = facts.last_request.clone().unwrap();
    assert_eq!(last, "요청 보기를 만들어줘\n긴 요청의 둘째 줄");
    assert!(facts.first_at_unix_ms.unwrap() <= facts.last_at_unix_ms.unwrap());
}

#[test]
fn a_subagent_file_folds_into_its_parent_and_makes_no_request() {
    let contents = fs::read_to_string(fixture(CLAUDE_SUBAGENT)).unwrap();
    let facts = parse_events(Agent::Claude, &contents).links;

    assert_eq!(
        facts.session_id.as_deref(),
        Some("a1b2c3d4-0000-4000-8000-000000000001")
    );
    assert!(facts.subagent);
    assert_eq!(facts.last_request, None);
    assert_eq!(facts.prs.len(), 1);
    assert_eq!(facts.prs[0].number, 13);
    assert_eq!(facts.prs[0].request, None);
}

fn claude(session: &str, entrypoint: &str, branch: &str, text: &str, at: &str) -> String {
    serde_json::json!({
        "type": "user", "isSidechain": false, "uuid": format!("u-{at}"), "parentUuid": null,
        "message": {"role": "user", "content": text}, "timestamp": at,
        "promptId": "p", "origin": {"kind": "human"}, "userType": "external",
        "entrypoint": entrypoint, "cwd": "/work/app", "sessionId": session, "gitBranch": branch,
    })
    .to_string()
}

#[test]
fn a_print_run_is_not_interactive_and_a_detached_head_is_no_branch() {
    let contents = [
        claude("s1", "sdk-cli", "HEAD", "판정해 줘", "2026-10-03T01:00:00Z"),
        claude("s1", "sdk-cli", "feat/x", "다음", "2026-10-03T01:00:05Z"),
    ]
    .join("\n");
    let facts = parse_events(Agent::Claude, &contents).links;

    assert_eq!(facts.interactive, Some(false));
    let branches: Vec<_> = facts.spans.iter().map(|s| s.branch.clone()).collect();
    assert_eq!(branches, vec![None, Some("feat/x".to_owned())]);
    assert_eq!(facts.spans[1].last_request.as_deref(), Some("다음"));
}

#[test]
fn a_branch_change_starts_a_new_span_with_its_own_last_request() {
    let contents = [
        claude("s1", "cli", "a", "첫 요청", "2026-10-03T01:00:00Z"),
        claude("s1", "cli", "a", "둘째 요청", "2026-10-03T01:00:10Z"),
        claude("s1", "cli", "b", "다른 브랜치", "2026-10-03T01:00:20Z"),
    ]
    .join("\n");
    let facts = parse_events(Agent::Claude, &contents).links;

    assert_eq!(facts.spans.len(), 2);
    assert_eq!(facts.spans[0].branch.as_deref(), Some("a"));
    assert_eq!(facts.spans[0].last_request.as_deref(), Some("둘째 요청"));
    assert_eq!(facts.spans[1].branch.as_deref(), Some("b"));
    assert_eq!(facts.spans[1].last_request.as_deref(), Some("다른 브랜치"));
}

#[test]
fn a_long_request_is_cut_to_five_hundred_characters() {
    let long = "가".repeat(700);
    let contents = claude("s1", "cli", "a", &long, "2026-10-03T01:00:00Z");
    let facts = parse_events(Agent::Claude, &contents).links;

    let (_, request) = facts.last_request.unwrap();
    assert_eq!(request.chars().count(), links::REQUEST_CHARS);
}

fn codex_meta(id: &str, extra: serde_json::Value) -> String {
    let mut payload = serde_json::json!({
        "id": id, "cwd": "/work/app", "originator": "codex-tui", "source": "cli",
        "git": {"branch": "feat/codex", "commit_hash": "abc"},
    });
    for (key, value) in extra.as_object().unwrap() {
        payload[key] = value.clone();
    }
    serde_json::json!({"timestamp": "2026-10-03T01:00:00Z", "type": "session_meta", "payload": payload})
        .to_string()
}

fn codex_user(text: &str, at: &str) -> String {
    serde_json::json!({"timestamp": at, "type": "response_item", "payload": {
        "type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}})
    .to_string()
}

fn codex_output(text: &str, at: &str) -> String {
    serde_json::json!({"timestamp": at, "type": "response_item", "payload": {
        "type": "function_call_output", "call_id": "c", "output": text}})
    .to_string()
}

#[test]
fn a_codex_session_takes_its_branch_from_session_meta() {
    let contents = [
        codex_meta("cx-1", serde_json::json!({})),
        codex_user("PR 올려 줘", "2026-10-03T01:00:10Z"),
        codex_output("https://github.com/acme/app/pull/7", "2026-10-03T01:00:20Z"),
    ]
    .join("\n");
    let facts = parse_events(Agent::Codex, &contents).links;

    assert_eq!(facts.session_id.as_deref(), Some("cx-1"));
    assert_eq!(facts.interactive, Some(true));
    assert_eq!(facts.spans.len(), 1);
    assert_eq!(facts.spans[0].branch.as_deref(), Some("feat/codex"));
    assert_eq!(facts.prs[0].number, 7);
    assert_eq!(facts.prs[0].request.as_deref(), Some("PR 올려 줘"));
}

#[test]
fn codex_exec_is_not_interactive_and_a_fork_keeps_its_own_meta() {
    let exec = codex_meta(
        "cx-2",
        serde_json::json!({"source": "exec", "originator": "codex_exec"}),
    );
    assert_eq!(
        parse_events(Agent::Codex, &exec).links.interactive,
        Some(false)
    );

    let fork = [
        codex_meta("cx-3", serde_json::json!({"forked_from_id": "cx-1"})),
        codex_meta("cx-1", serde_json::json!({"git": {"branch": "other"}})),
    ]
    .join("\n");
    let facts = parse_events(Agent::Codex, &fork).links;
    assert_eq!(facts.session_id.as_deref(), Some("cx-3"));
    assert_eq!(facts.forked_from.as_deref(), Some("cx-1"));
    assert_eq!(facts.spans[0].branch.as_deref(), Some("feat/codex"));
}

#[test]
fn a_codex_subagent_thread_folds_into_its_parent() {
    let contents = [
        codex_meta(
            "cx-child",
            serde_json::json!({"thread_source": "subagent",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "cx-parent"}}}}),
        ),
        codex_user("subagent prompt", "2026-10-03T01:00:10Z"),
    ]
    .join("\n");
    let facts = parse_events(Agent::Codex, &contents).links;

    assert_eq!(facts.session_id.as_deref(), Some("cx-parent"));
    assert!(facts.subagent);
    assert_eq!(facts.last_request, None);
}

fn home_with_claude() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude/projects/-work-app");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("s1.jsonl");
    fs::write(
        &file,
        claude("s1", "cli", "a", "첫 요청", "2026-10-03T01:00:00Z") + "\n",
    )
    .unwrap();
    (home, file)
}

#[test]
fn a_read_resumes_from_its_checkpoint_and_reads_only_what_was_appended() {
    let (home, file) = home_with_claude();
    let listed = links::candidates(home.path(), 0, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].agent, Agent::Claude);

    let request = ReadRequest {
        agent: Agent::Claude,
        path: listed[0].path.clone(),
        checkpoint: None,
    };
    let first = links::read(home.path(), &[request]).remove(0);
    assert_eq!(first.error, None);
    assert_eq!(first.facts.last_request.unwrap().1, "첫 요청");

    let mut contents = fs::read_to_string(&file).unwrap();
    contents.push_str(&(claude("s1", "cli", "a", "둘째 요청", "2026-10-03T01:01:00Z") + "\n"));
    fs::write(&file, contents).unwrap();
    let second = links::read(
        home.path(),
        &[ReadRequest {
            agent: Agent::Claude,
            path: listed[0].path.clone(),
            checkpoint: first.checkpoint,
        }],
    )
    .remove(0);
    assert!(!second.rescanned);
    let requests: Vec<_> = second
        .facts
        .spans
        .iter()
        .filter_map(|span| span.last_request.clone())
        .collect();
    assert_eq!(requests, vec!["둘째 요청".to_owned()]);
}

#[test]
fn a_read_refuses_a_file_outside_the_agent_roots() {
    let (home, _) = home_with_claude();
    let outside = home.path().join("elsewhere.jsonl");
    fs::write(&outside, "{}\n").unwrap();
    let answer = links::read(
        home.path(),
        &[ReadRequest {
            agent: Agent::Claude,
            path: outside.to_string_lossy().into_owned(),
            checkpoint: None,
        }],
    )
    .remove(0);
    assert_eq!(answer.error.as_deref(), Some("links_session_outside_roots"));
    assert!(answer.facts.is_empty());
}

#[test]
fn a_listing_leaves_out_files_older_than_its_start() {
    let (home, _) = home_with_claude();
    let future = u64::MAX / 2;
    assert!(
        links::candidates(home.path(), future, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_later_page_takes_only_files_at_or_before_its_end() {
    let (home, _) = home_with_claude();
    let modified = links::candidates(home.path(), 0, None).unwrap()[0].modified_unix_ms;
    assert_eq!(
        links::candidates(home.path(), 0, Some(modified))
            .unwrap()
            .len(),
        1
    );
    assert!(
        links::candidates(home.path(), 0, Some(modified - 1))
            .unwrap()
            .is_empty()
    );
}

/// D-35: only a file that is not there reads as gone; a file the system
/// refuses to open is a failure that keeps its rows.
#[cfg(unix)]
#[test]
fn a_refused_file_is_unreadable_and_only_a_missing_one_is_gone() {
    use std::os::unix::fs::PermissionsExt;
    let (home, file) = home_with_claude();
    let request = |path: &std::path::Path| ReadRequest {
        agent: Agent::Claude,
        path: path.to_string_lossy().into_owned(),
        checkpoint: None,
    };
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let refused = links::read(home.path(), &[request(&file)]).remove(0);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(refused.error.as_deref(), Some("session_unreadable"));

    fs::remove_file(&file).unwrap();
    let gone = links::read(home.path(), &[request(&file)]).remove(0);
    assert_eq!(gone.error.as_deref(), Some("session_file_missing"));
}

/// The link graph reads each OpenCode session as its own row records it: the
/// root in its checkout, and a subagent's child session as a subagent line
/// of that root, never as part of the root's conversation.
#[test]
fn an_opencode_root_and_its_child_session_are_separate_link_lines() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join(".local/share/opencode");
    fs::create_dir_all(&folder).unwrap();
    rusqlite::Connection::open(folder.join("opencode.db"))
        .unwrap()
        .execute_batch(&fs::read_to_string(fixture("opencode-1.18.30/opencode.sql")).unwrap())
        .unwrap();
    let answers = links::read(
        home.path(),
        &[
            ReadRequest {
                agent: Agent::OpenCode,
                path: "opencode/ses_0a1b2c3d4e5f60718293a4b5c6".into(),
                checkpoint: None,
            },
            ReadRequest {
                agent: Agent::OpenCode,
                path: "opencode/ses_0a1b2c3d4e5f60718293child".into(),
                checkpoint: None,
            },
        ],
    );
    let [root, child] = answers.as_slice() else {
        panic!("two answers");
    };
    assert_eq!(root.error, None);
    assert_eq!(child.error, None);
    assert_eq!(root.facts.cwd.as_deref(), Some("/work/app"));
    assert!(!root.facts.subagent);
    assert!(child.facts.subagent);
    assert_eq!(
        child.facts.session_id.as_deref(),
        Some("ses_0a1b2c3d4e5f60718293a4b5c6"),
        "a child line belongs to its root session"
    );
}

/// Each format's fixture records one shell call, `bash scripts/ship.sh`,
/// whose output prints pull request 12, and the folder the record says the
/// call ran in.
const CALL_FIXTURES: [(Agent, &str, Option<&str>); 6] = [
    (Agent::Claude, CLAUDE, Some("/work/app")),
    (
        Agent::Codex,
        "codex-0.160.0/sessions/2026/10/03/rollout-2026-10-03T01-00-00-0199a000-0000-7000-8000-000000000002.jsonl",
        None,
    ),
    (Agent::Grok, "grok-1.0.46/updates.jsonl", None),
    (Agent::Pi, "pi-1.0.4/session.jsonl", None),
    (Agent::Omp, "omp-18.7.0/session.jsonl", None),
    (
        Agent::Antigravity,
        "antigravity-1.3.6/transcript_full.jsonl",
        Some("/work/app"),
    ),
];

fn named_twelve(folder: Option<&str>) -> Vec<hide_session::NamedPr> {
    vec![hide_session::NamedPr {
        repository: None,
        number: 12,
        folder: folder.map(str::to_owned),
    }]
}

fn named(facts: &links::LinkFacts) -> Vec<hide_session::NamedPr> {
    facts.named.iter().map(|mark| mark.pr.clone()).collect()
}

#[test]
fn a_pull_request_a_tool_printed_is_seen_and_one_its_call_named_is_named_in_every_format() {
    for (agent, path, folder) in CALL_FIXTURES {
        let contents = fs::read_to_string(fixture(path)).unwrap();
        let printed = parse_events(agent, &contents).links;
        assert!(
            printed.prs.iter().any(|pr| pr.number == 12),
            "{agent:?} sees the printed address"
        );
        assert_eq!(named(&printed), vec![], "{agent:?} names nothing");

        let viewed = contents.replace("bash scripts/ship.sh", "gh pr view 12");
        assert_ne!(viewed, contents, "{agent:?} records the call");
        let facts = parse_events(agent, &viewed).links;
        assert_eq!(named(&facts), named_twelve(folder), "{agent:?}");
        assert!(facts.named[0].at_unix_ms > 0);
    }
}

#[test]
fn an_opencode_call_that_names_a_pull_request_is_named_in_both_schemas() {
    for schema in ["opencode-1.18.30", "opencode-2.0.29"] {
        for (command, expected) in [
            ("bash scripts/ship.sh", vec![]),
            ("gh pr view 12", named_twelve(None)),
        ] {
            let home = tempfile::tempdir().unwrap();
            let folder = home.path().join(".local/share/opencode");
            fs::create_dir_all(&folder).unwrap();
            let sql = fs::read_to_string(fixture(&format!("{schema}/opencode.sql")))
                .unwrap()
                .replace("bash scripts/ship.sh", command);
            rusqlite::Connection::open(folder.join("opencode.db"))
                .unwrap()
                .execute_batch(&sql)
                .unwrap();
            let answer = links::read(
                home.path(),
                &[ReadRequest {
                    agent: Agent::OpenCode,
                    path: "opencode/ses_0a1b2c3d4e5f60718293a4b5c6".into(),
                    checkpoint: None,
                }],
            )
            .remove(0);
            assert_eq!(answer.error, None, "{schema}");
            assert!(
                answer.facts.prs.iter().any(|pr| pr.number == 12),
                "{schema} sees the printed address"
            );
            assert_eq!(named(&answer.facts), expected, "{schema}: {command}");
        }
    }
}
