//! `resolve`'s contract (PRD agent-profiles D-12 to D-15, D-29): for each of
//! the seven agents, its default profile, a profile of its own account and
//! the inputs it refuses. The expected places are the agent research's
//! table, written here by hand, never read from the definitions; this file
//! runs on the macOS, Linux and Windows lanes.

use std::ffi::OsString;
use std::path::PathBuf;

use hide_agent_adapter::definition::{By, builtin};
use hide_agent_adapter::{
    AccountKey, AccountPath, AccountRef, NodeEnv, Os, Place, Profile, ProfileId, Refusal, resolve,
};

/// An absolute path on the system the test runs on.
fn absolute(path: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:{}", path.replace('/', "\\")))
    } else {
        PathBuf::from(path)
    }
}

/// `base` with `relative`'s `/`-separated folder names below it.
fn under(base: &std::path::Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

fn home() -> PathBuf {
    absolute("/home/me")
}

fn node() -> NodeEnv {
    NodeEnv::new(Some(home()))
}

fn profile(kind: &str, account: Option<AccountRef>, env: &[(&str, &str)]) -> Profile {
    Profile {
        id: ProfileId("work".to_owned()),
        kind: kind.to_owned(),
        name: "work".to_owned(),
        account,
        program: None,
        env: env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        version: 1,
    }
}

/// Where a place is expected: none, the shared folder, below the home, or
/// below the profile's account folder.
#[derive(Clone, Copy)]
enum At {
    None,
    Shared,
    Home(&'static str),
    Account(&'static str),
}

struct Agent {
    kind: &'static str,
    by: By,
    /// Each variable a profile sets, with its value below the account
    /// folder ("" for the folder itself); a named account sets the name.
    vars: &'static [(&'static str, &'static str)],
    forbid: &'static [&'static str],
    default: [At; 9],
    profile: [At; 9],
}

// The order of AccountPath::ALL: hooks, hook_trust, plugins, skills,
// folder_trust, sessions, login, state, usage.
const AGENTS: [Agent; 7] = [
    Agent {
        kind: "claude-code",
        by: By::Folder,
        vars: &[("CLAUDE_CONFIG_DIR", "")],
        forbid: &[],
        default: [
            At::Home(".claude/settings.json"),
            At::None,
            At::None,
            At::Home(".claude/skills"),
            At::Home(".claude.json"),
            At::Home(".claude/projects"),
            At::Home(".claude.json"),
            At::Home(".claude"),
            At::Home(".claude/.credentials.json"),
        ],
        profile: [
            At::Account("settings.json"),
            At::None,
            At::None,
            At::Account("skills"),
            At::Account(".claude.json"),
            At::Account("projects"),
            At::Account(".claude.json"),
            At::Account(""),
            At::Account(".credentials.json"),
        ],
    },
    Agent {
        kind: "codex",
        by: By::Folder,
        vars: &[("CODEX_HOME", "")],
        forbid: &["CODEX_SQLITE_HOME"],
        default: [
            At::Home(".codex/hooks.json"),
            At::Home(".codex/config.toml"),
            At::None,
            At::Shared,
            At::Home(".codex/config.toml"),
            At::Home(".codex/sessions"),
            At::Home(".codex/auth.json"),
            At::Home(".codex"),
            At::Home(".codex/archived_sessions"),
        ],
        profile: [
            At::Account("hooks.json"),
            At::Account("config.toml"),
            At::None,
            At::Shared,
            At::Account("config.toml"),
            At::Account("sessions"),
            At::Account("auth.json"),
            At::Account(""),
            At::Account("archived_sessions"),
        ],
    },
    Agent {
        kind: "opencode",
        by: By::Folder,
        vars: &[
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
        ],
        forbid: &["OPENCODE_CONFIG_DIR", "OPENCODE_DB"],
        default: [
            At::None,
            At::None,
            At::Home(".config/opencode/plugins"),
            At::Shared,
            At::None,
            At::Home(".local/share/opencode/opencode.db"),
            At::None,
            At::Home(".config/opencode"),
            At::None,
        ],
        profile: [
            At::None,
            At::None,
            At::Account("config/opencode/plugins"),
            At::Shared,
            At::None,
            At::Account("data/opencode/opencode.db"),
            At::None,
            At::Account("config/opencode"),
            At::None,
        ],
    },
    Agent {
        kind: "pi",
        by: By::Folder,
        vars: &[("PI_CODING_AGENT_DIR", "")],
        forbid: &["PI_CODING_AGENT_SESSION_DIR"],
        default: [
            At::None,
            At::None,
            At::Home(".pi/agent/extensions"),
            At::Shared,
            At::Home(".pi/agent/trust.json"),
            At::Home(".pi/agent/sessions"),
            At::Home(".pi/agent/auth.json"),
            At::Home(".pi/agent"),
            At::None,
        ],
        profile: [
            At::None,
            At::None,
            At::Account("extensions"),
            At::Shared,
            At::Account("trust.json"),
            At::Account("sessions"),
            At::Account("auth.json"),
            At::Account(""),
            At::None,
        ],
    },
    Agent {
        kind: "omp",
        by: By::Name,
        vars: &[("OMP_PROFILE", "")],
        forbid: &["PI_CODING_AGENT_DIR", "PI_PROFILE", "PI_CONFIG_DIR"],
        default: [
            At::None,
            At::None,
            At::Home(".omp/agent/extensions"),
            At::Shared,
            At::None,
            At::Home(".omp/agent/sessions"),
            At::Home(".omp/agent/agent.db"),
            At::Home(".omp/agent"),
            At::None,
        ],
        profile: [
            At::None,
            At::None,
            At::Home(".omp/profiles/work/agent/extensions"),
            At::Shared,
            At::None,
            At::Home(".omp/profiles/work/agent/sessions"),
            At::Home(".omp/profiles/work/agent/agent.db"),
            At::Home(".omp/profiles/work/agent"),
            At::None,
        ],
    },
    Agent {
        kind: "grok",
        by: By::Folder,
        vars: &[("GROK_HOME", "")],
        forbid: &[],
        default: [
            At::Home(".grok/hooks/hide.json"),
            At::None,
            At::None,
            At::Shared,
            At::Home(".grok/trusted_folders.toml"),
            At::Home(".grok/sessions"),
            At::Home(".grok/auth.json"),
            At::Home(".grok"),
            At::None,
        ],
        profile: [
            At::Account("hooks/hide.json"),
            At::None,
            At::None,
            At::Shared,
            At::Account("trusted_folders.toml"),
            At::Account("sessions"),
            At::Account("auth.json"),
            At::Account(""),
            At::None,
        ],
    },
    Agent {
        kind: "antigravity",
        by: By::None,
        vars: &[],
        forbid: &[],
        default: [
            At::Home(".gemini/config/hooks.json"),
            At::None,
            At::None,
            At::Home(".gemini/config/skills"),
            At::Home(".gemini/antigravity-cli/settings.json"),
            At::Home(".gemini/antigravity-cli/brain"),
            At::None,
            At::Home(".gemini/config"),
            At::None,
        ],
        // agy has no profile of its own account (D-14).
        profile: [At::None; 9],
    },
];

fn place(at: At, account: &std::path::Path) -> Place {
    match at {
        At::None => Place::Absent,
        At::Shared => Place::Shared,
        At::Home(relative) => Place::At(under(&home(), relative)),
        At::Account(relative) => Place::At(under(account, relative)),
    }
}

fn own_account(agent: &Agent) -> Option<(AccountRef, PathBuf)> {
    match agent.by {
        By::Folder => {
            let folder = absolute("/accounts/work");
            Some((
                AccountRef::Folder(folder.to_string_lossy().into_owned()),
                folder,
            ))
        }
        By::Name => Some((AccountRef::Name("work".to_owned()), home())),
        By::None => None,
    }
}

#[test]
fn the_default_profile_reads_each_agents_default_places_and_sets_nothing() {
    for agent in &AGENTS {
        let definition = builtin(agent.kind).unwrap();
        let account = resolve(definition, &Profile::builtin_default(agent.kind), &node())
            .unwrap_or_else(|refusal| panic!("{}: {refusal}", agent.kind));
        assert_eq!(account.key, AccountKey::Default, "{}", agent.kind);
        assert!(account.env.is_empty(), "{}", agent.kind);
        for (path, at) in AccountPath::ALL.into_iter().zip(agent.default) {
            assert_eq!(
                account.place(path),
                &place(at, &home()),
                "{} {path:?}",
                agent.kind
            );
        }
    }
}

#[test]
fn a_profile_of_its_own_account_sets_its_variables_and_reads_there() {
    for agent in &AGENTS {
        let definition = builtin(agent.kind).unwrap();
        let Some((account_ref, folder)) = own_account(agent) else {
            continue;
        };
        let account = resolve(
            definition,
            &profile(agent.kind, Some(account_ref.clone()), &[("EXTRA", "1")]),
            &node(),
        )
        .unwrap_or_else(|refusal| panic!("{}: {refusal}", agent.kind));
        let expected_key = match &account_ref {
            AccountRef::Folder(folder) => AccountKey::Folder(folder.clone()),
            AccountRef::Name(name) => AccountKey::Name(name.clone()),
        };
        assert_eq!(account.key, expected_key, "{}", agent.kind);
        let mut env: Vec<(String, String)> = agent
            .vars
            .iter()
            .map(|(var, below)| {
                let value = match &account_ref {
                    AccountRef::Folder(_) => under(&folder, below).to_string_lossy().into_owned(),
                    AccountRef::Name(name) => name.clone(),
                };
                ((*var).to_owned(), value)
            })
            .collect();
        env.push(("EXTRA".to_owned(), "1".to_owned()));
        assert_eq!(account.env, env, "{}", agent.kind);
        for (path, at) in AccountPath::ALL.into_iter().zip(agent.profile) {
            assert_eq!(
                account.place(path),
                &place(at, &folder),
                "{} {path:?}",
                agent.kind
            );
        }
    }
}

#[test]
fn a_folder_is_set_as_spelled_with_its_final_separator() {
    let claude = builtin("claude-code").unwrap();
    let folder = format!("{}/", absolute("/accounts/a").display());
    let account = resolve(
        claude,
        &profile("claude-code", Some(AccountRef::Folder(folder.clone())), &[]),
        &node(),
    )
    .unwrap();
    assert_eq!(
        account.env,
        [("CLAUDE_CONFIG_DIR".to_owned(), folder.clone())]
    );
    assert_eq!(account.key, AccountKey::Folder(folder));
}

#[test]
fn the_default_profile_follows_the_named_account_its_node_chose() {
    let omp = builtin("omp").unwrap();
    // The node already runs omp in its `work` profile: the default profile
    // reads there, as omp itself would, and sets nothing.
    let chosen = node().with_variable("OMP_PROFILE", Some("work".into()));
    let account = resolve(omp, &Profile::builtin_default("omp"), &chosen).unwrap();
    assert_eq!(account.key, AccountKey::Default);
    assert!(account.env.is_empty(), "the default sets no variable");
    assert_eq!(
        account.path(AccountPath::Sessions),
        Some(under(&home(), ".omp/profiles/work/agent/sessions").as_path())
    );
    assert_eq!(
        account.path(AccountPath::Login),
        Some(under(&home(), ".omp/profiles/work/agent/agent.db").as_path())
    );
    // An empty name is unset.
    let empty = node().with_variable("OMP_PROFILE", Some(OsString::new()));
    let account = resolve(omp, &Profile::builtin_default("omp"), &empty).unwrap();
    assert_eq!(
        account.path(AccountPath::State),
        Some(under(&home(), ".omp/agent").as_path())
    );
    // A name that leaves the profiles folder is refused.
    for name in ["../escape", ".hidden", "a/b"] {
        let bad = node().with_variable("OMP_PROFILE", Some(name.into()));
        assert_eq!(
            resolve(omp, &Profile::builtin_default("omp"), &bad),
            Err(Refusal::NodeVariable {
                variable: "OMP_PROFILE".to_owned()
            }),
            "{name}"
        );
    }
    // A profile of its own sets its name whatever the node chose.
    let account = resolve(
        omp,
        &profile("omp", Some(AccountRef::Name("home".to_owned())), &[]),
        &chosen,
    )
    .unwrap();
    assert_eq!(account.env, [("OMP_PROFILE".to_owned(), "home".to_owned())]);
    assert_eq!(
        account.path(AccountPath::State),
        Some(under(&home(), ".omp/profiles/home/agent").as_path())
    );
}

#[test]
fn the_default_profile_follows_the_variables_its_node_honours() {
    let codex = builtin("codex").unwrap();
    let elsewhere = absolute("/elsewhere/codex");
    let honoured = node().with_variable("CODEX_HOME", Some(elsewhere.clone().into_os_string()));
    let account = resolve(codex, &Profile::builtin_default("codex"), &honoured).unwrap();
    assert_eq!(account.path(AccountPath::State), Some(elsewhere.as_path()));
    assert!(account.env.is_empty(), "the default sets no variable");
    // An empty value is unset, as Codex reads it.
    let empty = node().with_variable("CODEX_HOME", Some(OsString::new()));
    let account = resolve(codex, &Profile::builtin_default("codex"), &empty).unwrap();
    assert_eq!(
        account.path(AccountPath::State),
        Some(home().join(".codex").as_path())
    );
    let relative = node().with_variable("CODEX_HOME", Some("relative-codex".into()));
    assert_eq!(
        resolve(codex, &Profile::builtin_default("codex"), &relative),
        Err(Refusal::NodeVariable {
            variable: "CODEX_HOME".to_owned()
        })
    );
    // A node that honours no variable reads the default place.
    let account = resolve(codex, &Profile::builtin_default("codex"), &node()).unwrap();
    assert_eq!(
        account.path(AccountPath::State),
        Some(home().join(".codex").as_path())
    );
    // A folder where the default profile reads is the default account.
    assert_eq!(
        resolve(
            codex,
            &profile(
                "codex",
                Some(AccountRef::Folder(elsewhere.to_string_lossy().into_owned())),
                &[]
            ),
            &honoured
        ),
        Err(Refusal::AccountIsDefault)
    );
}

#[test]
fn without_a_home_only_places_that_need_none_resolve() {
    let codex = builtin("codex").unwrap();
    let homeless = NodeEnv::new(None);
    assert_eq!(
        resolve(codex, &Profile::builtin_default("codex"), &homeless),
        Err(Refusal::NoAccountHome)
    );
    let folder = absolute("/accounts/codex");
    let account = resolve(
        codex,
        &profile(
            "codex",
            Some(AccountRef::Folder(folder.to_string_lossy().into_owned())),
            &[],
        ),
        &homeless,
    )
    .unwrap();
    assert_eq!(account.path(AccountPath::State), Some(folder.as_path()));
}

/// The account inputs each agent refuses (D-14, D-15).
#[test]
fn each_agent_refuses_an_account_it_cannot_run_in() {
    for agent in &AGENTS {
        let definition = builtin(agent.kind).unwrap();
        let refused = |account: AccountRef| {
            resolve(
                definition,
                &profile(agent.kind, Some(account), &[]),
                &node(),
            )
            .unwrap_err()
        };
        match agent.by {
            By::Folder => {
                assert_eq!(
                    refused(AccountRef::Folder(String::new())),
                    Refusal::AccountEmpty
                );
                for relative in ["work", "./work", "~/work", "~"] {
                    assert_eq!(
                        refused(AccountRef::Folder(relative.to_owned())),
                        Refusal::AccountRelative,
                        "{} {relative}",
                        agent.kind
                    );
                }
                assert_eq!(
                    refused(AccountRef::Name("work".to_owned())),
                    Refusal::AccountKind { by: By::Folder }
                );
            }
            By::Name => {
                assert_eq!(
                    refused(AccountRef::Name(String::new())),
                    Refusal::AccountEmpty
                );
                for name in ["a/b", "..", ".hidden", "a b", r"a\b"] {
                    assert_eq!(
                        refused(AccountRef::Name(name.to_owned())),
                        Refusal::AccountName,
                        "{name}"
                    );
                }
                assert_eq!(
                    refused(AccountRef::Folder(
                        absolute("/accounts/omp").to_string_lossy().into_owned()
                    )),
                    Refusal::AccountKind { by: By::Name }
                );
            }
            By::None => {
                let refusal = refused(AccountRef::Folder(
                    absolute("/accounts/agy").to_string_lossy().into_owned(),
                ));
                assert!(
                    matches!(&refusal, Refusal::AccountUnsupported { reason } if reason.contains("HOME")),
                    "{refusal:?}"
                );
            }
        }
    }
    // The default place itself, spelled with or without a final separator.
    for (kind, default) in [
        ("claude-code", ".claude"),
        ("codex", ".codex"),
        ("pi", ".pi/agent"),
        ("grok", ".grok"),
    ] {
        let folder = under(&home(), default).to_string_lossy().into_owned();
        for spelling in [
            folder.clone(),
            format!("{folder}{}", std::path::MAIN_SEPARATOR),
        ] {
            assert_eq!(
                resolve(
                    builtin(kind).unwrap(),
                    &profile(kind, Some(AccountRef::Folder(spelling)), &[]),
                    &node()
                ),
                Err(Refusal::AccountIsDefault),
                "{kind}"
            );
        }
    }
}

/// The variables a profile may add (D-29), for every agent: none of the
/// account's, its forbidden ones, the home's or Hide's, within the limits.
#[test]
fn each_agent_refuses_the_variables_a_profile_may_not_set() {
    for agent in &AGENTS {
        let definition = builtin(agent.kind).unwrap();
        let refused = |env: &[(&str, &str)]| {
            resolve(definition, &profile(agent.kind, None, env), &node()).unwrap_err()
        };
        let reserved = |key: &str| Refusal::VariableReserved {
            key: key.to_owned(),
        };
        for (key, _) in agent.vars {
            assert_eq!(refused(&[(key, "x")]), reserved(key), "{}", agent.kind);
        }
        for key in agent.forbid {
            assert_eq!(refused(&[(key, "x")]), reserved(key), "{}", agent.kind);
        }
        for key in ["HOME", "USERPROFILE", "HIDE_STATE_DIR", "HIDE_ANYTHING"] {
            assert_eq!(refused(&[(key, "x")]), reserved(key), "{}", agent.kind);
        }
        for key in ["", "1ABC", "A-B", "A=B", "A\0B"] {
            assert_eq!(refused(&[(key, "x")]), Refusal::VariableName, "{key:?}");
        }
        let long_key = "K".repeat(257);
        assert_eq!(refused(&[(&long_key, "x")]), Refusal::VariableName);
        let long_value = "v".repeat(32 * 1024 + 1);
        assert_eq!(
            refused(&[("LONG", &long_value)]),
            Refusal::VariableValue {
                key: "LONG".to_owned()
            }
        );
        assert_eq!(
            refused(&[("NUL", "a\0b")]),
            Refusal::VariableValue {
                key: "NUL".to_owned()
            }
        );
        assert_eq!(
            refused(&[("TWICE", "a"), ("TWICE", "b")]),
            Refusal::VariableRepeated {
                key: "TWICE".to_owned()
            }
        );
        let many: Vec<(String, String)> = (0..65).map(|n| (format!("V{n}"), "x".into())).collect();
        let many: Vec<(&str, &str)> = many.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(refused(&many), Refusal::TooManyVariables { limit: 64 });
        let fine: Vec<(String, String)> = (0..64).map(|n| (format!("V{n}"), "x".into())).collect();
        let fine: Vec<(&str, &str)> = fine.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let at_limit = resolve(definition, &profile(agent.kind, None, &fine), &node()).unwrap();
        assert_eq!(
            at_limit.key,
            AccountKey::Default,
            "a wrapper profile keeps the default account"
        );
        assert_eq!(at_limit.env.len(), 64);
    }
}

/// Windows reads a variable's name regardless of case, so a profile cannot
/// set the account's variable, the home or Hide's under another case there;
/// elsewhere those are other variables.
#[test]
fn windows_compares_variable_names_regardless_of_case() {
    let codex = builtin("codex").unwrap();
    let on = |os: Os, env: &[(&str, &str)]| {
        let node = NodeEnv::new(Some(home())).on(os);
        resolve(codex, &profile("codex", None, env), &node)
    };
    for key in [
        "codex_home",
        "Home",
        "userprofile",
        "hide_state_dir",
        "codex_sqlite_home",
    ] {
        assert_eq!(
            on(Os::Windows, &[(key, "x")]),
            Err(Refusal::VariableReserved {
                key: key.to_owned()
            }),
            "{key}"
        );
        assert!(on(Os::Unix, &[(key, "x")]).is_ok(), "{key}");
    }
    assert_eq!(
        on(Os::Windows, &[("Path2", "a"), ("PATH2", "b")]),
        Err(Refusal::VariableRepeated {
            key: "PATH2".to_owned()
        })
    );
    assert!(on(Os::Unix, &[("Path2", "a"), ("PATH2", "b")]).is_ok());
}

/// Only a path absolute on the node's system is a folder: Windows'
/// drive-relative and root-relative forms depend on a current drive or folder
/// Hide does not share with the agent.
#[test]
fn an_account_folder_is_absolute_on_the_nodes_system() {
    let codex = builtin("codex").unwrap();
    let on = |os: Os, folder: &str| {
        let node = NodeEnv::new(Some(home())).on(os);
        resolve(
            codex,
            &profile("codex", Some(AccountRef::Folder(folder.to_owned())), &[]),
            &node,
        )
        .map(|account| account.key)
    };
    for folder in [
        r"C:\accounts\codex",
        "C:/accounts/codex",
        r"\\server\share\codex",
    ] {
        assert_eq!(
            on(Os::Windows, folder),
            Ok(AccountKey::Folder(folder.to_owned())),
            "{folder}"
        );
    }
    for folder in [r"C:codex", r"\codex", "/codex", "codex"] {
        assert_eq!(
            on(Os::Windows, folder),
            Err(Refusal::AccountRelative),
            "{folder}"
        );
    }
    assert_eq!(
        on(Os::Unix, "/accounts/codex"),
        Ok(AccountKey::Folder("/accounts/codex".to_owned()))
    );
    for folder in [r"C:\accounts\codex", "codex", r"\\server\share"] {
        assert_eq!(
            on(Os::Unix, folder),
            Err(Refusal::AccountRelative),
            "{folder}"
        );
    }
}

#[test]
fn a_profile_resolves_only_against_its_own_agent_and_says_no_value() {
    let claude = builtin("claude-code").unwrap();
    assert_eq!(
        resolve(claude, &Profile::builtin_default("codex"), &node()),
        Err(Refusal::OtherAgent)
    );
    let refusal = resolve(
        claude,
        &profile("claude-code", None, &[("SECRETLY", "a\0topsecret")]),
        &node(),
    )
    .unwrap_err();
    let said = refusal.to_string();
    assert!(
        said.contains("SECRETLY") && !said.contains("topsecret"),
        "{said}"
    );
    // A definition with no account keeps no folder, and runs only in the
    // default account.
    let gemini = builtin("gemini").unwrap();
    let account = resolve(gemini, &Profile::builtin_default("gemini"), &node()).unwrap();
    assert!(
        AccountPath::ALL
            .into_iter()
            .all(|path| account.place(path) == &Place::Absent)
    );
    assert!(matches!(
        resolve(
            gemini,
            &profile(
                "gemini",
                Some(AccountRef::Folder(
                    absolute("/accounts/g").to_string_lossy().into_owned()
                )),
                &[]
            ),
            &node()
        ),
        Err(Refusal::AccountUnsupported { .. })
    ));
}
