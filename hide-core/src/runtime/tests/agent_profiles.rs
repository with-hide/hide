//! A machine's agent profiles as Settings › Agents and `hide agent profile`
//! change them, and the profile a new start picks (PRD agent-profiles
//! B11, B14-B16, B27, B28).

use super::*;
use crate::agent_profiles::{self as profiles, AnswerState, ProfileCommand};
use crate::launch::LaunchProfile;
use crate::model::{ProfileSnapshot, ProfilesSnapshot, ProfilesState};
use hide_agent_adapter::{AccountKey, ProfileId};

const ROOT: &str = "/fixture/blog";

fn profile(id: &str, kind: &str, name: &str) -> ProfileSnapshot {
    ProfileSnapshot {
        id: id.to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        account: Some(format!("/home/me/accounts/{name}")),
        program: None,
        env_keys: Vec::new(),
        version: 1,
        install: None,
        folder: None,
        hook: None,
        skill: None,
        login: None,
        wrapper_folder: false,
        startable: true,
        account_key: Some(hide_agent_adapter::AccountKey::Folder(format!(
            "/home/me/accounts/{name}"
        ))),
        account_variables: Vec::new(),
    }
}

/// A runtime whose own machine lists Claude's `work` and `side` and Codex's
/// `team`, with `side` Claude's default there.
fn listed() -> Runtime {
    let mut runtime = runtime();
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_state(&node);
    kit.profiles = Some(ProfilesSnapshot {
        state: ProfilesState::Listed,
        version: 3,
        profiles: vec![
            profile("p-work", "claude-code", "work"),
            profile("p-side", "claude-code", "side"),
            profile("p-team", "codex", "team"),
        ],
        defaults: [("claude-code".to_owned(), "p-side".to_owned())].into(),
    });
    runtime.kit_states.insert(node, kit);
    runtime
}

/// The project at `ROOT` starts Claude with `profile`.
fn project_default(runtime: &mut Runtime, profile: &str) {
    let node = runtime.node.to_string();
    let mut record = crate::project_settings::ProjectSettings::new(&node, ROOT);
    record.name = Some("blog".to_owned());
    record
        .agent_profiles
        .insert("claude-code".to_owned(), profile.to_owned());
    runtime
        .snapshot
        .ui_state
        .project_settings
        .insert(crate::project_settings::key(&node, ROOT), record);
}

/// A Claude agent running in `pane` with the profile `id`, as its pane's
/// launch record says.
fn running(runtime: &mut Runtime, pane: &str, id: &str, title: &str) {
    let mut row = project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id": pane, "agent": "claude", "agent_status": "working", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0);
    row.started_name = Some(format!("{pane}-agent"));
    row.identity_label = title.to_owned();
    row.workspace_label = "blog".to_owned();
    runtime.snapshot.navigator.agents.push(row);
    runtime.snapshot.ui_state.pane_launches.insert(
        pane.to_owned(),
        crate::model::PaneLaunch {
            kind: "claude".to_owned(),
            name: format!("{pane}-agent"),
            choice: Default::default(),
            session: None,
            profile: Some(LaunchProfile {
                id: ProfileId(id.to_owned()),
                name: id.to_owned(),
                account: AccountKey::Folder(format!("/home/me/accounts/{id}")),
            }),
        },
    );
}

fn change(runtime: &mut Runtime, request: &str, change: serde_json::Value) {
    let node = runtime.node.to_string();
    assert!(
        runtime.dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": SCHEMA_VERSION,
                "kind": "agent_profile_change",
                "payload": {"device_id": node, "request_id": request, "change": change},
            }))
            .unwrap()
        )
    );
}

fn answer(runtime: &Runtime) -> &profiles::ProfileAnswer {
    runtime
        .snapshot
        .status
        .agent_profile_answer
        .as_ref()
        .unwrap()
}

