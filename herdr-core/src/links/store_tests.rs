//! The record's rules against real session files read by the shared parser
//! (PRD link-graph D-27, D-35, D-44, B27, B28, B30, B32, B33, B37, B38).

use super::*;
use crate::links::{PrFact, ProjectFacts, WorktreeFact};
use hide_session::Agent;
use hide_session::links::{self, ReadRequest};
use std::fs;

const DEVICE: &str = "local";
const PROJECT: &str = "project:app";
const ROOT: &str = "/work/app";
const T0: u64 = 1_790_000_000_000;
const MIN: u64 = 60_000;

fn iso(ms: u64) -> String {
    jiff::Timestamp::from_millisecond(ms as i64)
        .unwrap()
        .to_string()
}

struct Turn<'a> {
    at: u64,
    branch: &'a str,
    text: &'a str,
}

/// A Claude session file: one human request per turn, then an optional PR
/// address printed by the agent's own `pr-link` record.
fn claude_file(
    home: &Path,
    id: &str,
    cwd: &str,
    entrypoint: &str,
    turns: &[Turn<'_>],
    pr: Option<(u64, u64)>,
) -> String {
    let dir = home.join(".claude/projects/-work-app");
    fs::create_dir_all(&dir).unwrap();
    let mut lines = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        lines.push(
            serde_json::json!({
                "type": "user", "isSidechain": false, "uuid": format!("{id}-u{index}"),
                "parentUuid": null, "message": {"role": "user", "content": turn.text},
                "timestamp": iso(turn.at), "promptId": "p", "origin": {"kind": "human"},
                "userType": "external", "entrypoint": entrypoint, "cwd": cwd,
                "sessionId": id, "gitBranch": turn.branch,
            })
            .to_string(),
        );
    }
    if let Some((number, at)) = pr {
        lines.push(
            serde_json::json!({
                "type": "pr-link", "sessionId": id, "prNumber": number,
                "prRepository": "Acme/App", "prUrl": format!("https://github.com/acme/app/pull/{number}"),
                "timestamp": iso(at),
            })
            .to_string(),
        );
    }
    let path = dir.join(format!("{id}.jsonl"));
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path.to_string_lossy().into_owned()
}

/// Reads a file to its end through the worker's own path.
fn ingest(store: &mut LinkStore, home: &Path, path: &str) {
    ingest_as(store, home, Agent::Claude, path);
}

fn ingest_as(store: &mut LinkStore, home: &Path, agent: Agent, path: &str) {
    loop {
        let checkpoint = store.checkpoint(DEVICE, path).unwrap();
        let answer = links::read(
            home,
            &[ReadRequest {
                agent,
                path: path.to_owned(),
                checkpoint,
            }],
        )
        .remove(0);
        assert_eq!(answer.error, None);
        store.apply_answer(DEVICE, &answer, "stamp").unwrap();
        if !answer.has_more {
            break;
        }
    }
}

fn pr(number: u64, branch: &str, created: u64, closed: Option<u64>) -> PrFact {
    PrFact {
        repository: "acme/app".into(),
        number,
        branch: branch.into(),
        title: format!("PR {number}"),
        url: format!("https://github.com/acme/app/pull/{number}"),
        created_at: Some(created),
        closed_at: closed,
        merged_at: closed,
        issues: Vec::new(),
        hide_issue_known: false,
    }
}

fn project(prs: Vec<PrFact>) -> ProjectFacts {
    ProjectFacts {
        key: PROJECT.into(),
        device_id: DEVICE.into(),
        workspace_id: "ws".into(),
        root: ROOT.into(),
        repository: Some("acme/app".into()),
        repository_id: Some("R_1".into()),
        connection_epoch: 0,
        worktrees: vec![WorktreeFact {
            path: ROOT.into(),
            branch: Some("main".into()),
            created_at_unix_ms: None,
        }],
        prs,
        prs_read: true,
        outputs: Vec::new(),
    }
}

fn open(dir: &Path) -> LinkStore {
    LinkStore::open(&dir.join("links.sqlite3")).unwrap().0
}

