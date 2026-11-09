//! The kit's pieces in the accounts profiles run in (PRD agent-profiles
//! D-09, D-20, D-28), against a private HOME.

use super::*;
use crate::{KitProfiles, ProfilePlace, ProfileReport, prepare_account};
use hide_agent_adapter::{AccountRef, Profile, ProfileId};
use std::collections::{BTreeMap, BTreeSet};

fn claude(id: &str, folder: &str) -> Profile {
    Profile {
        id: ProfileId(id.to_owned()),
        name: id.to_owned(),
        account: Some(AccountRef::Folder(folder.to_owned())),
        version: 1,
        ..Profile::builtin_default("claude-code")
    }
}

fn with_profiles(fixture: &mut Fixture, live: &[&Profile], going: &[&Profile]) {
    fixture.target.profiles = Some(crate::ProfileSource::fixed(KitProfiles {
        live: live.iter().map(|profile| (*profile).clone()).collect(),
        going: going.iter().map(|profile| (*profile).clone()).collect(),
    }));
}

fn place<'a>(reports: &'a [ProfileReport], id: &str) -> &'a ProfilePlace {
    &reports
        .iter()
        .find(|report| report.id.0 == id)
        .unwrap_or_else(|| panic!("no profile {id} in {reports:?}"))
        .place
}

fn hook_state(reports: &[ProfileReport], id: &str) -> ComponentState {
    match place(reports, id) {
        ProfilePlace::Own {
            hook: Some(hook), ..
        } => hook.state,
        other => panic!("{id} has no hook of its own: {other:?}"),
    }
}

/// How many of Hide's hook commands a settings file holds.
fn hide_hooks(settings: &Path) -> usize {
    std::fs::read_to_string(settings)
        .map(|text| text.matches("hide-subagents").count())
        .unwrap_or(0)
}

/// The profile accounts the app's installation recorded.
fn recorded_targets(fixture: &Fixture) -> Value {
    let record = record_of(fixture);
    let part = match record["installs"].get(&fixture.target.install) {
        Some(part) => part.clone(),
        None => record,
    };
    part.get("targets").cloned().unwrap_or(Value::Null)
}

fn added(fixture: &Fixture, profile: &Profile) {
    prepare_account(fixture.home(), profile).unwrap();
}

#[test]
fn two_profiles_of_one_folder_get_the_pieces_once_and_lose_them_with_the_last() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let spelled = folder.display().to_string();
    let first = claude("p-first", &spelled);
    // The same folder spelled with a final `/`, which Claude keys apart.
    let second = claude("p-second", &format!("{spelled}/"));
    added(&fixture, &first);
    added(&fixture, &second);
    with_profiles(&mut fixture, &[&first, &second], &[]);

    let report = apply(&fixture.target, &Scope::automatic());

    for id in ["p-first", "p-second"] {
        assert_eq!(hook_state(&report.profiles, id), ComponentState::Installed);
    }
    let settings = folder.join("settings.json");
    assert_eq!(
        hide_hooks(&settings),
        hide_hooks(&fixture.home().join(".claude/settings.json")),
        "the account holds Hide's hook once"
    );
    assert!(
        folder
            .join("skills")
            .join(crate::agents::GUIDE_SKILL.name)
            .is_dir()
    );
    let targets = recorded_targets(&fixture);
    let targets = targets.as_object().unwrap();
    assert_eq!(targets.len(), 1, "{targets:?}");
    let target = targets.values().next().unwrap();
    assert_eq!(target["refs"], serde_json::json!(["p-first", "p-second"]));

    with_profiles(&mut fixture, &[&first], &[&second]);
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        place(&report.profiles, "p-second"),
        &ProfilePlace::Removing { failure: None }
    );
    assert!(hide_hooks(&settings) > 0, "the first profile still uses it");
    let targets = recorded_targets(&fixture);
    assert_eq!(
        targets.as_object().unwrap().values().next().unwrap()["refs"],
        serde_json::json!(["p-first"])
    );

    with_profiles(&mut fixture, &[], &[&first]);
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        place(&report.profiles, "p-first"),
        &ProfilePlace::Removing { failure: None }
    );
    assert_eq!(hide_hooks(&settings), 0);
    assert!(
        !folder
            .join("skills")
            .join(crate::agents::GUIDE_SKILL.name)
            .exists()
    );
    assert_eq!(recorded_targets(&fixture), Value::Null);
    assert!(
        hide_hooks(&fixture.home().join(".claude/settings.json")) > 0,
        "the default account keeps its hook"
    );
}