#[test]
fn a_new_start_takes_the_chosen_profile_then_the_projects_then_the_agents_default() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let start = |runtime: &Runtime, root: Option<&str>, chosen: Option<&str>| {
        runtime
            .new_start_profile(&node, Some("claude"), root, chosen)
            .map(|profile| profile.map(|(id, _)| id.0))
    };
    // No project choice: the agent's default on the machine (B28).
    assert_eq!(start(&runtime, Some(ROOT), None), Ok(Some("p-side".into())));
    project_default(&mut runtime, "p-work");
    // The project's choice wins over the agent's default (B15).
    assert_eq!(start(&runtime, Some(ROOT), None), Ok(Some("p-work".into())));
    // Another project keeps the agent's default.
    assert_eq!(
        start(&runtime, Some("/fixture/other"), None),
        Ok(Some("p-side".into()))
    );
    // The start panel's choice wins over both (B14), the built-in one too.
    assert_eq!(
        start(&runtime, Some(ROOT), Some("p-side")),
        Ok(Some("p-side".into()))
    );
    assert_eq!(start(&runtime, Some(ROOT), Some("default")), Ok(None));
    // A choice the machine does not list, or another agent's, is refused.
    assert!(start(&runtime, Some(ROOT), Some("p-gone")).is_err());
    assert!(start(&runtime, Some(ROOT), Some("p-team")).is_err());
    // A project that names the built-in default starts it, though the agent's
    // default is `side` (D-31).
    project_default(&mut runtime, "default");
    assert_eq!(start(&runtime, Some(ROOT), None), Ok(None));
    // A project's choice the machine no longer lists falls through to the
    // agent's default.
    project_default(&mut runtime, "p-gone");
    assert_eq!(start(&runtime, Some(ROOT), None), Ok(Some("p-side".into())));
    // A start of no agent has no profile.
    assert_eq!(
        runtime.new_start_profile(&node, None, Some(ROOT), None),
        Ok(None)
    );
}

