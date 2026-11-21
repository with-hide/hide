use super::*;
use hide_node_link::profiles::ProfileRefusal;
use hide_node_link::{HostError, LinkAnswer};
use std::sync::Mutex as StdMutex;

/// A node that answers each call from a script, in order, and keeps what it
/// was asked.
struct Scripted {
    answers: StdMutex<Vec<Result<Value, ErrorCode>>>,
    asked: StdMutex<Vec<Call>>,
}

impl Scripted {
    fn new(answers: Vec<Result<Value, ErrorCode>>) -> Arc<Self> {
        Arc::new(Self {
            answers: StdMutex::new(answers),
            asked: StdMutex::new(Vec::new()),
        })
    }
}

impl NodeLink for Scripted {
    fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
        self.asked.lock().unwrap().push(call);
        match self.answers.lock().unwrap().remove(0) {
            Ok(value) => Ok(value.into()),
            Err(code) => Err(LinkError::Refused(HostError::new(code, "refused"))),
        }
    }
}

fn list(version: u64, env: Option<Vec<(&str, &str)>>) -> ProfileList {
    ProfileList {
        version,
        profiles: vec![ListedProfile {
            id: ProfileId("p-work".into()),
            kind: "claude-code".into(),
            name: "work".into(),
            account: Some(AccountRef::Folder("/home/me/accounts/work".into())),
            program: Some("claude-gateway".into()),
            env_keys: vec!["A".into(), "B".into()],
            env: env.map(|env| {
                env.into_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect()
            }),
            version: 1,
            removing: false,
            place: None,
            login: None,
            account_variables: Vec::new(),
            account_key: Some(hide_agent_adapter::AccountKey::Folder(
                "/home/me/accounts/work".into(),
            )),
        }],
        defaults: Default::default(),
        unjudged: None,
    }
}

fn job(link: Arc<Scripted>, change: Change, version: u64) -> Prepared {
    Prepared {
        machine: "this".into(),
        link,
        change,
        version,
        kind: None,
        request: None,
        runtime: Weak::new(),
        notifier: crate::handle::ChangeNotifier::new(),
        // Nothing here settles into a runtime.
        settled: true,
    }
}

fn applied(refused: Option<ProfileRefusal>, version: u64) -> Value {
    serde_json::to_value(ProfileApplied {
        refused,
        added: None,
        list: list(version, None),
    })
    .unwrap()
}

#[test]
fn a_field_the_node_refuses_reaches_the_caller_with_its_field_and_words() {
    let refusal = ProfileRefusal {
        field: ProfileField::Env,
        text: "HOME cannot be set by a profile".into(),
        env_key: Some("HOME".into()),
        reason: Some(hide_node_link::profiles::ProfileReason::EnvReserved),
        limit: None,
    };
    let link = Scripted::new(vec![Ok(applied(Some(refusal), 3))]);
    let prepared = job(
        link,
        Change::Add {
            kind: "claude-code".into(),
            name: "x".into(),
            account: None,
            program: None,
            env: vec![("HOME".into(), "/tmp".into())],
            default: false,
        },
        3,
    );
    let outcome = prepared.call();
    let refused = outcome.result.as_ref().err().unwrap();
    assert_eq!(refused.code, REFUSED);
    assert_eq!(
        refused.field.as_ref().unwrap().env_key.as_deref(),
        Some("HOME")
    );
    let line = prepared.answer(&outcome).unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&line),
        (REFUSED, Some("env: HOME cannot be set by a profile"))
    );
    // The screen gets the reason to word itself, and the variable's name.
    let answer =
        serde_json::to_value(ProfileAnswer::pending("r1", "local").refused(refused)).unwrap();
    assert_eq!(answer["reason"], "env_reserved");
    assert_eq!(answer["env_key"], "HOME");
    // The list after the refusal still reaches the snapshot.
    assert!(outcome.list.is_some());
}

#[test]
fn a_node_before_reason_codes_and_one_after_this_build_still_answer() {
    // An older node sends no reason; a newer one may name one this build
    // does not know. Both refusals still read, the screen then words them
    // generically.
    let old: ProfileRefusal = serde_json::from_value(
        serde_json::json!({"field": "name", "text": "A profile needs a name"}),
    )
    .unwrap();
    assert_eq!(old.reason, None);
    let newer: ProfileRefusal = serde_json::from_value(
        serde_json::json!({"field": "name", "text": "x", "reason": "name_from_the_future"}),
    )
    .unwrap();
    assert_eq!(
        newer.reason,
        Some(hide_node_link::profiles::ProfileReason::Other)
    );
}