#[test]
fn a_missing_profile_folder_is_made_private_and_gets_the_hook() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("new/work");
    let profile = claude("p-new", &folder.display().to_string());

    prepare_account(fixture.home(), &profile).unwrap();

    for made in [fixture.root.join("new"), folder.clone()] {
        assert!(
            hide_platform::fs::private::is_private(&made).unwrap(),
            "{}",
            made.display()
        );
    }
    with_profiles(&mut fixture, &[&profile], &[]);
    let applied = crate::apply_profiles(&fixture.target, &BTreeSet::new()).unwrap();
    assert_eq!(applied.unsaved, None);
    assert_eq!(
        hook_state(&applied.profiles, "p-new"),
        ComponentState::Installed
    );
    assert!(hide_hooks(&folder.join("settings.json")) > 0);
    assert_eq!(
        crate::profile_status(&fixture.target).unwrap(),
        applied.profiles,
        "a read repeats what the pass found"
    );
}

#[test]
fn a_folder_another_account_can_change_is_refused_and_left_alone() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("shared");
    std::fs::create_dir_all(&folder).unwrap();
    super::widen::widen(&folder, true);
    let profile = claude("p-open", &folder.display().to_string());

    let refused = prepare_account(fixture.home(), &profile).unwrap_err();
    assert!(
        refused.contains("can be changed by another account"),
        "{refused}"
    );

    with_profiles(&mut fixture, &[&profile], &[]);
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        hook_state(&report.profiles, "p-open"),
        ComponentState::Failed
    );
    assert!(!folder.join("settings.json").exists());
    assert_eq!(
        recorded_targets(&fixture)[&format!("claude-code:{}", folder.display())]["pieces"],
        Value::Null
    );
}

#[test]
fn a_profile_hide_refuses_makes_no_folder() {
    let fixture = Fixture::new();
    let folder = fixture.root.join("refused");
    let spelled = folder.display().to_string();
    let reserved = Profile {
        env: vec![("CLAUDE_CONFIG_DIR".to_owned(), "/elsewhere".to_owned())],
        ..claude("p-reserved", &spelled)
    };
    let crowded = Profile {
        env: (0..=hide_agent_adapter::account::MAX_PROFILE_ENV)
            .map(|n| (format!("V{n}"), "x".to_owned()))
            .collect(),
        ..claude("p-crowded", &spelled)
    };
    let relative = claude("p-relative", "work");

    for (profile, says) in [
        (&reserved, "CLAUDE_CONFIG_DIR"),
        (&crowded, "64"),
        (&relative, "absolute"),
    ] {
        let refused = prepare_account(fixture.home(), profile).unwrap_err();
        assert!(refused.contains(says), "{}: {refused}", profile.id.0);
    }
    assert!(!folder.exists());
}

#[test]
fn the_agents_switch_reinstall_and_removal_reach_every_account() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    with_profiles(&mut fixture, &[&profile], &[]);
    apply(&fixture.target, &Scope::automatic());
    let settings = folder.join("settings.json");
    assert!(hide_hooks(&settings) > 0);

    // The operator takes the hook out of the account: no pass puts it back
    // until a Reinstall of the profile asks for it.
    let kept = std::fs::read_to_string(&settings).unwrap();
    let mut document: Value = serde_json::from_str(&kept).unwrap();
    document.as_object_mut().unwrap().remove("hooks");
    std::fs::write(&settings, document.to_string()).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        hook_state(&report.profiles, "p-work"),
        ComponentState::Removed
    );
    let reinstall = BTreeSet::from([ProfileId("p-work".to_owned())]);
    let applied = crate::apply_profiles(&fixture.target, &reinstall).unwrap();
    assert_eq!(
        hook_state(&applied.profiles, "p-work"),
        ComponentState::Installed
    );

    let off = Scope {
        agent_off: BTreeSet::from(["claude-code".to_owned()]),
        ..Scope::default()
    };
    let report = apply(&fixture.target, &off);
    assert_eq!(hook_state(&report.profiles, "p-work"), ComponentState::Off);
    assert_eq!(hide_hooks(&settings), 0);

    let on = Scope {
        agent_on: BTreeSet::from(["claude-code".to_owned()]),
        ..Scope::default()
    };
    let report = apply(&fixture.target, &on);
    assert_eq!(
        hook_state(&report.profiles, "p-work"),
        ComponentState::Installed
    );

    let removed = remove(&fixture.target);
    let key = format!("profile:claude-code:{}", folder.display());
    assert!(
        removed
            .agents
            .iter()
            .any(|(code, outcome)| *code == key && *outcome == RemoveOutcome::Removed),
        "{removed:?}"
    );
    assert_eq!(hide_hooks(&settings), 0);
}

