//! The request view's verb and pull request rules (PRD overview-request-view
//! B9-B16, B48, B54, B56), on rows projected the way the engine's agents are.

use super::*;
use crate::labels::facts::{Request, Requester};
use crate::model::{GithubProjectSnapshot, PullRequestSnapshot};
use crate::sidebar::{SessionSnapshotPayload, project_agents};

const ROOT: &str = "/work/app";
const ASKED: u64 = 1_000_000;

/// One row per `(pane, status)`, unread, with the operator's request at `ASKED`.
fn rows(agents: &[(&str, &str)]) -> Vec<SidebarAgentSnapshot> {
    let payload: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({
        "agents": agents.iter().map(|(pane, status)| serde_json::json!({
            "pane_id": pane, "agent": "claude", "agent_status": status, "state_change_seq": 1,
        })).collect::<Vec<_>>()
    }))
    .unwrap();
    let mut rows = project_agents(payload).agents;
    for row in &mut rows {
        row.row_facts = Some(RowFacts {
            operator_request: Some(request("요청", Requester::Operator, ASKED, true)),
            ..RowFacts::default()
        });
    }
    rows
}

fn request(text: &str, requester: Requester, at: u64, first: bool) -> Request {
    Request {
        text: text.to_owned(),
        offset: None,
        cut: false,
        images: 0,
        at_unix_ms: at,
        requester,
        first,
    }
}

fn pull_request(
    number: u32,
    branch: &str,
    badge: PullRequestBadge,
    checks: PullRequestChecks,
) -> PullRequestSnapshot {
    PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: format!("PR {number}"),
        checks,
        number,
        head_branch: branch.to_owned(),
        base_branch: "main".to_owned(),
        url: format!("https://github.com/acme/app/pull/{number}"),
        badge,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(u64::from(number)),
        created_at_unix_ms: Some(u64::from(number)),
        closed_at_unix_ms: None,
        head_oid: Some(head_of(branch)),
        cross_repository: false,
        work: Default::default(),
    }
}

/// The commit every test checkout on `branch` is on, and the head of the
/// pull requests these tests make for it: the work belongs to its branch.
fn head_of(branch: &str) -> String {
    format!("head-of-{branch}")
}

/// The row's list as every PR mark draws it (`agent_state::work::family`),
/// with no link record behind it.
fn listed(
    rows: &[SidebarAgentSnapshot],
    github: &GithubSnapshot,
    index: usize,
) -> Option<crate::agent_state::work::family::SessionPulls> {
    use crate::agent_state::work::family::Sources;
    let sessions = vec![None; rows.len()];
    let list = Sources::new(rows.iter(), &sessions, [github]).pulls(Some(index), None, &[]);
    (!list.pulls.is_empty()).then_some(list)
}

fn github(pull_requests: Vec<PullRequestSnapshot>) -> GithubSnapshot {
    GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            root_path: ROOT.to_owned(),
            pull_requests,
            ..GithubProjectSnapshot::default()
        }],
    }
}

/// Every row's branch, by pane.
fn run(rows: &mut [SidebarAgentSnapshot], branches: &[(&str, &str)], github: &GithubSnapshot) {
    let branches: HashMap<String, (String, String)> = branches
        .iter()
        .map(|(pane, branch)| ((*pane).to_owned(), ((*branch).to_owned(), head_of(branch))))
        .collect();
    apply(
        rows,
        |pane| {
            Some(RowPlace {
                branch: branches.get(pane).map(|(branch, _)| branch.as_str()),
                head_sha: branches.get(pane).map(|(_, head)| head.as_str()),
                root_path: ROOT,
            })
        },
        github,
        &Default::default(),
        &mut BTreeMap::new(),
        2_000_000,
    );
}

fn verb(row: &SidebarAgentSnapshot) -> RequestVerb {
    row.request.as_ref().unwrap().verb
}

