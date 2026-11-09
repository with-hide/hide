//! A pane read in the account its hook reported (PRD agent-profiles B24,
//! B25): an alias the operator ran by hand, and a wrapper that runs its
//! profile's agent in another of the machine's accounts.

use super::*;
use crate::launch::LaunchProfile;
use crate::model::{ProfileSnapshot, ProfilesSnapshot, ProfilesState};
use hide_agent_adapter::{AccountKey, ProfileId};

/// A Claude profile whose account is `folder`, listed with the variable a
/// start of it sets, as the node lists it.
fn profile(id: &str, name: &str, folder: &str) -> ProfileSnapshot {
    ProfileSnapshot {
        id: id.to_owned(),
        kind: "claude-code".to_owned(),
        name: name.to_owned(),
        account: Some(folder.to_owned()),
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
        account_key: Some(AccountKey::Folder(folder.to_owned())),
        account_variables: vec![("CLAUDE_CONFIG_DIR".to_owned(), folder.to_owned())],
    }
}

fn listed(profiles: Vec<ProfileSnapshot>) -> Runtime {
    let mut runtime = runtime();
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_state(&node);
    kit.profiles = Some(ProfilesSnapshot {
        state: ProfilesState::Listed,
        version: 1,
        profiles,
        defaults: Default::default(),
    });
    runtime.set_kit_state(&node, kit);
    runtime
}

/// The hook in `pane` reports a session's start in the account `folder`
/// sets, or in the default account for none.
fn started(runtime: &mut Runtime, pane: &str, session: &str, folder: Option<&str>) {
    let report = hide_node_link::panes::HookStatusReport {
        runtime: "claude-code".to_owned(),
        version: hide_agent_hooks::HOOK_VERSION,
        session: Some(session.to_owned()),
        turn: None,
        counts: None,
        session_start: true,
        waiting: None,
        account: Some(
            folder
                .map(|folder| ("CLAUDE_CONFIG_DIR".to_owned(), folder.to_owned()))
                .into_iter()
                .collect(),
        ),
    };
    runtime
        .ingest_hook_status(crate::node::TEST_NODE, pane, &report)
        .expect("the report is recorded");
}

fn account(runtime: &Runtime, pane: &str) -> AccountKey {
    runtime.pane_account(pane, "claude", None, None)
}

fn row(pane: &str) -> SidebarAgentSnapshot {
    project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id": pane, "agent": "claude", "agent_status": "working", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0)
}

fn shown(runtime: &Runtime, pane: &str) -> Option<String> {
    let mut rows = [row(pane)];
    runtime.lay_profiles(&mut rows);
    rows[0].profile.take()
}

#[test]
fn an_alias_typed_in_a_pane_is_read_in_the_profile_account_it_sets() {
    let mut runtime = listed(vec![profile("p-a", "a", "/accounts/claude-a")]);
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a"));
    assert_eq!(
        account(&runtime, "p1"),
        AccountKey::Folder("/accounts/claude-a".into())
    );
    assert_eq!(shown(&runtime, "p1").as_deref(), Some("a"));
    let mut rows = [row("p1")];
    rows[0].agent_kind = "claude".to_owned();
    assert_eq!(
        runtime.launch_profile_of(&rows[0]),
        Some(LaunchProfile {
            id: ProfileId("p-a".into()),
            name: "a".into(),
            account: AccountKey::Folder("/accounts/claude-a".into()),
        }),
        "a wake starts it in the same account"
    );
}

#[test]
fn an_account_two_profiles_share_names_the_agent_alone() {
    let mut runtime = listed(vec![
        profile("p-a", "a", "/accounts/claude-a"),
        profile("p-b", "b", "/accounts/claude-a"),
    ]);
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a"));
    assert_eq!(
        account(&runtime, "p1"),
        AccountKey::Folder("/accounts/claude-a".into())
    );
    assert_eq!(shown(&runtime, "p1"), None);
}

#[test]
fn the_default_account_and_a_folder_hide_keeps_nothing_of_read_as_before() {
    let mut runtime = listed(vec![profile("p-a", "a", "/accounts/claude-a")]);
    started(&mut runtime, "p1", "s-1", None);
    started(&mut runtime, "p2", "s-2", Some("/elsewhere/claude"));
    for pane in ["p1", "p2"] {
        assert_eq!(account(&runtime, pane), AccountKey::Default, "{pane}");
        assert_eq!(shown(&runtime, pane), None, "{pane}");
    }
}