fn rows(store: &LinkStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn panel(store: &LinkStore, number: u64) -> Vec<LinkedSession> {
    store
        .pr_panel(PROJECT, number, Some(here()))
        .unwrap()
        .unwrap()
        .sessions
}

#[test]
fn the_session_that_printed_a_pull_request_when_github_made_it_is_its_creator() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0 + 10 * MIN, None)]), T0)
        .unwrap();
    let maker = claude_file(
        home.path(),
        "s-maker",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "PR 올려 줘",
        }],
        Some((7, T0 + 10 * MIN + 1_000)),
    );
    let worker = claude_file(
        home.path(),
        "s-worker",
        "/work/app/sub",
        "cli",
        &[Turn {
            at: T0 + 20 * MIN,
            branch: "feat/x",
            text: "리뷰 반영해 줘",
        }],
        None,
    );
    ingest(&mut store, home.path(), &maker);
    ingest(&mut store, home.path(), &worker);

    let lines = panel(&store, 7);
    let roles: Vec<_> = lines
        .iter()
        .map(|line| (line.id.as_str(), line.role, line.request.as_deref()))
        .collect();
    assert_eq!(
        roles,
        vec![
            ("s-worker", SessionRole::Worked, Some("리뷰 반영해 줘")),
            ("s-maker", SessionRole::Created, Some("PR 올려 줘")),
        ]
    );
    assert_eq!(lines[1].file, FileState::Present);

    // The file's state is what this machine's node answers: a removed file
    // is missing, and a node that cannot answer leaves it unknown.
    fs::remove_file(&maker).unwrap();
    assert_eq!(panel(&store, 7)[1].file, FileState::Missing);
    let silent = |_: &[String]| Err("node_not_connected".to_owned());
    let unanswered = store
        .pr_panel(
            PROJECT,
            7,
            Some(LocalFiles {
                device: DEVICE,
                present: &silent,
            }),
        )
        .unwrap()
        .unwrap();
    assert_eq!(unanswered.sessions[1].file, FileState::Unknown);
}

#[test]
fn reading_the_same_files_again_adds_no_rows() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s1",
        ROOT,
        "cli",
        &[
            Turn {
                at: T0,
                branch: "feat/x",
                text: "첫 요청",
            },
            Turn {
                at: T0 + MIN,
                branch: "feat/x",
                text: "둘째 요청",
            },
        ],
        Some((7, T0 + 500)),
    );
    ingest(&mut store, home.path(), &path);
    let counts = [
        "sessions",
        "session_branches",
        "session_prs",
        "prs",
        "pr_issues",
    ]
    .map(|table| rows(&store, table));

    // A restart that lost every cursor reads the whole file again (B37).
    store.connection.execute("DELETE FROM cursors", []).unwrap();
    ingest(&mut store, home.path(), &path);
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0 + MIN)
        .unwrap();

    assert_eq!(
        [
            "sessions",
            "session_branches",
            "session_prs",
            "prs",
            "pr_issues"
        ]
        .map(|table| rows(&store, table)),
        counts
    );
    assert_eq!(panel(&store, 7).len(), 1);
}

#[test]
fn appended_turns_extend_the_same_branch_span() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s1",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "첫 요청",
        }],
        None,
    );
    ingest(&mut store, home.path(), &path);
    let mut contents = fs::read_to_string(&path).unwrap();
    contents.push_str(
        &(serde_json::json!({
            "type": "user", "isSidechain": false, "uuid": "u-late", "parentUuid": "s1-u0",
            "message": {"role": "user", "content": "마지막 요청"}, "timestamp": iso(T0 + 5 * MIN),
            "promptId": "p", "origin": {"kind": "human"}, "userType": "external",
            "entrypoint": "cli", "cwd": ROOT, "sessionId": "s1", "gitBranch": "feat/x",
        })
        .to_string()
            + "\n"),
    );
    fs::write(&path, contents).unwrap();
    ingest(&mut store, home.path(), &path);

    assert_eq!(rows(&store, "session_branches"), 1);
    let lines = panel(&store, 7);
    assert_eq!(lines[0].request.as_deref(), Some("마지막 요청"));
    assert_eq!(lines[0].ended_at_unix_ms, Some(T0 + 5 * MIN));
}