/// A file as a pass would leave it: its bytes, and which file it is, which
/// every write the kit makes changes, since each replaces the file whole.
type Seen = Option<(Vec<u8>, hide_platform::fs::identity::FileId)>;

/// Each file a pass may write, as it is now.
fn written(paths: &[PathBuf]) -> Vec<(PathBuf, Seen)> {
    paths
        .iter()
        .map(|path| {
            let seen = std::fs::read(path)
                .ok()
                .map(|bytes| (bytes, hide_platform::fs::identity::file_id(path).unwrap()));
            (path.clone(), seen)
        })
        .collect()
}

/// An account a released build's device helper set up, upgraded by a pass
/// that knows profiles: its pieces stay the agent's default account's, and
/// the pass after the upgrade writes nothing.
#[test]
fn a_released_account_keeps_its_pieces_in_the_default_account_and_a_second_pass_writes_nothing() {
    for released in crate::upgrade_tests::homes() {
        let mut fixture = Fixture::fresh();
        std::fs::remove_file(fixture.home().join(".claude/settings.json")).unwrap();
        crate::upgrade_tests::copy_tree(&released, fixture.home());
        let before: Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap(),
        )
        .unwrap();
        let (install, _) = before["installs"]
            .as_object()
            .unwrap()
            .iter()
            .next()
            .unwrap();
        fixture.target.install = install.clone();
        let default = Profile::builtin_default("claude-code");
        with_profiles(&mut fixture, &[&default], &[]);

        let first = apply(&fixture.target, &Scope::automatic());
        assert!(first.refused.is_none(), "{}: {first:?}", released.display());
        assert_eq!(
            state(&first, ComponentId::ClaudeCodeHook),
            ComponentState::Installed,
            "{}: {first:?}",
            released.display()
        );
        assert_eq!(place(&first.profiles, "default"), &ProfilePlace::Default);
        assert_eq!(recorded_targets(&fixture), Value::Null);
        let installs = &record_of(&fixture)["installs"][install];
        assert!(
            installs.get("targets").is_none(),
            "{}: {installs}",
            released.display()
        );
        let files = [
            fixture.home().join(".hide/kit/installed.json"),
            fixture.home().join(".claude/settings.json"),
            fixture.home().join(".claude.json"),
        ];
        let once = written(&files);

        let second = apply(&fixture.target, &Scope::automatic());

        assert!(second.refused.is_none(), "{second:?}");
        assert_eq!(written(&files), once, "{}", released.display());
    }
}

fn record_of(fixture: &Fixture) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap(),
    )
    .unwrap()
}

/// Profiles a test changes as the node changes its file, read by every
/// pass the way the node's source reads the file.
#[derive(Clone, Default)]
struct ProfileFile(Arc<std::sync::Mutex<KitProfiles>>);

impl ProfileFile {
    fn set(&self, live: &[&Profile], going: &[&Profile]) {
        *self.0.lock().unwrap() = KitProfiles {
            live: live.iter().map(|profile| (*profile).clone()).collect(),
            going: going.iter().map(|profile| (*profile).clone()).collect(),
        };
    }

    fn source(&self) -> crate::ProfileSource {
        let file = self.clone();
        crate::ProfileSource::new(move || Ok(file.0.lock().unwrap().clone()))
    }