#[test]
fn saved_pr_is_drawn_before_confirmation_without_assigning_current_work() {
    use crate::labels::record::RecordConnection;
    use crate::links::ledger::SessionMetadata;
    let mut rows = rows(&[("resumed", "idle")]);
    rows[0].row_facts = None;
    rows[0].record_connection = Some(RecordConnection::Unconfirmed);
    rows[0].record_metadata = Some(SessionMetadata {
        created_prs: vec![
            ("acme/app".into(), 32, ASKED),
            ("acme/app".into(), 31, ASKED),
        ],
        ..Default::default()
    });
    let mut merged = pull_request(
        31,
        "old",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged.merged_at_unix_ms = Some(ASKED + 1);
    let github = github(vec![
        pull_request(
            32,
            "another-branch",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        ),
        merged,
    ]);
    run(&mut rows, &[("resumed", "main")], &github);
    assert_eq!(verb(&rows[0]), RequestVerb::Idle);
    let shown = listed(&rows, &github, 0).unwrap();
    assert_eq!(shown.pulls.len(), 2, "the merged historical PR stays drawn");
    assert_eq!(shown.pulls[0].number, 32);
    let pull = rows[0]
        .request
        .as_ref()
        .unwrap()
        .pull_requests
        .iter()
        .find(|pull| pull.number == 32)
        .unwrap();
    assert!(pull.provisional);
    assert!(!pull.duty && !pull.created);

    rows[0].record_connection = Some(RecordConnection::Confirmed);
    run(&mut rows, &[("resumed", "main")], &github);
    let shown = listed(&rows, &github, 0).unwrap();
    assert_eq!(shown.pulls[0].number, 32);
    let pull = rows[0]
        .request
        .as_ref()
        .unwrap()
        .pull_requests
        .iter()
        .find(|pull| pull.number == 32)
        .unwrap();
    assert!(!pull.provisional);
    assert!(pull.duty && pull.created);
}

#[test]
fn a_rows_verb_follows_its_demand_its_activity_and_its_pull_requests() {
    let mut rows = rows(&[
        ("asks", "blocked"),
        ("works", "working"),
        ("failed", "idle"),
        ("ready", "idle"),
        ("ci", "idle"),
        ("stopped", "idle"),
        ("finished", "done"),
        ("quiet", "idle"),
    ]);
    rows[5].row_facts.as_mut().unwrap().end = Some(crate::labels::analysis::LabelEnd::Unfinished);
    let github = github(vec![
        pull_request(
            1,
            "failed",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        ),
        pull_request(
            2,
            "ready",
            PullRequestBadge::Review,
            PullRequestChecks::Passing,
        ),
        pull_request(3, "ci", PullRequestBadge::Open, PullRequestChecks::Pending),
        pull_request(
            4,
            "works",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        ),
    ]);
    run(
        &mut rows,
        &[
            ("failed", "failed"),
            ("ready", "ready"),
            ("ci", "ci"),
            ("works", "works"),
        ],
        &github,
    );
    let verbs: Vec<_> = rows.iter().map(verb).collect();
    assert_eq!(
        verbs,
        [
            RequestVerb::Answer,
            RequestVerb::Working,
            RequestVerb::Fix,
            RequestVerb::Review,
            RequestVerb::Waiting,
            RequestVerb::Stopped,
            RequestVerb::Result,
            RequestVerb::Idle,
        ]
    );
    // Sessions groups by the agent's own state, the sidebar's names (D-16):
    // a failed check or an unfinished turn on a stopped row is Idle.
    use crate::agent_state::sessions::Group;
    let groups: Vec<_> = rows.iter().map(|row| row.state.session.group).collect();
    assert_eq!(
        groups,
        [
            Group::NeedsYou,
            Group::Working,
            Group::Idle,
            Group::Idle,
            Group::Idle,
            Group::Idle,
            Group::Done,
            Group::Idle,
        ]
    );
    assert!(
        rows[5].state.session.unfinished,
        "an unfinished turn is the Idle row's remaining work"
    );
}

/// A closed pull request stays in the row's list, drawn dimmed, as the
/// operator decided for mission-17; it was left out before.
#[test]
fn a_rows_list_holds_its_duty_prs_worst_first_with_a_closed_one_last() {
    use crate::model::PrState;
    let mut rows = rows(&[("maker", "idle"), ("other", "idle")]);
    rows[0].row_facts.as_mut().unwrap().created_prs = vec![
        ("acme/app".into(), 1, 1),
        ("acme/app".into(), 2, 2),
        ("acme/app".into(), 3, 3),
        ("acme/app".into(), 4, 4),
    ];
    let mut merged = pull_request(
        3,
        "three",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged.merged_at_unix_ms = Some(ASKED + 10);
    let mut closed = pull_request(
        4,
        "four",
        PullRequestBadge::Closed,
        PullRequestChecks::Failed,
    );
    closed.closed_at_unix_ms = Some(ASKED + 10);
    let github = github(vec![
        pull_request(1, "one", PullRequestBadge::Open, PullRequestChecks::Failed),
        pull_request(2, "two", PullRequestBadge::Open, PullRequestChecks::Passing),
        merged,
        closed,
    ]);
    run(&mut rows, &[("maker", "main"), ("other", "one")], &github);
    // PR 1 is on `other`'s checkout branch, so `other` holds its duty.
    let maker = listed(&rows, &github, 0).expect("maker owns PRs");
    let states: Vec<_> = maker.pulls.iter().map(|pull| pull.state).collect();
    assert_eq!(
        states,
        [PrState::Mergeable, PrState::Merged, PrState::Closed]
    );
    let other = listed(&rows, &github, 1).expect("other owns PR 1");
    let states: Vec<_> = other
        .pulls
        .iter()
        .map(|pull| (pull.number, pull.state))
        .collect();
    assert_eq!(states, [(1, PrState::Failed)]);
}

#[test]
fn a_pr_reads_draft_then_failed_or_changes_requested_then_mergeable_then_pending() {
    use crate::model::PrState;
    for (draft, checks, review, expected) in [
        (false, PullRequestChecks::Passing, None, PrState::Mergeable),
        (
            false,
            PullRequestChecks::Passing,
            Some(ReviewDecision::Approved),
            PrState::Mergeable,
        ),
        (
            false,
            PullRequestChecks::Passing,
            Some(ReviewDecision::ReviewRequired),
            PrState::Pending,
        ),
        // A reviewer asking for changes is something to fix, like a failed check.
        (
            false,
            PullRequestChecks::Passing,
            Some(ReviewDecision::ChangesRequested),
            PrState::Failed,
        ),
        (
            false,
            PullRequestChecks::Unknown,
            Some(ReviewDecision::Approved),
            PrState::Pending,
        ),
        (false, PullRequestChecks::None, None, PrState::Pending),
        (false, PullRequestChecks::Pending, None, PrState::Pending),
        (false, PullRequestChecks::Failed, None, PrState::Failed),
        // A draft is grey whatever its checks say.
        (true, PullRequestChecks::Failed, None, PrState::Draft),
        (true, PullRequestChecks::Passing, None, PrState::Draft),
    ] {
        let mut rows = rows(&[("agent", "idle")]);
        let mut pull = pull_request(1, "feature", PullRequestBadge::Open, checks);
        pull.review = review;
        pull.is_draft = draft;
        let github = github(vec![pull]);
        run(&mut rows, &[("agent", "feature")], &github);
        assert_eq!(
            listed(&rows, &github, 0).map(|list| list.pulls[0].state),
            Some(expected),
            "{draft} {checks:?} {review:?}"
        );
    }
}

#[test]
fn a_session_row_takes_its_group_and_line_from_its_own_state() {
    // The 태스크 list's second line (PRD sidebar-task-lens B8) reads each
    // row's own session facts; which task holds a row is `tally/tasks.rs`'s.
    use crate::agent_state::sessions::Group;
    let mut rows = rows(&[
        ("approval", "blocked"),
        ("working", "working"),
        ("quiet", "idle"),
        ("child", "blocked"),
        ("unfinished", "idle"),
        ("failed", "idle"),
        ("ready", "idle"),
    ]);
    rows[3].delegated = true;
    rows[4].row_facts.as_mut().unwrap().end = Some(crate::labels::analysis::LabelEnd::Unfinished);
    rows[4].row_facts.as_mut().unwrap().line = Some("Grok 훅 연결 마무리 남음".into());
    run(
        &mut rows,
        &[("failed", "failed"), ("ready", "ready")],
        &github(vec![
            pull_request(
                1,
                "failed",
                PullRequestBadge::Open,
                PullRequestChecks::Failed,
            ),
            pull_request(
                2,
                "ready",
                PullRequestBadge::Open,
                PullRequestChecks::Passing,
            ),
        ]),
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.state.session.group)
            .collect::<Vec<_>>(),
        [
            Group::NeedsYou,
            Group::Working,
            Group::Idle,
            Group::NeedsYou,
            Group::Idle,
            Group::Idle,
            Group::Idle,
        ]
    );
    assert_eq!(
        rows[4].state.session.line.as_deref(),
        Some("Grok 훅 연결 마무리 남음")
    );
    assert_eq!(
        rows[5].state.session.line, None,
        "an Idle row's PR state is its chip, not a line"
    );
}

#[test]
fn automatic_resolution_waits_for_every_assigned_pr_and_newer_settlement_after_input() {
    use crate::agent_state::sessions::auto_resolvable;
    let mut agent = rows(&[("maker", "idle")]);
    agent[0].row_facts.as_mut().unwrap().created_prs =
        vec![("acme/app".into(), 1, 1), ("acme/app".into(), 2, 2)];
    let mut merged = pull_request(
        1,
        "one",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged.merged_at_unix_ms = Some(ASKED + 10);
    let mut closed = pull_request(
        2,
        "two",
        PullRequestBadge::Closed,
        PullRequestChecks::Passing,
    );
    closed.closed_at_unix_ms = Some(ASKED + 20);
    run(
        &mut agent,
        &[("maker", "main")],
        &github(vec![
            merged.clone(),
            pull_request(2, "two", PullRequestBadge::Open, PullRequestChecks::Passing),
        ]),
    );
    assert!(
        !auto_resolvable(&agent[0], None),
        "one settled PR cannot hide the remaining duty"
    );
    run(
        &mut agent,
        &[("maker", "main")],
        &github(vec![merged, closed]),
    );
    assert!(auto_resolvable(&agent[0], None));
    assert!(
        !auto_resolvable(&agent[0], Some(ASKED + 10)),
        "input after a settlement restores the session until all duties settle again"
    );
    agent[0].blocked = true;
    assert!(!auto_resolvable(&agent[0], None));
    agent[0].blocked = false;
    agent[0].activity = "working".into();
    assert!(!auto_resolvable(&agent[0], None));
}

#[test]
fn a_merged_pull_request_resolves_an_unread_finish_only_once_it_is_read() {
    use crate::agent_state::sessions::auto_resolvable;
    let mut agent = rows(&[("maker", "done")]);
    agent[0].row_facts.as_mut().unwrap().created_prs = vec![("acme/app".into(), 1, 1)];
    let mut merged = pull_request(
        1,
        "one",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged.merged_at_unix_ms = Some(ASKED + 10);
    run(&mut agent, &[("maker", "main")], &github(vec![merged]));
    assert!(agent[0].unread, "Herdr's done is an unread finish");
    assert!(
        !auto_resolvable(&agent[0], None),
        "an unread finish stays under 읽을 것 though its pull request merged"
    );
    agent[0].unread = false;
    crate::agent_state::rederive(&mut agent[0]);
    assert!(auto_resolvable(&agent[0], None));
    agent[0].wait = Some(crate::model::AgentWait::Background);
    crate::agent_state::rederive(&mut agent[0]);
    assert!(
        !auto_resolvable(&agent[0], None),
        "a row still waiting on its background task stays under 대기"
    );
}

#[test]
fn a_session_that_made_pull_requests_on_main_shows_the_one_to_look_at_first() {
    let mut rows = rows(&[("main-agent", "idle")]);
    rows[0].row_facts.as_mut().unwrap().created_prs = vec![
        ("acme/app".to_owned(), 10, 5),
        ("acme/app".to_owned(), 11, 6),
        ("acme/app".to_owned(), 12, 7),
    ];
    let mut merged_before = pull_request(
        10,
        "prd/old",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged_before.merged_at_unix_ms = Some(ASKED - 1);
    let github = github(vec![
        merged_before,
        pull_request(
            11,
            "prd/a",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        ),
        pull_request(
            12,
            "prd/b",
            PullRequestBadge::Review,
            PullRequestChecks::Passing,
        ),
        pull_request(
            13,
            "prd/c",
            PullRequestBadge::Open,
            PullRequestChecks::Pending,
        ),
    ]);
    run(&mut rows, &[("main-agent", "main")], &github);
    let request = rows[0].request.as_ref().unwrap();
    assert_eq!(
        request.verb,
        RequestVerb::Fix,
        "the older PR's failure is not hidden"
    );
    let shown: Vec<_> = request
        .pull_requests
        .iter()
        .map(|pr| (pr.number, pr.live, pr.created))
        .collect();
    assert_eq!(
        shown,
        [(11, true, true), (12, true, true), (10, false, true)],
        "a PR it did not make is not its"
    );
    let marked: Vec<_> = listed(&rows, &github, 0)
        .unwrap()
        .pulls
        .iter()
        .map(|pull| pull.number)
        .collect();
    assert_eq!(
        marked,
        [11, 12, 10],
        "a PR merged before the last request keeps its mark"
    );
}

#[test]
fn a_pull_request_settled_after_the_request_is_a_result_and_one_settled_before_is_history() {
    let mut rows = rows(&[("after", "idle"), ("before", "idle")]);
    let mut after = pull_request(
        1,
        "after",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    after.merged_at_unix_ms = Some(ASKED + 1);
    let mut before = pull_request(
        2,
        "before",
        PullRequestBadge::Closed,
        PullRequestChecks::Passing,
    );
    before.closed_at_unix_ms = Some(ASKED - 1);
    run(
        &mut rows,
        &[("after", "after"), ("before", "before")],
        &github(vec![after, before]),
    );
    assert_eq!(verb(&rows[0]), RequestVerb::Result);
    assert!(rows[0].request.as_ref().unwrap().pull_requests[0].live);
    assert_eq!(verb(&rows[1]), RequestVerb::Idle);
    assert!(!rows[1].request.as_ref().unwrap().pull_requests[0].live);
}

/// B15, D-29: opening a result a merged pull request gave sends the row to
/// rest, and a pull request that settles after the opening is a result again.
#[test]
fn an_opened_result_rests_until_another_pull_request_settles() {
    let lay = |rows: &mut Vec<SidebarAgentSnapshot>,
               github: &GithubSnapshot,
               verbs: &mut BTreeMap<String, VerbRecord>,
               head: &str,
               now| {
        apply(
            rows,
            |_| {
                Some(RowPlace {
                    branch: Some("after"),
                    head_sha: Some(head),
                    root_path: ROOT,
                })
            },
            github,
            &Default::default(),
            verbs,
            now,
        );
    };
    let mut merged = pull_request(
        1,
        "after",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    merged.merged_at_unix_ms = Some(ASKED + 1);
    let mut verbs = BTreeMap::new();
    let first_head = head_of("after");
    let mut shown = rows(&[("p", "idle")]);
    lay(
        &mut shown,
        &github(vec![merged.clone()]),
        &mut verbs,
        &first_head,
        ASKED + 10,
    );
    assert_eq!(verb(&shown[0]), RequestVerb::Result);

    assert!(open_result(&mut verbs, "p", ASKED + 20));
    let mut shown = rows(&[("p", "idle")]);
    lay(
        &mut shown,
        &github(vec![merged.clone()]),
        &mut verbs,
        &first_head,
        ASKED + 30,
    );
    assert_eq!(verb(&shown[0]), RequestVerb::Idle);
    assert!(
        !open_result(&mut verbs, "p", ASKED + 40),
        "only a result is opened"
    );

    let mut later = pull_request(
        2,
        "after",
        PullRequestBadge::Closed,
        PullRequestChecks::Passing,
    );
    later.closed_at_unix_ms = Some(ASKED + 50);
    // The branch was worked on again after the merge, so the checkout is on
    // the later pull request's commit, not the merged one's.
    later.head_oid = Some("second-round".to_owned());
    let mut shown = rows(&[("p", "idle")]);
    lay(
        &mut shown,
        &github(vec![merged, later]),
        &mut verbs,
        "second-round",
        ASKED + 60,
    );
    assert_eq!(verb(&shown[0]), RequestVerb::Result);
}

#[test]
fn a_shared_pull_request_gives_its_duty_to_the_row_on_its_branch() {
    let mut rows = rows(&[("maker", "idle"), ("branch", "idle")]);
    rows[0].row_facts.as_mut().unwrap().created_prs = vec![("acme/app".to_owned(), 5, 1)];
    run(
        &mut rows,
        &[("maker", "main"), ("branch", "prd/x")],
        &github(vec![pull_request(
            5,
            "prd/x",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        )]),
    );
    assert_eq!(
        verb(&rows[0]),
        RequestVerb::Idle,
        "the chip stays, the duty goes"
    );
    assert!(!rows[0].request.as_ref().unwrap().pull_requests[0].duty);
    assert_eq!(verb(&rows[1]), RequestVerb::Fix);
}

#[test]
fn a_checkouts_pull_request_stays_with_its_row_nearest_the_root_whichever_works_last() {
    // A lead on main spawned an Observer, which spawned an implementor; both
    // work on the pull request's checkout, and the implementor's session
    // printed it. Rows arrive in activity order, so either may come first.
    for implementor_first in [false, true] {
        let mut rows = rows(&[
            ("lead", "idle"),
            ("observer", "idle"),
            ("implementor", "idle"),
        ]);
        rows[1].lineage_depth = 1;
        rows[2].lineage_depth = 2;
        rows[2].row_facts.as_mut().unwrap().created_prs = vec![("acme/app".to_owned(), 5, 1)];
        if implementor_first {
            rows.swap(1, 2);
        }
        run(
            &mut rows,
            &[
                ("lead", "main"),
                ("observer", "prd/x"),
                ("implementor", "prd/x"),
            ],
            &github(vec![pull_request(
                5,
                "prd/x",
                PullRequestBadge::Open,
                PullRequestChecks::Failed,
            )]),
        );
        let row = |pane: &str| rows.iter().find(|row| row.pane_id == pane).unwrap();
        let pulls = |pane: &str| &row(pane).request.as_ref().unwrap().pull_requests;
        assert_eq!(
            verb(row("observer")),
            RequestVerb::Fix,
            "implementor first: {implementor_first}"
        );
        assert_eq!(verb(row("implementor")), RequestVerb::Idle);
        assert!(
            !pulls("implementor")[0].duty,
            "the link stays, the duty goes"
        );
    }
}

#[test]
fn rows_alike_on_the_checkout_keep_one_holder_whatever_their_order() {
    let holder = |helper_first: bool| {
        let mut rows = rows(&[("helper", "idle"), ("other", "idle")]);
        if !helper_first {
            rows.swap(0, 1);
        }
        run(
            &mut rows,
            &[("helper", "prd/x"), ("other", "prd/x")],
            &github(vec![pull_request(
                5,
                "prd/x",
                PullRequestBadge::Open,
                PullRequestChecks::Failed,
            )]),
        );
        let holders: Vec<String> = rows
            .iter()
            .filter(|row| row.request.as_ref().unwrap().pull_requests[0].duty)
            .map(|row| row.pane_id.clone())
            .collect();
        assert_eq!(holders.len(), 1, "one row holds the duty");
        holders.into_iter().next().unwrap()
    };
    assert_eq!(holder(true), holder(false));
}

#[test]
fn rows_at_one_depth_on_the_checkout_give_the_duty_to_the_session_that_made_it() {
    let mut rows = rows(&[("helper", "idle"), ("maker", "idle")]);
    rows[1].row_facts.as_mut().unwrap().created_prs = vec![("acme/app".to_owned(), 5, 1)];
    run(
        &mut rows,
        &[("helper", "prd/x"), ("maker", "prd/x")],
        &github(vec![pull_request(
            5,
            "prd/x",
            PullRequestBadge::Open,
            PullRequestChecks::Failed,
        )]),
    );
    assert_eq!(verb(&rows[1]), RequestVerb::Fix);
    assert_eq!(verb(&rows[0]), RequestVerb::Idle);
}

#[test]
fn who_sent_the_shown_request_and_who_came_after() {
    let mut rows = rows(&[("parent", "idle"), ("child", "idle")]);
    rows[1].delegated = true;
    rows[1].lineage_parent_pane_id = Some("parent".to_owned());
    rows[0].identity_label = "요청 보기".to_owned();
    rows[0].row_facts.as_mut().unwrap().other_request = Some(request(
        "CI 다시 봐줘",
        Requester::Named("ci-lead".to_owned()),
        ASKED + 5,
        false,
    ));
    rows[1].row_facts = Some(RowFacts {
        operator_request: Some(request("맡긴 일", Requester::Unobserved, ASKED, true)),
        ..RowFacts::default()
    });
    run(&mut rows, &[], &GithubSnapshot::default());
    let parent = rows[0].request.as_ref().unwrap();
    assert_eq!(
        parent.request.as_ref().unwrap().sender,
        RequestSender::Operator
    );
    assert_eq!(
        parent.later_by,
        Some(RequestSender::Named("ci-lead".to_owned()))
    );
    let child = rows[1].request.as_ref().unwrap();
    assert_eq!(
        child.request.as_ref().unwrap().sender,
        RequestSender::Named("요청 보기".to_owned()),
        "a delegated child's first request is its parent's"
    );
}

#[test]
fn a_sender_cannot_pass_as_the_operator_or_carry_hidden_text() {
    let named_as = |name: &str| {
        let mut rows = rows(&[("p", "idle")]);
        rows[0].row_facts = Some(RowFacts {
            other_request: Some(request(
                "봐줘",
                Requester::Named(name.to_owned()),
                ASKED,
                false,
            )),
            ..RowFacts::default()
        });
        run(&mut rows, &[], &GithubSnapshot::default());
        rows[0].request.clone().unwrap()
    };
    for reserved in [
        "나",
        "에이전트",
        "Operator",
        " 나 ",
        "나\u{200B}",
        "ｏｐｅｒａｔｏｒ",
        "\u{1102}\u{1161}",
    ] {
        assert_eq!(
            named_as(reserved).request.unwrap().sender,
            RequestSender::Agent,
            "{reserved}"
        );
    }
    let shown = named_as(&format!("ci\u{202E}-lead\n{}", "x".repeat(100)));
    let RequestSender::Named(name) = shown.request.unwrap().sender else {
        panic!("a plain name is shown");
    };
    assert!(name.starts_with("ci-lead x"), "{name}");
    assert_eq!(name.chars().count(), 64);
}

#[test]
fn a_verbs_time_holds_while_the_verb_holds_and_starts_over_when_it_moves() {
    let mut verbs = BTreeMap::new();
    let mut idle = rows(&[("p", "idle")]);
    assert!(apply(
        &mut idle,
        |_| None,
        &GithubSnapshot::default(),
        &Default::default(),
        &mut verbs,
        10
    ));
    assert!(!apply(
        &mut idle,
        |_| None,
        &GithubSnapshot::default(),
        &Default::default(),
        &mut verbs,
        20
    ));
    assert_eq!(idle[0].request.as_ref().unwrap().verb_since_unix_ms, 10);
    let mut working = rows(&[("p", "working")]);
    assert!(apply(
        &mut working,
        |_| None,
        &GithubSnapshot::default(),
        &Default::default(),
        &mut verbs,
        30
    ));
    assert_eq!(working[0].request.as_ref().unwrap().verb_since_unix_ms, 30);
    assert!(prune_verbs(&mut verbs, |_| true));
}

#[test]
fn a_merged_pull_request_of_a_reused_branch_name_is_not_a_rows_result() {
    let merged = pull_request(
        1,
        "feature",
        PullRequestBadge::Merged,
        PullRequestChecks::Passing,
    );
    let on = |head: &str| {
        let mut shown = rows(&[("p", "idle")]);
        apply(
            &mut shown,
            |_| {
                Some(RowPlace {
                    branch: Some("feature"),
                    head_sha: Some(head),
                    root_path: ROOT,
                })
            },
            &github(vec![merged.clone()]),
            &Default::default(),
            &mut BTreeMap::new(),
            2_000_000,
        );
        shown
            .remove(0)
            .request
            .map_or(0, |block| block.pull_requests.len())
    };
    let current = head_of("feature");
    assert_eq!(on("new-work"), 0);
    assert_eq!(on(&current), 1);
}

fn turn(offset: u64, request: &str, end: Option<LabelEnd>) -> RecentTurn {
    RecentTurn {
        offset,
        request: request.to_owned(),
        end,
        line: format!("{request} 결과"),
        at_unix_ms: offset,
        witness: None,
    }
}

/// PRD pane-last-request D-02, D-10, D-16: the shown request reads as the
/// label's summary of the message at its offset, and as its own first line
/// (a letter's header left out, spaces folded) until there is one. The turn
/// card gets up to two turns before it, newest first, never a later one.
#[test]
fn the_shown_request_reads_as_its_summary_or_its_own_first_line() {
    let mut rows = rows(&[("w1:p1", "working")]);
    let facts = rows[0].row_facts.as_mut().unwrap();
    let mut asked = request(
        "Hide letter l-1 from lead (claude) [request]\n\n  ㅇㅇ   그렇게 해  \n점검도",
        Requester::Operator,
        ASKED,
        false,
    );
    asked.offset = Some(40);
    facts.operator_request = Some(asked);
    run(&mut rows, &[], &github(Vec::new()));
    let view = rows[0].request.as_ref().unwrap();
    assert_eq!(view.request_summary.as_deref(), Some("ㅇㅇ 그렇게 해"));
    assert!(view.recent_turns.is_empty());

    let facts = rows[0].row_facts.as_mut().unwrap();
    facts.recent_turns = vec![
        turn(10, "첫 요청 요약", Some(LabelEnd::Done)),
        turn(20, "둘째 요청 요약", Some(LabelEnd::Working)),
        turn(30, "셋째 요청 요약", Some(LabelEnd::Question)),
        turn(
            40,
            "제안대로 진행 + 애매한 상태 점검",
            Some(LabelEnd::Working),
        ),
        turn(50, "나중 편지 요약", None),
    ];
    run(&mut rows, &[], &github(Vec::new()));
    let view = rows[0].request.as_ref().unwrap();
    assert_eq!(
        view.request_summary.as_deref(),
        Some("제안대로 진행 + 애매한 상태 점검")
    );
    let earlier: Vec<_> = view
        .recent_turns
        .iter()
        .map(|turn| (turn.request.as_deref(), turn.end))
        .collect();
    assert_eq!(
        earlier,
        [
            (Some("셋째 요청 요약"), Some(LabelEnd::Question)),
            // A turn judged only while it ran says nothing of how it ended.
            (Some("둘째 요청 요약"), None),
        ]
    );
}

fn letter_between(
    id: u64,
    from: &str,
    to: &str,
    kind: &str,
    body: &str,
    at: u64,
) -> crate::delivery::ledger::Letter {
    let actor = |pane: &str| {
        serde_json::json!({
            "pane_id": pane, "name": format!("agent {pane}"), "kind": "claude",
            "device_id": "device", "session": "s",
        })
    };
    serde_json::from_value(serde_json::json!({
        "id": format!("letter-{id}"), "intent": format!("intent-{id}"),
        "sender": actor(from), "recipient": actor(to), "kind": kind, "body": body,
        "state": "delivered", "waiting_answer": false, "reply_to": null,
        "created_at_unix_ms": at, "finished_at_unix_ms": null,
        "bell_errors": 0, "bell_sent": false
    }))
    .unwrap()
}

/// The turn card's later half (`docs/UI_BEHAVIOR.md`, the turn card): the
/// answer to the operator's request once its turn is over, the letters since
/// it, and the questions letter turns put to the operator since it, on this
/// machine's row and a device's alike.
#[test]
fn the_card_shows_the_answer_the_letters_and_the_questions_since_the_request() {
    const LEAD: &str = "w1:p1";
    const DEVICE: &str = "remote:mini:pane:w2:p1";
    let mut shown = rows(&[(LEAD, "idle"), (DEVICE, "idle")]);
    for row in &mut shown {
        let facts = row.row_facts.as_mut().unwrap();
        facts.operator_request.as_mut().unwrap().offset = Some(10);
        // A letter began a later turn, which the lead's header does not show.
        facts.other_request = Some(Request {
            offset: Some(50),
            ..request(
                "🦉 리뷰어 보고",
                Requester::Named("reviewer".into()),
                ASKED + 3_000,
                false,
            )
        });
        facts.recent_turns = vec![RecentTurn {
            witness: Some("w".repeat(64)),
            ..turn(10, "프로필 폴더 질문", Some(LabelEnd::Done))
        }];
        facts.replies = vec![crate::labels::facts::Reply {
            request_offset: 10,
            head: "네, 실제로 생길 수 있습니다.\n다만 흔한 경우는 아닙니다.".into(),
            bytes: 900,
            more: true,
            at_unix_ms: ASKED + 500,
        }];
        facts.asked = vec![
            crate::labels::store::AskedTurn {
                offset: 5,
                line: "요청 전 질문".into(),
                at_unix_ms: ASKED - 1,
            },
            crate::labels::store::AskedTurn {
                offset: 30,
                line: "닫은 탭 결함 어떻게 할지 정해 주세요".into(),
                at_unix_ms: ASKED + 2_000,
            },
            crate::labels::store::AskedTurn {
                offset: 50,
                line: "머지할까요?".into(),
                at_unix_ms: ASKED + 3_500,
            },
        ];
    }
    let ledger = crate::delivery::ledger::Ledger {
        letters: vec![
            letter_between(1, LEAD, "w1:p9", "report", "요청 전 편지", ASKED - 10),
            letter_between(2, "w1:p9", LEAD, "block", "디스크가 찼습니다", ASKED + 100),
            letter_between(3, LEAD, "w1:p9", "progress", "중간 보고", ASKED + 200),
            letter_between(4, "w1:p9", DEVICE, "report", "기기에서 보고", ASKED + 300),
        ],
        ..Default::default()
    };
    let index = crate::delivery::correspondence::Correspondence::of(&ledger);
    apply(
        &mut shown,
        |_| None,
        &GithubSnapshot::default(),
        &index,
        &mut BTreeMap::new(),
        ASKED + 4_000,
    );

    let lead = shown[0].request.as_ref().unwrap();
    let answer = lead.answer.as_ref().expect("the request's turn is over");
    assert_eq!(answer.offset, 10);
    assert_eq!(answer.end, Some(LabelEnd::Done));
    assert_eq!(answer.line.as_deref(), Some("프로필 폴더 질문 결과"));
    assert_eq!(
        answer.head.as_deref(),
        Some("네, 실제로 생길 수 있습니다.\n다만 흔한 경우는 아닙니다.")
    );
    assert!(answer.whole);
    let letters = lead.letters.as_ref().unwrap();
    assert_eq!((letters.total, letters.progress), (1, 1));
    assert_eq!(letters.recent[0].line.as_deref(), Some("디스크가 찼습니다"));
    assert_eq!(
        lead.asked
            .iter()
            .map(|asked| (asked.line.as_str(), asked.current))
            .collect::<Vec<_>>(),
        vec![
            ("닫은 탭 결함 어떻게 할지 정해 주세요", false),
            ("머지할까요?", true)
        ]
    );
    // A device's agent is read from the same ledger by its scoped pane id.
    let device = shown[1].request.as_ref().unwrap();
    assert_eq!(
        device.letters.as_ref().unwrap().recent[0].line.as_deref(),
        Some("기기에서 보고")
    );
}

/// A letter's other side goes by its screen name: the name it registered
/// under unless Hide made it up for its pane, else its pane's title on these
/// rows, else its runtime's name; an unnamed agent's registration id, its wire
/// name, never shows.
#[test]
fn a_letters_peer_goes_by_its_registered_name_then_its_title_then_its_runtime() {
    const LEAD: &str = "w1:p1";
    let mut shown = rows(&[(LEAD, "idle"), ("w1:p2", "idle")]);
    shown[1].identity_label = "리뷰 반영하기".to_owned();
    let mut ledger = crate::delivery::ledger::Ledger {
        letters: vec![
            letter_between(1, "w1:p2", LEAD, "report", "제목으로", ASKED + 100),
            letter_between(2, "w1:p3", LEAD, "report", "이름으로", ASKED + 200),
            letter_between(3, "w1:p9", LEAD, "report", "런타임으로", ASKED + 300),
        ],
        ..Default::default()
    };
    // `w1:p2` registered under the name Hide made for its pane, which reads as none.
    let made = crate::fork::task_agent_name("claude", "w1:p2");
    for (id, pane, name) in [
        ("agent-2", "w1:p2", made.as_str()),
        ("agent-3", "w1:p3", "reviewer"),
    ] {
        ledger.agents.push(
            serde_json::from_value(serde_json::json!({
                "id": id, "name": name, "machine": "device", "host_scope": "scope",
                "native_machine": "device", "session": "s", "instance": "i",
                "terminal_id": null, "pane": pane, "parent": null, "project": null,
                "actor": {"pane_id": pane, "name": name, "kind": "claude", "device_id": "device", "session": "s"},
                "ended": false
            }))
            .unwrap(),
        );
    }
    apply(
        &mut shown,
        |_| None,
        &GithubSnapshot::default(),
        &crate::delivery::correspondence::Correspondence::of(&ledger),
        &mut BTreeMap::new(),
        ASKED + 1_000,
    );
    let peers: Vec<_> = shown[0]
        .request
        .as_ref()
        .unwrap()
        .letters
        .as_ref()
        .unwrap()
        .recent
        .iter()
        .map(|line| line.peer.clone())
        .collect();
    assert_eq!(
        peers,
        vec![
            RequestSender::Named("리뷰 반영하기".into()),
            RequestSender::Named("reviewer".into()),
            RequestSender::Named(crate::sidebar::provider_name(Some("claude"))),
        ]
    );
}

/// While the request's own turn still works there is no answer yet, and an
/// answer whose turn was never judged offers no whole read.
#[test]
fn a_working_turn_has_no_answer_and_an_unjudged_one_reads_no_whole() {
    let mut shown = rows(&[("p", "working"), ("q", "idle")]);
    for row in &mut shown {
        let facts = row.row_facts.as_mut().unwrap();
        facts.operator_request.as_mut().unwrap().offset = Some(10);
        facts.replies = vec![crate::labels::facts::Reply {
            request_offset: 10,
            head: "먼저 확인해 볼게요".into(),
            bytes: 30,
            more: true,
            at_unix_ms: ASKED + 1,
        }];
    }
    apply(
        &mut shown,
        |_| None,
        &GithubSnapshot::default(),
        &Default::default(),
        &mut BTreeMap::new(),
        ASKED + 10,
    );
    assert_eq!(shown[0].request.as_ref().unwrap().answer, None);
    let answer = shown[1].request.as_ref().unwrap().answer.as_ref().unwrap();
    assert_eq!(
        (answer.line.as_deref(), answer.head.as_deref(), answer.whole),
        (None, Some("먼저 확인해 볼게요"), false)
    );
}