#[test]
fn work_after_the_previous_pull_request_closed_belongs_to_the_next_one() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    // #7 lived T0..T0+60m on the branch; #9 opened on the same name later.
    store
        .apply_project(
            &project(vec![
                pr(7, "feat/x", T0, Some(T0 + 60 * MIN)),
                pr(9, "feat/x", T0 + 120 * MIN, None),
            ]),
            T0,
        )
        .unwrap();
    let early = claude_file(
        home.path(),
        "s-early",
        ROOT,
        "cli",
        &[Turn {
            at: T0 + 10 * MIN,
            branch: "feat/x",
            text: "앞 작업",
        }],
        None,
    );
    let late = claude_file(
        home.path(),
        "s-late",
        ROOT,
        "cli",
        &[Turn {
            at: T0 + 90 * MIN,
            branch: "feat/x",
            text: "뒤 작업",
        }],
        None,
    );
    ingest(&mut store, home.path(), &early);
    ingest(&mut store, home.path(), &late);

    let ids = |number| {
        panel(&store, number)
            .into_iter()
            .map(|line| line.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(7), vec!["s-early".to_owned()]);
    assert_eq!(ids(9), vec!["s-late".to_owned()]);
}

#[test]
fn a_print_run_and_a_session_outside_the_project_make_no_line() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let print = claude_file(
        home.path(),
        "s-print",
        ROOT,
        "sdk-cli",
        &[Turn {
            at: T0 + MIN,
            branch: "feat/x",
            text: "판정",
        }],
        Some((7, T0 + 500)),
    );
    let elsewhere = claude_file(
        home.path(),
        "s-else",
        "/work/other",
        "cli",
        &[Turn {
            at: T0 + MIN,
            branch: "feat/x",
            text: "다른 저장소",
        }],
        None,
    );
    ingest(&mut store, home.path(), &print);
    ingest(&mut store, home.path(), &elsewhere);

    assert!(panel(&store, 7).is_empty());
}

#[test]
fn a_printer_in_a_folder_holding_another_project_does_not_widen_this_one() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    store
        .apply_project(
            &ProjectFacts {
                key: "project:other".into(),
                root: "/work/other".into(),
                repository: Some("acme/other".into()),
                repository_id: Some("R_2".into()),
                connection_epoch: 0,
                worktrees: Vec::new(),
                ..project(Vec::new())
            },
            T0,
        )
        .unwrap();
    // An agent started in the folder above both projects prints the address.
    let printer = claude_file(
        home.path(),
        "s-above",
        "/work",
        "cli",
        &[Turn {
            at: T0 - MIN,
            branch: "feat/x",
            text: "PR 올려 줘",
        }],
        Some((7, T0 + 500)),
    );
    // The other project's session on a branch of the same name, while #7 lives.
    let other = claude_file(
        home.path(),
        "s-other",
        "/work/other",
        "cli",
        &[Turn {
            at: T0 + MIN,
            branch: "feat/x",
            text: "다른 프로젝트 일",
        }],
        None,
    );
    ingest(&mut store, home.path(), &printer);
    ingest(&mut store, home.path(), &other);

    let ids: Vec<String> = panel(&store, 7).into_iter().map(|line| line.id).collect();
    assert_eq!(ids, vec!["s-above".to_owned()]);
    assert!(
        store
            .session_links(PROJECT, "s-other", Some(here()))
            .unwrap()
            .is_none()
    );
    assert!(!crate::links::within("/work/app", "/"));
}

#[test]
fn a_rename_keeps_the_pull_requests_and_their_sessions() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s1",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "만들어 줘",
        }],
        Some((7, T0 + 500)),
    );
    ingest(&mut store, home.path(), &path);

    // GitHub now answers the same repository id under a new name.
    let mut renamed = project(Vec::new());
    renamed.repository = Some("acme/app-renamed".into());
    let mut moved = pr(7, "feat/x", T0, None);
    moved.repository = "acme/app-renamed".into();
    renamed.prs = vec![moved];
    store.apply_project(&renamed, T0 + MIN).unwrap();

    assert_eq!(rows(&store, "prs"), 1);
    let lines = panel(&store, 7);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].role, SessionRole::Created);
}

#[test]
fn a_pull_request_known_by_name_first_moves_to_its_id() {
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let mut nameless = project(vec![pr(7, "feat/x", T0, None)]);
    nameless.repository_id = None;
    store.apply_project(&nameless, T0).unwrap();
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0 + MIN)
        .unwrap();

    assert_eq!(rows(&store, "prs"), 1);
    let repo: String = store
        .connection
        .query_row("SELECT repo FROM prs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repo, "id:R_1");
}

