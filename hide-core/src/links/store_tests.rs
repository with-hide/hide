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

/// A publish pass reads a project's places, fences and repository names once
/// and answers from them. The expected answers are written out from the
/// fixture below, not read back from another path, so a pass that drops or
/// adds a line fails here.
#[test]
fn a_pass_over_a_project_answers_what_the_fixture_says() {
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

    let pass = store.pass(PROJECT).unwrap().unwrap();

    let summary = store
        .summary_in(&pass, &[], &mut crate::links::pr_cache::PrCache::default())
        .unwrap();
    assert_eq!(summary.prs.len(), 1);
    assert_eq!(summary.prs.get(&8), Some(&1));
    assert_eq!(summary.issues.get("github:acme/app#3"), Some(&1));
    assert_eq!(
        summary.sessions.get("s-maker"),
        Some(&vec![SessionPrChip {
            number: 8,
            created: true
        }])
    );

    let eight = store.pr(PROJECT, 8).unwrap().unwrap();
    let lines = store.pr_lines_in(&pass, &eight, Some(here())).unwrap();
    assert_eq!(
        lines
            .iter()
            .map(|line| (line.id.as_str(), line.pr, line.role))
            .collect::<Vec<_>>(),
        vec![("s-maker", 8, SessionRole::Created)]
    );
    let seven = store.pr(PROJECT, 7).unwrap().unwrap();
    assert!(
        store
            .pr_lines_in(&pass, &seven, Some(here()))
            .unwrap()
            .is_empty()
    );

    // No ledger session names the issue, so the ledger's issue list is empty;
    // the one session that made the pull request has one work summary.
    assert!(store.ledger_issue_refs_in(&pass).unwrap().is_empty());
    let work = store.ledger_summaries_in(&pass).unwrap();
    assert_eq!(
        work.iter()
            .map(|item| item.session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["s-maker"]
    );
}

/// A learned place inside another project's root belongs to that project: a
/// publish pass fences it out of this project's paths, the same as a read of
/// the project alone does, and the expected paths are written out here.
#[test]
fn a_pass_fences_out_a_learned_place_inside_another_projects_root() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(8, "feat/y", T0, None)]), T0)
        .unwrap();
    let nested = claude_file(
        home.path(),
        "s-nested",
        "/work/app/sub/tree",
        "cli",
        &[Turn {
            at: T0 + 20 * MIN,
            branch: "feat/y",
            text: "두 번째 PR",
        }],
        Some((8, T0 + 1_000)),
    );
    ingest(&mut store, home.path(), &nested);
    store
        .apply_project(
            &ProjectFacts {
                key: "project:sub".into(),
                device_id: DEVICE.into(),
                workspace_id: "workspace-sub".into(),
                root: "/work/app/sub".into(),
                repository: None,
                repository_id: None,
                connection_epoch: 0,
                worktrees: Vec::new(),
                prs: Vec::new(),
                prs_read: false,
                outputs: Vec::new(),
            },
            T0,
        )
        .unwrap();

    let own = store.pass(PROJECT).unwrap().unwrap();
    assert_eq!(own.paths, vec![(DEVICE.to_owned(), ROOT.to_owned())]);
    let sub = store.pass("project:sub").unwrap().unwrap();
    assert_eq!(
        sub.paths,
        vec![(DEVICE.to_owned(), "/work/app/sub".to_owned())]
    );
}

