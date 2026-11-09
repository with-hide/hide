//! Which registered account holds a session a resume names, and the line a
//! profile account's sessions are read and listed inside (PRD agent-profiles
//! B19, D-36, B29).

use std::path::{Path, PathBuf};

use hide_agent_adapter::{AccountKey, AccountRef, Profile};
use hide_session::session_account::{self, AMBIGUOUS, SessionAccountRequest};
use hide_session::{Accounts, Agent, RootRefusal};

fn profile(kind: &str, folder: &Path) -> Profile {
    Profile {
        account: Some(AccountRef::Folder(folder.to_str().unwrap().to_owned())),
        ..Profile::builtin_default(kind)
    }
}

fn key(folder: &Path) -> AccountKey {
    AccountKey::Folder(folder.to_str().unwrap().to_owned())
}

fn file(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "{}\n").unwrap();
    path.to_path_buf()
}

fn of(accounts: &Accounts, agent: Agent, path: &Path) -> Result<AccountKey, String> {
    session_account::read(
        accounts,
        &SessionAccountRequest {
            agent,
            reference_kind: "path".into(),
            reference_value: path.to_str().unwrap().into(),
        },
    )
    .map_err(String::from)
}

fn listed(accounts: &Accounts) -> Vec<String> {
    hide_session::links::candidates(accounts, 0, None)
        .unwrap()
        .into_iter()
        .map(|candidate| candidate.path)
        .collect()
}

#[test]
fn a_session_file_belongs_to_the_innermost_registered_account_that_holds_it() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    let outer = root.join("outer");
    let inner = outer.join("projects/inner");
    let accounts = Accounts::new(&home).with_profiles([
        &profile("claude-code", &outer),
        &profile("claude-code", &inner),
    ]);
    let in_inner = file(&inner.join("projects/-w/s.jsonl"));
    let in_outer = file(&outer.join("projects/-w/s.jsonl"));
    let in_default = file(&home.join(".claude/projects/-w/s.jsonl"));
    let elsewhere = file(&root.join("stranger/projects/-w/s.jsonl"));

    assert_eq!(of(&accounts, Agent::Claude, &in_inner), Ok(key(&inner)));
    assert_eq!(of(&accounts, Agent::Claude, &in_outer), Ok(key(&outer)));
    assert_eq!(
        of(&accounts, Agent::Claude, &in_default),
        Ok(AccountKey::Default)
    );
    assert_eq!(
        of(&accounts, Agent::Claude, &elsewhere),
        Err(RootRefusal::Outside.reason().to_owned())
    );
    assert_eq!(
        of(
            &accounts,
            Agent::Claude,
            &inner.join("projects/-w/gone.jsonl")
        ),
        Err(RootRefusal::Missing.reason().to_owned())
    );
}

/// An agent whose accounts are named (omp's profiles) keeps a profile's
/// sessions in the folder its name gives under the home, which is read in
/// that account and in no other (journey run 42586901056: an omp profile's
/// session on Windows was refused as outside every account).
#[test]
fn a_named_accounts_session_belongs_to_that_account() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap().join("home");
    let journey = Profile {
        account: Some(AccountRef::Name("journey".to_owned())),
        ..Profile::builtin_default("omp")
    };
    let accounts = Accounts::new(&home).with_profiles([&journey]);
    let workspace = "--w--";
    let in_profile = file(
        &home
            .join(".omp")
            .join("profiles")
            .join("journey")
            .join("agent")
            .join("sessions")
            .join(workspace)
            .join("s.jsonl"),
    );
    let in_default = file(
        &home
            .join(".omp")
            .join("agent")
            .join("sessions")
            .join(workspace)
            .join("s.jsonl"),
    );
    let in_other = file(
        &home
            .join(".omp")
            .join("profiles")
            .join("other")
            .join("agent")
            .join("sessions")
            .join(workspace)
            .join("s.jsonl"),
    );

    assert_eq!(
        of(&accounts, Agent::Omp, &in_profile),
        Ok(AccountKey::Name("journey".to_owned()))
    );
    assert_eq!(
        of(&accounts, Agent::Omp, &in_default),
        Ok(AccountKey::Default)
    );
    assert!(hide_session::inside_session_root(&accounts, &[Agent::Omp], &in_profile).is_ok());
    assert_eq!(
        hide_session::inside_session_root(&accounts, &[Agent::Omp], &in_other),
        Err(RootRefusal::Outside)
    );
}

