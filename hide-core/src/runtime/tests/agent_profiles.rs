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
            pending: false,
            start: 0,
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
    assert!(!checked.hide_ai, "Hide AI names no profile yet");
    // Hide AI's fallback list naming it is named too, and the delete leaves
    // it there as a missing profile (PRD agent-profiles-phase2 B22).
    let mut settings = hide_ai::AiSettings::default();
    settings
        .add_fallback(
            hide_ai::Item::of(
                hide_ai::ProviderId::CLAUDE,
                Some(ProfileId("p-work".into())),
            ),
            None,
        )
        .unwrap();
    runtime.ingest_ai_settings(settings, None);
    change(
        &mut runtime,
        "check-3",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    assert!(answer(&runtime).hide_ai);
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

/// A delete check names how many of Settings › Mission's candidates name
/// the profile, beside its projects (PRD agent-profiles-phase2 B22, D-16):
/// the rows of that profile, whatever their model, and no row of the
/// built-in default or of another profile; a profile no row names says
/// nothing of candidates.
#[test]
fn a_delete_check_counts_the_mission_candidates_that_name_the_profile() {
    let mut runtime = listed();
    let row =
        |agent: &str, profile: Option<&str>, model: Option<&str>| crate::missions::Candidate {
            agent: agent.into(),
            profile: profile.map(|id| ProfileId(id.into())),
            model: model.map(str::to_owned),
            effort: None,
            purpose: String::new(),
        };
    runtime.snapshot.ui_state.conductor_settings.candidates = vec![
        row("claude", Some("p-work"), None),
        row("claude", Some("p-work"), Some("opus")),
        row("claude", None, None),
        row("codex", Some("p-team"), None),
    ];
    change(
        &mut runtime,
        "check-1",
        serde_json::json!({"op": "remove_check", "id": "p-work"}),
    );
    assert_eq!(answer(&runtime).candidates, 2);
    assert_eq!(
        serde_json::to_value(answer(&runtime)).unwrap()["candidates"],
        2
    );
    change(
        &mut runtime,
        "check-2",
        serde_json::json!({"op": "remove_check", "id": "p-side"}),
    );
    assert_eq!(answer(&runtime).candidates, 0);
    assert!(
        serde_json::to_value(answer(&runtime))
            .unwrap()
            .get("candidates")
            .is_none()
    );
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
fn hold(runtime: &mut Runtime, id: &str) -> Result<crate::launch::Request, crate::launch::Refusal> {
    let node = runtime.node.to_string();
    runtime.hold_profile_start(
        &node,
        crate::launch::Request::new("claude", Some((ProfileId(id.into()), id.into()))),
    )
}

/// The worker submits the resolved profile and completes its attempt. The
/// guard hands it off on success, unknown, and unwind alike.
fn submitted(
    request: &crate::launch::Request,
    pane: &str,
    id: &str,
) -> crate::launch::holds::Finish {
    let finish = crate::launch::holds::Finish::new(request.hold());
    let launch = crate::engine::AgentLaunch {
        pane_id: pane.into(),
        name: format!("{pane}-agent"),
        launch: crate::launch::Launch::answered(
            "claude",
            id,
            hide_node_link::profiles::ProfileStart {
                name: id.into(),
                account: AccountKey::Folder(format!(
                    "/home/me/accounts/{}",
                    id.trim_start_matches("p-")
                )),
                version: 1,
                env: Vec::new(),
                program: None,
                exec: "/opt/hide".into(),
            },
        ),
        args: Vec::new(),
        codex_daemon: Default::default(),
        wakes: false,
        program: None,
    };
    finish.submitted(&launch);
    finish
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
    let start = hold(&mut runtime, "p-work").unwrap();
    assert_eq!(in_use(&mut runtime), (1, 0));
    change(
        &mut runtime,
        "remove",
        serde_json::json!({"op": "remove", "id": "p-work"}),
    );
    assert_eq!(answer(&runtime).code.as_deref(), Some(profiles::IN_USE));
    // Another profile is free, and the built-in default holds nothing.
    assert!(hold(&mut runtime, "p-side").is_ok());
    // A start that ended before anything was typed lets the profile go.
    drop(start);
    assert_eq!(in_use(&mut runtime), (0, 0));

    // The other order: a removal whose job is under way refuses a start.
    runtime
        .profile_holds
        .remove(&node, &ProfileId("p-work".into()), 3)
        .unwrap();
    assert_eq!(
        hold(&mut runtime, "p-work").unwrap_err().code,
        crate::launch::PROFILE_GONE
    );
    // A node that did not answer may still make it: still refused.
    let (job, outcome) = removal(&runtime, Err(profiles::UNANSWERED));
    runtime.settle_agent_profiles(&job, &outcome);
    assert!(hold(&mut runtime, "p-work").is_err());
    // A node that refused it leaves the profile to its starts.
    let (job, outcome) = removal(&runtime, Err(profiles::REFUSED));
    runtime.settle_agent_profiles(&job, &outcome);
    assert!(hold(&mut runtime, "p-work").is_ok());
}

#[test]
fn a_completed_start_transfers_to_a_record_before_the_next_removal_check() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    let finish = submitted(&start, "w1:p1", "p-work");
    assert_eq!(in_use(&mut runtime), (1, 0));
    drop(finish);
    // No publish or row is needed: removal admission drains completion.
    assert_eq!(in_use(&mut runtime), (0, 1));
    assert_eq!(runtime.profile_holds.unrecorded(), 0);
    assert!(runtime.snapshot.ui_state.pane_launches["w1:p1"].pending);
    // A matching agent takes over the record, then can finish in its pane.
    let record = runtime.snapshot.ui_state.pane_launches["w1:p1"].clone();
    running(&mut runtime, "w1:p1", "p-work", "fix-bell");
    runtime
        .snapshot
        .ui_state
        .pane_launches
        .insert("w1:p1".into(), record);
    let mut rows = runtime.snapshot.navigator.agents.clone();
    let live = ["w1:p1".to_owned()].into();
    runtime.apply_pane_launches(
        &mut rows,
        crate::agent_state::ReadRecordScope::Local(&crate::herdr_import::Held::Nothing),
        Some(&live),
    );
    assert_eq!(in_use(&mut runtime), (0, 1));
    runtime.snapshot.navigator.agents.clear();
    assert_eq!(in_use(&mut runtime), (0, 0));
}

#[test]
fn an_unknown_result_keeps_its_record_until_its_pane_is_seen_gone() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    drop(submitted(&start, "w1:p2", "p-work"));
    assert_eq!(in_use(&mut runtime), (0, 1));
    let none = std::collections::HashSet::new();
    runtime.apply_pane_launches(
        &mut [],
        crate::agent_state::ReadRecordScope::Retain,
        Some(&none),
    );
    assert_eq!(in_use(&mut runtime), (0, 1));
    runtime.apply_pane_launches(
        &mut [],
        crate::agent_state::ReadRecordScope::Local(&crate::herdr_import::Held::Nothing),
        Some(&none),
    );
    assert_eq!(in_use(&mut runtime), (0, 0));
}

