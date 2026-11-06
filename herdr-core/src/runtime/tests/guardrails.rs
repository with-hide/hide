//! Project Settings protections and the policy each machine's hooks read
//! (PRD agent-guardrails B2 to B4, B11, B16, B17, B21, B30, B31).

use super::*;
use crate::project_settings::WriteState;
use hide_agent_hooks::guardrails::{Policy, ProjectPolicy, Rules, read_policy};

const ROOT: &str = "/fixture/project";

fn registration(device: &str, path: &str) -> WorkspaceRegistration {
    WorkspaceRegistration {
        primary_checkout_id: None,
        id: format!("{device}:{path}"),
        label: "Project".into(),
        path: path.into(),
        device_id: device.into(),
        pinned: false,
        home: false,
    }
}

/// A registered Git project of this machine whose remote names `main`.
fn fixture() -> Runtime {
    let mut runtime = runtime();
    let mut project = workspace(
        "visible-id",
        "Project",
        ROOT,
        vec![checkout("visible-id", "main", ROOT, None)],
    );
    project.is_git = true;
    project.default_branch = Some("main".into());
    project.branches = vec!["main".into(), "develop".into()];
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.ui_state.workspace_registrations =
        vec![registration(crate::node::TEST_NODE, ROOT)];
    runtime
}

/// The save waits for its delivery; nothing here runs the delivery itself.
fn held(mut runtime: Runtime) -> Runtime {
    runtime.worker_context = Some(RuntimeWorkerContext {
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
    });
    runtime
}

fn change(runtime: &mut Runtime, request: &str, patch: serde_json::Value) -> bool {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "project_settings_set", "payload": {
                "device_id": crate::node::TEST_NODE, "root": ROOT, "request_id": request, "patch": patch
            }
        }))
        .unwrap(),
    )
}

fn id() -> String {
    crate::project_settings::key(crate::node::TEST_NODE, ROOT)
}

fn receipt(runtime: &Runtime) -> (WriteState, Option<String>) {
    let receipt = &runtime.snapshot.ui_state.project_settings_writes[&id()];
    (receipt.state, receipt.error.clone())
}

fn files_on() -> serde_json::Value {
    serde_json::json!({"guardrails": {
        "worktree_folder": false, "base_branch": false, "agent_merge": false,
        "git_commands": false, "files": true,
        "paths": [".env", ".env.*", "*.pem", "*.key", "~/.ssh/"],
    }})
}

fn files_rules() -> Rules {
    Rules {
        files: true,
        ..Rules::default()
    }
}

#[test]
fn a_protection_is_saved_only_after_its_machine_has_the_policy() {
    let mut runtime = held(fixture());
    assert!(change(&mut runtime, "files-on", files_on()));
    // Saving, and not yet in the frame the state writer saves (B3, B31).
    assert_eq!(receipt(&runtime), (WriteState::Pending, None));
    assert!(
        !runtime
            .ui_state_to_save()
            .project_settings
            .contains_key(&id())
    );

    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("the delivery is queued");
    assert_eq!(
        job_policy(&job),
        Policy::new(vec![ProjectPolicy {
            root: ROOT.into(),
            base: None,
            rules: files_rules(),
        }])
    );
    assert!(runtime.guardrail_policy_answered(crate::node::TEST_NODE, job, Ok(None)));
    // Delivered: now the state writer takes it, and its save settles it.
    let saved = runtime.ui_state_to_save();
    assert_eq!(saved.project_settings[&id()].version, 2);
    assert!(runtime.acknowledge_project_settings_save(&saved, true));
    assert_eq!(receipt(&runtime), (WriteState::Saved, None));
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id()].guardrails,
        Some(files_rules())
    );
}

#[test]
fn a_node_holding_a_newer_write_is_written_again_above_it_and_the_save_waits() {
    let mut runtime = held(fixture());
    change(&mut runtime, "files-on", files_on());
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .unwrap();
    // Another core's write, or this machine's clock set back.
    let newer = job.sequence() + 60_000;
    assert!(!runtime.guardrail_policy_answered(crate::node::TEST_NODE, job, Ok(Some(newer))));
    assert_eq!(receipt(&runtime), (WriteState::Pending, None));
    let again = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("written again");
    assert!(again.sequence() > newer);
    assert_eq!(job_policy(&again).projects.len(), 1);
    assert!(runtime.guardrail_policy_answered(crate::node::TEST_NODE, again, Ok(None)));
    let saved = runtime.ui_state_to_save();
    assert!(runtime.acknowledge_project_settings_save(&saved, true));
    assert_eq!(receipt(&runtime), (WriteState::Saved, None));
}

#[test]
fn a_node_that_always_holds_a_newer_write_fails_the_save_after_a_few_tries() {
    let mut runtime = held(fixture());
    change(&mut runtime, "files-on", files_on());
    let mut tries = 0;
    while let Some(job) = runtime.take_guardrail_publication(crate::node::TEST_NODE) {
        tries += 1;
        assert!(tries <= 4, "the worker keeps writing");
        runtime.guardrail_policy_answered(crate::node::TEST_NODE, job, Ok(Some(u64::MAX)));
        if receipt(&runtime).0 == WriteState::Failed {
            break;
        }
    }
    assert_eq!(tries, 4);
    assert_eq!(
        receipt(&runtime),
        (
            WriteState::Failed,
            Some("project_settings_policy_failed".into())
        )
    );
}