#[test]
fn a_removal_is_refused_while_an_agent_runs_with_the_profile_and_names_the_projects_otherwise() {
    let mut runtime = listed();
    project_default(&mut runtime, "p-work");
    running(&mut runtime, "w1:p1", "p-work", "fix-bell");
    change(
        &mut runtime,
        "check-1",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    let checked = answer(&runtime);
    assert_eq!(checked.state, AnswerState::Done);
    assert_eq!(checked.in_use.len(), 1);
    // The answer rides the status the screen is sent, not only the snapshot.
    let rest = crate::model::RestSections::capture(&runtime.snapshot, None, 1);
    let sent = serde_json::to_value(&rest.status).unwrap();
    assert_eq!(sent["agent_profile_answer"]["request_id"], "check-1");
    assert_eq!(
        sent["agent_profile_answer"]["in_use"][0]["title"],
        "fix-bell"
    );
    assert_eq!(checked.in_use[0].title, "fix-bell");
    assert!(!checked.in_use[0].asleep);
    // The removal itself is refused the same way, before the node is asked.
    change(
        &mut runtime,
        "remove-1",
        serde_json::json!({"op": "remove", "id": "p-work"}),
    );
    let refused = answer(&runtime);
    assert_eq!(refused.state, AnswerState::Refused);
    assert_eq!(refused.code.as_deref(), Some(profiles::IN_USE));
    // With no agent on it, the check names the projects that go back to the
    // agent's default (B27).
    runtime.snapshot.navigator.agents.clear();
    change(
        &mut runtime,
        "check-2",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    let checked = answer(&runtime);
    assert!(checked.in_use.is_empty());
    assert_eq!(checked.projects, ["blog"]);
    // A sleeping agent counts as one that runs with it (D-11).
    let mut sleeping = project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id": "w1:p2", "agent": "claude", "agent_status": "idle", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0);
    sleeping.session_id = Some("s-1".into());
    let mut asleep = crate::agent_sleep::SleepRecord::ending(&sleeping, None, 0).unwrap();
    asleep.profile = Some(LaunchProfile {
        id: ProfileId("p-work".into()),
        name: "work".into(),
        account: AccountKey::Folder("/home/me/accounts/work".into()),
    });
    running(&mut runtime, "w1:p2", "p-other", "usage-store");
    runtime.snapshot.ui_state.pane_launches.remove("w1:p2");
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .insert("w1:p2".into(), asleep);
    change(
        &mut runtime,
        "check-3",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    let checked = answer(&runtime);
    assert_eq!(checked.in_use.len(), 1);
    assert!(checked.in_use[0].asleep);
}

#[test]
fn the_command_and_the_screen_refuse_the_same_profiles_by_the_same_codes() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let refused = |runtime: &mut Runtime, command: ProfileCommand| {
        let line = runtime
            .prepare_agent_profile_command(&node, command)
            .err()
            .expect("refused");
        crate::coordination::split_refusal(&line).0.to_owned()
    };
    let named = |name: &str| ProfileCommand::Remove {
        machine: None,
        kind: "claude".into(),
        name: name.into(),
    };
    // The built-in profile is neither edited nor removed (B28).
    assert_eq!(refused(&mut runtime, named("default")), profiles::BUILT_IN);
    // A name the machine does not list (B16).
    assert_eq!(refused(&mut runtime, named("missing")), profiles::GONE);
    // An agent Hide keeps no profiles for, and an account for one that
    // keeps none apart (D-14, D-25).
    let add = |kind: &str, account: Option<&str>| ProfileCommand::Add {
        machine: None,
        kind: kind.into(),
        name: "x".into(),
        account: account.map(str::to_owned),
        program: None,
        env: Vec::new(),
        default: false,
    };
    assert_eq!(
        refused(&mut runtime, add("cursor", None)),
        profiles::INVALID
    );
    assert_eq!(
        refused(&mut runtime, add("antigravity", Some("/home/me/agy"))),
        profiles::INVALID
    );
    // A variable that is not KEY=VALUE.
    let mut bad_env = add("claude", None);
    if let ProfileCommand::Add { env, .. } = &mut bad_env {
        env.push("NOEQUALS".into());
    }
    assert_eq!(refused(&mut runtime, bad_env), profiles::INVALID);
    // In use, by the command as on the screen.
    running(&mut runtime, "w1:p1", "p-work", "fix-bell");
    assert_eq!(refused(&mut runtime, named("work")), profiles::IN_USE);
    change(
        &mut runtime,
        "remove",
        serde_json::json!({"op": "remove", "id": "p-work"}),
    );
    assert_eq!(answer(&runtime).code.as_deref(), Some(profiles::IN_USE));
    change(
        &mut runtime,
        "builtin",
        serde_json::json!({"op": "remove", "id": "default"}),
    );
    assert_eq!(answer(&runtime).code.as_deref(), Some(profiles::BUILT_IN));
    // A machine that predates profiles keeps none to change (D-04).
    let mut kit = runtime.kit_state(&node);
    kit.profiles = Some(ProfilesSnapshot::unsupported());
    runtime.kit_states.insert(node.clone(), kit);
    assert_eq!(refused(&mut runtime, named("work")), profiles::UNSUPPORTED);
}

/// A start of Claude with `id` on the runtime's own machine, holding its
/// profile as every start path takes it.
fn hold(runtime: &Runtime, id: &str) -> Result<crate::launch::Request, crate::launch::Refusal> {
    let node = runtime.node.to_string();
    runtime.hold_profile_start(
        &node,
        crate::launch::Request::new("claude", Some((ProfileId(id.into()), id.into()))),
    )
}

/// What the removal's check finds on `p-work`: (starting, running) agents.
fn in_use(runtime: &mut Runtime) -> (usize, usize) {
    change(
        runtime,
        "check",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    let checked = answer(runtime);
    assert_eq!(checked.state, AnswerState::Done, "{checked:?}");
    let starting = checked.in_use.iter().filter(|agent| agent.starting).count();
    (starting, checked.in_use.len() - starting)
}

/// A finished removal job of `p-work` and what the node answered.
fn removal(
    runtime: &Runtime,
    result: Result<(), &'static str>,
) -> (profiles::Prepared, profiles::Outcome) {
    let node = runtime.node.to_string();
    let job = profiles::Prepared {
        machine: node,
        link: runtime.own_node(),
        change: profiles::Change::Remove {
            id: ProfileId("p-work".into()),
            projects: Vec::new(),
        },
        version: 3,
        kind: None,
        request: None,
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        settled: true,
    };
    let outcome = profiles::Outcome {
        list: None,
        result: result
            .map(|()| profiles::Done::default())
            .map_err(profiles::Refusal::code),
    };
    (job, outcome)
}

/// `node`'s list at `version`, with `p-work` as `work` says: listed and
/// being removed or not, or gone.
fn list_at(version: u64, work: Option<bool>) -> crate::runtime::kit::ProfilesRead {
    let profiles = work
        .map(|removing| {
            serde_json::json!([{"id": "p-work", "kind": "claude-code", "name": "work",
                "version": 1, "removing": removing}])
        })
        .unwrap_or_else(|| serde_json::json!([]));
    crate::runtime::kit::ProfilesRead::Listed(
        serde_json::from_value(serde_json::json!({"version": version, "profiles": profiles}))
            .unwrap(),
    )
}

#[test]
fn a_start_and_a_removal_of_one_profile_refuse_each_other_in_either_order() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    // A start resolving its profile: the removal's check lists it, with no
    // pane to show, and the removal is refused (B27).
    let start = hold(&runtime, "p-work").unwrap();
    assert_eq!(in_use(&mut runtime), (1, 0));
    change(
        &mut runtime,
        "remove",
        serde_json::json!({"op": "remove", "id": "p-work"}),
    );
    assert_eq!(answer(&runtime).code.as_deref(), Some(profiles::IN_USE));
    // Another profile is free, and the built-in default holds nothing.
    assert!(hold(&runtime, "p-side").is_ok());
    // A start that ended before anything was typed lets the profile go.
    drop(start);
    assert_eq!(in_use(&mut runtime), (0, 0));

    // The other order: a removal whose job is under way refuses a start.
    runtime
        .profile_holds
        .remove(&node, &ProfileId("p-work".into()), 3)
        .unwrap();
    assert_eq!(
        hold(&runtime, "p-work").unwrap_err().code,
        crate::launch::PROFILE_GONE
    );
    // A node that did not answer may still make it: still refused.
    let (job, outcome) = removal(&runtime, Err(profiles::UNANSWERED));
    runtime.settle_agent_profiles(&job, &outcome);
    assert!(hold(&runtime, "p-work").is_err());
    // A node that refused it leaves the profile to its starts.
    let (job, outcome) = removal(&runtime, Err(profiles::REFUSED));
    runtime.settle_agent_profiles(&job, &outcome);
    assert!(hold(&runtime, "p-work").is_ok());
}

#[test]
fn a_typed_start_holds_its_profile_until_its_agent_shows_and_frees_it_once_the_agent_ends() {
    let mut runtime = listed();
    let live: std::collections::HashSet<String> = ["w1:p1".to_owned()].into();
    let scope = crate::agent_state::ReadRecordScope::Local(&crate::herdr_import::Held::Nothing);
    let start = hold(&runtime, "p-work").unwrap();
    start.hold().unwrap().typed("w1:p1");
    drop(start);
    // Typed, with no record and no row yet: it may run, so it holds.
    runtime.apply_pane_launches(&mut [], scope, Some(&live));
    assert_eq!(in_use(&mut runtime), (1, 0));
    // Its record is written before its row shows: still held, never a gap.
    running(&mut runtime, "w1:p1", "p-work", "fix-bell");
    let rows = std::mem::take(&mut runtime.snapshot.navigator.agents);
    runtime.apply_pane_launches(&mut [], scope, Some(&live));
    assert_eq!(in_use(&mut runtime), (1, 0));
    // Its row shows, described by the record: the row carries the use.
    runtime.snapshot.navigator.agents = rows.clone();
    runtime.apply_pane_launches(&mut rows.clone(), scope, Some(&live));
    assert_eq!(in_use(&mut runtime), (0, 1));
    // The agent ends and its pane keeps the shell: the record stays, and
    // the profile is free to remove without closing the pane.
    runtime.snapshot.navigator.agents.clear();
    runtime.apply_pane_launches(&mut [], scope, Some(&live));
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_launches
            .contains_key("w1:p1")
    );
    assert_eq!(in_use(&mut runtime), (0, 0));
}

#[test]
fn an_unconfirmed_start_holds_its_profile_until_its_pane_is_seen_gone() {
    let mut runtime = listed();
    let local = crate::agent_state::ReadRecordScope::Local(&crate::herdr_import::Held::Nothing);
    let start = hold(&runtime, "p-work").unwrap();
    start.hold().unwrap().typed("w1:p2");
    drop(start);
    // A pass that does not hold the whole topology proves nothing gone.
    let none = std::collections::HashSet::new();
    runtime.apply_pane_launches(
        &mut [],
        crate::agent_state::ReadRecordScope::Retain,
        Some(&none),
    );
    runtime.apply_pane_launches(&mut [], local, None);
    assert_eq!(in_use(&mut runtime), (1, 0));
    let live: std::collections::HashSet<String> = ["w1:p2".to_owned()].into();
    runtime.apply_pane_launches(&mut [], local, Some(&live));
    assert_eq!(in_use(&mut runtime), (1, 0));
    // The machine's topology no longer has the pane: the start is over.
    runtime.apply_pane_launches(&mut [], local, Some(&none));
    assert_eq!(in_use(&mut runtime), (0, 0));
}

#[test]
fn a_removal_whose_answer_was_lost_refuses_starts_until_a_list_newer_than_it_settles_it() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let mark = |runtime: &Runtime| {
        runtime
            .profile_holds
            .remove(&node, &ProfileId("p-work".into()), 3)
            .unwrap()
    };
    mark(&runtime);
    let (job, outcome) = removal(&runtime, Err(profiles::UNANSWERED));
    runtime.settle_agent_profiles(&job, &outcome);
    // A list read before the change could apply, at the version it names.
    runtime.ingest_profiles(&node, list_at(3, Some(false)));
    assert!(hold(&runtime, "p-work").is_err());
    // The change applied late: being removed.
    runtime.ingest_profiles(&node, list_at(4, Some(true)));
    assert!(hold(&runtime, "p-work").is_err());
    // Its removal finished.
    runtime.ingest_profiles(&node, list_at(5, None));
    assert!(hold(&runtime, "p-work").is_ok());

    // A list read before the profile existed, answered late, says nothing of
    // a removal planned on a newer one: not listing the profile is settled
    // only by a list newer than the removal.
    mark(&runtime);
    runtime.ingest_profiles(&node, list_at(2, None));
    assert!(hold(&runtime, "p-work").is_err());
    runtime.ingest_profiles(&node, list_at(4, None));
    assert!(hold(&runtime, "p-work").is_ok());

    // A removal that never applied: a list past its version settles it, as
    // the change can no longer apply on that list.
    mark(&runtime);
    runtime.ingest_profiles(&node, list_at(4, Some(false)));
    assert!(hold(&runtime, "p-work").is_ok());
}