/// A learned place is the only way this session reaches the project: its folder
/// is outside the project's root and no other project holds it. The pass must
/// keep that place, so the session's work stays in the project's ledger list.
#[test]
fn a_pass_keeps_a_learned_place_that_only_a_pull_request_reaches() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(8, "feat/y", T0, None)]), T0)
        .unwrap();
    let far = claude_file(
        home.path(),
        "s-far",
        "/work/far",
        "cli",
        &[Turn {
            at: T0 + 10_000,
            branch: "feat/y",
            text: "먼 폴더의 작업",
        }],
        Some((8, T0 + 10_000)),
    );
    ingest(&mut store, home.path(), &far);

    let pass = store.pass(PROJECT).unwrap().unwrap();
    assert!(
        pass.paths
            .contains(&(DEVICE.to_owned(), "/work/far".to_owned())),
        "the far folder is a learned place of the project"
    );
    let work = store.ledger_summaries_in(&pass).unwrap();
    assert_eq!(
        work.iter()
            .map(|item| item.session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["s-far"]
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
    let identity = |id: &str| SessionIdentity {
        device_id: DEVICE.into(),
        agent: "claude".into(),
        session_id: id.into(),
    };
    let asked = [identity("s1"), identity("never-read")];
    assert_eq!(
        store.held(&asked).unwrap(),
        BTreeSet::from([identity("s1")])
    );
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
    assert!(
        store.held(&asked).unwrap().is_empty(),
        "a pruned session's record can no longer be opened"
    );
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

/// Whether the store's file holds the table `name`.
fn has_table(store: &LinkStore, name: &str) -> bool {
    store
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

/// Version 15 stops reading the passive direct-input facts the hooks' direct
/// observation kept, and an upgrade leaves their table and rows as they were.
#[test]
fn an_upgrade_keeps_the_retired_direct_inputs() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    let (store, _) = LinkStore::open(&path).unwrap();
    assert!(!has_table(&store, "direct_observations"));
    store
        .connection
        .execute_batch(
            "CREATE TABLE direct_observations(id TEXT PRIMARY KEY, at INTEGER NOT NULL,
                bytes INTEGER NOT NULL, fact TEXT NOT NULL);
            INSERT INTO direct_observations VALUES('one',1,2,'{}');
            PRAGMA user_version=14;",
        )
        .unwrap();
    drop(store);
    let (store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, Opened::Existing);
    let kept: (String, i64, i64, String) = store
        .connection
        .query_row(
            "SELECT id, at, bytes, fact FROM direct_observations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(kept, ("one".to_owned(), 1, 2, "{}".to_owned()));
}

#[test]
fn a_version_4_store_migrates_without_recreating_retention_aliases() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    let (store, _) = LinkStore::open(&path).unwrap();
    store
        .connection
        .execute_batch(ledger::UNDO_CLOSED_FLOW_SCHEMA)
        .unwrap();
    store
        .connection
        .execute_batch(ledger::UNDO_FLOW_SCHEMA)
        .unwrap();
    store.connection.execute_batch("ALTER TABLE ledger_titles DROP COLUMN goal;
        ALTER TABLE ledger_titles DROP COLUMN summary;
        ALTER TABLE ledger_titles DROP COLUMN label_at;
        ALTER TABLE ledger_titles DROP COLUMN analysed_start;
        ALTER TABLE ledger_titles DROP COLUMN analysed_end;
        PRAGMA user_version=4;
        INSERT INTO retention_aliases VALUES('old-project','new-project');
        INSERT INTO ledger_backlog VALUES('backlog-1','local','old-project','fixture-workspace','fixture/repo','Preserved backlog',1,'creating',NULL,NULL);").unwrap();
    drop(store);
    let (store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, Opened::Existing);
    assert!(!has_table(&store, "direct_observations"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT new FROM retention_aliases WHERE old='old-project'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "new-project"
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT title FROM ledger_backlog WHERE intent='backlog-1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Preserved backlog"
    );
}

/// A Claude session on `main` in `cwd` that asks once and makes one shell
/// call; the call's output prints pull request `printed`'s address when
/// given. The request is at `at`, the call a second later and its output a
/// second after that.
fn claude_call(
    home: &Path,
    id: &str,
    cwd: &str,
    at: u64,
    request: &str,
    command: &str,
    printed: Option<u64>,
) -> String {
    let dir = home.join(".claude/projects/-work-app");
    fs::create_dir_all(&dir).unwrap();
    let common = |kind: &str, uuid: &str, at: u64, message: serde_json::Value| {
        serde_json::json!({
            "type": kind, "isSidechain": false, "uuid": uuid, "parentUuid": null,
            "message": message, "timestamp": iso(at), "userType": "external",
            "entrypoint": "cli", "cwd": cwd, "sessionId": id, "gitBranch": "main",
        })
    };
    let mut ask = common(
        "user",
        &format!("{id}-u"),
        at,
        serde_json::json!({"role": "user", "content": request}),
    );
    ask["promptId"] = "p".into();
    ask["origin"] = serde_json::json!({"kind": "human"});
    let mut lines = vec![
        ask.to_string(),
        common(
            "assistant",
            &format!("{id}-a"),
            at + 1_000,
            serde_json::json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": command},
            }]}),
        )
        .to_string(),
    ];
    if let Some(number) = printed {
        lines.push(
            common(
                "user",
                &format!("{id}-r"),
                at + 2_000,
                serde_json::json!({"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "t1",
                    "content": format!("https://github.com/acme/app/pull/{number}"),
                }]}),
            )
            .to_string(),
        );
    }
    let path = dir.join(format!("{id}.jsonl"));
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path.to_string_lossy().into_owned()
}

fn roles(lines: &[LinkedSession]) -> Vec<(&str, SessionRole, Option<&str>)> {
    lines
        .iter()
        .map(|line| (line.id.as_str(), line.role, line.request.as_deref()))
        .collect()
}

#[test]
fn an_address_a_tool_only_printed_links_nothing_and_a_call_that_named_the_pull_request_worked_on_it()
 {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
        .unwrap();
    // Made it: its `gh pr create` printed the address as GitHub made it.
    let maker = claude_call(
        home.path(),
        "s-maker",
        ROOT,
        T0 - 1_000,
        "PR 올려 줘",
        "gh pr create --fill",
        Some(7),
    );
    // Listed the browser's tabs, which held the pull request's page.
    let tabs = claude_call(
        home.path(),
        "s-tabs",
        ROOT,
        T0 + 20 * MIN,
        "탭 정리해 줘",
        "aside tabs",
        Some(7),
    );
    // Reviewed it.
    let review = claude_call(
        home.path(),
        "s-review",
        ROOT,
        T0 + 30 * MIN,
        "7번 리뷰해 줘",
        "gh pr view 7 --comments",
        Some(7),
    );
    for path in [&maker, &tabs, &review] {
        ingest(&mut store, home.path(), path);
    }

    assert_eq!(
        roles(&panel(&store, 7)),
        vec![
            ("s-review", SessionRole::Worked, Some("7번 리뷰해 줘")),
            ("s-maker", SessionRole::Created, Some("PR 올려 줘")),
        ]
    );
    let summary = store.summary(PROJECT, &[]).unwrap();
    assert_eq!(summary.prs.get(&7), Some(&2));
    assert!(
        store
            .session_links(PROJECT, "s-tabs", Some(here()))
            .unwrap()
            .is_some_and(|(_, prs)| prs.is_empty()),
        "the tab lister works in the project and on none of its pull requests"
    );
}

#[test]
fn a_number_named_alone_belongs_to_the_project_whose_folder_the_call_ran_in() {
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
                worktrees: Vec::new(),
                ..project(vec![PrFact {
                    repository: "acme/other".into(),
                    url: "https://github.com/acme/other/pull/7".into(),
                    ..pr(7, "feat/y", T0, None)
                }])
            },
            T0,
        )
        .unwrap();
    let calls = [
        ("s-here", ROOT, "gh pr view 7"),
        ("s-there", ROOT, "cd /work/other && gh pr diff 7"),
        ("s-sub", "/work/other", "cd ../app/sub && gh pr checks 7"),
        ("s-repo", "/work/other", "gh pr view 7 -R acme/app"),
        ("s-lost", ROOT, "cd ~/elsewhere && gh pr view 7"),
    ];
    for (id, cwd, command) in calls {
        let path = claude_call(home.path(), id, cwd, T0 + 10 * MIN, "봐 줘", command, None);
        ingest(&mut store, home.path(), &path);
    }

    let mut app: Vec<String> = panel(&store, 7).into_iter().map(|line| line.id).collect();
    app.sort();
    assert_eq!(app, vec!["s-here", "s-repo", "s-sub"]);
    let mut other: Vec<String> = store
        .pr_panel("project:other", 7, Some(here()))
        .unwrap()
        .unwrap()
        .sessions
        .into_iter()
        .map(|line| line.id)
        .collect();
    other.sort();
    assert_eq!(other, vec!["s-there"]);
}