#[test]
fn a_failed_delivery_fails_the_save_keeps_the_confirmed_value_and_writes_it_once_more() {
    let mut runtime = held(fixture());
    change(&mut runtime, "files-on", files_on());
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .unwrap();
    assert!(runtime.guardrail_policy_answered(
        crate::node::TEST_NODE,
        job,
        Err("the node did not answer".into())
    ));
    assert_eq!(
        receipt(&runtime),
        (
            WriteState::Failed,
            Some("project_settings_policy_failed".into())
        )
    );
    assert!(
        !runtime
            .snapshot
            .ui_state
            .project_settings
            .contains_key(&id())
    );
    // The node may have written before it failed to answer: the confirmed
    // policy, every protection off, is written once more.
    let retry = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("one more write");
    assert_eq!(job_policy(&retry), Policy::new(Vec::new()));
    assert!(!runtime.guardrail_policy_answered(
        crate::node::TEST_NODE,
        retry,
        Err("the node did not answer".into())
    ));
    assert!(
        runtime
            .take_guardrail_publication(crate::node::TEST_NODE)
            .is_none(),
        "a second failure waits for the next start or connection"
    );
    // A retry of the same intent is a new attempt.
    assert!(change(&mut runtime, "files-on", files_on()));
    assert_eq!(receipt(&runtime), (WriteState::Pending, None));
}

#[test]
fn a_change_that_moves_no_policy_saves_without_a_delivery() {
    let mut runtime = held(fixture());
    change(
        &mut runtime,
        "source",
        serde_json::json!({"issue_source": "local"}),
    );
    assert!(
        runtime
            .take_guardrail_publication(crate::node::TEST_NODE)
            .is_none()
    );
    let saved = runtime.ui_state_to_save();
    assert_eq!(
        saved.project_settings[&id()].version,
        1,
        "a record without protections stays readable by an older build"
    );
}

#[test]
fn a_new_base_branch_reaches_the_policy_before_it_is_saved() {
    let mut runtime = held(fixture());
    let record = crate::project_settings::ProjectSettings::new(crate::node::TEST_NODE, ROOT)
        .patched(crate::project_settings::Patch {
            guardrails: Some(Rules {
                base_branch: true,
                ..Rules::default()
            }),
            ..Default::default()
        })
        .unwrap();
    runtime
        .snapshot
        .ui_state
        .project_settings
        .insert(id(), record);
    change(
        &mut runtime,
        "develop",
        serde_json::json!({"base_branch": {"mode": "branch", "branch": "develop"}}),
    );
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("the base branch the protection guards moved (B11)");
    assert_eq!(
        job_policy(&job).projects[0].base.as_deref(),
        Some("develop")
    );
}

#[test]
fn folder_projects_refuse_the_git_protections_and_an_unknown_base_refuses_its_guard() {
    let mut runtime = held(fixture());
    runtime.snapshot.navigator.workspaces[0].is_git = false;
    let git = serde_json::json!({"guardrails": {"git_commands": true}});
    change(&mut runtime, "git", git);
    assert_eq!(
        receipt(&runtime),
        (WriteState::Failed, Some("project_settings_not_git".into()))
    );
    // The files protection is a folder's too (B17).
    change(&mut runtime, "files", files_on());
    assert_eq!(receipt(&runtime), (WriteState::Pending, None));

    let mut runtime = held(fixture());
    runtime.snapshot.navigator.workspaces[0].default_branch = None;
    change(
        &mut runtime,
        "base",
        serde_json::json!({"guardrails": {"base_branch": true}}),
    );
    assert_eq!(
        receipt(&runtime),
        (
            WriteState::Failed,
            Some("project_settings_base_unknown".into())
        )
    );
    // A chosen branch is a base the protection can guard.
    change(
        &mut runtime,
        "chosen",
        serde_json::json!({
            "base_branch": {"mode": "branch", "branch": "develop"},
            "guardrails": {"base_branch": true},
        }),
    );
    assert_eq!(receipt(&runtime), (WriteState::Pending, None));
}

#[test]
fn a_path_list_past_its_limits_is_refused_with_its_reason() {
    for (paths, code) in [
        (
            (0..33).map(|n| format!("p{n}")).collect::<Vec<_>>(),
            "project_settings_paths_limit",
        ),
        (vec!["a".repeat(201)], "project_settings_path_too_long"),
        (
            vec![".env".into(), ".env".into()],
            "project_settings_path_duplicate",
        ),
        (vec!["[z-a]".into()], "project_settings_path_invalid"),
    ] {
        let mut runtime = held(fixture());
        change(
            &mut runtime,
            "paths",
            serde_json::json!({"guardrails": {"files": true, "paths": paths}}),
        );
        assert_eq!(receipt(&runtime), (WriteState::Failed, Some(code.into())));
    }
    // A field the change names beyond this build's protections is not kept;
    // the record's own are.
    let mut runtime = held(fixture());
    change(
        &mut runtime,
        "unknown",
        serde_json::json!({"guardrails": {"files": true, "later": "x".repeat(1024)}}),
    );
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .unwrap();
    assert!(job_policy(&job).projects[0].rules.unknown.is_empty());
}