#[test]
fn starts_with_a_profile_take_only_launch_records_there_are_and_the_built_in_default_still_starts()
{
    let mut runtime = listed();
    let limit = crate::runtime::agent_choice::PANE_LAUNCH_LIMIT;
    let launch = |name: &str| crate::model::PaneLaunch {
        kind: "claude".into(),
        name: name.into(),
        choice: Default::default(),
        session: None,
        profile: None,
    };
    for n in 0..limit - 2 {
        runtime
            .snapshot
            .ui_state
            .pane_launches
            .insert(format!("w9:p{n}"), launch("x"));
    }
    // Two slots left: two starts planned together take them, a third is
    // refused before it runs.
    let first = hold(&runtime, "p-work").unwrap();
    let second = hold(&runtime, "p-side").unwrap();
    assert_eq!(
        hold(&runtime, "p-work").unwrap_err().code,
        crate::launch::LAUNCH_RECORDS_FULL
    );
    // A start on the built-in default is never refused, and takes no slot
    // a start with a profile holds: it is recorded nowhere.
    let node = runtime.node.to_string();
    assert!(
        runtime
            .hold_profile_start(&node, crate::launch::Request::new("claude", None))
            .is_ok()
    );
    runtime.record_pane_launch("w1:d1", "claude", "plain", &Default::default(), None, None);
    assert!(
        !runtime
            .snapshot
            .ui_state
            .pane_launches
            .contains_key("w1:d1")
    );
    // The held starts get their records.
    let profile = |id: &str| LaunchProfile {
        id: ProfileId(id.into()),
        name: id.into(),
        account: AccountKey::Folder(format!("/home/me/accounts/{id}")),
    };
    first.hold().unwrap().typed("w1:p1");
    second.hold().unwrap().typed("w1:p2");
    runtime.record_pane_launch(
        "w1:p1",
        "claude",
        "a",
        &Default::default(),
        Some(&profile("p-work")),
        None,
    );
    runtime.record_pane_launch(
        "w1:p2",
        "claude",
        "b",
        &Default::default(),
        Some(&profile("p-side")),
        None,
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_launches
            .contains_key("w1:p1")
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .pane_launches
            .contains_key("w1:p2")
    );
    assert_eq!(runtime.snapshot.ui_state.pane_launches.len(), limit);
}

