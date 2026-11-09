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
    assert_eq!(
        launch.line(line.clone()),
        Ok(StartLine {
            words: line,
            attempt: None,
        })
    );
    assert_eq!(launch.record(), None);
}

#[test]
fn a_profiles_line_names_its_attempt_and_the_agents_words_and_nothing_of_its_profile() {
    let launch = Launch::answered("claude", "p1", start("work"));
    let line = launch.line(argv(&["claude", "--resume", "abc"])).unwrap();
    let attempt = line
        .attempt
        .clone()
        .expect("a profile's start runs under an attempt");
    assert!(hide_agent_hooks::hook_request::valid_attempt(&attempt.id));
    assert_eq!(
        line.words,
        argv(&[
            "/opt/hide/hide",
            "agent",
            "exec",
            "--attempt",
            &attempt.id,
            "--",
            "claude",
            "--resume",
            "abc",
        ])
    );
    // What the executor asks for is the profile and version the start was
    // checked with, kept by the attempt rather than typed.
    assert_eq!(
        attempt,
        ExecAttempt {
            id: attempt.id.clone(),
            kind: "claude".to_owned(),
            profile: ProfileId("p1".to_owned()),
            name: "work".to_owned(),
            version: 3,
        }
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
fn a_profiles_own_program_stays_off_its_line_and_each_start_has_its_own_attempt() {
    let mut answered = start("gateway");
    answered.program = Some(hide_node_link::profiles::ProfileProgram {
        file: "C:/tools/node.exe".to_owned(),
        leading: vec!["C:/tools/gw.js".to_owned()],
    });
    let launch = Launch::answered("codex", "p2", answered);
    let line = launch.line(argv(&["codex", "--no-daemon"])).unwrap();
    let attempt = line.attempt.clone().unwrap().id;
    assert_ne!(
        launch.line(argv(&["codex"])).unwrap().attempt.unwrap().id,
        attempt
    );
    assert_eq!(
        line.words,
        argv(&[
            "/opt/hide/hide",
            "agent",
            "exec",
            "--attempt",
            &attempt,
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
    let parent = Parent::Profile(recorded("b", "b"));
    let project = ProfileId("a".to_owned());
    let chosen = Some((ProfileId("a".to_owned()), "a".to_owned()));
    let pick =
        |chosen, parent: &Parent, project| pick("claude", chosen, parent, project, Some(&profiles));
    assert_eq!(
        picked(&pick(chosen.clone(), &parent, None)),
        Some("a".to_owned())
    );
    assert_eq!(
        picked(&pick(None, &parent, Some(&project))),
        Some("b".to_owned())
    );
    assert_eq!(
        picked(&pick(None, &Parent::None, Some(&project))),
        Some("a".to_owned())
    );
    assert_eq!(
        picked(&pick(None, &Parent::None, None)),
        Some("c".to_owned())
    );
    // A project that names the built-in default starts it, over the
    // machine's default; a parent still comes first.
    let builtin_project = ProfileId::default_profile();
    assert_eq!(
        picked(&pick(None, &Parent::None, Some(&builtin_project))),
        None
    );
    assert_eq!(
        picked(&pick(None, &parent, Some(&builtin_project))),
        Some("b".to_owned())
    );
    // A parent on the built-in default hands that down, over the project's
    // and the machine's default.
    let builtin = pick(None, &Parent::Default, Some(&project));
    assert_eq!(builtin.profile(), None);
    assert!(builtin.target.profile.is_default());
    // A parent's profile the machine no longer lists is still the one asked
    // for, so its machine refuses it by name rather than the child starting
    // in another account.
    let gone = pick(None, &Parent::Profile(recorded("gone", "old")), None);
    assert_eq!(
        gone.profile(),
        Some((ProfileId("gone".to_owned()), "old".to_owned()))
    );
    // Another agent's default is never this one's.
    assert_eq!(
        picked(&super::pick(
            "claude",
            None,
            &Parent::None,
            None,
            Some(&machine(&profiles.profiles, &[("codex", "x")]))
        )),
        None
    );
    // A machine whose profiles were never read starts the built-in default.
    assert_eq!(
        picked(&super::pick("claude", None, &Parent::None, None, None)),
        None
    );
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
        let machine = machine(
            &[listed_profile("c", "claude-code", "c")],
            &[("claude-code", "c")],
        );
        let start = pick(
            "claude",
            Some(builtin),
            &Parent::Profile(recorded("c", "c")),
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
    /// The reason a refusal names.
    reason: &'static str,
    older: bool,
    calls: Mutex<Vec<Call>>,
}

impl Node {
    fn answering(answer: Result<serde_json::Value, ErrorCode>) -> Self {
        Self {
            answer,
            reason: "refused",
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
            Err(code) => Err(LinkError::Refused(HostError::new(*code, self.reason))),
        }
    }

    fn predates_current_protocol(&self) -> bool {
        self.older
    }

    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        static CURRENT: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::OnceLock::new();
        static OLDER: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::OnceLock::new();
        Some(if self.older {
            OLDER.get_or_init(hide_node_link::sessions::ReaderFeatures::protocol24)
        } else {
            CURRENT.get_or_init(hide_node_link::sessions::ReaderFeatures::implemented)
        })
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

fn asked(version: u64) -> ExecAttempt {
    ExecAttempt {
        id: "9e4eedfa8bf06be0".to_owned(),
        kind: "claude".to_owned(),
        profile: ProfileId("p1".to_owned()),
        name: "work".to_owned(),
        version,
    }
}

fn answered(launch: ProfileLaunch) -> Node {
    Node::answering(Ok(serde_json::to_value(launch).unwrap()))
}

fn failure(answer: hide_agent_hooks::hook_request::ExecProfile) -> String {
    match answer {
        hide_agent_hooks::hook_request::ExecProfile::Refused { failure } => failure,
        ready => panic!("expected a refusal, got {ready:?}"),
    }
}

#[test]
fn an_executor_is_handed_its_profile_as_its_machine_has_it_at_the_checked_version() {
    let mut gateway = start("work");
    gateway.program = Some(hide_node_link::profiles::ProfileProgram {
        file: "C:/tools/node.exe".to_owned(),
        leading: vec!["C:/tools/gw.js".to_owned()],
    });
    let node = answered(ProfileLaunch::Ready(gateway));
    assert_eq!(
        exec_start(Some(&node), &asked(3)),
        hide_agent_hooks::hook_request::ExecProfile::Ready(
            hide_agent_hooks::hook_request::ExecStart {
                label: "Claude · work".to_owned(),
                env: start("work").env,
                program: Some("C:/tools/node.exe".to_owned()),
                leading: vec!["C:/tools/gw.js".to_owned()],
            }
        )
    );
    // The machine is asked as a start asks it, by agent and profile.
    assert_eq!(
        *node.calls.lock().unwrap(),
        [Call::ProfileLaunch {
            kind: "claude".to_owned(),
            profile: ProfileId("p1".to_owned()),
        }]
    );
}

#[test]
fn an_executor_whose_profile_changed_went_or_is_refused_now_is_refused_naming_it() {
    // An edit since the start was checked: the start runs nothing rather
    // than values its record and its words were not made from.
    let changed = failure(exec_start(
        Some(&answered(ProfileLaunch::Ready(start("work")))),
        &asked(2),
    ));
    assert!(changed.contains("profile work changed"), "{changed}");
    let gone = failure(exec_start(Some(&answered(ProfileLaunch::Gone)), &asked(3)));
    assert!(gone.contains("profile work is gone"), "{gone}");
    let removing = failure(exec_start(
        Some(&answered(ProfileLaunch::Refused {
            text: "the profile work is being removed".to_owned(),
        })),
        &asked(3),
    ));
    assert!(
        removing.ends_with("the profile work is being removed"),
        "{removing}"
    );
    // A machine that does not answer, or cannot be reached, runs nothing.
    for node in [Some(Node::answering(Err(ErrorCode::Io))), None] {
        let unanswered = failure(exec_start(
            node.as_ref().map(|node| node as &dyn NodeLink),
            &asked(3),
        ));
        assert!(
            unanswered.contains("with the profile work was not started"),
            "{unanswered}"
        );
    }
}

#[test]
fn an_executors_start_larger_than_one_answer_is_refused() {
    let mut large = start("work");
    large.env = (0..3)
        .map(|n| (format!("LARGE_{n}"), "x".repeat(32 * 1024)))
        .collect();
    let refused = failure(exec_start(
        Some(&answered(ProfileLaunch::Ready(large))),
        &asked(3),
    ));
    assert!(
        refused.contains("more variables than a start takes"),
        "{refused}"
    );
}

/// A session line's resume runs in the account the session's machine says
/// holds it; a machine on the older protocol keeps no profile, so its
/// sessions are the default account's, while a machine that keeps profiles
/// and does not answer, or holds the session in two accounts, starts
/// nothing (PRD agent-profiles B19).
#[test]
fn a_session_line_resumes_in_the_account_its_machine_says_holds_it() {
    let reference = crate::sidebar::SessionAgentSessionPayload {
        kind: "id".into(),
        value: "ses_1".into(),
    };
    let work = AccountKey::Folder("/accounts/work".into());
    let node = Node::answering(Ok(serde_json::to_value(&work).unwrap()));
    assert_eq!(session_account(&node, "opencode", &reference), Ok(work));
    assert_eq!(
        *node.calls.lock().unwrap(),
        [Call::SessionAccount {
            request: hide_session::session_account::SessionAccountRequest {
                agent: hide_session::Agent::OpenCode,
                reference_kind: "id".into(),
                reference_value: "ses_1".into(),
            },
        }]
    );

    let older = Node {
        older: true,
        ..Node::answering(Err(ErrorCode::InvalidRequest))
    };
    let file = crate::sidebar::SessionAgentSessionPayload {
        kind: "path".into(),
        value: "/home/me/.claude/projects/-repo/s.jsonl".into(),
    };
    assert_eq!(
        session_account(&older, "claude", &file),
        Ok(AccountKey::Default)
    );
    for code in [
        ErrorCode::InvalidRequest,
        ErrorCode::Unsupported,
        ErrorCode::Io,
    ] {
        let current = Node::answering(Err(code));
        assert_eq!(
            session_account(&current, "opencode", &reference)
                .unwrap_err()
                .code,
            SESSION_ACCOUNT_UNANSWERED,
            "{code:?}"
        );
    }
    let ambiguous = Node {
        reason: hide_session::session_account::AMBIGUOUS,
        ..Node::answering(Err(ErrorCode::Io))
    };
    assert_eq!(
        session_account(&ambiguous, "opencode", &reference)
            .unwrap_err()
            .code,
        SESSION_ACCOUNT_AMBIGUOUS
    );
    // A Claude Code id with no file on a machine that keeps another account
    // for it is refused there, and the refusal is never the default account.
    let unproven = Node {
        reason: hide_session::session_account::UNPROVEN,
        ..Node::answering(Err(ErrorCode::Io))
    };
    let id = crate::sidebar::SessionAgentSessionPayload {
        kind: "id".into(),
        value: "019a-session".into(),
    };
    assert_eq!(
        session_account(&unproven, "claude", &id).unwrap_err().code,
        SESSION_ACCOUNT_UNPROVEN
    );
}