#[test]
fn a_dropped_closing_reference_closes_the_link_and_a_failed_read_closes_nothing() {
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let mut linked = pr(7, "feat/x", T0, None);
    linked.issues = vec![("github:acme/app#3".into(), IssueSource::Closes)];
    store.apply_project(&project(vec![linked]), T0).unwrap();
    let issues = |store: &LinkStore| {
        let pr = store.pr(PROJECT, 7).unwrap().unwrap();
        store.pr_issues(&pr).unwrap()
    };
    assert_eq!(issues(&store).len(), 1);

    // GitHub could not be read: the record keeps what it had (D-35).
    let mut failed = project(Vec::new());
    failed.prs_read = false;
    store.apply_project(&failed, T0 + MIN).unwrap();
    assert_eq!(issues(&store).len(), 1);

    // GitHub answered the pull request without the reference (B27).
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0 + 2 * MIN)
        .unwrap();
    assert!(issues(&store).is_empty());
    let ended: Option<i64> = store
        .connection
        .query_row("SELECT end_at FROM pr_issues", [], |row| row.get(0))
        .unwrap();
    assert_eq!(ended, Some((T0 + 2 * MIN) as i64));
}

#[test]
fn a_hide_link_closes_only_while_its_branch_can_be_read() {
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let mut linked = pr(7, "feat/x", T0, None);
    linked.issues = vec![("local:/work/app#1".into(), IssueSource::Hide)];
    linked.hide_issue_known = true;
    store.apply_project(&project(vec![linked]), T0).unwrap();

    // The worktree and its branch were removed: the link is not unset.
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0 + MIN)
        .unwrap();
    let pr7 = store.pr(PROJECT, 7).unwrap().unwrap();
    assert_eq!(store.pr_issues(&pr7).unwrap().len(), 1);
}

#[test]
fn the_issue_panel_joins_its_pull_requests_and_names_which_one_a_session_made() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let mut first = pr(7, "feat/x", T0, None);
    first.issues = vec![("github:acme/app#3".into(), IssueSource::Closes)];
    let mut second = pr(8, "feat/y", T0 + 30 * MIN, None);
    second.issues = vec![("github:acme/app#3".into(), IssueSource::Closes)];
    store
        .apply_project(&project(vec![first, second]), T0)
        .unwrap();
    let maker = claude_file(
        home.path(),
        "s-maker",
        ROOT,
        "cli",
        &[Turn {
            at: T0 + 20 * MIN,
            branch: "feat/y",
            text: "두 번째 PR",
        }],
        Some((8, T0 + 30 * MIN)),
    );
    ingest(&mut store, home.path(), &maker);

    let links = store
        .issue_panel(PROJECT, "github:acme/app#3", Some(here()))
        .unwrap();
    assert_eq!(links.prs, vec![8, 7]);
    assert_eq!(links.sessions.len(), 1);
    assert_eq!(
        (links.sessions[0].pr, links.sessions[0].role),
        (8, SessionRole::Created)
    );
    let summary = store.summary(PROJECT, &[]).unwrap();
    assert_eq!(summary.prs.get(&8), Some(&1));
    assert_eq!(summary.issues.get("github:acme/app#3"), Some(&1));
    assert_eq!(
        summary.sessions.get("s-maker"),
        Some(&vec![SessionPrChip {
            number: 8,
            created: true
        }])
    );
}