#[test]
fn a_record_from_before_named_pull_requests_reads_its_files_again_and_finds_them() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    let review = claude_call(
        home.path(),
        "s-review",
        ROOT,
        T0 + 30 * MIN,
        "7번 리뷰해 줘",
        "gh pr view 7",
        Some(7),
    );
    {
        let (mut store, _) = LinkStore::open(&path).unwrap();
        store
            .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
            .unwrap();
        ingest(&mut store, home.path(), &review);
        // The record as version 9 left it: read to its end, with no named
        // pull requests kept.
        store
            .connection
            .execute_batch(
                "DROP TABLE session_named_prs;
                 INSERT INTO meta VALUES('backfill_done','1'),('listed_at','5'),('listed_at:mini','6');
                 PRAGMA user_version=9;",
            )
            .unwrap();
    }
    let (mut store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, Opened::Existing);
    assert!(panel(&store, 7).is_empty());
    assert_eq!(
        store
            .stamps(DEVICE)
            .unwrap()
            .get(&review)
            .map(String::as_str),
        Some("")
    );
    assert_eq!(store.checkpoint(DEVICE, &review).unwrap(), None);
    assert_eq!(store.meta("listed_at").unwrap(), None);
    assert_eq!(store.meta("listed_at:mini").unwrap(), None);
    assert_eq!(store.meta("backfill_done").unwrap().as_deref(), Some("1"));

    ingest(&mut store, home.path(), &review);
    assert_eq!(
        roles(&panel(&store, 7)),
        vec![("s-review", SessionRole::Worked, Some("7번 리뷰해 줘"))]
    );
}

/// The review of #1859: mission-13's #1810 to #1813 were each made by one
/// call that wrote the body and ran `gh pr create ... | tail -1`, which
/// printed the address 2.6 s to 3.2 s before GitHub's `createdAt`, on another
/// clock, outside the -2 s edge. The address its successful creating call
/// gave back is its making within 10 s before; a listing printed as early
/// stays a look, and so does a made print 12 s early.
#[test]
fn a_session_whose_creating_call_printed_the_address_made_it_within_ten_seconds() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(
            &project(vec![pr(7, "feat/x", T0, None), pr(8, "feat/y", T0, None)]),
            T0,
        )
        .unwrap();
    // `claude_call` prints the address 2 s after the request.
    let made = claude_call(
        home.path(),
        "s-make",
        ROOT,
        T0 - 5_200,
        "PR 올려줘",
        "cd /work/app && B=/tmp/s/pr-body.md; cat > $B <<'EOF'\n## Summary\nbody\nEOF\ngh pr create --repo acme/app --base main --head feat/x --title \"fix: x\" --body-file $B 2>&1 | tail -1",
        Some(7),
    );
    let listed = claude_call(
        home.path(),
        "s-list",
        ROOT,
        T0 - 5_000,
        "열린 PR 보여줘",
        "gh pr list",
        Some(7),
    );
    let early = claude_call(
        home.path(),
        "s-early",
        ROOT,
        T0 - 14_000,
        "PR 올려줘",
        "gh pr create --fill",
        Some(8),
    );
    for file in [&made, &listed, &early] {
        ingest(&mut store, home.path(), file);
    }
    assert_eq!(
        roles(&panel(&store, 7)),
        vec![("s-make", SessionRole::Created, Some("PR 올려줘"))]
    );
    assert!(panel(&store, 8).is_empty(), "12 s before is no making");
}

/// The review of #1859: one call ran `gh pr create --fill || gh pr view 42`,
/// the create failed and the view printed #42, which another session made a
/// day before. Neither that call nor one joined by `&&` makes #42 its own:
/// the first claims nothing, and GitHub's time keeps the second out. Both
/// still name #42, so each worked on it.
#[test]
fn a_creating_call_that_also_prints_an_old_pull_request_does_not_make_it() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut store = open(state.path());
    store
        .apply_project(&project(vec![pr(42, "feat/old", T0 - DAY_MS, None)]), T0)
        .unwrap();
    let fallback = claude_call(
        home.path(),
        "s-fallback",
        ROOT,
        T0 - 2_000,
        "PR 올려줘",
        "gh pr create --fill || gh pr view 42 --json url --jq .url",
        Some(42),
    );
    let joined = claude_call(
        home.path(),
        "s-joined",
        ROOT,
        T0 - 2_000,
        "PR 올리고 42 보여줘",
        "gh pr create --fill && gh pr view 42 --json url --jq .url",
        Some(42),
    );
    ingest(&mut store, home.path(), &fallback);
    ingest(&mut store, home.path(), &joined);
    let linked = panel(&store, 42);
    let mut lines = roles(&linked);
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("s-fallback", SessionRole::Worked, Some("PR 올려줘")),
            ("s-joined", SessionRole::Worked, Some("PR 올리고 42 보여줘")),
        ]
    );
}

/// `made` (version 12): a record from before it gains the column and reads
/// every file again, so a pull request made before the upgrade is found.
#[test]
fn a_record_from_before_made_pull_requests_reads_its_files_again() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    let made = claude_call(
        home.path(),
        "s-make",
        ROOT,
        T0 - 5_400,
        "PR 올려줘",
        "gh pr create --fill",
        Some(7),
    );
    {
        let (mut store, _) = LinkStore::open(&path).unwrap();
        store
            .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
            .unwrap();
        ingest(&mut store, home.path(), &made);
        store
            .connection
            .execute_batch("ALTER TABLE session_prs DROP COLUMN made; PRAGMA user_version=11;")
            .unwrap();
    }
    let (mut store, _) = LinkStore::open(&path).unwrap();
    assert!(panel(&store, 7).is_empty(), "nothing said it was made");
    assert_eq!(store.checkpoint(DEVICE, &made).unwrap(), None);
    ingest(&mut store, home.path(), &made);
    assert_eq!(
        roles(&panel(&store, 7)),
        vec![("s-make", SessionRole::Created, Some("PR 올려줘"))]
    );
}

