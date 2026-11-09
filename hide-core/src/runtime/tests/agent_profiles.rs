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