/// The node confirms a session twice: once by the engine's native path and
/// again by the resolved path it returns. On Windows, a long file can keep
/// its extended path prefix while its shorter account home does not.
#[test]
fn a_long_named_account_session_stays_in_its_account_after_resolution() {
    let root = tempfile::tempdir().unwrap();
    let home = hide_platform::fs::identity::canonical(root.path())
        .unwrap()
        .join("home");
    let journey = Profile {
        account: Some(AccountRef::Name("journey".to_owned())),
        ..Profile::builtin_default("omp")
    };
    let accounts = Accounts::new(&home).with_profiles([&journey]);
    let name = format!("session-{}.jsonl", "x".repeat(220));
    let native = file(
        &home
            .join(".omp/profiles/journey/agent/sessions/--w--")
            .join(name),
    );
    assert!(native.as_os_str().len() > 259);

    let resolved = hide_session::inside_session_root(&accounts, &[Agent::Omp], &native).unwrap();
    assert_eq!(
        hide_session::inside_session_root(&accounts, &[Agent::Omp], &resolved),
        Ok(resolved.clone())
    );
    let expected = AccountKey::Name("journey".to_owned());
    assert_eq!(accounts.account_of(Agent::Omp, &resolved), Some(&expected));
    assert_eq!(of(&accounts, Agent::Omp, &resolved), Ok(expected));
}

/// A link below a profile's folder never makes a session of an account the
/// node did not register readable or listed, nor does `..` in a path; the
/// default account's own link is followed as before profiles.
#[cfg(unix)]
#[test]
fn a_link_below_a_profile_account_leads_nowhere_while_the_default_account_s_link_is_followed() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    let a = root.join("a");
    let codex_a = root.join("codex-a");
    let middle = root.join("middle");
    let b = root.join("b");
    let theirs = file(&b.join("projects/-w/s.jsonl"));
    let codex_theirs = file(&b.join("sessions/2026/11/09/rollout-1.jsonl"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&codex_a).unwrap();
    symlink(b.join("projects"), a.join("projects")).unwrap();
    symlink(b.join("sessions"), codex_a.join("sessions")).unwrap();
    // A folder between the account's sessions folder and the file.
    std::fs::create_dir_all(middle.join("projects")).unwrap();
    symlink(b.join("projects/-w"), middle.join("projects/-w")).unwrap();
    let linked_default = file(&root.join("elsewhere/claude-projects/-w/s.jsonl"));
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    symlink(
        root.join("elsewhere/claude-projects"),
        home.join(".claude/projects"),
    )
    .unwrap();
    let accounts = Accounts::new(&home).with_profiles([
        &profile("claude-code", &a),
        &profile("claude-code", &middle),
        &profile("codex", &codex_a),
    ]);
    let outside = Err(RootRefusal::Outside);

    for path in [
        a.join("projects/-w/s.jsonl"),
        middle.join("projects/-w/s.jsonl"),
        a.join("projects/../../b/projects/-w/s.jsonl"),
        theirs.clone(),
    ] {
        assert_eq!(
            hide_session::inside_session_root(&accounts, &[Agent::Claude], &path),
            outside,
            "{path:?}"
        );
        assert!(of(&accounts, Agent::Claude, &path).is_err(), "{path:?}");
    }
    assert_eq!(
        hide_session::inside_session_root(
            &accounts,
            &[Agent::Codex],
            &codex_a.join("sessions/2026/11/09/rollout-1.jsonl")
        ),
        outside
    );
    let listed = listed(&accounts);
    assert!(
        !listed.iter().any(|path| path.contains("/b/")
            || path.starts_with(a.to_str().unwrap())
            || path.starts_with(codex_a.to_str().unwrap())),
        "{listed:?} lists {theirs:?} or {codex_theirs:?}"
    );

    let default = home.join(".claude/projects/-w/s.jsonl");
    assert_eq!(
        hide_session::inside_session_root(&accounts, &[Agent::Claude], &default),
        Ok(linked_default)
    );
    assert_eq!(
        of(&accounts, Agent::Claude, &default),
        Ok(AccountKey::Default)
    );
    assert!(
        listed.iter().any(|path| Path::new(path) == default),
        "{listed:?}"
    );
}

/// An OpenCode database at `folder`'s profile place holding `ids`.
fn opencode_account(folder: &Path, ids: &[&str]) {
    let database = folder.join("data/opencode/opencode.db");
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let sql = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/adapters/opencode-1.18.30/opencode.sql"),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch(&sql).unwrap();
    connection.execute("DELETE FROM session", []).unwrap();
    for id in ids {
        connection
            .execute(
                "INSERT INTO session (id, project_id, slug, directory, title, version, \
                 time_created, time_updated) VALUES (?1, 'prj_1', 'otter', '/work/app', 't', \
                 '1.18.30', 1790989200000, 1790989331000)",
                [id],
            )
            .unwrap();
    }
}