/// A checkout's work landed when the pull requests its sessions made or
/// worked on include a merged one and no open one; a session belongs to the
/// deepest checkout holding its folder, and a checkout on no branch (one
/// left on a main commit after its branches merged) still has its sessions.
/// A pull request a session only printed is not work; a session counts only
/// if it started after its checkout was added, so a worktree made again at a
/// used path and branch inherits nothing even from a session still running,
/// and one whose age is unread has none; and a checkout on a branch weighs
/// only that branch's pull requests, so a branch switched in place is clean.
#[test]
fn a_checkout_landed_when_its_sessions_pull_requests_merged_and_none_is_open() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let merged = |number, branch| pr(number, branch, T0, Some(T0 + 60 * MIN));
    let open_pr = |number, branch| pr(number, branch, T0, None);
    let mut dropped = pr(15, "feat/dropped", T0, Some(T0 + 60 * MIN));
    dropped.merged_at = None;
    let mut facts = project(vec![
        merged(10, "feat/done"),
        merged(13, "feat/nested"),
        open_pr(14, "feat/root"),
        dropped,
        merged(16, "feat/mixed"),
        pr(17, "feat/mixed", T0 + 70 * MIN, None),
        merged(18, "feat/gone"),
        merged(19, "feat/old"),
        merged(20, "feat/a"),
    ]);
    facts.worktrees = [
        (ROOT, Some("main")),
        ("/work/app/nested", Some("feat/nested")),
        ("/work/app-done", Some("feat/done")),
        ("/work/app-mixed", Some("feat/mixed")),
        ("/work/app-dropped", Some("feat/dropped")),
        ("/work/app-detached", None),
        ("/work/app-untouched", Some("gen-prd/spec")),
        ("/work/app-looked", None),
        ("/work/app-reused", Some("feat/old")),
        ("/work/app-unread", Some("feat/done")),
        ("/work/app-switched", Some("feat/b")),
    ]
    .map(|(path, branch)| WorktreeFact {
        path: path.into(),
        branch: branch.map(Into::into),
        created_at_unix_ms: match path {
            ROOT | "/work/app-unread" => None,
            // Added again after its earlier sessions' pull request merged.
            "/work/app-reused" => Some(T0 + 100 * MIN),
            _ => Some(T0 - DAY_MS),
        },
    })
    .into();
    store.apply_project(&facts, T0).unwrap();
    let turn = |branch| Turn {
        at: T0 + MIN,
        branch,
        text: "작업해 줘",
    };
    let files = [
        claude_file(
            home.path(),
            "s-root",
            ROOT,
            "cli",
            &[turn("feat/root")],
            None,
        ),
        claude_file(
            home.path(),
            "s-nested",
            "/work/app/nested",
            "cli",
            &[turn("feat/nested")],
            None,
        ),
        claude_file(
            home.path(),
            "s-done",
            "/work/app-done",
            "cli",
            &[turn("feat/done")],
            None,
        ),
        claude_file(
            home.path(),
            "s-mixed",
            "/work/app-mixed",
            "cli",
            &[
                turn("feat/mixed"),
                Turn {
                    at: T0 + 90 * MIN,
                    branch: "feat/mixed",
                    text: "다음 것",
                },
            ],
            None,
        ),
        claude_file(
            home.path(),
            "s-dropped",
            "/work/app-dropped",
            "cli",
            &[turn("feat/dropped")],
            None,
        ),
        claude_file(
            home.path(),
            "s-detached",
            "/work/app-detached",
            "cli",
            &[turn("feat/gone")],
            Some((18, T0 + 1_000)),
        ),
        claude_file(
            home.path(),
            "s-untouched",
            "/work/app-untouched",
            "cli",
            &[turn("gen-prd/spec")],
            None,
        ),
        // Printed a merged pull request's address long after it was made.
        claude_file(
            home.path(),
            "s-looked",
            "/work/app-looked",
            "cli",
            &[turn("gen-prd/looked")],
            Some((10, T0 + 40 * MIN)),
        ),
        // Worked on a pull request's branch at this path before the worktree
        // there now was added, and still ran after it was.
        claude_file(
            home.path(),
            "s-reused",
            "/work/app-reused",
            "cli",
            &[
                turn("feat/old"),
                Turn {
                    at: T0 + 120 * MIN,
                    branch: "feat/old",
                    text: "계속",
                },
            ],
            None,
        ),
        // Made a pull request on the branch the worktree has since left.
        claude_file(
            home.path(),
            "s-switched",
            "/work/app-switched",
            "cli",
            &[turn("feat/a")],
            Some((20, T0 + 1_000)),
        ),
        claude_file(
            home.path(),
            "s-unread",
            "/work/app-unread",
            "cli",
            &[turn("feat/done")],
            Some((10, T0 + 1_000)),
        ),
    ];
    for file in &files {
        ingest(&mut store, home.path(), file);
    }

    let summary = store.summary(PROJECT, &facts.worktrees).unwrap();
    assert_eq!(
        summary.landed,
        BTreeSet::from(
            ["/work/app-detached", "/work/app-done", "/work/app/nested"].map(String::from)
        )
    );
    // The session that only printed #10's address is not its agent and its
    // folder is not one of the pull request's checkouts (PRD work-tab-pr-lens D-09).
    assert!(summary.pr_workers[&10].contains("s-done"));
    assert!(!summary.pr_workers[&10].contains("s-looked"));
    assert!(!summary.pr_checkouts[&10].contains("/work/app-looked"));
}