#[test]
fn a_wrapper_that_runs_another_account_is_read_there_and_warns_on_its_profile() {
    let mut runtime = listed(vec![
        profile("p-gate", "gate", "/accounts/claude-gate"),
        profile("p-a", "a", "/accounts/claude-a"),
    ]);
    runtime.snapshot.ui_state.pane_launches.insert(
        "p1".to_owned(),
        crate::model::PaneLaunch {
            kind: "claude".to_owned(),
            name: "p1-agent".to_owned(),
            choice: Default::default(),
            session: None,
            profile: Some(LaunchProfile {
                id: ProfileId("p-gate".into()),
                name: "gate".into(),
                account: AccountKey::Folder("/accounts/claude-gate".into()),
            }),
        },
    );
    let warned = |runtime: &Runtime| {
        runtime
            .machine_profiles(crate::node::TEST_NODE)
            .unwrap()
            .profiles
            .iter()
            .filter(|profile| profile.wrapper_folder)
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>()
    };
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a"));
    assert_eq!(
        account(&runtime, "p1"),
        AccountKey::Folder("/accounts/claude-a".into()),
        "the agent is read where it runs"
    );
    assert_eq!(warned(&runtime), ["p-gate"]);
    // A list read again keeps the mark.
    let node = runtime.node.to_string();
    let list = hide_node_link::profiles::ProfileList {
        version: 2,
        profiles: ["p-gate", "p-a"]
            .into_iter()
            .map(|id| {
                let folder = if id == "p-a" {
                    "/accounts/claude-a"
                } else {
                    "/accounts/claude-gate"
                };
                hide_node_link::profiles::ListedProfile {
                    id: ProfileId(id.into()),
                    kind: "claude-code".into(),
                    name: id.trim_start_matches("p-").into(),
                    account: Some(hide_agent_adapter::AccountRef::Folder(folder.into())),
                    program: None,
                    env_keys: Vec::new(),
                    env: None,
                    version: 1,
                    removing: false,
                    place: None,
                    login: None,
                    account_variables: vec![("CLAUDE_CONFIG_DIR".into(), folder.into())],
                    account_key: Some(AccountKey::Folder(folder.into())),
                }
            })
            .collect(),
        defaults: Default::default(),
        unjudged: None,
    };
    runtime.ingest_profiles(&node, crate::runtime::kit::ProfilesRead::Listed(list));
    assert_eq!(warned(&runtime), ["p-gate"]);
    // Its next start runs in its own account again.
    started(&mut runtime, "p1", "s-2", Some("/accounts/claude-gate"));
    assert!(warned(&runtime).is_empty());
}

/// The label reads take the reported account with the launch records, and
/// again whenever a report moves it (B24, B25).
#[test]
fn the_label_reads_take_each_reported_account_and_its_moves() {
    let mut runtime = listed(vec![profile("p-a", "a", "/accounts/claude-a")]);
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a"));
    let (edit, reads) = runtime.account_launches(None, None).unwrap();
    assert_eq!(
        reads.reported.get("p1"),
        Some(&(
            "claude-code".to_owned(),
            AccountKey::Folder("/accounts/claude-a".into())
        ))
    );
    assert!(
        runtime.account_launches(None, Some(edit)).is_none(),
        "nothing moved"
    );
    started(&mut runtime, "p1", "s-2", None);
    let (_, reads) = runtime
        .account_launches(None, Some(edit))
        .expect("the report moved");
    assert_eq!(
        reads.reported.get("p1").map(|(_, key)| key),
        Some(&AccountKey::Default)
    );
}

/// D-15: an account is its value as set. `/a` and `/a/` are two accounts
/// (Claude Code keys its login by the string), each its own profile's; a
/// spelling no profile sets reads as before.
#[test]
fn an_account_is_matched_by_its_value_as_set() {
    let mut runtime = listed(vec![
        profile("p-a", "a", "/accounts/claude-a"),
        profile("p-slash", "slash", "/accounts/claude-a/"),
    ]);
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a/"));
    started(&mut runtime, "p2", "s-2", Some("/accounts/claude-a"));
    started(&mut runtime, "p3", "s-3", Some("/links/claude-a"));
    assert_eq!(shown(&runtime, "p1").as_deref(), Some("slash"));
    assert_eq!(
        account(&runtime, "p1"),
        AccountKey::Folder("/accounts/claude-a/".into())
    );
    assert_eq!(shown(&runtime, "p2").as_deref(), Some("a"));
    assert_eq!(account(&runtime, "p3"), AccountKey::Default);
    assert_eq!(shown(&runtime, "p3"), None);
}