#[test]
fn a_child_of_the_same_agent_takes_its_parents_account_only_from_a_record_that_describes_the_parent()
 {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let actor = |pane: &str, kind: &str, name: &str| crate::delivery::Actor {
        pane_id: pane.into(),
        name: name.into(),
        kind: kind.into(),
        device_id: node.clone(),
        session: None,
    };
    let child = |runtime: &Runtime, parent: &crate::delivery::Actor| {
        let profiles = runtime.machine_profiles(&node).cloned();
        let project = runtime.project_agent_profile(&node, ROOT, "claude");
        crate::launch::pick(
            "claude",
            None,
            &runtime.caller_launch_profile(parent, "claude"),
            project.as_ref(),
            profiles.as_ref(),
        )
        .profile()
        .map(|(id, _)| id.0)
    };
    // A parent Hide started on the built-in default is recorded so, and its
    // child starts the same, though the agent's default is `side` (D-32).
    runtime.record_pane_launch("w1:p1", "claude", "boss", &Default::default(), None, None);
    let boss = actor("w1:p1", "claude", "boss");
    assert_eq!(
        runtime.caller_launch_profile(&boss, "claude"),
        crate::launch::Parent::Default
    );
    assert_eq!(child(&runtime, &boss), None);
    // One Hide has no record of (started by hand, or recorded before this
    // rule) is not assumed to run on the default: its child picks as one
    // with no parent does, the project's choice then the agent's default.
    let unknown = actor("w1:p3", "claude", "by-hand");
    assert_eq!(
        runtime.caller_launch_profile(&unknown, "claude"),
        crate::launch::Parent::Unknown
    );
    assert_eq!(child(&runtime, &unknown).as_deref(), Some("p-side"));
    project_default(&mut runtime, "p-work");
    assert_eq!(child(&runtime, &unknown).as_deref(), Some("p-work"));
    // A record that describes another agent of the pane is no answer.
    let other = actor("w1:p1", "claude", "someone-else");
    assert_eq!(
        runtime.caller_launch_profile(&other, "claude"),
        crate::launch::Parent::Unknown
    );
    // Another agent hands down nothing.
    assert_eq!(
        runtime.caller_launch_profile(&actor("w1:p1", "codex", "boss"), "claude"),
        crate::launch::Parent::None
    );
}

