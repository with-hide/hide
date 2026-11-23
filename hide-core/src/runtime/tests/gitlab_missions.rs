//! A mission on a gitlab.com project: the letters Hide writes to its
//! Conductor, the report it closes the mission with and the stack hand-off
//! `hide mission show` answers name a merge request and its create command
//! in gitlab.com's words, while the same mission on a GitHub project keeps
//! GitHub's, byte for byte.

use super::*;
use crate::delivery::worker::Worker;
use crate::handle::ChangeNotifier;
use crate::missions::{MissionCommand, WakeEvent};
use crate::model::{
    GithubProjectSnapshot, GithubSnapshot, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
};
use crate::runtime::delivery::tests::{authority, fixture};
use serde_json::{Value, json};

const PROJECT: &str = "/checkouts/fixture";

struct Fixture {
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    _worker: Worker,
}

/// The fixture's `sender` pane conducts in the project `/checkouts/fixture`.
fn conducting() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, path) = fixture(root.path());
    let (worker, client) =
        Worker::spawn(Arc::downgrade(&runtime), ChangeNotifier::noop(), path).unwrap();
    {
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        guard
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: PROJECT.into(),
                ..Default::default()
            });
    }
    Fixture {
        _root: root,
        runtime,
        _worker: worker,
    }
}

fn command(runtime: &mut Runtime, command: Value) -> Value {
    let actor = crate::delivery::Actor {
        pane_id: "sender".into(),
        name: "sender".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: None,
    };
    let command: MissionCommand = serde_json::from_value(command).unwrap();
    runtime
        .mission_command(
            crate::node::TEST_NODE,
            "sender",
            &authority(&actor).context,
            command,
        )
        .unwrap()
        .0
}

fn pull(number: u32, head: &str, badge: PullRequestBadge) -> PullRequestSnapshot {
    PullRequestSnapshot {
        reference: None,
        closing_issues: Vec::new(),
        title: format!("Change {number}"),
        checks: PullRequestChecks::Passing,
        number,
        head_branch: "fix".into(),
        base_branch: "main".into(),
        url: format!("https://gitlab.com/example-user/project/-/merge_requests/{number}"),
        badge,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(1),
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        head_oid: Some(head.into()),
        cross_repository: false,
        work: Default::default(),
    }
}

/// The project's read as `unit`'s host answered it; GitHub's read of today
/// carries no declaration, which reads as GitHub.
fn read_by(
    unit: Option<&dyn crate::code_host::CodeHost>,
    pulls: Vec<PullRequestSnapshot>,
) -> GithubSnapshot {
    let mut project = GithubProjectSnapshot {
        pull_requests_read: true,
        root_path: PROJECT.into(),
        pull_requests: pulls,
        ..Default::default()
    };
    project.status.host = unit.map(crate::code_host::declaration);
    GithubSnapshot {
        projects: vec![project],
    }
}

fn gitlab() -> Option<&'static dyn crate::code_host::CodeHost> {
    Some(&crate::gitlab::host::GITLAB)
}

/// A started mission whose change node `fix` delivers MR or PR 7.
fn delivering(runtime: &mut Runtime) {
    command(runtime, json!({"op": "create", "title": "Fix the bug"}));
    command(
        runtime,
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
        ]}}),
    );
    command(runtime, json!({"op": "start"}));
    command(
        runtime,
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    );
}

/// The letter of the mission's last wake of `event`, as Hide sends it.
fn letter(runtime: &Runtime, event: WakeEvent) -> String {
    let mission = runtime.missions.book.get("mission-1").unwrap();
    let wake = mission
        .wakes
        .iter()
        .rev()
        .find(|wake| wake.event == event)
        .expect("the wake is recorded");
    crate::runtime::missions::wake_words(mission, wake, runtime.mission_unit(mission))
}

/// The CI and merge letters of a gitlab.com project's mission name `MR !7`;
/// the same answers on a GitHub project still read `PR #7`.
#[test]
fn a_gitlab_projects_mission_letters_name_its_merge_request() {
    for (unit, name, other) in [(gitlab(), "MR !7", "PR #7"), (None, "PR #7", "MR !7")] {
        let fixture = conducting();
        let mut runtime = fixture.runtime.lock().unwrap();
        delivering(&mut runtime);
        let node = crate::node::TEST_NODE;

        let open = read_by(unit, vec![pull(7, "a1", PullRequestBadge::Open)]);
        runtime.github = open.clone();
        assert_eq!(
            runtime.note_mission_pulls(node, &open),
            vec!["mission:mission-1:ci:fix:7:a1:passed:sender"]
        );
        let ci = letter(&runtime, WakeEvent::Ci);
        assert!(
            ci.starts_with(&format!(
                "{name}: CI passed on commit a1 (mission mission-1"
            )),
            "{ci}"
        );
        assert!(!ci.contains(other), "{ci}");

        let merged = read_by(unit, vec![pull(7, "a1", PullRequestBadge::Merged)]);
        runtime.github = merged.clone();
        runtime.note_mission_pulls(node, &merged);
        let landed = letter(&runtime, WakeEvent::Merged);
        assert!(
            landed.starts_with(&format!("{name} was merged (mission mission-1")),
            "{landed}"
        );
    }
}