/// `pane_first_at` (version 11): an upgrade marks every session the record
/// already held, so only a session no pane carried before is first, once.
#[test]
fn a_session_is_first_carried_once_and_an_upgrade_marks_every_held_session() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (store, _) = LinkStore::open(&path).unwrap();
    store
        .connection
        .execute_batch(
            "ALTER TABLE sessions DROP COLUMN pane_first_at; PRAGMA user_version=10;
             INSERT INTO sessions(device,agent,id,started_at) VALUES('local','claude','held',1000);",
        )
        .unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let pane = |id: &str| PaneFact {
        device_id: DEVICE.into(),
        agent: "claude".into(),
        session_id: id.into(),
        cwd: ROOT.into(),
        branch: None,
    };
    let first = store.apply_panes(&[pane("held"), pane("new")], T0).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|s| s.session_id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    assert!(
        store
            .apply_panes(&[pane("held"), pane("new")], T0 + MIN)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_capacity_crossing_is_reported_once_until_the_count_falls_back_under() {
    let mut crossings = CapacityCrossings::default();
    assert!(
        crossings.enter(PROJECT),
        "the first call over the cap reports"
    );
    assert!(
        !crossings.enter(PROJECT),
        "a call while still over stays quiet"
    );
    assert!(
        crossings.enter("project:other"),
        "each key crosses on its own"
    );
    crossings.leave(PROJECT);
    assert!(
        crossings.enter(PROJECT),
        "a key that fell back under reports again"
    );
}

/// A pre-check's session reads as the pull request's agent review, over its
/// work on the branch, and keeps that role in a store reopened after the
/// upgrade that added it (PRD pr-review-status B14, D-17).
#[test]
fn a_pre_check_session_reads_as_reviewed_and_keeps_it_across_a_reopen_and_an_upgrade() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("links.sqlite3");
    {
        let mut store = open(state.path());
        store
            .apply_project(&project(vec![pr(7, "feat/x", T0, None)]), T0)
            .unwrap();
        let check = claude_call(
            home.path(),
            "s-check",
            ROOT,
            T0 + 30 * MIN,
            "7번 미리 점검해 줘",
            "gh pr view 7 --comments",
            Some(7),
        );
        ingest(&mut store, home.path(), &check);
        assert_eq!(
            roles(&panel(&store, 7)),
            vec![("s-check", SessionRole::Worked, Some("7번 미리 점검해 줘"))]
        );
        // A store from before version 13 had no table for it.
        store
            .connection
            .execute_batch("DROP TABLE session_reviews; PRAGMA user_version=12;")
            .unwrap();
    }
    let mut store = open(state.path());
    let review = ReviewFact {
        device_id: DEVICE.into(),
        agent: "claude".into(),
        session_id: "s-check".into(),
        repo_name: "Acme/App".into(),
        number: 7,
    };
    store
        .apply_reviews(std::slice::from_ref(&review), T0 + 31 * MIN)
        .unwrap();
    store.apply_reviews(&[review], T0 + 40 * MIN).unwrap();
    assert_eq!(rows(&store, "session_reviews"), 1);
    drop(store);

    let store = LinkStore::open(&path).unwrap().0;
    assert_eq!(
        roles(&panel(&store, 7)),
        vec![("s-check", SessionRole::Reviewed, Some("7번 미리 점검해 줘"))]
    );
}

/// Captures the last summaries a pass published.
#[derive(Default)]
struct Published(std::cell::RefCell<BTreeMap<String, crate::links::ProjectLinkSummary>>);

impl crate::links::worker::Sink for Published {
    fn summaries(&self, summaries: BTreeMap<String, crate::links::ProjectLinkSummary>) {
        *self.0.borrow_mut() = summaries;
    }
    fn panel(&self, _: u64, _: Result<crate::links::worker::PanelAnswer, String>) {}
    fn filling(&self, _: bool) {}
    fn devices(&self) -> Vec<(String, std::sync::Arc<dyn crate::node_access::NodeLink>)> {
        Vec::new()
    }
}

/// Three projects for the scoped summary pass: an app with a second
/// worktree and two pull requests, a library with one, and a folder with no
/// repository.
fn scoped_projects(merged: Option<u64>) -> Vec<ProjectFacts> {
    let mut app = project(vec![
        pr(1, "feat", T0 + 10 * MIN, merged),
        pr(2, "fix", T0 + 30 * MIN, None),
    ]);
    app.worktrees.push(WorktreeFact {
        path: "/work/app-feat".into(),
        branch: Some("feat".into()),
        created_at_unix_ms: Some(T0),
    });
    let lib = ProjectFacts {
        key: "project:lib".into(),
        workspace_id: "ws-lib".into(),
        root: "/work/lib".into(),
        repository: Some("acme/lib".into()),
        repository_id: Some("R_2".into()),
        worktrees: vec![WorktreeFact {
            path: "/work/lib".into(),
            branch: Some("main".into()),
            created_at_unix_ms: None,
        }],
        prs: vec![PrFact {
            repository: "acme/lib".into(),
            url: "https://github.com/acme/lib/pull/5".into(),
            ..pr(5, "lib-x", T0 + 20 * MIN, None)
        }],
        ..project(Vec::new())
    };
    let docs = ProjectFacts {
        key: "project:docs".into(),
        workspace_id: "ws-docs".into(),
        root: "/work/docs".into(),
        repository: None,
        repository_id: None,
        worktrees: Vec::new(),
        ..project(Vec::new())
    };
    vec![app, lib, docs]
}