fn retention_case(days: Option<u16>, ended_days_ago: u64, still_open: bool) -> bool {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    let now = T0 + 800 * DAY_MS;
    let ended = now - ended_days_ago * DAY_MS;
    let closed = (!still_open).then_some(ended + DAY_MS);
    store
        .apply_project(&project(vec![pr(7, "feat/x", ended - MIN, closed)]), now)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s1",
        ROOT,
        "cli",
        &[Turn {
            at: ended,
            branch: "feat/x",
            text: "요청",
        }],
        None,
    );
    ingest(&mut store, home.path(), &path);
    store.prune(&|_| days, DEVICE, &mut present, now).unwrap();
    rows(&store, "sessions") == 1
}

/// The files of `paths` that are there, as the node answers.
fn present(paths: &[String]) -> Result<BTreeSet<String>, String> {
    Ok(paths
        .iter()
        .filter(|path| Path::new(path).is_file())
        .cloned()
        .collect())
}

#[test]
fn retention_follows_copied_history_with_the_open_pull_request_exception() {
    // Default 90 days.
    assert!(retention_case(None, 80, false));
    assert!(!retention_case(None, 100, false));
    // 30 days.
    assert!(!retention_case(Some(30), 40, false));
    // An open pull request keeps its sessions, up to a year after they end.
    assert!(retention_case(Some(30), 200, true));
    assert!(!retention_case(Some(30), 400, true));
    // Off keeps a session while its file is there, with no exception.
    assert!(retention_case(Some(0), 400, false));
}

#[test]
fn copied_history_off_removes_a_session_whose_file_is_gone() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s1",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "요청",
        }],
        None,
    );
    ingest(&mut store, home.path(), &path);
    assert_eq!(
        store
            .prune(
                &|_| Some(90),
                DEVICE,
                &mut |_| panic!("only Off asks about files"),
                T0 + MIN
            )
            .unwrap()
            .len(),
        0
    );
    assert_eq!(panel(&store, 7)[0].file, FileState::Present);
    fs::remove_file(&path).unwrap();
    assert_eq!(panel(&store, 7)[0].file, FileState::Missing);

    assert_eq!(
        store
            .prune(&|_| Some(0), DEVICE, &mut present, T0 + MIN)
            .unwrap()
            .len(),
        1
    );
    assert!(panel(&store, 7).is_empty());
}

#[test]
fn a_corrupt_file_is_set_aside_once_and_made_again() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    fs::write(&path, b"not a database at all, just bytes that fill a page").unwrap();

    let (store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, Opened::Rebuilt);
    assert_eq!(store.meta("backfill_done").unwrap(), None);
    assert!(corrupt_path(&path).is_file());
    drop(store);
    assert_eq!(LinkStore::open(&path).unwrap().1, Opened::Existing);
}

#[test]
fn a_newer_schema_is_refused_and_left_as_it_is() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    drop(LinkStore::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();

    assert_eq!(
        LinkStore::open(&path).err().as_deref(),
        Some("links_store_newer")
    );
    assert!(!corrupt_path(&path).exists());
}

/// The mode bits are a Unix fact; on Windows the private-file helper owns
/// the ACL and its own contract tests check it.
#[cfg(unix)]
#[test]
fn the_store_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    drop(LinkStore::open(&path).unwrap());
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
}

/// The old session search file keeps setting record retention (D-20): its
/// saved days are read, never written, and a file that is missing or holds
/// no `policy` table is the default for every project.
#[test]
fn copied_history_days_are_read_from_the_old_search_file_and_never_written() {
    let state = tempfile::tempdir().unwrap();
    let search = state.path().join("session search.sqlite3");
    let written = rusqlite::Connection::open(&search).unwrap();
    written
        .execute_batch("CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);")
        .unwrap();
    written
        .execute("INSERT INTO policy VALUES(?1, 30)", [PROJECT])
        .unwrap();
    drop(written);
    let before = fs::read(&search).unwrap();
    let store = open(state.path());

    let policies = store.policies(&search).unwrap();
    assert_eq!(policies.get(PROJECT), Some(&30));
    assert_eq!(fs::read(&search).unwrap(), before, "the file is only read");
    assert!(
        store
            .policies(&state.path().join("absent.sqlite3"))
            .unwrap()
            .is_empty()
    );
    let tableless = state.path().join("tableless.sqlite3");
    rusqlite::Connection::open(&tableless)
        .unwrap()
        .execute_batch("CREATE TABLE messages(id INTEGER PRIMARY KEY);")
        .unwrap();
    assert!(store.policies(&tableless).unwrap().is_empty());
}