    /// A source that waits for `go` before it reads, while its pass holds
    /// the account's lock.
    fn held_source(&self, go: std::sync::mpsc::Receiver<()>) -> crate::ProfileSource {
        let file = self.clone();
        let go = std::sync::Mutex::new(Some(go));
        crate::ProfileSource::new(move || {
            if let Some(go) = go.lock().unwrap().take() {
                go.recv().unwrap();
            }
            Ok(file.0.lock().unwrap().clone())
        })
    }
}

/// Whether another holder has the account's kit lock now.
fn account_lock_held(home: &Path) -> bool {
    use hide_platform::fs::lock::{Mode, Waited, lock_file};
    let path = home.join(".hide/kit/.lock");
    let file = hide_platform::fs::private::open_or_create_file(&path).unwrap();
    matches!(
        lock_file(file, Mode::Exclusive, std::time::Duration::ZERO, &|| false).unwrap(),
        Waited::TimedOut
    )
}

#[test]
fn a_pass_reads_the_profiles_only_while_it_holds_the_accounts_lock() {
    let mut fixture = Fixture::new();
    let home = fixture.home().to_path_buf();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&seen);
    fixture.target.profiles = Some(crate::ProfileSource::new(move || {
        observed.lock().unwrap().push(account_lock_held(&home));
        Ok(KitProfiles::default())
    }));

    apply(&fixture.target, &Scope::automatic());
    crate::apply_profiles(&fixture.target, &BTreeSet::new()).unwrap();

    assert_eq!(*seen.lock().unwrap(), [true, true]);
}

/// A kit pass that waited for the account's lock while a profile was added
/// and installed reads the profile once it has the lock: it keeps the
/// account's pieces rather than taking out what the add just put in.
#[test]
fn a_pass_that_waited_through_an_add_keeps_the_new_accounts_pieces() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    let file = ProfileFile::default();
    let (go, wait) = std::sync::mpsc::channel();
    let mut adding = fixture.target.clone();
    adding.profiles = Some(file.held_source(wait));
    fixture.target.profiles = Some(file.source());

    std::thread::scope(|threads| {
        // The add's own pass holds the lock until the add has saved.
        let add = threads.spawn(|| crate::apply_profiles(&adding, &BTreeSet::new()).unwrap());
        while !account_lock_held(fixture.home()) {
            std::thread::yield_now();
        }
        let waited = threads.spawn(|| apply(&fixture.target, &Scope::automatic()));
        file.set(&[&profile], &[]);
        go.send(()).unwrap();
        add.join().unwrap();
        waited.join().unwrap();
    });

    let settings = folder.join("settings.json");
    assert!(hide_hooks(&settings) > 0);
    let targets = recorded_targets(&fixture);
    assert_eq!(
        targets.as_object().unwrap().values().next().unwrap()["refs"],
        serde_json::json!(["p-work"])
    );
}

/// The mirror: a pass that waited through a removal does not put the
/// removed account's pieces back.
#[test]
fn a_pass_that_waited_through_a_removal_puts_nothing_back() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    let file = ProfileFile::default();
    file.set(&[&profile], &[]);
    fixture.target.profiles = Some(file.source());
    apply(&fixture.target, &Scope::automatic());
    let settings = folder.join("settings.json");
    assert!(hide_hooks(&settings) > 0);
    let (go, wait) = std::sync::mpsc::channel();
    let mut removing = fixture.target.clone();
    removing.profiles = Some(file.held_source(wait));

    std::thread::scope(|threads| {
        let remove = threads.spawn(|| crate::apply_profiles(&removing, &BTreeSet::new()).unwrap());
        while !account_lock_held(fixture.home()) {
            std::thread::yield_now();
        }
        let waited = threads.spawn(|| apply(&fixture.target, &Scope::automatic()));
        file.set(&[], &[&profile]);
        go.send(()).unwrap();
        remove.join().unwrap();
        waited.join().unwrap();
    });

    assert_eq!(hide_hooks(&settings), 0);
    assert_eq!(recorded_targets(&fixture), Value::Null);
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(hide_hooks(&settings), 0);
}