/// One chunk of a session's file, as a read answers it.
fn chunk(id: &str, cwd: &str, at: u64) -> links::LinkFacts {
    links::LinkFacts {
        session_id: Some(id.into()),
        cwd: Some(cwd.into()),
        interactive: Some(true),
        first_at_unix_ms: Some(at),
        last_at_unix_ms: Some(at + MIN),
        last_request: Some((at, format!("{id} 요청"))),
        ..Default::default()
    }
}

fn span(branch: &str, at: u64) -> links::BranchSpan {
    links::BranchSpan {
        branch: Some(branch.into()),
        inherited: false,
        first_at_unix_ms: at,
        last_at_unix_ms: at + MIN,
        last_request: None,
    }
}

fn mark(repository: &str, number: u64, at: u64, made: bool) -> links::PrMark {
    links::PrMark {
        repository: repository.into(),
        number,
        at_unix_ms: at,
        request: None,
        made,
    }
}

fn session(id: &str) -> SessionIdentity {
    SessionIdentity {
        device_id: DEVICE.into(),
        agent: "claude".into(),
        session_id: id.into(),
    }
}

/// One read of a session file, written as the worker writes it and marked
/// for the next summary pass.
fn read_facts(
    store: &mut LinkStore,
    scoped: &mut crate::links::worker::State,
    path: &str,
    facts: links::LinkFacts,
) {
    let answer = links::ReadAnswer {
        agent: Agent::Claude,
        path: path.into(),
        checkpoint: None,
        has_more: false,
        rescanned: false,
        facts,
        error: None,
    };
    store.apply_answer(DEVICE, &answer, "stamp").unwrap();
    scoped.read_written(store, DEVICE, path);
}

/// A summary pass through `scoped` and one through a state that never
/// published, which reads everything, asserting they publish the same;
/// answers what `scoped` recomputed.
fn publish_and_compare(
    store: &LinkStore,
    scoped: &mut crate::links::worker::State,
    sink: &Published,
    what: &str,
) -> crate::links::worker::Recomputed {
    use crate::links::worker::{State, publish_summaries};
    let recomputed = publish_summaries(store, scoped, sink);
    let mut full = State::new();
    full.projects = std::sync::Arc::clone(&scoped.projects);
    let everything = Published::default();
    publish_summaries(store, &mut full, &everything);
    assert_eq!(*sink.0.borrow(), *everything.0.borrow(), "{what}");
    recomputed
}