#[test]
fn a_definite_non_start_releases_even_while_another_clone_survives() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    let finish = submitted(&start, "w1:p1", "p-work");
    finish.not_started();
    drop(finish);
    assert_eq!(in_use(&mut runtime), (0, 0));
    assert!(
        !runtime
            .snapshot
            .ui_state
            .pane_launches
            .contains_key("w1:p1")
    );
}

#[test]
fn a_start_unwinding_after_submission_hands_off_its_record_and_a_removed_device_releases_it() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _finish = submitted(&start, "w1:p1", "p-work");
        panic!("worker unwound after submission");
    }));
    assert_eq!(in_use(&mut runtime), (0, 1));
    let holds = runtime.profile_holds.clone();
    let remote = holds
        .hold(
            "device-b",
            &ProfileId("p-work".into()),
            "claude",
            0,
            8,
            &Default::default(),
        )
        .unwrap();
    holds.forget("device-b");
    assert!(
        holds
            .starting("device-b", &ProfileId("p-work".into()))
            .is_empty()
    );
    drop(remote);
    assert_eq!(
        in_use(&mut runtime),
        (0, 1),
        "the other machine's use stays"
    );
}

#[test]
fn a_late_result_cannot_replace_a_newer_start_or_its_bound_session() {
    let mut runtime = listed();
    let first = hold(&mut runtime, "p-work").unwrap();
    let old = submitted(&first, "w1:p1", "p-work");
    let next = hold(&mut runtime, "p-side").unwrap();
    drop(submitted(&next, "w1:p1", "p-side"));
    runtime.finish_profile_starts();
    runtime
        .snapshot
        .ui_state
        .pane_launches
        .get_mut("w1:p1")
        .unwrap()
        .session = Some("new-session".into());
    drop(old);
    runtime.finish_profile_starts();
    let record = &runtime.snapshot.ui_state.pane_launches["w1:p1"];
    assert_eq!(record.profile.as_ref().unwrap().id.0, "p-side");
    assert_eq!(record.session.as_deref(), Some("new-session"));
}