/// A node that answers each call from a script, in order, and keeps what it
/// was asked.
struct Scripted {
    answers: Mutex<Vec<Result<serde_json::Value, hide_node_link::ErrorCode>>>,
    asked: Mutex<Vec<hide_node_link::protocol::Call>>,
}

impl crate::node_access::NodeLink for Scripted {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        _: std::time::Duration,
    ) -> Result<hide_node_link::LinkAnswer, hide_node_link::LinkError> {
        self.asked.lock().unwrap().push(call);
        match self.answers.lock().unwrap().remove(0) {
            Ok(value) => Ok(value.into()),
            Err(code) => Err(hide_node_link::LinkError::Refused(
                hide_node_link::HostError::new(code, "refused"),
            )),
        }
    }
}

#[test]
fn a_removal_made_again_on_a_moved_list_is_refused_for_an_agent_that_started_with_the_profile_meanwhile()
 {
    let shared = SharedRuntime::new(listed());
    let node = shared.lock().unwrap().node.to_string();
    // The removal was planned on version 3 with nothing on the profile.
    let link = Arc::new(Scripted {
        answers: Mutex::new(vec![
            Err(hide_node_link::ErrorCode::Conflict),
            Ok(serde_json::json!({"version": 4, "profiles": [
                {"id": "p-work", "kind": "claude-code", "name": "work", "version": 1}
            ]})),
        ]),
        asked: Mutex::new(Vec::new()),
    });
    let job = {
        let mut runtime = shared.lock().unwrap();
        runtime
            .profile_holds
            .remove(&node, &ProfileId("p-work".into()), 3)
            .unwrap();
        // Meanwhile a list past the removal ended its mark, an agent started
        // with the profile, and its row carries the use: no start holds it.
        runtime.ingest_profiles(&node, list_at(4, Some(false)));
        let start = hold(&runtime, "p-work").unwrap();
        running(&mut runtime, "w1:p1", "p-work", "fix-bell");
        drop(start);
        assert!(
            !runtime
                .profile_holds
                .is_removing(&node, &ProfileId("p-work".into()))
        );
        assert!(
            runtime
                .profile_holds
                .starting(&node, &ProfileId("p-work".into()))
                .is_empty()
        );
        profiles::Prepared {
            machine: node.clone(),
            link: Arc::clone(&link) as Arc<dyn crate::node_access::NodeLink>,
            change: profiles::Change::Remove {
                id: ProfileId("p-work".into()),
                projects: Vec::new(),
            },
            version: 3,
            kind: None,
            request: Some("delete".into()),
            runtime: shared.weak(),
            notifier: crate::handle::ChangeNotifier::noop(),
            settled: false,
        }
    };
    // The node refuses the stale version; the removal made again is checked
    // as its plan was, and refused without asking the node again.
    let line = job.run().unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&line).0,
        profiles::IN_USE,
        "{line}"
    );
    let asked = link.asked.lock().unwrap();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(matches!(
        asked[1],
        hide_node_link::protocol::Call::ProfileList { .. }
    ));
    drop(asked);
    // The screen's answer lists the agent the delete would pull the account
    // from; the agent keeps its profile, and the runtime is free for the
    // next change.
    let runtime = shared.lock().unwrap();
    let answered = answer(&runtime);
    assert_eq!(answered.code.as_deref(), Some(profiles::IN_USE));
    assert_eq!(
        answered
            .in_use
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent.starting))
            .collect::<Vec<_>>(),
        [("w1:p1", false)]
    );
    assert!(hold(&runtime, "p-work").is_ok());
}