/// Two installations on the account, the app and a device helper, put
/// their hooks into one profile account and share its skill stubs: the
/// first to let the profile go takes out only its own hook, and the stubs
/// leave with the last.
#[test]
fn skill_stubs_another_installation_holds_stay_until_it_lets_go() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    let app = ProfileFile::default();
    let helper = ProfileFile::default();
    app.set(&[&profile], &[]);
    helper.set(&[&profile], &[]);
    fixture.target.profiles = Some(app.source());
    let mut device = fixture.target.clone();
    device.install = hide_agent_hooks::route::install_of_root(&fixture.root.join("helper-root"));
    device.profiles = Some(helper.source());
    let stub = folder.join("skills").join(crate::agents::GUIDE_SKILL.name);

    apply(&fixture.target, &Scope::automatic());
    let device_pass = crate::apply_profiles(&device, &BTreeSet::new()).unwrap();
    assert_eq!(
        hook_state(&device_pass.profiles, "p-work"),
        ComponentState::Installed
    );
    let settings = folder.join("settings.json");
    let both = hide_hooks(&settings);
    assert!(stub.is_dir());

    app.set(&[], &[&profile]);
    apply(&fixture.target, &Scope::automatic());

    assert!(stub.is_dir(), "the helper still holds the stubs");
    let left = hide_hooks(&settings);
    assert!(
        left > 0 && left < both,
        "{left} of {both}: only the app's hook went"
    );

    helper.set(&[], &[&profile]);
    crate::apply_profiles(&device, &BTreeSet::new()).unwrap();

    assert!(!stub.exists());
    assert_eq!(hide_hooks(&settings), 0);
}

/// A profile that named its account through a link: when the link is moved
/// to another account a profile in use holds, the old account's pieces come
/// out of the folder they went into, and the other account keeps its own.
#[test]
fn moving_a_link_takes_out_the_old_accounts_pieces_and_keeps_the_new_ones() {
    let mut fixture = Fixture::new();
    let first = fixture.root.join("accounts/first");
    let second = fixture.root.join("accounts/second");
    let link = fixture.root.join("accounts/current");
    let linked = claude("p-linked", &link.display().to_string());
    let direct = claude("p-direct", &second.display().to_string());
    added(&fixture, &direct);
    hide_platform::fs::private::create_dir_all(&first).unwrap();
    symlink(&first, &link);
    added(&fixture, &linked);
    with_profiles(&mut fixture, &[&linked, &direct], &[]);
    apply(&fixture.target, &Scope::automatic());
    assert!(hide_hooks(&first.join("settings.json")) > 0);
    assert!(hide_hooks(&second.join("settings.json")) > 0);

    std::fs::remove_file(&link).unwrap();
    symlink(&second, &link);
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(hide_hooks(&first.join("settings.json")), 0, "{report:?}");
    assert!(hide_hooks(&second.join("settings.json")) > 0);
    assert!(
        second
            .join("skills")
            .join(crate::agents::GUIDE_SKILL.name)
            .is_dir()
    );
    let targets = recorded_targets(&fixture);
    let targets = targets.as_object().unwrap();
    assert_eq!(targets.len(), 1, "{targets:?}");
    assert_eq!(
        targets.values().next().unwrap()["refs"],
        serde_json::json!(["p-direct", "p-linked"])
    );
    for id in ["p-linked", "p-direct"] {
        assert_eq!(hook_state(&report.profiles, id), ComponentState::Installed);
    }
}

/// Every file and folder under `path` with its bytes, links followed; the
/// path alone when it is a file, nothing when it is not there.
fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            found.insert(path.to_path_buf(), Vec::new());
            for entry in std::fs::read_dir(path).unwrap() {
                found.extend(tree(&entry.unwrap().path()));
            }
        }
        Ok(_) => {
            found.insert(path.to_path_buf(), std::fs::read(path).unwrap());
        }
        Err(_) => {}
    }
    found
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Linked {
    /// No link: the account gets its pieces, the control of every other row.
    Nothing,
    Account,
    Hook,
    Skill,
}