/// A summary pass over the projects a change can move publishes exactly what
/// a pass over every project publishes on the same store, after each write
/// of a mix that reaches projects through a folder, a learned place, a
/// repository, a number named from another folder, a continuation chain, a
/// parent's missed work and a renamed parent edge
/// (with-hide/hide-private#571).
#[test]
fn a_pass_over_the_changed_projects_publishes_what_a_full_pass_publishes() {
    use crate::links::worker::State;
    let directory = tempfile::tempdir().unwrap();
    let mut store = open(directory.path());
    let mut scoped = State::new();
    let sink = Published::default();
    // Runs both passes and answers how many projects the scoped one read.
    let check = |store: &LinkStore, scoped: &mut State, what: &str| -> usize {
        publish_and_compare(store, scoped, &sink, what).projects
    };
    let read = read_facts;
    let projects = |merged| std::sync::Arc::new(scoped_projects(merged));

    scoped.projects = projects(None);
    for facts in scoped.projects.iter() {
        store.apply_project(facts, T0).unwrap();
    }
    store.refresh_ledger_outputs().unwrap();
    scoped.changed_all();
    assert_eq!(check(&store, &mut scoped, "projects"), 3);

    // A session in the app's second worktree makes PR 1 on its branch.
    let mut facts = chunk("s1", "/work/app-feat", T0 + 9 * MIN);
    facts.spans = vec![span("feat", T0 + 9 * MIN)];
    facts.prs = vec![mark("acme/app", 1, T0 + 10 * MIN, true)];
    facts.last_uuid = Some("s1-last".into());
    facts.issues = vec![mark("acme/app", 9, T0 + 11 * MIN, false)];
    read(&mut store, &mut scoped, "/f/s1.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a folder and a repository"), 1);

    // A session in the library works its pull request's branch.
    let mut facts = chunk("s2", "/work/lib", T0 + 21 * MIN);
    facts.spans = vec![span("lib-x", T0 + 21 * MIN)];
    read(&mut store, &mut scoped, "/f/s2.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a branch in a folder"), 1);

    // The docs folder names the app's PR 2 by its repository.
    let mut facts = chunk("s3", "/work/docs", T0 + 31 * MIN);
    facts.named = vec![links::NamedMark {
        pr: hide_session::NamedPr {
            repository: Some("acme/app".into()),
            number: 2,
            folder: None,
        },
        at_unix_ms: T0 + 31 * MIN,
        request: None,
    }];
    read(&mut store, &mut scoped, "/f/s3.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a named pull request"), 2);

    // A session outside every project names PR 5 by number from the library.
    let mut facts = chunk("s4", "/tmp/scratch", T0 + 22 * MIN);
    facts.named = vec![links::NamedMark {
        pr: hide_session::NamedPr {
            repository: None,
            number: 5,
            folder: Some("/work/lib".into()),
        },
        at_unix_ms: T0 + 22 * MIN,
        request: None,
    }];
    read(&mut store, &mut scoped, "/f/s4.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a number named elsewhere"), 1);

    // A session on PR 2's branch in a folder no project holds yet, then the
    // session that made PR 2 from there, which teaches the app the folder.
    let mut facts = chunk("s12", "/work/app-wt2", T0 + 35 * MIN);
    facts.spans = vec![span("fix", T0 + 35 * MIN)];
    read(&mut store, &mut scoped, "/f/s12.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a folder no project holds"), 0);
    let mut facts = chunk("s11", "/work/app-wt2", T0 + 29 * MIN);
    facts.prs = vec![mark("acme/app", 2, T0 + 30 * MIN, true)];
    read(&mut store, &mut scoped, "/f/s11.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "a learned place"), 1);

    // A session whose first record answers a session not read yet, then
    // that session, outside every project, continuing s1: the three are one
    // line.
    let mut facts = chunk("s9", "/work/app-feat", T0 + 50 * MIN);
    facts.spans = vec![span("feat", T0 + 50 * MIN)];
    facts.first_parent_uuid = Some("s7-last".into());
    read(&mut store, &mut scoped, "/f/s9.jsonl", facts);
    check(&store, &mut scoped, "a continuation of an unread session");
    let mut facts = chunk("s7", "/tmp/elsewhere", T0 + 40 * MIN);
    facts.first_parent_uuid = Some("s1-last".into());
    facts.last_uuid = Some("s7-last".into());
    read(&mut store, &mut scoped, "/f/s7.jsonl", facts);
    assert_eq!(check(&store, &mut scoped, "the middle of a chain"), 1);

    // The library's session is the app session's child, and its missed
    // work rolls up to its parent.
    store
        .apply_parents(&[crate::links::ParentFact {
            device_id: DEVICE.into(),
            agent: "claude".into(),
            session_id: "s2".into(),
            parent_device_id: DEVICE.into(),
            parent_agent: "claude".into(),
            parent_session_id: "s1".into(),
            parent_name: "app".into(),
        }])
        .unwrap();
    scoped.changed_all();
    check(&store, &mut scoped, "lineage");
    store
        .connection
        .execute(
            "INSERT INTO ledger_missed(device,agent,id,missed,kind,text,status,at,turn_offset) \
             VALUES(?1,'claude','s2','m1','\"follow_up\"','테스트 남김','\"open\"',?2,0)",
            params![DEVICE, (T0 + 22 * MIN) as i64],
        )
        .unwrap();
    scoped.changed(session("s2"));
    assert_eq!(check(&store, &mut scoped, "a child's missed work"), 2);

    // A parent known first by its file: the read that names the file moves
    // the docs session's edge to the reader's name. The app is read again
    // too, since its PR 2 names the docs session.
    store
        .apply_parents(&[crate::links::ParentFact {
            device_id: DEVICE.into(),
            agent: "claude".into(),
            session_id: "s3".into(),
            parent_device_id: DEVICE.into(),
            parent_agent: "claude".into(),
            parent_session_id: "/f/p1.jsonl".into(),
            parent_name: "lib".into(),
        }])
        .unwrap();
    scoped.changed_all();
    check(&store, &mut scoped, "a parent named by its file");
    read(
        &mut store,
        &mut scoped,
        "/f/p1.jsonl",
        chunk("p1", "/work/lib", T0 + 60 * MIN),
    );
    assert_eq!(check(&store, &mut scoped, "a renamed parent edge"), 3);

    // A pane moves the library session to another branch.
    store
        .apply_panes(
            &[crate::links::PaneFact {
                device_id: DEVICE.into(),
                agent: "claude".into(),
                session_id: "s2".into(),
                cwd: "/work/lib".into(),
                branch: Some("lib-y".into()),
            }],
            T0 + 70 * MIN,
        )
        .unwrap();
    scoped.changed(session("s2"));
    check(&store, &mut scoped, "a pane");

    // A session the docs folder holds touches nothing else.
    read(
        &mut store,
        &mut scoped,
        "/f/s5.jsonl",
        chunk("s5", "/work/docs", T0 + 80 * MIN),
    );
    assert_eq!(check(&store, &mut scoped, "one folder"), 1);

    // PR 1 merges: a project fact moves every project.
    scoped.projects = projects(Some(T0 + 90 * MIN));
    for facts in scoped.projects.iter() {
        store.apply_project(facts, T0 + 90 * MIN).unwrap();
    }
    store.refresh_ledger_outputs().unwrap();
    scoped.changed_all();
    assert_eq!(check(&store, &mut scoped, "a merge"), 3);
    assert_eq!(check(&store, &mut scoped, "nothing changed"), 0);

    // The app is connected to another repository and keeps its pull
    // requests: a session naming one of them still reaches it.
    let mut moved = scoped_projects(Some(T0 + 90 * MIN));
    moved[0].repository = Some("acme/app2".into());
    moved[0].repository_id = Some("R_3".into());
    moved[0].prs = Vec::new();
    moved[0].connection_epoch = 1;
    scoped.projects = std::sync::Arc::new(moved);
    for facts in scoped.projects.iter() {
        store.apply_project(facts, T0 + 95 * MIN).unwrap();
    }
    scoped.changed_all();
    check(&store, &mut scoped, "another repository");
    let mut facts = chunk("s13", "/tmp/review", T0 + 96 * MIN);
    facts.named = vec![links::NamedMark {
        pr: hide_session::NamedPr {
            repository: Some("acme/app".into()),
            number: 1,
            folder: None,
        },
        at_unix_ms: T0 + 96 * MIN,
        request: None,
    }];
    read(&mut store, &mut scoped, "/f/s13.jsonl", facts);
    assert_eq!(
        check(&store, &mut scoped, "a pull request kept from before"),
        1
    );
}