/// D-30, D-32, B14: a project's default whose removal its machine is
/// making (an answer that was lost leaves the project naming it) is passed
/// over by a new start, as the start panel preselects it from the same
/// judgment the snapshot carries, while a start that chose it is refused.
#[test]
fn a_project_default_being_removed_is_passed_over_by_the_start_and_the_screen_alike() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let listed_profile = |id: &str, name: &str, removing: bool| {
        hide_node_link::profiles::ListedProfile {
            id: ProfileId(id.into()),
            kind: "claude-code".into(),
            name: name.into(),
            account: Some(hide_agent_adapter::AccountRef::Folder(format!(
                "/home/me/accounts/{name}"
            ))),
            program: None,
            env_keys: Vec::new(),
            env: None,
            version: 1,
            removing,
            place: None,
            login: None,
            account_variables: Vec::new(),
            account_key: Some(AccountKey::Folder(format!("/home/me/accounts/{name}"))),
        }
    };
    let list = hide_node_link::profiles::ProfileList {
        version: 4,
        profiles: vec![
            listed_profile("p-work", "work", true),
            listed_profile("p-side", "side", false),
        ],
        defaults: [("claude-code".to_owned(), ProfileId("p-side".into()))].into(),
        unjudged: None,
    };
    runtime.ingest_profiles(&node, crate::runtime::kit::ProfilesRead::Listed(list));
    project_default(&mut runtime, "p-work");
    let start = runtime
        .new_start_profile(&node, Some("claude"), Some(ROOT), None)
        .map(|profile| profile.map(|(id, _)| id.0));
    assert_eq!(start, Ok(Some("p-side".into())));
    let shown = serde_json::to_value(runtime.machine_profiles(&node).unwrap()).unwrap();
    let startable = |id: &str| {
        shown["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == id)
            .map(|profile| profile["startable"].clone())
    };
    assert_eq!(startable("p-work"), Some(serde_json::json!(false)));
    assert_eq!(startable("p-side"), Some(serde_json::json!(true)));
    assert!(
        runtime
            .new_start_profile(&node, Some("claude"), Some(ROOT), Some("p-work"))
            .is_err(),
        "a start that chose it is refused"
    );
}