/// A project whose id a conversion and then a move replaced keeps the days
/// the search file names under its first id, unless its newest id has a row
/// of its own.
#[test]
fn retention_follows_a_project_through_each_id_it_was_given() {
    let state = tempfile::tempdir().unwrap();
    let search = state.path().join("session-search.sqlite3");
    let written = rusqlite::Connection::open(&search).unwrap();
    written
        .execute_batch(
            "CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);\
             INSERT INTO policy VALUES('first', 30), ('kept', 7), ('own', 1);",
        )
        .unwrap();
    drop(written);
    let mut store = open(state.path());
    let pair = |old: &str, new: &str| (old.to_owned(), new.to_owned());
    assert_eq!(
        store
            .alias_retention(&[pair("first", "second"), pair("kept", "own")])
            .unwrap(),
        2
    );
    store.alias_retention(&[pair("second", "third")]).unwrap();
    let policies = store.policies(&search).unwrap();
    assert_eq!(policies.get("third"), Some(&30));
    assert_eq!(policies.get("own"), Some(&1), "its own row wins");
}

#[test]
fn a_session_that_continues_another_joins_its_line() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    let first = claude_file(
        home.path(),
        "s-a",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "처음",
        }],
        None,
    );
    let dir = home.path().join(".claude/projects/-work-app");
    let second = dir.join("s-b.jsonl");
    fs::write(
        &second,
        serde_json::json!({
            "type": "user", "isSidechain": false, "uuid": "b-0", "parentUuid": "s-a-u0",
            "message": {"role": "user", "content": "이어서"}, "timestamp": iso(T0 + 10 * MIN),
            "promptId": "p", "origin": {"kind": "human"}, "userType": "external",
            "entrypoint": "cli", "cwd": ROOT, "sessionId": "s-b", "gitBranch": "feat/x",
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    ingest(&mut store, home.path(), &first);
    ingest(&mut store, home.path(), &second.to_string_lossy());

    let lines = panel(&store, 7);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].ids, vec!["s-a".to_owned(), "s-b".to_owned()]);
    assert_eq!(lines[0].id, "s-b");
    assert_eq!(lines[0].started_at_unix_ms, Some(T0));
}

fn codex_line(kind: &str, at: u64, payload: serde_json::Value) -> String {
    serde_json::json!({"timestamp": iso(at), "type": kind, "payload": payload}).to_string() + "\n"
}

/// A Codex file names its session once, in its first line: what is appended
/// later still belongs to that session, not to one without a name.
#[test]
fn what_a_codex_session_appends_later_stays_on_its_line() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(
            &project(vec![pr(7, "feat/codex", T0 + 4 * MIN - 500, None)]),
            T0,
        )
        .unwrap();
    let dir = home.path().join(".codex/sessions/2026/10/03");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-cx-1.jsonl");
    let user = |text: &str| {
        serde_json::json!({"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": text}]})
    };
    let mut contents = codex_line(
        "session_meta",
        T0 + MIN,
        serde_json::json!({"id": "cx-1", "cwd": ROOT, "originator": "codex-tui",
            "source": "cli", "git": {"branch": "feat/codex"}}),
    ) + &codex_line("response_item", T0 + 2 * MIN, user("PR 올려 줘"));
    fs::write(&path, &contents).unwrap();
    let path = path.to_string_lossy().into_owned();
    ingest_as(&mut store, home.path(), Agent::Codex, &path);

    contents += &codex_line("response_item", T0 + 3 * MIN, user("CI도 봐 줘"));
    contents += &codex_line(
        "response_item",
        T0 + 4 * MIN,
        serde_json::json!({"type": "function_call_output", "call_id": "c",
            "output": "https://github.com/acme/app/pull/7"}),
    );
    fs::write(&path, &contents).unwrap();
    ingest_as(&mut store, home.path(), Agent::Codex, &path);

    assert_eq!(rows(&store, "sessions"), 1);
    let lines = panel(&store, 7);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (lines[0].id.as_str(), lines[0].role),
        ("cx-1", SessionRole::Created)
    );
    assert_eq!(lines[0].ended_at_unix_ms, Some(T0 + 4 * MIN));
}