/// A pass reads again only the pull requests a change can reach, and still
/// publishes exactly what a pass that reads every pull request publishes:
/// each step puts one rule of the cache's invalidation to work, so a rule
/// dropped leaves a summary that is stale against the full pass
/// (with-hide/hide-private#571).
#[test]
fn a_pass_reads_again_only_the_pull_requests_a_change_reaches() {
    use crate::links::worker::{Recomputed, State};
    let directory = tempfile::tempdir().unwrap();
    let mut store = open(directory.path());
    let mut cached = State::new();
    let sink = Published::default();
    // Six pull requests on branches b1..b6; one may be closed early.
    let prs = |closed: Option<(u64, u64)>| -> Vec<PrFact> {
        (1..=6u64)
            .map(|n| {
                let closed = closed.filter(|(number, _)| *number == n).map(|(_, at)| at);
                pr(n, &format!("b{n}"), T0 + n * 10 * MIN, closed)
            })
            .collect()
    };
    let on = |id: &str, cwd: &str, branches: &[&str]| -> links::LinkFacts {
        let mut facts = chunk(id, cwd, T0 + 40 * MIN);
        facts.spans = branches
            .iter()
            .map(|branch| span(branch, T0 + 40 * MIN))
            .collect();
        facts
    };
    let pass = |store: &LinkStore, cached: &mut State, what: &str| {
        publish_and_compare(store, cached, &sink, what)
    };
    let reads = |projects: usize, prs: usize| Recomputed { projects, prs };
    // The pull requests the published summary shows a session on.
    let chips = |id: &str| -> Vec<u64> {
        sink.0.borrow()["ws"]
            .sessions
            .get(id)
            .map(|chips| chips.iter().map(|chip| chip.number).collect())
            .unwrap_or_default()
    };

    cached.projects = std::sync::Arc::new(vec![project(prs(None))]);
    store.apply_project(&cached.projects[0], T0).unwrap();
    store.refresh_ledger_outputs().unwrap();
    cached.changed_all();
    for (id, cwd, branches) in [
        ("s1", "/work/app", &["b1"][..]),
        ("s1b", "/work/app", &["b1"]),
        ("s2", "/work/app", &["b2"]),
        ("s3", "/work/app", &["b3"]),
        ("s4", "/work/app", &["b4"]),
        ("s5", "/work/app", &["b5"]),
        ("s6", "/work/app", &["b6"]),
        ("s10", "/work/app-wt", &["b2"]),
    ] {
        read_facts(
            &mut store,
            &mut cached,
            &format!("/f/{id}.jsonl"),
            on(id, cwd, branches),
        );
    }
    assert_eq!(pass(&store, &mut cached, "first"), reads(1, 6));
    assert_eq!(chips("s1b"), [1]);
    assert_eq!(chips("s10"), Vec::<u64>::new(), "outside every place");

    assert_eq!(
        pass(&store, &mut cached, "unchanged"),
        reads(0, 0),
        "inputs that did not move recompute no project and read no pull request"
    );

    // A session that already works PR 3's branch works more branches: it is
    // reached by the line that holds it and by the branch it is on.
    read_facts(
        &mut store,
        &mut cached,
        "/f/s3.jsonl",
        on("s3", "/work/app", &["b3", "b3-more"]),
    );
    assert_eq!(pass(&store, &mut cached, "a held session"), reads(1, 1));

    // A new session in a place makes PR 4 and works no branch: only the
    // number it printed reaches it.
    let mut facts = chunk("s7", "/work/app", T0 + 40 * MIN);
    facts.prs = vec![mark("acme/app", 4, T0 + 40 * MIN, true)];
    read_facts(&mut store, &mut cached, "/f/s7.jsonl", facts);
    assert_eq!(pass(&store, &mut cached, "a number printed"), reads(1, 1));
    assert_eq!(chips("s7"), [4]);

    // A new session works PR 5's branch and names nothing: only the branch
    // reaches it.
    read_facts(
        &mut store,
        &mut cached,
        "/f/s8.jsonl",
        on("s8", "/work/app", &["b5"]),
    );
    assert_eq!(pass(&store, &mut cached, "a branch worked"), reads(1, 1));
    assert_eq!(chips("s8"), [5]);

    // A session outside every place names PR 6 by number from the project's
    // folder.
    let mut facts = chunk("s9", "/tmp/elsewhere", T0 + 42 * MIN);
    facts.named = vec![links::NamedMark {
        pr: hide_session::NamedPr {
            repository: None,
            number: 6,
            folder: Some("/work/app".into()),
        },
        at_unix_ms: T0 + 42 * MIN,
        request: None,
    }];
    read_facts(&mut store, &mut cached, "/f/s9.jsonl", facts);
    assert_eq!(pass(&store, &mut cached, "a number named"), reads(1, 1));
    assert_eq!(chips("s9"), [6]);

    // A session of PR 1 moves to a branch no pull request has: what it left is
    // reached only by the line that held it.
    read_facts(
        &mut store,
        &mut cached,
        "/f/s1b.jsonl",
        on("s1b", "/work/app", &["elsewhere"]),
    );
    assert_eq!(pass(&store, &mut cached, "a session leaves"), reads(1, 1));
    assert_eq!(chips("s1b"), Vec::<u64>::new());

    // A pre-check of PR 5 starts a session of PR 2: the reviewed number
    // reaches PR 5, and the session's own line PR 2.
    store
        .apply_reviews(
            &[crate::links::ReviewFact {
                device_id: DEVICE.into(),
                agent: "claude".into(),
                session_id: "s2".into(),
                repo_name: "acme/app".into(),
                number: 5,
            }],
            T0 + 43 * MIN,
        )
        .unwrap();
    cached.changed(session("s2"));
    assert_eq!(pass(&store, &mut cached, "a review"), reads(1, 2));
    assert_eq!(chips("s2"), [5, 2]);

    // PR 3 closes before its session worked, while only an unrelated session
    // is marked: its row differs from the one held.
    assert!(sink.0.borrow()["ws"].prs.contains_key(&3));
    cached.projects = std::sync::Arc::new(vec![project(prs(Some((3, T0 + 31 * MIN))))]);
    store
        .apply_project(&cached.projects[0], T0 + 50 * MIN)
        .unwrap();
    read_facts(
        &mut store,
        &mut cached,
        "/f/s6.jsonl",
        on("s6", "/work/app", &["b6"]),
    );
    assert_eq!(pass(&store, &mut cached, "a row"), reads(1, 2));
    assert!(!sink.0.borrow()["ws"].prs.contains_key(&3));

    // A worktree is added at a folder a session already worked in, while only
    // an unrelated session is marked: the places differ from the ones held,
    // so every pull request is read again.
    let mut facts = project(prs(Some((3, T0 + 31 * MIN))));
    facts.worktrees.push(WorktreeFact {
        path: "/work/app-wt".into(),
        branch: Some("b2".into()),
        created_at_unix_ms: None,
    });
    cached.projects = std::sync::Arc::new(vec![facts]);
    store
        .apply_project(&cached.projects[0], T0 + 55 * MIN)
        .unwrap();
    read_facts(
        &mut store,
        &mut cached,
        "/f/s6.jsonl",
        on("s6", "/work/app", &["b6"]),
    );
    assert_eq!(pass(&store, &mut cached, "a place"), reads(1, 6));
    assert_eq!(chips("s10"), [2]);

    // A change no session can place reads every pull request again.
    store
        .connection
        .execute("DELETE FROM session_branches WHERE id='s5'", [])
        .unwrap();
    cached.changed_all();
    assert_eq!(pass(&store, &mut cached, "unplaced"), reads(1, 6));
    assert_eq!(chips("s5"), Vec::<u64>::new());

    assert_eq!(pass(&store, &mut cached, "settled"), reads(0, 0));
}