/// How many times Codex was asked to store its trust of Hide's hooks with
/// `folder` as its `CODEX_HOME`.
fn trust_writes(folder: &Path) -> usize {
    std::fs::read_to_string(folder.join("fake-calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == "config/batchWrite")
        .count()
}

/// A link on each layer of a profile account's paths, to a place the
/// default account keeps, to the shared skills folder, or to nothing yet:
/// nothing outside the account is written or taken out through it, Codex is
/// not asked to write its trust through it, the link stays, and the
/// account's other piece comes and goes as usual.
#[test]
fn a_link_on_any_layer_of_a_profile_account_leads_no_write_or_removal_out_of_it() {
    let guide = crate::agents::GUIDE_SKILL.name;
    let stub = format!("skills/{guide}");
    let file = format!("skills/{guide}/SKILL.md");
    // The agent; where the link is, relative to the account's folder ("" is
    // the folder itself); where it leads, relative to the home; the piece it
    // is on.
    let link = |at: &str, to: &str| Some((at.to_owned(), to.to_owned()));
    let rows = [
        ("claude-code", None, Linked::Nothing),
        ("claude-code", link("", ".claude"), Linked::Account),
        (
            "claude-code",
            link("settings.json", ".claude/settings.json"),
            Linked::Hook,
        ),
        (
            "claude-code",
            link("skills", ".agents/skills"),
            Linked::Skill,
        ),
        (
            "claude-code",
            link(&stub, &format!(".agents/{stub}")),
            Linked::Skill,
        ),
        (
            "claude-code",
            link(&file, &format!(".agents/{file}")),
            Linked::Skill,
        ),
        (
            "claude-code",
            link(&file, "elsewhere/SKILL.md"),
            Linked::Skill,
        ),
        ("codex", None, Linked::Nothing),
        ("codex", link("", ".codex"), Linked::Account),
        (
            "codex",
            link("hooks.json", ".codex/hooks.json"),
            Linked::Hook,
        ),
        (
            "codex",
            link("config.toml", ".codex/config.toml"),
            Linked::Hook,
        ),
        (
            "codex",
            link("config.toml", "elsewhere/config.toml"),
            Linked::Hook,
        ),
    ];
    for (kind, linked_at, linked) in rows {
        // Codex's rows need its stand-in app-server, a script started by its
        // first line, which only Unix does (`codex_trust_cases`).
        if cfg!(not(unix)) && kind == "codex" {
            continue;
        }
        let row = match &linked_at {
            Some((at, to)) => format!("{kind}: {at} -> ~/{to}"),
            None => format!("{kind}: no link"),
        };
        let mut fixture = match kind {
            #[cfg(unix)]
            "codex" => {
                let fixture = super::codex_trust_cases::with_codex();
                let home = fixture.home();
                std::fs::write(home.join(".codex/config.toml"), "model = \"kept\"\n").unwrap();
                std::fs::create_dir_all(home.join("elsewhere")).unwrap();
                std::fs::write(home.join("elsewhere/config.toml"), "model = \"outside\"\n")
                    .unwrap();
                // Codex is installed: its program is what says so.
                executable(&home.join(".local/bin/codex"), "#!/bin/sh\n");
                fixture
            }
            _ => Fixture::new(),
        };
        let home = fixture.home().to_path_buf();
        crate::agents::install_skill(crate::agents::SkillDir::Shared, &home, false).unwrap();
        apply(&fixture.target, &Scope::automatic());
        let folder = fixture.root.join("accounts/work");
        let link = linked_at.as_ref().map(|(at, to)| {
            let link = if at.is_empty() {
                hide_platform::fs::private::create_dir_all(folder.parent().unwrap()).unwrap();
                folder.clone()
            } else {
                hide_platform::fs::private::create_dir_all(&folder).unwrap();
                let link = folder.join(at);
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                link
            };
            symlink(&home.join(to), &link);
            (link, home.join(to))
        });
        let profile = Profile {
            kind: kind.to_owned(),
            ..claude("p-work", &folder.display().to_string())
        };
        added(&fixture, &profile);
        let outside = [".agents", ".claude", ".codex", "elsewhere"].map(|name| home.join(name));
        // The stand-in Codex logs every call of the default account's pass
        // beside its files.
        let outside_now = || -> Vec<BTreeMap<PathBuf, Vec<u8>>> {
            outside
                .iter()
                .map(|path| {
                    let mut found = tree(path);
                    found.retain(|path, _| {
                        !path
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("fake-"))
                    });
                    found
                })
                .collect()
        };
        let before = outside_now();

        with_profiles(&mut fixture, &[&profile], &[]);
        let report = apply(&fixture.target, &Scope::automatic());

        assert_eq!(
            outside_now(),
            before,
            "{row}: nothing outside the account is written"
        );
        if kind == "codex" && linked != Linked::Account {
            let asked = usize::from(linked == Linked::Nothing);
            assert_eq!(trust_writes(&folder), asked, "{row}: Codex's trust write");
        }
        let state = |piece: Option<&crate::PieceReport>| piece.map(|piece| piece.state);
        match (linked, place(&report.profiles, "p-work")) {
            (Linked::Account, ProfilePlace::Default) => {}
            (Linked::Nothing, ProfilePlace::Own { hook, skill, .. }) => {
                assert_eq!(
                    state(hook.as_ref()),
                    Some(ComponentState::Installed),
                    "{row}: {hook:?}"
                );
                let skill = state(skill.as_ref());
                assert!(
                    skill.is_none_or(|skill| skill == ComponentState::Installed),
                    "{row}"
                );
            }
            (
                Linked::Hook,
                ProfilePlace::Own {
                    hook: Some(hook),
                    skill,
                    ..
                },
            ) => {
                assert_eq!(hook.state, ComponentState::Absent, "{row}: {hook:?}");
                assert!(
                    hook.reason.as_deref().unwrap().contains("is a link"),
                    "{row}"
                );
                let skill = state(skill.as_ref());
                assert!(
                    skill.is_none_or(|skill| skill == ComponentState::Installed),
                    "{row}"
                );
            }
            (
                Linked::Skill,
                ProfilePlace::Own {
                    hook: Some(hook),
                    skill: Some(skill),
                    ..
                },
            ) => {
                assert_eq!(skill.state, ComponentState::Absent, "{row}: {skill:?}");
                assert!(
                    skill.reason.as_deref().unwrap().contains("is a link"),
                    "{row}"
                );
                assert_eq!(hook.state, ComponentState::Installed, "{row}");
            }
            (_, other) => panic!("{row}: {other:?}"),
        }

        with_profiles(&mut fixture, &[], &[&profile]);
        let report = apply(&fixture.target, &Scope::automatic());

        assert_eq!(
            place(&report.profiles, "p-work"),
            &ProfilePlace::Removing { failure: None },
            "{row}"
        );
        assert_eq!(
            outside_now(),
            before,
            "{row}: nothing outside the account is taken out"
        );
        if let Some((link, to)) = &link {
            let kept = std::fs::symlink_metadata(link).unwrap();
            assert!(kept.file_type().is_symlink(), "{row}: the link stays");
            assert_eq!(&std::fs::read_link(link).unwrap(), to, "{row}");
        }
        let hooks = match kind {
            "codex" => folder.join("hooks.json"),
            _ => folder.join("settings.json"),
        };
        if linked != Linked::Account && linked != Linked::Hook {
            let left = std::fs::read_to_string(&hooks).unwrap_or_default();
            assert!(
                !left.contains("hide-agent-hooks"),
                "{row}: the hook went: {left}"
            );
        }
        if linked != Linked::Account && linked != Linked::Skill {
            assert!(
                !folder.join("skills").join(guide).exists(),
                "{row}: the stubs went"
            );
        }
        assert_eq!(recorded_targets(&fixture), Value::Null, "{row}");
    }
}

/// A worktree Hide made under `root`: a folder with a `.git` entry.
fn made_worktree(root: &Path) -> PathBuf {
    let tree = root.join("repo-task");
    std::fs::create_dir_all(tree.join(".git")).unwrap();
    std::fs::canonicalize(&tree).unwrap()
}

#[test]
fn the_folder_trust_hide_wrote_in_a_profile_account_leaves_with_its_last_profile() {
    use hide_agent_hooks::account::AccountAt;
    use hide_agent_hooks::folder_trust::{self, Agent, Tools};
    let mut fixture = Fixture::new();
    let home = fixture.home().to_path_buf();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    with_profiles(&mut fixture, &[&profile], &[]);
    apply(&fixture.target, &Scope::automatic());

    crate::trust_writable(&home, &profile).unwrap();
    let adapter = hide_agent_adapter::adapter("claude-code").unwrap();
    let account = adapter
        .resolve(
            &profile,
            &hide_agent_adapter::NodeEnv::new(Some(home.clone())),
        )
        .unwrap();
    let at = AccountAt::Account {
        home: &home,
        account: &account,
    };
    let tree = made_worktree(&fixture.root);
    // Claude Code keys a folder by its path as written.
    let key = tree.to_str().unwrap().to_owned();
    folder_trust::trust_worktree(Agent::ClaudeCode, at, &tree, Tools::none()).unwrap();
    folder_trust::trust_worktree(Agent::ClaudeCode, &home, &tree, Tools::none()).unwrap();
    let in_account = Agent::ClaudeCode.settings_path(at);
    let in_default = Agent::ClaudeCode.settings_path(&home);
    assert!(in_account.starts_with(&folder), "{in_account:?}");
    let trusted = |file: &Path| {
        serde_json::from_slice::<Value>(&std::fs::read(file).unwrap()).unwrap()["projects"]
            .get(&key)
            .is_some()
    };
    assert!(trusted(&in_account) && trusted(&in_default));

    with_profiles(&mut fixture, &[], &[&profile]);
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        place(&report.profiles, "p-work"),
        &ProfilePlace::Removing { failure: None }
    );
    assert!(!trusted(&in_account), "the account's trust left with it");
    assert!(trusted(&in_default), "the default account's trust stays");
}