#[test]
fn an_observed_session_before_completion_stays_observed_when_the_result_arrives() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    let finish = submitted(&start, "w1:p1", "p-work");
    running(&mut runtime, "w1:p1", "p-work", "working");
    runtime.snapshot.ui_state.pane_launches.clear();
    runtime.snapshot.navigator.agents[0].session_id = Some("observed-session".into());
    drop(finish);
    runtime.finish_profile_starts();
    let record = &runtime.snapshot.ui_state.pane_launches["w1:p1"];
    assert!(!record.pending);
    assert_eq!(record.session.as_deref(), Some("observed-session"));
}

#[test]
fn a_late_profile_result_cannot_replace_the_next_built_in_default_start() {
    let mut runtime = listed();
    let start = hold(&mut runtime, "p-work").unwrap();
    let finish = submitted(&start, "w1:p1", "p-work");
    runtime.record_pane_launch(
        "w1:p1",
        "claude",
        "next-agent",
        &Default::default(),
        None,
        Some("default-session"),
    );
    drop(finish);
    runtime.finish_profile_starts();
    let record = &runtime.snapshot.ui_state.pane_launches["w1:p1"];
    assert!(record.profile.is_none());
    assert_eq!(record.name, "next-agent");
    assert_eq!(record.session.as_deref(), Some("default-session"));
}

#[test]
fn a_finished_wrapper_start_frees_the_profile_it_did_not_run_in() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    for profile in runtime
        .kit_states
        .get_mut(&node)
        .unwrap()
        .profiles
        .as_mut()
        .unwrap()
        .profiles
        .iter_mut()
    {
        if let Some(folder) = &profile.account {
            profile.account_variables = vec![("CLAUDE_CONFIG_DIR".into(), folder.clone())];
        }
    }
    let start = hold(&mut runtime, "p-work").unwrap();
    drop(submitted(&start, "w1:p1", "p-work"));
    running(&mut runtime, "w1:p1", "p-work", "wrapper");
    runtime
        .ingest_hook_status(
            &node,
            "w1:p1",
            &hide_node_link::panes::HookStatusReport {
                runtime: "claude-code".into(),
                version: hide_agent_hooks::HOOK_VERSION,
                session: Some("wrapper-session".into()),
                turn: None,
                counts: None,
                session_start: true,
                waiting: None,
                account: Some(vec![(
                    "CLAUDE_CONFIG_DIR".into(),
                    "/home/me/accounts/side".into(),
                )]),
                refused: None,
            },
        )
        .unwrap();
    let mut rows = runtime.snapshot.navigator.agents.clone();
    let live = ["w1:p1".to_owned()].into();
    runtime.apply_pane_launches(
        &mut rows,
        crate::agent_state::ReadRecordScope::Local(&crate::herdr_import::Held::Nothing),
        Some(&live),
    );
    assert_eq!(
        in_use(&mut runtime),
        (0, 0),
        "the wrapper now runs in side, so work is free"
    );
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
    assert!(hold(&mut runtime, "p-work").is_err());
    // The change applied late: being removed.
    runtime.ingest_profiles(&node, list_at(4, Some(true)));
    assert!(hold(&mut runtime, "p-work").is_err());
    // Its removal finished.
    runtime.ingest_profiles(&node, list_at(5, None));
    assert!(hold(&mut runtime, "p-work").is_ok());

    // A list read before the profile existed, answered late, says nothing of
    // a removal planned on a newer one: not listing the profile is settled
    // only by a list newer than the removal.
    mark(&runtime);
    runtime.ingest_profiles(&node, list_at(2, None));
    assert!(hold(&mut runtime, "p-work").is_err());
    runtime.ingest_profiles(&node, list_at(4, None));
    assert!(hold(&mut runtime, "p-work").is_ok());

    // A removal that never applied: a list past its version settles it, as
    // the change can no longer apply on that list.
    mark(&runtime);
    runtime.ingest_profiles(&node, list_at(4, Some(false)));
    assert!(hold(&mut runtime, "p-work").is_ok());
}