/// D-35: the profile's last start decides the warning, whatever its panes
/// are called; here the later start is in the pane whose id sorts first.
#[test]
fn a_wrapper_warning_follows_the_profiles_last_start() {
    let gate = || LaunchProfile {
        id: ProfileId("p-gate".into()),
        name: "gate".into(),
        account: AccountKey::Folder("/accounts/claude-gate".into()),
    };
    let launched = |runtime: &mut Runtime, pane: &str| {
        runtime.snapshot.ui_state.pane_launches.insert(
            pane.to_owned(),
            crate::model::PaneLaunch {
                kind: "claude".to_owned(),
                name: format!("{pane}-agent"),
                choice: Default::default(),
                session: None,
                profile: Some(gate()),
            },
        );
    };
    let warned = |runtime: &Runtime| {
        runtime
            .machine_profiles(crate::node::TEST_NODE)
            .unwrap()
            .profiles
            .iter()
            .any(|profile| profile.id == "p-gate" && profile.wrapper_folder)
    };
    for elsewhere_last in [true, false] {
        let mut runtime = listed(vec![
            profile("p-gate", "gate", "/accounts/claude-gate"),
            profile("p-a", "a", "/accounts/claude-a"),
        ]);
        launched(&mut runtime, "p1");
        launched(&mut runtime, "p2");
        let (own, elsewhere) = (Some("/accounts/claude-gate"), Some("/accounts/claude-a"));
        let (first, last) = if elsewhere_last {
            (own, elsewhere)
        } else {
            (elsewhere, own)
        };
        // p2 starts first, then p1.
        started(&mut runtime, "p2", "s-2", first);
        started(&mut runtime, "p1", "s-1", last);
        assert_eq!(
            warned(&runtime),
            elsewhere_last,
            "elsewhere last: {elsewhere_last}"
        );
        // Worked out again, it decides the same.
        runtime.refresh_reported_accounts();
        assert_eq!(warned(&runtime), elsewhere_last);
        // p2's start delivered again, and a plain status report of it, do
        // not make it the last start.
        started(&mut runtime, "p2", "s-2", first);
        let status = hide_node_link::panes::HookStatusReport {
            runtime: "claude-code".to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: Some("s-2".to_owned()),
            turn: None,
            counts: None,
            session_start: false,
            waiting: None,
            account: None,
        };
        runtime
            .ingest_hook_status(crate::node::TEST_NODE, "p2", &status)
            .expect("the report is recorded");
        assert_eq!(
            warned(&runtime),
            elsewhere_last,
            "p2 again: {elsewhere_last}"
        );
    }
}

/// D-03, D-32, B16, B24: a parent the operator started by hand with an
/// alias runs in the profile account its hook reported, and a child of the
/// same agent it starts with no `--profile` starts there too, not in the
/// agent's default on the machine. One started by hand in the default
/// account hands down the built-in default; one with neither a record nor
/// a report is still unknown.
#[test]
fn a_child_of_a_parent_started_with_an_alias_starts_in_the_parents_account() {
    let mut runtime = listed(vec![
        profile("p-a", "a", "/accounts/claude-a"),
        profile("p-b", "b", "/accounts/claude-b"),
    ]);
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_states[&node].clone();
    if let Some(profiles) = kit.profiles.as_mut() {
        profiles
            .defaults
            .insert("claude-code".to_owned(), "p-b".to_owned());
    }
    runtime.set_kit_state(&node, kit);
    let parent = |pane: &str, session: &str| crate::delivery::Actor {
        pane_id: pane.into(),
        name: "by-hand".into(),
        kind: "claude".into(),
        device_id: node.clone(),
        session: Some(session.into()),
    };
    let child = |runtime: &Runtime, parent: &crate::delivery::Actor| {
        crate::launch::pick(
            "claude",
            None,
            &runtime.caller_launch_profile(parent, "claude"),
            None,
            runtime.machine_profiles(&node),
        )
        .profile()
        .map(|(id, _)| id.0)
    };
    started(&mut runtime, "p1", "s-1", Some("/accounts/claude-a"));
    assert_eq!(child(&runtime, &parent("p1", "s-1")).as_deref(), Some("p-a"));
    started(&mut runtime, "p2", "s-2", None);
    assert_eq!(
        runtime.caller_launch_profile(&parent("p2", "s-2"), "claude"),
        crate::launch::Parent::Default
    );
    assert_eq!(child(&runtime, &parent("p2", "s-2")), None);
    assert_eq!(
        runtime.caller_launch_profile(&parent("p3", "s-3"), "claude"),
        crate::launch::Parent::Unknown
    );
    assert_eq!(child(&runtime, &parent("p3", "s-3")).as_deref(), Some("p-b"));
}