/// A project holding more pull requests than the cache's limit still
/// publishes every summary: the ones past it are read on each pass, and the
/// crossing is reported once.
#[test]
fn pull_requests_past_the_cache_limit_are_read_on_each_pass() {
    use crate::links::pr_cache::PrCache;
    let directory = tempfile::tempdir().unwrap();
    let mut store = open(directory.path());
    store
        .apply_project(
            &project(
                (1..=3u64)
                    .map(|n| pr(n, &format!("b{n}"), T0 + n * MIN, None))
                    .collect(),
            ),
            T0,
        )
        .unwrap();
    for n in 1..=3u64 {
        let mut facts = chunk(&format!("s{n}"), ROOT, T0 + 40 * MIN);
        facts.spans = vec![span(&format!("b{n}"), T0 + 40 * MIN)];
        let path = format!("/f/s{n}.jsonl");
        let answer = links::ReadAnswer {
            agent: Agent::Claude,
            path,
            checkpoint: None,
            has_more: false,
            rescanned: false,
            facts,
            error: None,
        };
        store.apply_answer(DEVICE, &answer, "stamp").unwrap();
    }
    let pass = store.pass(PROJECT).unwrap().unwrap();
    let mut cache = PrCache::with_limit(2);
    let (summaries, diagnostics) = crate::diagnostics::capture(|| {
        let first = store.summary_in(&pass, &[], &mut cache).unwrap();
        let before = cache.read_count();
        let second = store.summary_in(&pass, &[], &mut cache).unwrap();
        assert_eq!(first, second);
        (first, cache.read_count() - before)
    });
    assert_eq!(summaries.0.prs.len(), 3);
    assert_eq!(summaries.1, 1, "only the pull request past the limit");
    let crossings = diagnostics
        .iter()
        .filter(|record| record["kind"] == "summary.pr_cache_capacity")
        .count();
    assert_eq!(crossings, 1, "the crossing is reported once, not per pass");
}

/// The candidate pull requests are those any evidence names, as the
/// per-pull-request question the set-based read replaced asked it.
#[test]
fn the_candidate_pull_requests_are_the_ones_any_evidence_names() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = open(directory.path());
    store
        .apply_project(
            &project(
                (1..=8u64)
                    .map(|n| pr(n, &format!("b{n}"), T0 + n * MIN, None))
                    .collect(),
            ),
            T0,
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO repo_names(name, repo) VALUES('Acme/App', 'acme/app')",
            [],
        )
        .unwrap();
    let read = |store: &mut LinkStore, id: &str, facts: links::LinkFacts| {
        let answer = links::ReadAnswer {
            agent: Agent::Claude,
            path: format!("/f/{id}.jsonl"),
            checkpoint: None,
            has_more: false,
            rescanned: false,
            facts,
            error: None,
        };
        store.apply_answer(DEVICE, &answer, "stamp").unwrap();
    };
    // PR 1: a branch. PR 2: printed by the repository's alias. PR 3: named by
    // repository. PR 4: named by number alone. PR 5: printed for another
    // repository's number 5. PR 6: reviewed only. PR 7 and 8: no evidence.
    let mut facts = chunk("a", ROOT, T0);
    facts.spans = vec![span("b1", T0)];
    facts.prs = vec![
        mark("Acme/App", 2, T0, false),
        mark("other/repo", 5, T0, false),
    ];
    facts.named = vec![
        links::NamedMark {
            pr: hide_session::NamedPr {
                repository: Some("acme/app".into()),
                number: 3,
                folder: None,
            },
            at_unix_ms: T0,
            request: None,
        },
        links::NamedMark {
            pr: hide_session::NamedPr {
                repository: None,
                number: 4,
                folder: Some(ROOT.into()),
            },
            at_unix_ms: T0,
            request: None,
        },
    ];
    read(&mut store, "a", facts);
    store
        .apply_reviews(
            &[crate::links::ReviewFact {
                device_id: DEVICE.into(),
                agent: "claude".into(),
                session_id: "a".into(),
                repo_name: "acme/app".into(),
                number: 6,
            }],
            T0,
        )
        .unwrap();

    // The question asked of each pull request in turn.
    let mut statement = store
        .connection
        .prepare(&format!(
            "SELECT {PR_COLUMNS_P} FROM prs p WHERE p.project=?1 AND (\
             EXISTS(SELECT 1 FROM session_branches b WHERE b.branch=p.branch) OR \
             EXISTS(SELECT 1 FROM session_prs s LEFT JOIN repo_names r ON r.name=s.repo_name \
             WHERE s.number=p.number AND COALESCE(r.repo, s.repo_name)=p.repo) OR \
             EXISTS(SELECT 1 FROM session_named_prs n LEFT JOIN repo_names r ON r.name=n.repo_name \
             WHERE n.number=p.number AND (n.repo_name='' OR COALESCE(r.repo, n.repo_name)=p.repo))) \
             ORDER BY p.number"
        ))
        .unwrap();
    let asked: Vec<PrRow> = statement
        .query_map([PROJECT], pr_row)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let numbers = |prs: &[PrRow]| prs.iter().map(|pr| pr.number).collect::<Vec<_>>();
    assert_eq!(numbers(&asked), [1, 2, 3, 4]);
    assert_eq!(store.candidate_prs(PROJECT).unwrap(), asked);
}