#[test]
fn a_version_1_record_reads_with_every_protection_off_and_a_newer_one_is_read_only() {
    let mut runtime = held(fixture());
    let mut old: crate::project_settings::ProjectSettings = serde_json::from_value(
        serde_json::json!({"version": 1, "device_id": crate::node::TEST_NODE, "root": ROOT, "issue_source": "local"}),
    )
    .unwrap();
    assert_eq!(old.guardrails, None);
    runtime
        .snapshot
        .ui_state
        .project_settings
        .insert(id(), old.clone());
    assert!(
        runtime
            .take_guardrail_publication(crate::node::TEST_NODE)
            .is_none()
    );
    old.version = 3;
    runtime.snapshot.ui_state.project_settings.insert(id(), old);
    change(&mut runtime, "files", files_on());
    assert_eq!(
        receipt(&runtime),
        (
            WriteState::Failed,
            Some("project_settings_newer_version".into())
        )
    );
}

#[test]
fn a_removed_registration_takes_its_project_out_of_the_policy() {
    let mut runtime = held(fixture());
    change(&mut runtime, "files-on", files_on());
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .unwrap();
    runtime.guardrail_policy_answered(crate::node::TEST_NODE, job, Ok(None));
    let saved = runtime.ui_state_to_save();
    runtime.acknowledge_project_settings_save(&saved, true);
    assert!(
        runtime
            .take_guardrail_publication(crate::node::TEST_NODE)
            .is_none()
    );
    let registration = runtime.snapshot.ui_state.workspace_registrations[0]
        .id
        .clone();
    assert!(runtime.retire_workspace_registration(&registration));
    runtime.persist_ui_state();
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("the hooks stop guarding a project Hide no longer has");
    assert_eq!(job_policy(&job), Policy::new(Vec::new()));
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id()].guardrails,
        None,
        "added again, the project starts with every protection off"
    );
}

#[test]
fn a_save_on_its_way_cannot_bring_a_removed_projects_protections_back() {
    let mut runtime = held(fixture());
    change(&mut runtime, "files-on", files_on());
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .unwrap();
    let registration = runtime.snapshot.ui_state.workspace_registrations[0]
        .id
        .clone();
    assert!(runtime.retire_workspace_registration(&registration));
    runtime.guardrail_policy_answered(crate::node::TEST_NODE, job, Ok(None));
    let saved = runtime.ui_state_to_save();
    assert_eq!(saved.project_settings[&id()].guardrails, None);
}

#[test]
fn a_project_shown_from_an_engine_workspace_is_guarded_without_a_registration() {
    let mut runtime = held(fixture());
    runtime.snapshot.ui_state.workspace_registrations.clear();
    runtime.snapshot.navigator.workspaces[0].registered = false;
    change(&mut runtime, "files-on", files_on());
    let job = runtime
        .take_guardrail_publication(crate::node::TEST_NODE)
        .expect("the protection is delivered");
    assert_eq!(job_policy(&job).projects[0].root, ROOT);
}

#[test]
fn the_start_writes_this_machines_policy_and_a_save_lands_in_its_file() {
    let home = scratch_dir("herdr-core-guardrails-home-");
    let mut runtime = fixture();
    runtime.own_node = Arc::new(hide_node::Local::new(Some(home.path().to_path_buf())));
    runtime.set_guardrails_own_file(true);
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    shared.lock().unwrap().publish_own_guardrail_policy();
    wait_for("the empty policy at start", || {
        read_policy(home.path()) == Ok(Policy::new(Vec::new()))
    });
    assert!(change(&mut shared.lock().unwrap(), "files-on", files_on()));
    wait(&shared, "the save after its delivery", |runtime| {
        receipt(runtime).0 == WriteState::Saved
    });
    assert_eq!(
        read_policy(home.path()).unwrap().projects[0].rules,
        files_rules()
    );
}

fn job_policy(job: &crate::runtime::guardrails::Publication) -> Policy {
    job.policy().clone()
}

#[test]
fn a_core_on_a_relocated_state_folder_writes_no_policy_and_still_saves() {
    let home = scratch_dir("herdr-core-guardrails-relocated-");
    let mut runtime = fixture();
    runtime.own_node = Arc::new(hide_node::Local::new(Some(home.path().to_path_buf())));
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    shared.lock().unwrap().publish_own_guardrail_policy();
    assert!(change(&mut shared.lock().unwrap(), "files-on", files_on()));
    wait(&shared, "the save", |runtime| {
        receipt(runtime).0 == WriteState::Saved
    });
    // The account's own core keeps the file its hooks read.
    assert!(!hide_agent_hooks::guardrails::policy_path(home.path()).exists());
}
