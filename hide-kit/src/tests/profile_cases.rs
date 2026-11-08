//! The kit's pieces in the accounts profiles run in (PRD agent-profiles
//! D-09, D-20, D-28), against a private HOME.

use super::*;
use crate::{KitProfiles, ProfilePlace, ProfileReport, prepare_account};
use hide_agent_adapter::{AccountRef, Profile, ProfileId};
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

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
        let mode = std::fs::metadata(&made).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{}", made.display());
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
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o777)).unwrap();
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
fn another_spelling_of_the_default_folder_is_the_default_account() {
    let mut fixture = Fixture::new();
    let link = fixture.root.join("claude-link");
    std::os::unix::fs::symlink(fixture.home().join(".claude"), &link).unwrap();
    let profile = claude("p-link", &link.display().to_string());
    added(&fixture, &profile);
    with_profiles(&mut fixture, &[&profile], &[]);

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(place(&report.profiles, "p-link"), &ProfilePlace::Default);
    assert_eq!(recorded_targets(&fixture), Value::Null);

    with_profiles(&mut fixture, &[], &[&profile]);
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        place(&report.profiles, "p-link"),
        &ProfilePlace::Removing { failure: None }
    );
    assert!(hide_hooks(&fixture.home().join(".claude/settings.json")) > 0);
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

/// A profile account whose skills folder is a link to the folder every
/// agent shares has no skills piece of its own: nothing of it is written
/// or taken out there.
#[test]
fn a_skills_folder_linked_to_the_shared_one_stays_the_shared_ones() {
    let mut fixture = Fixture::new();
    let folder = fixture.root.join("accounts/work");
    let profile = claude("p-work", &folder.display().to_string());
    added(&fixture, &profile);
    let shared = crate::agents::SkillDir::Shared.path(fixture.home());
    crate::agents::install_skill(crate::agents::SkillDir::Shared, fixture.home(), false).unwrap();
    std::os::unix::fs::symlink(&shared, folder.join("skills")).unwrap();
    let stub = shared.join(crate::agents::GUIDE_SKILL.name);
    assert!(stub.is_dir());
    with_profiles(&mut fixture, &[&profile], &[]);

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(
        matches!(
            place(&report.profiles, "p-work"),
            ProfilePlace::Own {
                skill: None,
                hook: Some(_),
                ..
            }
        ),
        "{report:?}"
    );
    with_profiles(&mut fixture, &[], &[&profile]);
    apply(&fixture.target, &Scope::automatic());

    assert!(stub.is_dir(), "the shared folder's stubs stay");
    assert_eq!(hide_hooks(&folder.join("settings.json")), 0);
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
    std::os::unix::fs::symlink(&first, &link).unwrap();
    added(&fixture, &linked);
    with_profiles(&mut fixture, &[&linked, &direct], &[]);
    apply(&fixture.target, &Scope::automatic());
    assert!(hide_hooks(&first.join("settings.json")) > 0);
    assert!(hide_hooks(&second.join("settings.json")) > 0);

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
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
