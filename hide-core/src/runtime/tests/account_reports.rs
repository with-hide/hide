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
        account_key: AccountKey::Folder(folder.to_owned()),
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