#[test]
fn a_change_made_against_a_list_that_moved_is_made_once_more_on_the_new_one() {
    let link = Scripted::new(vec![
        Err(ErrorCode::Conflict),
        Ok(serde_json::to_value(list(7, None)).unwrap()),
        Ok(applied(None, 8)),
    ]);
    let prepared = job(
        Arc::clone(&link),
        Change::Default {
            kind: "claude-code".into(),
            id: ProfileId("p-work".into()),
        },
        3,
    );
    assert!(prepared.call().result.is_ok());
    let asked = link.asked.lock().unwrap();
    assert!(matches!(
        asked.as_slice(),
        [
            Call::ProfileApply {
                expected_version: 3,
                ..
            },
            Call::ProfileList { values: false },
            Call::ProfileApply {
                expected_version: 7,
                ..
            },
        ]
    ));
}

#[test]
fn an_edit_keeps_what_it_does_not_name_and_patches_the_variables() {
    let link = Scripted::new(vec![
        Ok(serde_json::to_value(list(4, Some(vec![("A", "1"), ("B", "2")]))).unwrap()),
        Ok(applied(None, 5)),
    ]);
    let prepared = job(
        Arc::clone(&link),
        Change::Edit {
            id: ProfileId("p-work".into()),
            name: None,
            program: None,
            env: EnvEdit::Patch {
                set: vec![("B".into(), "3".into()), ("C".into(), "4".into())],
                unset: vec!["A".into()],
            },
        },
        0,
    );
    assert!(prepared.call().result.is_ok());
    let asked = link.asked.lock().unwrap();
    let Call::ProfileApply {
        change: ProfileOp::Edit {
            name, program, env, ..
        },
        expected_version,
    } = &asked[1]
    else {
        panic!("{:?}", asked[1]);
    };
    assert_eq!(*expected_version, 4);
    assert_eq!(name, "work");
    assert_eq!(program.as_deref(), Some("claude-gateway"));
    assert_eq!(
        env,
        &vec![
            ("B".to_owned(), "3".to_owned()),
            ("C".to_owned(), "4".to_owned())
        ]
    );
}

#[test]
fn values_reach_only_the_request_that_asked_for_them() {
    let link = Scripted::new(vec![Ok(serde_json::to_value(list(
        4,
        Some(vec![("A", "1")]),
    ))
    .unwrap())]);
    let prepared = job(
        link,
        Change::Values {
            id: ProfileId("p-work".into()),
        },
        0,
    );
    let outcome = prepared.call();
    assert_eq!(
        outcome.result.as_ref().ok().unwrap().values,
        Some(vec![("A".to_owned(), "1".to_owned())])
    );
    // The list that goes into the snapshot carries no value.
    assert!(
        outcome
            .list
            .unwrap()
            .profiles
            .iter()
            .all(|profile| profile.env.is_none())
    );
}

#[test]
fn a_machine_that_predates_profiles_is_refused_by_its_own_code() {
    let link = Scripted::new(vec![
        Err(ErrorCode::Unsupported),
        Err(ErrorCode::Unsupported),
    ]);
    let prepared = job(
        link,
        Change::Reinstall {
            id: ProfileId("p".into()),
        },
        0,
    );
    assert_eq!(prepared.call().result.err().unwrap().code, UNSUPPORTED);
}

#[test]
fn a_command_is_read_back_as_it_was_sent() {
    for command in [
        ProfileCommand::Add {
            machine: Some("mini".into()),
            kind: "claude".into(),
            name: "work".into(),
            account: Some("~/accounts/work/".into()),
            program: None,
            env: vec!["A=1".into()],
            default: true,
        },
        ProfileCommand::Edit {
            machine: None,
            kind: "codex".into(),
            name: "team".into(),
            rename: None,
            program: None,
            no_program: true,
            env: Vec::new(),
            unset_env: vec!["A".into()],
        },
    ] {
        let wire = serde_json::to_value(&command).unwrap();
        assert_eq!(
            serde_json::from_value::<ProfileCommand>(wire).unwrap(),
            command
        );
        assert!(command.valid());
    }
    assert!(
        !ProfileCommand::List {
            machine: Some(String::new()),
            kind: None,
        }
        .valid()
    );
}