fn opencode_of(accounts: &Accounts, id: &str) -> Result<AccountKey, String> {
    session_account::read(
        accounts,
        &SessionAccountRequest {
            agent: Agent::OpenCode,
            reference_kind: "id".into(),
            reference_value: id.into(),
        },
    )
    .map_err(String::from)
}

#[test]
fn an_opencode_session_belongs_to_the_one_account_whose_database_holds_it() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));
    opencode_account(&a, &["ses_only_in_a", "ses_in_both"]);
    opencode_account(&b, &["ses_in_both"]);
    std::fs::create_dir_all(c.join("data/opencode")).unwrap();
    std::fs::write(c.join("data/opencode/opencode.db"), "not a database").unwrap();
    // `a` spelled twice is two accounts of one database, read once. A
    // Windows extended path accepts only its native trailing separator.
    let a_again = PathBuf::from(format!(
        "{}{sep}",
        a.display(),
        sep = std::path::MAIN_SEPARATOR
    ));
    let two = Accounts::new(&home).with_profiles([
        &profile("opencode", &a),
        &profile("opencode", &a_again),
        &profile("opencode", &b),
    ]);

    assert_eq!(opencode_of(&two, "ses_only_in_a"), Ok(key(&a)));
    assert_eq!(opencode_of(&two, "ses_in_both"), Err(AMBIGUOUS.to_owned()));
    assert_eq!(
        opencode_of(&two, "ses_nowhere"),
        Err(RootRefusal::Missing.reason().to_owned())
    );
    let broken =
        Accounts::new(&home).with_profiles([&profile("opencode", &c), &profile("opencode", &a)]);
    let refused = opencode_of(&broken, "ses_only_in_a").unwrap_err();
    assert_ne!(
        refused,
        RootRefusal::Missing.reason(),
        "a database that cannot be read is no answer"
    );
}

fn by_id(accounts: &Accounts, agent: Agent, id: &str) -> Result<AccountKey, String> {
    session_account::read(
        accounts,
        &SessionAccountRequest {
            agent,
            reference_kind: "id".into(),
            reference_value: id.into(),
        },
    )
    .map_err(String::from)
}

/// A Claude Code or Codex resume by id alone keeps the default account while
/// that is the only one the node keeps for the agent, a profile that runs in
/// it included; another account of the agent, or registered accounts that
/// may not be all there are, refuse it rather than name the default.
#[test]
fn a_session_named_by_id_alone_keeps_the_default_account_only_while_it_is_the_agents_one() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    let wrapper = Profile {
        name: "wrapper".into(),
        program: Some("claude-gw".into()),
        ..Profile::builtin_default("claude-code")
    };
    let alone = Accounts::new(&home).with_profiles([&wrapper]);
    for agent in [Agent::Claude, Agent::Codex] {
        assert_eq!(by_id(&alone, agent, "019a-s"), Ok(AccountKey::Default));
    }
    assert_eq!(
        by_id(&alone, Agent::Claude, "--resume"),
        Err("session_id_invalid".to_owned())
    );
    assert_eq!(
        by_id(&alone, Agent::Pi, "019a-s"),
        Err("session_kind_unsupported".to_owned()),
        "an agent whose reader proves its sessions names their file"
    );

    let claude_work =
        Accounts::new(&home).with_profiles([&wrapper, &profile("claude-code", &root.join("a"))]);
    assert_eq!(
        by_id(&claude_work, Agent::Claude, "019a-s"),
        Err(session_account::UNPROVEN.to_owned())
    );
    assert_eq!(
        by_id(&claude_work, Agent::Codex, "019a-s"),
        Ok(AccountKey::Default)
    );

    let unsettled = hide_session::account::ACCOUNTS_UNSETTLED.to_owned();
    let unresolved = Accounts::new(&home).with_profiles([&Profile {
        account: Some(AccountRef::Folder("relative/work".into())),
        ..Profile::builtin_default("claude-code")
    }]);
    assert_eq!(
        by_id(&unresolved, Agent::Claude, "019a-s"),
        Err(unsettled.clone())
    );
    let unread = Accounts::new(&home).unread();
    for agent in [Agent::Claude, Agent::Codex] {
        assert_eq!(by_id(&unread, agent, "019a-s"), Err(unsettled.clone()));
    }
}