/// A trust Hide could not take out of a going profile account keeps the
/// account's target and says why, and the pass after the file is mended
/// takes the trust out and lets the target go: the removal is owned until
/// it is done, never forgotten on a failure.
#[test]
fn a_profile_account_trust_that_could_not_go_stays_owned_until_a_later_pass_takes_it_out() {
    use hide_agent_hooks::account::AccountAt;
    use hide_agent_hooks::folder_trust::{self, Agent, Tools};
    let mut fixture = Fixture::new();
    let home = fixture.home().to_path_buf();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    with_profiles(&mut fixture, &[&profile], &[]);
    apply(&fixture.target, &Scope::automatic());
    let account = hide_agent_adapter::adapter("claude-code")
        .unwrap()
        .resolve(
            &profile,
            &hide_agent_adapter::NodeEnv::new(Some(home.clone())),
        )
        .unwrap();
    let at = AccountAt::Account {
        home: &home,
        account: &account,
    };
    let tree = made_worktree(&fixture.root);
    let key = tree.to_str().unwrap().to_owned();
    folder_trust::trust_worktree(Agent::ClaudeCode, at, &tree, Tools::none()).unwrap();
    let in_account = Agent::ClaudeCode.settings_path(at);
    let written = std::fs::read(&in_account).unwrap();
    std::fs::write(&in_account, "{ not json").unwrap();

    with_profiles(&mut fixture, &[], &[&profile]);
    let report = apply(&fixture.target, &Scope::automatic());

    let ProfilePlace::Removing {
        failure: Some(failure),
    } = place(&report.profiles, "p-work")
    else {
        panic!("the removal reports its failure: {:?}", report.profiles);
    };
    assert!(failure.contains(".claude.json"), "{failure}");
    assert_ne!(
        recorded_targets(&fixture),
        Value::Null,
        "the target is kept"
    );

    std::fs::write(&in_account, &written).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        place(&report.profiles, "p-work"),
        &ProfilePlace::Removing { failure: None }
    );
    let trust = serde_json::from_slice::<Value>(&std::fs::read(&in_account).unwrap()).unwrap();
    assert!(trust["projects"].get(&key).is_none(), "{trust}");
    assert_eq!(recorded_targets(&fixture), Value::Null);
}

#[test]
fn a_profile_account_whose_trust_file_leads_out_of_it_is_refused_a_trust() {
    let mut fixture = Fixture::new();
    let home = fixture.home().to_path_buf();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    with_profiles(&mut fixture, &[&profile], &[]);
    apply(&fixture.target, &Scope::automatic());
    let theirs = home.join(".claude.json");
    std::fs::write(&theirs, "{}\n").unwrap();
    symlink(&theirs, &folder.join(".claude.json"));

    let refused = crate::trust_writable(&home, &profile).unwrap_err();

    assert!(refused.contains(".claude.json"), "{refused}");
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "{}\n");
}