/// A long file is read in pieces: an address printed in a later piece keeps
/// the request an earlier piece held (found reading a 2.8 MB session live).
#[test]
fn an_address_read_after_its_request_keeps_that_request() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0 + 10 * MIN, None)]), T0)
        .unwrap();
    let path = claude_file(
        home.path(),
        "s-maker",
        ROOT,
        "cli",
        &[Turn {
            at: T0,
            branch: "feat/x",
            text: "PR 올려 줘",
        }],
        None,
    );
    ingest(&mut store, home.path(), &path);
    let mut contents = fs::read_to_string(&path).unwrap();
    contents.push_str(
        &(serde_json::json!({
            "type": "pr-link", "sessionId": "s-maker", "prNumber": 7,
            "prRepository": "acme/app", "prUrl": "https://github.com/acme/app/pull/7",
            "timestamp": iso(T0 + 10 * MIN + 1_000),
        })
        .to_string()
            + "\n"),
    );
    fs::write(&path, contents).unwrap();
    ingest(&mut store, home.path(), &path);

    let lines = panel(&store, 7);
    assert_eq!(
        (lines[0].role, lines[0].request.as_deref()),
        (SessionRole::Created, Some("PR 올려 줘"))
    );
}

/// This machine as the panel reads see it: `DEVICE`, whose node is this
/// process.
fn here() -> LocalFiles<'static> {
    fn present(asked: &[String]) -> Result<std::collections::BTreeSet<String>, String> {
        crate::links::files_present(&hide_node::Local::of_process(), asked)
    }
    LocalFiles {
        device: DEVICE,
        present: &present,
    }
}

fn one_turn(home: &Path, id: &str, at: u64) -> String {
    claude_file(
        home,
        id,
        ROOT,
        "cli",
        &[Turn {
            at,
            branch: "feat/x",
            text: "요청",
        }],
        None,
    )
}

/// Deleting archived sessions removes Hide's lines of exactly those sessions
/// and leaves the agent's own conversation file (PRD session-archive D-15).
#[test]
fn deleting_named_sessions_removes_only_their_lines_and_keeps_the_files() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store.apply_project(&project(Vec::new()), T0).unwrap();
    let kept = one_turn(home.path(), "s-kept", T0);
    let gone = one_turn(home.path(), "s-gone", T0);
    ingest(&mut store, home.path(), &kept);
    ingest(&mut store, home.path(), &gone);
    let named = store
        .all_sessions()
        .unwrap()
        .into_iter()
        .find(|session| session.key.id == "s-gone")
        .unwrap()
        .key
        .identity();
    store.delete_sessions(&[named]).unwrap();
    let left = store
        .all_sessions()
        .unwrap()
        .into_iter()
        .map(|session| session.key.id)
        .collect::<Vec<_>>();
    assert_eq!(left, ["s-kept"]);
    assert!(Path::new(&gone).is_file(), "the conversation file stays");
}

/// An archived session the record holds no line for leaves once its
/// project's Copied history has run out since it was archived; Off keeps it
/// (D-14, B31).
#[test]
fn an_archived_session_without_a_line_expires_by_its_projects_history_from_its_archive_time() {
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store.apply_project(&project(Vec::new()), T0).unwrap();
    let held = |id: &str, at: u64| ArchivedSession {
        session: SessionIdentity {
            device_id: DEVICE.into(),
            agent: "claude".into(),
            session_id: id.into(),
        },
        project_root: ROOT.into(),
        archived_at_unix_ms: at,
    };
    let now = T0 + 100 * DAY_MS;
    let archived = [held("old", T0), held("new", now - DAY_MS)];
    let expired = store
        .expired_archived(&archived, &|_| Some(30), now)
        .unwrap();
    assert_eq!(
        expired
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        ["old"]
    );
    assert!(
        store
            .expired_archived(&archived, &|_| Some(0), now)
            .unwrap()
            .is_empty()
    );
}