/// The report Hide closes a gitlab.com project's mission with names the
/// node's merge requests `MR !7 !8`; GitHub's stays `PR #7 #8`.
#[test]
fn a_gitlab_projects_closing_report_names_its_merge_requests() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    delivering(&mut runtime);
    command(
        &mut runtime,
        json!({"op": "node", "node": "fix", "patch": {"pr": 8}}),
    );
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert_eq!(mission.node("fix").unwrap().prs, vec![7, 8]);
    let gitlab_report =
        crate::runtime::mission_health::closing_report_in(mission, &crate::gitlab::host::GITLAB);
    let github_report = crate::runtime::mission_health::closing_report(mission);
    assert!(gitlab_report.contains(" · MR !7 !8"), "{gitlab_report}");
    assert!(github_report.contains(" · PR #7 #8"), "{github_report}");
    assert_eq!(
        gitlab_report.replace("MR !7 !8", "PR #7 #8"),
        github_report,
        "only the names differ"
    );
}

/// A node stacked on another names the merge request below it and the
/// command that opens its own on that branch in gitlab.com's words, and in
/// GitHub's on a GitHub project.
#[test]
fn a_stacked_node_on_a_gitlab_project_is_handed_the_glab_command() {
    for (unit, name, create) in [
        (
            gitlab(),
            "MR !11",
            "glab mr create --target-branch m82/core",
        ),
        (None, "PR #11", "gh pr create --base m82/core"),
    ] {
        let fixture = conducting();
        let mut runtime = fixture.runtime.lock().unwrap();
        command(
            &mut runtime,
            json!({"op": "create", "title": "Contract first"}),
        );
        command(
            &mut runtime,
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "core", "title": "Core", "kind": "change", "delivery": "pr",
                 "criteria": ["the wire is fixed", "the core works"]},
                {"key": "host", "title": "Host", "kind": "change", "delivery": "pr",
                 "depends_on": ["core:c1"], "criteria": ["the host reads the wire"]},
            ]}}),
        );
        command(&mut runtime, json!({"op": "start"}));
        command(
            &mut runtime,
            json!({"op": "node", "node": "core", "patch": {"branch": "m82/core", "pr": 11}}),
        );
        runtime.github = read_by(unit, vec![pull(11, "a1", PullRequestBadge::Open)]);
        let shown = command(&mut runtime, json!({"op": "show", "node": "host"}));
        assert_eq!(shown["stack_pr"], name);
        assert_eq!(shown["stack_create"], create);
    }
}

const MERGED: &str = include_str!("../../../tests/fixtures/gitlab/merge-request-merged.json");
const OPEN: &str = include_str!("../../../tests/fixtures/gitlab/merge-request-open.json");

/// A node that answers each glab read from its queue, in order, keeps every
/// line it was asked, and holds each to the lines the node allows.
struct GlabReads {
    answers: Mutex<std::collections::VecDeque<String>>,
    asked: Mutex<Vec<Vec<String>>>,
}

impl crate::node_access::NodeLink for GlabReads {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        _timeout: std::time::Duration,
    ) -> Result<crate::node_access::LinkAnswer, hide_node_link::LinkError> {
        let hide_node_link::protocol::Call::Glab { args } = call else {
            panic!("a merge request's standing is read with glab alone: {call:?}");
        };
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        assert!(hide_node_link::glab::allowed(&words), "{words:?}");
        self.asked.lock().unwrap().push(args);
        let stdout = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("one read each");
        Ok(
            serde_json::to_value(hide_node_link::glab::GlabAnswer::Output { stdout, cut: false })
                .unwrap()
                .into(),
        )
    }
}

/// A node waiting on an outside merge request of a gitlab.com project is
/// read through the path GitHub's is (`ByNumber::standing`), one REST read
/// of that merge request each pass: open, it keeps waiting; merged, it is
/// done.
#[test]
fn an_outside_merge_request_on_a_gitlab_project_finishes_its_node_once_merged() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let mut settings =
        crate::project_settings::ProjectSettings::new(crate::node::TEST_NODE, PROJECT);
    settings.repository = Some(crate::project_settings::RepositoryConnection::Repository {
        repository: "gitlab.com/example-user/hide-code-host-check".into(),
    });
    runtime.snapshot.ui_state.project_settings.insert(
        crate::project_settings::key(crate::node::TEST_NODE, PROJECT),
        settings,
    );
    command(
        &mut runtime,
        json!({"op": "create", "title": "Wait on the parser"}),
    );
    command(
        &mut runtime,
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "parser", "title": "The parser MR", "kind": "external", "external": {"pr": 22}},
        ]}}),
    );
    command(&mut runtime, json!({"op": "start"}));
    // The recorded open merge request (!23), as !22 still open.
    let mut open: Value = serde_json::from_str(OPEN).unwrap();
    open[0]["iid"] = json!(22);
    let link = GlabReads {
        answers: Mutex::new([open.to_string(), MERGED.to_owned()].into()),
        asked: Mutex::default(),
    };
    let status = |runtime: &Runtime| {
        runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .node("parser")
            .unwrap()
            .status
    };

    runtime.read_outside_through(crate::node::TEST_NODE, PROJECT, &link);
    assert_eq!(status(&runtime), crate::missions::NodeStatus::Working);
    runtime.read_outside_through(crate::node::TEST_NODE, PROJECT, &link);
    assert_eq!(status(&runtime), crate::missions::NodeStatus::Done);
    assert_eq!(
        *link.asked.lock().unwrap(),
        vec![
            hide_node_link::glab::merge_request_arguments("example-user/hide-code-host-check", 22)
                .unwrap();
            2
        ]
    );
}