#[test]
fn starts_with_a_profile_take_only_launch_records_there_are_and_the_built_in_default_still_starts()
{
    let mut runtime = listed();
    let limit = crate::runtime::agent_choice::PANE_LAUNCH_LIMIT;
    let launch = |name: &str| crate::model::PaneLaunch {
        pending: false,
        start: 0,
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
    let first = hold(&mut runtime, "p-work").unwrap();
    let second = hold(&mut runtime, "p-side").unwrap();
    assert_eq!(
        hold(&mut runtime, "p-work").unwrap_err().code,
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
    drop(submitted(&first, "w1:p1", "p-work"));
    drop(submitted(&second, "w1:p2", "p-side"));
    runtime.finish_profile_starts();
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

#[test]
fn a_project_that_preselects_work_starts_the_default_account_when_the_default_is_named_and_work_when_nothing_is()
 {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    project_default(&mut runtime, "p-work");
    let profiles = runtime.machine_profiles(&node).cloned();
    let project = runtime.project_agent_profile(&node, ROOT, "claude");
    let pick = |chosen: Option<&str>, parent: crate::launch::Parent| {
        let chosen = chosen
            .map(|id| crate::launch::chosen_id("claude", id, profiles.as_ref()))
            .transpose()
            .unwrap();
        crate::launch::pick(
            "claude",
            chosen,
            &parent,
            project.as_ref(),
            profiles.as_ref(),
        )
        .profile()
        .map(|(id, _)| id.0)
    };
    // The start panel's `Claude Code` names the built-in default: the account
    // the project preselected for an unchosen start does not take its place.
    assert_eq!(pick(Some("default"), crate::launch::Parent::None), None);
    // `hide agent spawn` with no `--profile` and no same-kind parent takes the
    // project's preselection.
    assert_eq!(
        pick(None, crate::launch::Parent::None).as_deref(),
        Some("p-work")
    );
    // A same-kind child takes its parent's item, whatever the project chose.
    assert_eq!(pick(None, crate::launch::Parent::Default), None);
    let side = crate::launch::LaunchProfile {
        id: ProfileId("p-side".into()),
        name: "side".into(),
        account: AccountKey::Folder("/home/me/accounts/side".into()),
    };
    assert_eq!(
        pick(None, crate::launch::Parent::Profile(side)).as_deref(),
        Some("p-side")
    );
}

#[test]
fn the_panel_shows_the_choice_it_was_sent_while_a_start_naming_nothing_picks_again_when_it_starts()
{
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let mut project = workspace("project", "blog", ROOT, Vec::new());
    project.device_id = node.clone();
    project.path = ROOT.to_owned();
    runtime.snapshot.navigator.workspaces = vec![project];
    project_default(&mut runtime, "p-work");
    runtime.refresh_start_profile_catalog();
    let shown = |runtime: &Runtime| {
        runtime.snapshot.status.start_profiles["project"].profiles["claude-code"].clone()
    };
    assert_eq!(shown(&runtime), "p-work");
    // The project's choice moves in another window before the panel's start
    // reaches the core, and the panel has not yet read it: a start that names
    // nothing takes the new choice, one that names the shown item the shown one.
    project_default(&mut runtime, "p-side");
    assert_eq!(shown(&runtime), "p-work");
    let start = |chosen: Option<&str>| {
        runtime
            .new_start_profile(&node, Some("claude"), Some(ROOT), chosen)
            .map(|profile| profile.map(|(id, _)| id.0))
    };
    assert_eq!(start(None), Ok(Some("p-side".into())));
    assert_eq!(start(Some("p-work")), Ok(Some("p-work".into())));
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
        let start = hold(&mut runtime, "p-work").unwrap();
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
    let mut runtime = shared.lock().unwrap();
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
    assert!(hold(&mut runtime, "p-work").is_ok());
}

/// D-30, D-32, B14: a project's default whose removal its machine is
/// making (an answer that was lost leaves the project naming it) is passed
/// over by a new start, as the start panel preselects it from the same
/// judgment the snapshot carries, while a start that chose it is refused.
#[test]
fn a_project_default_being_removed_is_passed_over_by_the_start_and_the_screen_alike() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let listed_profile =
        |id: &str, name: &str, removing: bool| hide_node_link::profiles::ListedProfile {
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
    let mut project = workspace("project", "blog", ROOT, Vec::new());
    project.device_id = node.clone();
    project.path = ROOT.to_owned();
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.refresh_start_profile_catalog();
    assert_eq!(
        runtime.snapshot.status.start_profiles["project"].profiles["claude-code"],
        "p-side"
    );
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

/// A change of project, machine, agent or profile state must reach the
/// preselection before the next start panel reads it.
#[test]
fn automatic_start_choices_follow_their_inputs_and_forget_old_projects() {
    let mut runtime = listed();
    let node = runtime.node.to_string();
    let mut project = workspace("project", "blog", ROOT, Vec::new());
    project.device_id = node.clone();
    project.path = ROOT.to_owned();
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.refresh_start_profile_catalog();
    let picked = |runtime: &Runtime, kind: &str| {
        runtime.snapshot.status.start_profiles["project"].profiles[kind].clone()
    };
    assert_eq!(picked(&runtime, "claude-code"), "p-side");
    assert_eq!(picked(&runtime, "codex"), "default");
    project_default(&mut runtime, "default");
    // Exercise the same settings-change hook as a persisted project edit.
    let record = runtime
        .snapshot
        .ui_state
        .project_settings
        .values()
        .next()
        .unwrap()
        .clone();
    runtime.project_settings_changed(None, &record);
    assert_eq!(picked(&runtime, "claude-code"), "default");
    project_default(&mut runtime, "p-work");
    let record = runtime
        .snapshot
        .ui_state
        .project_settings
        .values()
        .next()
        .unwrap()
        .clone();
    runtime.project_settings_changed(None, &record);
    assert_eq!(picked(&runtime, "claude-code"), "p-work");
    let mut kit = runtime.kit_state(&node);
    kit.profiles.as_mut().unwrap().profiles[0].install =
        Some(crate::model::ProfileInstall::RemoveFailed);
    kit.profiles.as_mut().unwrap().profiles[0].startable = false;
    runtime.set_kit_state(&node, kit);
    assert_eq!(picked(&runtime, "claude-code"), "p-side");
    runtime.snapshot.navigator.workspaces[0].device_id = "other".to_owned();
    runtime.refresh_start_profile_catalog();
    assert_eq!(picked(&runtime, "claude-code"), "default");
    runtime.snapshot.navigator.workspaces.clear();
    runtime.refresh_start_profile_catalog();
    assert!(runtime.snapshot.status.start_profiles.is_empty());
}

/// The share of the weekly window `used` of an account, open for three
/// more days.
fn weekly_reading(
    used_percent: f64,
) -> crate::usage::model::Cell<crate::usage::model::LimitReading> {
    use crate::usage::model::*;
    Cell::Reading {
        value: LimitReading {
            windows: vec![WindowReading {
                kind: WindowKind::Weekly,
                scope: Scope::All,
                outcome: WindowOutcome::Open {
                    used_percent,
                    resets_at_s: unix_milliseconds() / 1000 + 3 * 24 * 3_600,
                },
            }],
        },
        freshness: Freshness::Fresh,
        observed_at_ms: 0,
    }
}

/// PRD agent-profiles-phase2 B11, D-09, B9, D-11: `hide agent catalog`
/// lists every agent item a start can take, its built-in default then each
/// startable profile by its `--profile` name, with the models its own
/// environment listed (the agent's default list, marked, when its read
/// failed), and its own account's weekly figure; a profile the read has not
/// reached yet is unread, and the model read asks for every such profile.
#[test]
fn the_catalog_lists_each_profile_as_an_item_with_its_own_models_and_its_accounts_usage() {
    use crate::model::BackgroundAiProfileModels;
    use crate::usage::model::{AgentUsage, Cell, ProfileUsage};
    use hide_node_link::profiles::ProfileLogin;
    use serde_json::json;
    use std::collections::BTreeMap;

    let mut runtime = listed();
    let node = runtime.node.to_string();
    {
        let profiles = runtime
            .kit_states
            .get_mut(&node)
            .unwrap()
            .profiles
            .as_mut()
            .unwrap();
        profiles.profiles[0].login = Some(ProfileLogin::SignedIn);
        profiles.profiles[1].login = Some(ProfileLogin::SignedOut);
        // A gateway: the default account through a wrapper of its own.
        let mut gateway = profile("p-gw", "claude-code", "gw");
        gateway.account = None;
        gateway.program = Some("/home/me/bin/claude-gw".to_owned());
        gateway.account_key = Some(AccountKey::Default);
        // Another profile in `work`'s account, and one being removed.
        let mut twin = profile("p-twin", "claude-code", "twin");
        twin.account_key = profiles.profiles[0].account_key.clone();
        let mut leaving = profile("p-old", "claude-code", "old");
        leaving.startable = false;
        profiles.profiles.splice(2..2, [gateway, twin, leaving]);
    }
    let asked: Vec<(&str, String)> = runtime
        .ai_request()
        .items
        .iter()
        .map(|item| (item.provider.as_str(), item.profile.0.clone()))
        .collect();
    assert_eq!(
        asked,
        [
            ("claude", "p-work".to_owned()),
            ("claude", "p-side".to_owned()),
            ("claude", "p-gw".to_owned()),
            ("claude", "p-twin".to_owned()),
            ("codex", "p-team".to_owned()),
        ],
        "every startable profile's list is read, in the list's order"
    );

    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    let mut read = super::agents_settings_remote::ready_read(&["claude", "codex"]);
    let claude = read
        .providers
        .iter_mut()
        .find(|row| row.id == "claude")
        .unwrap();
    claude.model_labels = BTreeMap::from([("m-1".to_owned(), "Model One".to_owned())]);
    claude.profile_models = vec![
        BackgroundAiProfileModels {
            profile: "p-work".to_owned(),
            version: 1,
            models: vec!["opus".to_owned()],
            model_labels: BTreeMap::from([("opus".to_owned(), "glm-4.6".to_owned())]),
            ..BackgroundAiProfileModels::default()
        },
        BackgroundAiProfileModels {
            profile: "p-side".to_owned(),
            version: 1,
            models_unavailable_reason: Some("models_not_read".to_owned()),
            ..BackgroundAiProfileModels::default()
        },
    ];
    runtime.ingest_background_ai(read);
    runtime.ingest_usage(vec![AgentUsage {
        agent: "claude".to_owned(),
        limits: weekly_reading(30.0),
        consumption: Cell::Unsupported,
        profiles: vec![
            ProfileUsage {
                id: "p-work".to_owned(),
                shared: false,
                limits: Some(weekly_reading(95.0)),
            },
            ProfileUsage {
                id: "p-twin".to_owned(),
                shared: true,
                limits: None,
            },
        ],
    }]);

    let catalog = runtime.agent_catalog(&node, None, false).unwrap();
    let items = catalog["items"].as_array().unwrap();
    let rows_of = |id: &str| -> Vec<&str> {
        items
            .iter()
            .filter(|item| item["id"] == id)
            .map(|item| item["profile"].as_str().unwrap())
            .collect()
    };
    assert_eq!(
        rows_of("claude-code"),
        ["default", "work", "side", "gw", "twin"]
    );
    assert_eq!(rows_of("codex"), ["default", "team"]);
    let row = |id: &str, profile: &str| {
        items
            .iter()
            .find(|item| item["id"] == id && item["profile"] == profile)
            .cloned()
            .unwrap_or_else(|| panic!("no {id} {profile} in {catalog}"))
    };

    let default = row("claude-code", "default");
    assert_eq!(default["models"], json!(["m-1", "m-2"]));
    assert_eq!(default["model_labels"], json!({"m-1": "Model One"}));
    assert_eq!(default["models_from_default"], false);

    let work = row("claude-code", "work");
    assert_eq!(work["models"], json!(["opus"]), "a start passes the value");
    assert_eq!(work["model_labels"], json!({"opus": "glm-4.6"}));
    assert_eq!(work["models_from_default"], false);
    assert_eq!(work["signed_in"], true);
    assert_eq!(work["weekly_used_percent"], 95.0);
    assert!(work["weekly_resets_at_ms"].is_u64());
    assert_eq!(work["past_usage_limit"], true, "past the 90% limit");

    let side = row("claude-code", "side");
    assert_eq!(
        side["models"],
        json!(["m-1", "m-2"]),
        "a list that could not be read is the agent's"
    );
    assert_eq!(side["models_from_default"], true);
    assert!(side["models_unavailable_reason"].is_null());
    assert_eq!(side["signed_in"], false);
    assert!(
        side["weekly_used_percent"].is_null(),
        "its account has no reading"
    );

    let twin = row("claude-code", "twin");
    assert_eq!(
        twin["weekly_used_percent"], 95.0,
        "a profile in another's account reads that account's figure"
    );

    let gateway = row("claude-code", "gw");
    assert!(gateway["models"].is_null());
    assert_eq!(gateway["models_unavailable_reason"], "models_unread");
    assert!(
        gateway["weekly_used_percent"].is_null(),
        "a wrapper may run the default account elsewhere"
    );
    assert_eq!(gateway["past_usage_limit"], false);
    assert_eq!(
        row("codex", "team")["models_unavailable_reason"],
        "models_unread"
    );
    assert_eq!(catalog["models_read_pending"], true);

    // `work` is edited: the list read before the edit is not its list, and
    // a read is asked for again.
    runtime
        .kit_states
        .get_mut(&node)
        .unwrap()
        .profiles
        .as_mut()
        .unwrap()
        .profiles[0]
        .version = 2;
    let edited = runtime.agent_catalog(&node, None, false).unwrap();
    let edited_work = edited["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["profile"] == "work")
        .unwrap();
    assert!(edited_work["models"].is_null(), "{edited_work}");
    assert_eq!(edited_work["models_unavailable_reason"], "models_unread");
    assert_eq!(edited["models_read_pending"], true);

    let late = runtime.agent_catalog(&node, None, true).unwrap();
    assert_eq!(late["models_read_pending"], false);
    let late_gateway = late["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["profile"] == "gw")
        .unwrap();
    assert_eq!(
        late_gateway["models_unavailable_reason"],
        "models_read_timeout"
    );
    assert_eq!(
        late_gateway["cli_default_model"],
        hide_ai::ProviderId::CLAUDE.default_model()
    );
}
