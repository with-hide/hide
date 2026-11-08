use std::sync::Mutex;

use hide_node_link::protocol::Call;
use hide_node_link::{HostError, LinkAnswer};

use super::*;
use crate::model::{ProfileSnapshot, ProfilesState};

fn start(name: &str) -> ProfileStart {
    ProfileStart {
        name: name.to_owned(),
        account: AccountKey::Folder("/home/me/.claude-work".to_owned()),
        version: 3,
        env: vec![
            (
                "CLAUDE_CONFIG_DIR".to_owned(),
                "/home/me/.claude-work".to_owned(),
            ),
            ("ROUTE".to_owned(), "a=b c".to_owned()),
        ],
        program: None,
        exec: "/opt/hide/hide".to_owned(),
    }
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn listed_profile(id: &str, kind: &str, name: &str) -> ProfileSnapshot {
    ProfileSnapshot {
        id: id.to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        account: Some(format!("~/.{name}")),
        program: None,
        env_keys: Vec::new(),
        version: 1,
        install: None,
        folder: None,
        hook: None,
        skill: None,
    }
}

fn machine(profiles: &[ProfileSnapshot], defaults: &[(&str, &str)]) -> ProfilesSnapshot {
    ProfilesSnapshot {
        state: ProfilesState::Listed,
        version: 1,
        profiles: profiles.to_vec(),
        defaults: defaults
            .iter()
            .map(|(agent, id)| ((*agent).to_owned(), (*id).to_owned()))
            .collect(),
    }
}

fn recorded(id: &str, name: &str) -> LaunchProfile {
    LaunchProfile {
        id: ProfileId(id.to_owned()),
        name: name.to_owned(),
        account: AccountKey::Folder(format!("/home/me/.{name}")),
    }
}

fn picked(request: &Request) -> Option<String> {
    request.profile().map(|(id, _)| id.0)
}

#[test]
fn the_built_in_default_starts_with_the_line_every_agent_always_had() {
    let line = argv(&["claude", "--resume", "abc"]);
    let launch = Launch::default_of("claude");
    assert_eq!(launch.line(line.clone()), line);
    assert_eq!(launch.record(), None);
}

#[test]
fn a_profiles_line_runs_hide_agent_exec_with_its_variables_and_the_agents_words() {
    let launch = Launch::answered("claude", "p1", start("work"));
    assert_eq!(
        launch.line(argv(&["claude", "--resume", "abc"])),
        argv(&[
            "/opt/hide/hide",
            "agent",
            "exec",
            "--profile",
            "Claude · work",
            "--env",
            "CLAUDE_CONFIG_DIR=/home/me/.claude-work",
            "--env",
            "ROUTE=a=b c",
            "--",
            "claude",
            "--resume",
            "abc",
        ])
    );
    assert_eq!(
        launch.record(),
        Some(LaunchProfile {
            id: ProfileId("p1".to_owned()),
            name: "work".to_owned(),
            account: AccountKey::Folder("/home/me/.claude-work".to_owned()),
        })
    );
}

#[test]
fn a_profiles_own_program_runs_in_place_of_the_agents_with_its_leading_words() {
    let mut answered = start("gateway");
    answered.env.clear();
    answered.program = Some(hide_node_link::profiles::ProfileProgram {
        file: "C:/tools/node.exe".to_owned(),
        leading: vec!["C:/tools/gw.js".to_owned()],
    });
    let launch = Launch::answered("codex", "p2", answered);
    assert_eq!(
        launch.line(argv(&["codex", "--no-daemon"])),
        argv(&[
            "/opt/hide/hide",
            "agent",
            "exec",
            "--profile",
            "Codex · gateway",
            "--program",
            "C:/tools/node.exe",
            "--leading",
            "C:/tools/gw.js",
            "--",
            "codex",
            "--no-daemon",
        ])
    );
}

#[test]
fn a_new_start_takes_its_choice_then_its_parents_then_the_project_then_the_machine_default() {
    let profiles = machine(
        &[
            listed_profile("a", "claude-code", "a"),
            listed_profile("b", "claude-code", "b"),
            listed_profile("c", "claude-code", "c"),
            listed_profile("x", "codex", "x"),
        ],
        &[("claude-code", "c"), ("codex", "x")],
    );
    let parent = recorded("b", "b");
    let project = ProfileId("a".to_owned());
    let chosen = Some((ProfileId("a".to_owned()), "a".to_owned()));
    let pick = |chosen, parent, project| pick("claude", chosen, parent, project, Some(&profiles));
    assert_eq!(
        picked(&pick(chosen.clone(), Some(&parent), None)),
        Some("a".to_owned())
    );
    assert_eq!(
        picked(&pick(None, Some(&parent), Some(&project))),
        Some("b".to_owned())
    );
    assert_eq!(
        picked(&pick(None, None, Some(&project))),
        Some("a".to_owned())
    );
    assert_eq!(picked(&pick(None, None, None)), Some("c".to_owned()));
    // A parent's profile the machine no longer lists falls through.
    assert_eq!(
        picked(&pick(None, Some(&recorded("gone", "gone")), None)),
        Some("c".to_owned())
    );
    // Another agent's default is never this one's.
    assert_eq!(
        picked(&super::pick(
            "claude",
            None,
            None,
            None,
            Some(&machine(&profiles.profiles, &[("codex", "x")]))
        )),
        None
    );
    // A machine whose profiles were never read starts the built-in default.
    assert_eq!(picked(&super::pick("claude", None, None, None, None)), None);
}

#[test]
fn a_profile_named_by_a_start_must_be_one_its_machine_lists_for_that_agent() {
    let profiles = machine(
        &[
            listed_profile("w", "claude-code", "Work"),
            listed_profile("x", "codex", "codex-only"),
        ],
        &[],
    );
    assert_eq!(
        chosen("claude", "work", Some(&profiles)),
        Ok((ProfileId("w".to_owned()), "Work".to_owned()))
    );
    for profiles in [Some(&profiles), None] {
        let builtin = chosen("claude", "default", profiles).unwrap();
        assert!(builtin.0.is_default());
        // Named, it wins over the machine's default and a parent's profile.
        let machine = machine(&[listed_profile("c", "claude-code", "c")], &[("claude-code", "c")]);
        let start = pick(
            "claude",
            Some(builtin),
            Some(&recorded("c", "c")),
            None,
            Some(&machine),
        );
        assert_eq!(start.profile(), None);
        assert!(start.target.profile.is_default());
    }
    for (name, profiles) in [
        ("codex-only", Some(&profiles)),
        ("missing", Some(&profiles)),
        ("work", None),
    ] {
        let refused = chosen("claude", name, profiles).unwrap_err();
        assert!(refused.starts_with("profile_gone: "), "{refused}");
    }
}

/// A node that answers a profile's start one way and records each call.
struct Node {
    answer: Result<serde_json::Value, ErrorCode>,
    older: bool,
    calls: Mutex<Vec<Call>>,
}

impl Node {
    fn answering(answer: Result<serde_json::Value, ErrorCode>) -> Self {
        Self {
            answer,
            older: false,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl NodeLink for Node {
    fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
        self.calls.lock().unwrap().push(call);
        match &self.answer {
            Ok(value) => Ok(value.clone().into()),
            Err(code) => Err(LinkError::Refused(HostError::new(*code, "refused"))),
        }
    }

    fn predates_current_protocol(&self) -> bool {
        self.older
    }
}

fn request(id: &str, name: &str) -> Request {
    Request::new("claude", Some((ProfileId(id.to_owned()), name.to_owned())))
}

#[test]
fn the_built_in_default_asks_nothing_of_its_machine() {
    let node = Node::answering(Err(ErrorCode::Io));
    let launch = resolve(Some(&node), &Request::default_of("claude")).unwrap();
    assert_eq!(launch, Launch::default_of("claude"));
    assert!(node.calls.lock().unwrap().is_empty());
    assert_eq!(
        resolve(None, &Request::recorded("codex", None)).unwrap(),
        Launch::default_of("codex")
    );
}

#[test]
fn a_profile_starts_as_its_machine_answers_for_it_now() {
    let node = Node::answering(Ok(serde_json::to_value(ProfileLaunch::Ready(start(
        "work",
    )))
    .unwrap()));
    let launch = resolve(Some(&node), &request("p1", "work")).unwrap();
    assert_eq!(launch, Launch::answered("claude", "p1", start("work")));
    assert_eq!(
        *node.calls.lock().unwrap(),
        [Call::ProfileLaunch {
            kind: "claude".to_owned(),
            profile: ProfileId("p1".to_owned()),
        }]
    );
}

#[test]
fn a_profile_gone_refused_or_unsupported_starts_nothing_and_says_why() {
    let gone = Node::answering(Ok(serde_json::to_value(ProfileLaunch::Gone).unwrap()));
    let refusal = resolve(Some(&gone), &request("p1", "work")).unwrap_err();
    assert_eq!(refusal.code, PROFILE_GONE);
    assert!(
        refusal.detail.contains("profile work is gone"),
        "{}",
        refusal.detail
    );

    let refused = Node::answering(Ok(serde_json::to_value(ProfileLaunch::Refused {
        text: "the profile work runs gw, which is not a program this machine finds".to_owned(),
    })
    .unwrap()));
    let refusal = resolve(Some(&refused), &request("p1", "work")).unwrap_err();
    assert_eq!(refusal.code, PROFILE_REFUSED);
    assert!(
        refusal
            .detail
            .ends_with("which is not a program this machine finds")
    );

    let unsupported = Node::answering(Err(ErrorCode::Unsupported));
    assert_eq!(
        resolve(Some(&unsupported), &request("p1", "work"))
            .unwrap_err()
            .code,
        PROFILES_UNSUPPORTED
    );
    let older = Node {
        older: true,
        ..Node::answering(Err(ErrorCode::InvalidRequest))
    };
    assert_eq!(
        resolve(Some(&older), &request("p1", "work"))
            .unwrap_err()
            .code,
        PROFILES_UNSUPPORTED
    );
    for node in [
        Some(Node::answering(Err(ErrorCode::InvalidRequest))),
        Some(Node::answering(Err(ErrorCode::Io))),
        None,
    ] {
        assert_eq!(
            resolve(
                node.as_ref().map(|node| node as &dyn NodeLink),
                &request("p1", "work")
            )
            .unwrap_err()
            .code,
            PROFILE_UNANSWERED
        );
    }
}
