//! The store of skills against a private HOME (PRD skill-mcp-manager): a
//! repository and a folder put skills into every agent's skills folder and
//! each profile's, and the switches, the undo list, adoption and the follow
//! of a folder do what the operator sees, on every system.
//!
//! A git repository in the fixture stands in for GitHub: the store reads it
//! with the same `git` and the same commands, by a `file://` address.

use super::*;
use crate::store::{
    Carried, CellState, ItemKind, Source, SourceSpec, StoreChange, StoreRefusalCause, StoreReport,
    TrashCause, UNDO_LIMIT, Version,
};
use hide_agent_adapter::{AccountRef, Profile, ProfileId};
use std::collections::BTreeMap;

/// Every supported agent, installed and switched on, with the folders the
/// agents make themselves, and the account's `git`.
fn every_agent() -> Fixture {
    let fixture = Fixture::new();
    let mut record = record::load(fixture.home(), &fixture.target.install).unwrap();
    for adapter in agents::ADAPTERS {
        installed(&fixture.home().join(".local/bin"), adapter.executables[0]);
        record.set_agent_choice(adapter.id, true);
    }
    record::save(fixture.home(), &record).unwrap();
    std::fs::create_dir_all(fixture.home().join(".gemini/config")).unwrap();
    with_git(&fixture);
    fixture
}

/// This machine's `git`, where the fixture's account finds its programs.
fn with_git(fixture: &Fixture) {
    let search = std::env::var_os("PATH").expect("the test has a PATH");
    let git = hide_platform::programs::Program::find(&search, "git").expect("git is installed");
    let bin = fixture.home().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(git.path(), bin.join("git")).unwrap();
    #[cfg(windows)]
    std::fs::write(
        bin.join("git.cmd"),
        format!("@\"{}\" %*\r\n", git.path().display()),
    )
    .unwrap();
}

/// Runs the fixture's git in `dir`, with no configuration of the account
/// running the tests.
fn git(fixture: &Fixture, dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(["-c", "tag.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .env("HOME", fixture.home())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_md(name: &str, line: &str) -> String {
    format!("---\nname: {name}\ndescription: \"{name} for tests\"\n---\n\n{line}\n")
}

/// A repository with two skills under `skills/`, one with a script, and
/// its address. The account's git turns line endings into the system's on
/// checkout, as Git for Windows does by default; the store still takes the
/// files as the commit holds them.
fn repository(fixture: &Fixture) -> (PathBuf, String) {
    write(
        &fixture.home().join(".gitconfig"),
        "[core]\n\tautocrlf = true\n",
    );
    let repo = fixture.root.join("upstream");
    write(
        &repo.join("skills/archify/SKILL.md"),
        &skill_md("archify", "Draw it."),
    );
    write(
        &repo.join("skills/archify/scripts/run.sh"),
        "#!/bin/sh\necho run\n",
    );
    write(
        &repo.join("skills/notes/SKILL.md"),
        &skill_md("notes", "Take notes."),
    );
    write(&repo.join("README.md"), "not a skill\n");
    git(fixture, &fixture.root, &["init", "-q", "upstream"]);
    git(fixture, &repo, &["add", "."]);
    git(fixture, &repo, &["commit", "-q", "-m", "first"]);
    let url = format!("file://{}", repo.display().to_string().replace('\\', "/"));
    let url = if url.starts_with("file:///") {
        url
    } else {
        url.replacen("file://", "file:///", 1)
    };
    (repo, url)
}

fn store(fixture: &Fixture, changes: Vec<StoreChange>) -> StoreReport {
    let report = crate::apply_store(&fixture.target, changes);
    assert_eq!(report.refused, None, "{report:#?}");
    for outcome in &report.outcomes {
        assert_eq!(outcome.refused, None, "{outcome:#?}");
    }
    report
}

/// Installs the named skills of the source's answer.
fn install(fixture: &Fixture, source: SourceSpec, names: &[&str]) -> StoreReport {
    let inspected = store(fixture, vec![StoreChange::Inspect { source }]);
    let preview = inspected.outcomes[0]
        .made
        .clone()
        .expect("the source answered");
    store(
        fixture,
        vec![StoreChange::Install {
            preview,
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        }],
    )
}

fn cells(report: &StoreReport, id: &str) -> BTreeMap<String, CellState> {
    report
        .items
        .iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("no {id} in {report:#?}"))
        .cells
        .iter()
        .map(|cell| (cell.column.clone(), cell.state))
        .collect()
}

fn all(state: CellState) -> BTreeMap<String, CellState> {
    ["agents", "antigravity", "claude"]
        .into_iter()
        .map(|column| (column.to_owned(), state))
        .collect()
}

/// The three skills folders of the default accounts.
fn folders(fixture: &Fixture) -> [PathBuf; 3] {
    let home = fixture.home();
    [
        home.join(".claude/skills"),
        home.join(".agents/skills"),
        home.join(".gemini/config/skills"),
    ]
}

fn claude_profile(fixture: &mut Fixture) -> PathBuf {
    let folder = fixture.root.join("accounts/work");
    let profile = Profile {
        id: ProfileId("p-work".to_owned()),
        name: "work".to_owned(),
        account: Some(AccountRef::Folder(folder.display().to_string())),
        version: 1,
        ..Profile::builtin_default("claude-code")
    };
    prepare_account(fixture.home(), &profile).unwrap();
    fixture.target.profiles = Some(ProfileSource::fixed(KitProfiles {
        live: vec![profile],
        going: Vec::new(),
    }));
    folder.join("skills")
}

/// Every file and folder under `root` with its bytes and its time, so a
/// pass that rewrote a file with the same bytes still reads as a change.
fn tree(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, Option<std::time::SystemTime>)> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            let metadata = std::fs::symlink_metadata(entry.path()).unwrap();
            let bytes = metadata
                .is_file()
                .then(|| std::fs::read(entry.path()).unwrap());
            found.insert(entry.path(), (bytes, metadata.modified().ok()));
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    found
}

#[test]
fn a_repository_skill_reaches_every_skills_folder_and_profile_and_a_second_pass_changes_nothing() {
    let mut fixture = every_agent();
    let profile_skills = claude_profile(&mut fixture);
    let (repo, url) = repository(&fixture);
    let commit = git(&fixture, &repo, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let inspected = store(
        &fixture,
        vec![StoreChange::Inspect {
            source: SourceSpec::Git { url: url.clone() },
        }],
    );
    let preview = inspected.previews.last().unwrap();
    assert_eq!(preview.commit.as_deref(), Some(commit.as_str()));
    let names: Vec<&str> = preview
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names, ["archify", "notes"]);
    let archify = &preview.skills[0];
    assert_eq!(archify.path, "skills/archify");
    assert_eq!(
        preview.agents.len(),
        agents::ADAPTERS.len(),
        "{:?}",
        preview.agents
    );

    let report = store(
        &fixture,
        vec![StoreChange::Install {
            preview: preview.id.clone(),
            names: vec!["archify".to_owned()],
        }],
    );

    assert_eq!(cells(&report, "skill:archify"), all(CellState::On));
    let item = &report.items[0];
    assert_eq!(item.version, Version::Commit(commit[..7].to_owned()));
    assert!(matches!(&item.source, Source::Git { path, .. } if path == "skills/archify"));
    for folder in folders(&fixture).iter().chain([&profile_skills]) {
        assert_eq!(
            std::fs::read_to_string(folder.join("archify/SKILL.md")).unwrap(),
            skill_md("archify", "Draw it."),
            "{}",
            folder.display()
        );
        assert!(folder.join("archify/scripts/run.sh").is_file());
        assert!(
            !folder.join("notes").exists(),
            "only the chosen skill goes in"
        );
    }
    // Hide's own stubs are no item of the store (B35).
    assert!(report.unmanaged.iter().all(|found| found.name != "hide"));
    assert!(report.items.iter().all(|item| item.name != "hide"));

    let before = tree(fixture.home());
    let profile_before = tree(&profile_skills);
    let again = store(&fixture, Vec::new());
    assert_eq!(cells(&again, "skill:archify"), all(CellState::On));
    assert_eq!(tree(fixture.home()), before, "a second pass writes nothing");
    assert_eq!(tree(&profile_skills), profile_before);
}

#[test]
fn a_tagged_commit_shows_its_tag_and_a_newer_commit_is_an_update_taken_only_when_asked() {
    let fixture = every_agent();
    let (repo, url) = repository(&fixture);
    git(&fixture, &repo, &["tag", "v1.0.0"]);
    let report = install(&fixture, SourceSpec::Git { url }, &["archify"]);
    assert_eq!(report.items[0].version, Version::Tag("v1.0.0".to_owned()));

    write(
        &repo.join("skills/archify/SKILL.md"),
        &skill_md("archify", "Draw it better."),
    );
    write(&repo.join("skills/archify/scripts/new.sh"), "#!/bin/sh\n");
    git(&fixture, &repo, &["add", "."]);
    git(&fixture, &repo, &["commit", "-q", "-m", "draw it better"]);
    let newer = git(&fixture, &repo, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let checked = store(&fixture, vec![StoreChange::CheckUpdates]);
    assert!(checked.checked_at_unix_ms.is_some());
    assert_eq!(
        checked.items[0].update,
        Some(Version::Commit(newer[..7].to_owned()))
    );
    let claude = fixture.home().join(".claude/skills/archify/SKILL.md");
    assert_eq!(
        std::fs::read_to_string(&claude).unwrap(),
        skill_md("archify", "Draw it."),
        "Hide never updates by itself (D-14)"
    );

    let previewed = store(
        &fixture,
        vec![StoreChange::PreviewUpdate {
            item: "skill:archify".to_owned(),
        }],
    );
    let diff = previewed.diffs.last().unwrap();
    assert_eq!(diff.from, Version::Tag("v1.0.0".to_owned()));
    assert_eq!(diff.commit, newer);
    assert_eq!(diff.summary, ["draw it better"]);
    let updated = store(
        &fixture,
        vec![StoreChange::Update {
            item: "skill:archify".to_owned(),
            commit: newer.clone(),
        }],
    );
    assert_eq!(updated.items[0].update, None);
    assert_eq!(cells(&updated, "skill:archify"), all(CellState::On));
    for folder in folders(&fixture) {
        assert_eq!(
            std::fs::read_to_string(folder.join("archify/SKILL.md")).unwrap(),
            skill_md("archify", "Draw it better.")
        );
    }
}

#[test]
fn each_inspect_names_the_preview_of_its_own_source_and_the_same_source_twice_is_read_once() {
    let fixture = every_agent();
    let source = |name: &str| {
        let folder = fixture.root.join("mine").join(name);
        write(&folder.join("SKILL.md"), &skill_md(name, "Write."));
        SourceSpec::Folder {
            path: folder.display().to_string(),
        }
    };
    let (a, b) = (source("writer"), source("reader"));

    let report = store(
        &fixture,
        vec![
            StoreChange::Inspect { source: a.clone() },
            StoreChange::Inspect { source: b.clone() },
            StoreChange::Inspect { source: a.clone() },
        ],
    );

    for (outcome, asked) in report.outcomes.iter().zip([&a, &b, &a]) {
        let made = outcome
            .made
            .as_deref()
            .expect("an inspect names its preview");
        let preview = report
            .previews
            .iter()
            .find(|preview| preview.id == made)
            .unwrap();
        assert_eq!(&preview.source, asked);
    }
    assert_eq!(report.outcomes[0].made, report.outcomes[2].made);
    assert_eq!(
        report
            .previews
            .iter()
            .filter(|preview| preview.source == a)
            .count(),
        1,
        "{:#?}",
        report.previews
    );
}

#[test]
fn a_folder_skill_is_followed_and_a_gone_folder_keeps_the_last_copy() {
    let fixture = every_agent();
    let source = fixture.root.join("mine/writer");
    write(&source.join("SKILL.md"), &skill_md("writer", "Write."));
    let report = install(
        &fixture,
        SourceSpec::Folder {
            path: source.display().to_string(),
        },
        &["writer"],
    );
    assert_eq!(report.items[0].version, Version::Folder);
    assert!(report.follows);
    // The follow runs every few seconds: with the folder as it was, it
    // reads the folder's files' sizes and times and nothing else.
    assert_eq!(crate::follow_store(&fixture.target), None);

    write(&source.join("SKILL.md"), &skill_md("writer", "Write more."));
    let followed = crate::follow_store(&fixture.target).expect("the folder changed");
    assert_eq!(followed.refused, None, "{followed:#?}");
    assert_eq!(cells(&followed, "skill:writer"), all(CellState::On));
    assert_eq!(crate::follow_store(&fixture.target), None);
    for folder in folders(&fixture) {
        assert_eq!(
            std::fs::read_to_string(folder.join("writer/SKILL.md")).unwrap(),
            skill_md("writer", "Write more.")
        );
    }

    std::fs::remove_dir_all(&source).unwrap();
    let gone = crate::follow_store(&fixture.target).expect("the folder went");
    assert!(gone.items[0].folder_missing);
    assert_eq!(crate::follow_store(&fixture.target), None);
    assert_eq!(cells(&gone, "skill:writer"), all(CellState::On));
    assert!(
        fixture
            .home()
            .join(".agents/skills/writer/SKILL.md")
            .is_file()
    );
}

#[test]
fn a_column_switch_takes_out_only_hides_copy_and_the_items_switch_turns_every_column_on() {
    let fixture = every_agent();
    let (_, url) = repository(&fixture);
    install(&fixture, SourceSpec::Git { url }, &["archify"]);
    let shared = fixture.home().join(".agents/skills");
    write(
        &shared.join("theirs/SKILL.md"),
        &skill_md("theirs", "Mine."),
    );

    let off = store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: Some("agents".to_owned()),
            on: false,
        }],
    );
    let mut expected = all(CellState::On);
    expected.insert("agents".to_owned(), CellState::Off);
    assert_eq!(cells(&off, "skill:archify"), expected);
    assert!(!shared.join("archify").exists());
    assert!(
        shared.join("theirs/SKILL.md").is_file(),
        "the operator's skill stays"
    );
    assert!(fixture.home().join(".claude/skills/archify").is_dir());

    // From a partial state the item's switch turns every column on (D-40).
    let on = store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: None,
            on: true,
        }],
    );
    assert_eq!(cells(&on, "skill:archify"), all(CellState::On));
    assert!(shared.join("archify/SKILL.md").is_file());

    let none = store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: None,
            on: false,
        }],
    );
    assert_eq!(cells(&none, "skill:archify"), all(CellState::Off));
    for folder in folders(&fixture) {
        assert!(!folder.join("archify").exists(), "{}", folder.display());
    }
    // The original stays in the store, so turning it on brings it back (B15).
    let back = store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: Some("claude".to_owned()),
            on: true,
        }],
    );
    let mut claude_only = all(CellState::Off);
    claude_only.insert("claude".to_owned(), CellState::On);
    assert_eq!(cells(&back, "skill:archify"), claude_only);
}

#[test]
fn a_deleted_skill_leaves_every_folder_and_the_undo_list_keeps_the_newest_twenty() {
    let fixture = every_agent();
    let source = fixture.root.join("many");
    let names: Vec<String> = (0..=UNDO_LIMIT)
        .map(|index| format!("s{index:02}"))
        .collect();
    for name in &names {
        write(&source.join(name).join("SKILL.md"), &skill_md(name, "x"));
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    install(
        &fixture,
        SourceSpec::Folder {
            path: source.display().to_string(),
        },
        &refs,
    );

    let deleted = store(
        &fixture,
        vec![StoreChange::Delete {
            item: "skill:s00".to_owned(),
        }],
    );
    for folder in folders(&fixture) {
        assert!(!folder.join("s00").exists());
        assert!(folder.join("s01/SKILL.md").is_file());
    }
    let entry = deleted.trash[0].id.clone();
    assert_eq!(deleted.trash[0].cause, TrashCause::Deleted);
    let restored = store(&fixture, vec![StoreChange::Restore { entry }]);
    assert!(restored.trash.is_empty());
    assert_eq!(cells(&restored, "skill:s00"), all(CellState::On));
    assert!(fixture.home().join(".agents/skills/s00/SKILL.md").is_file());

    let every: Vec<StoreChange> = names
        .iter()
        .map(|name| StoreChange::Delete {
            item: format!("skill:{name}"),
        })
        .collect();
    let emptied = store(&fixture, every);
    assert!(emptied.items.is_empty());
    assert_eq!(emptied.trash.len(), UNDO_LIMIT);
    let kept: Vec<&str> = emptied
        .trash
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert!(!kept.contains(&"s00"), "the oldest went for good: {kept:?}");
    let trash = crate::layout::store_dir(fixture.home()).join("trash");
    assert_eq!(std::fs::read_dir(&trash).unwrap().count(), UNDO_LIMIT);
}

#[test]
fn a_skill_of_the_same_name_the_operator_put_there_is_left_alone_until_taken_over() {
    let fixture = every_agent();
    let theirs = fixture.home().join(".agents/skills/archify/SKILL.md");
    write(&theirs, &skill_md("archify", "My own."));
    let (_, url) = repository(&fixture);

    let report = install(&fixture, SourceSpec::Git { url }, &["archify"]);

    let mut expected = all(CellState::On);
    expected.insert("agents".to_owned(), CellState::HandWritten);
    assert_eq!(cells(&report, "skill:archify"), expected);
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        skill_md("archify", "My own.")
    );

    // Turning the column off takes nothing of theirs out.
    store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: Some("agents".to_owned()),
            on: false,
        }],
    );
    assert!(theirs.is_file());
    store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: Some("agents".to_owned()),
            on: true,
        }],
    );

    // [가져오기] on that cell: Hide's copy goes in, theirs to the undo list.
    let taken = store(
        &fixture,
        vec![StoreChange::Adopt {
            confirmed: None,
            kind: ItemKind::Skill,
            name: "archify".to_owned(),
            from: None,
        }],
    );
    assert_eq!(cells(&taken, "skill:archify"), all(CellState::On));
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        skill_md("archify", "Draw it.")
    );
    assert!(matches!(taken.trash[0].cause, TrashCause::Replaced { .. }));
}

#[test]
fn a_skill_one_agent_had_is_adopted_into_every_folder_and_then_managed_there() {
    let fixture = every_agent();
    let own = fixture.home().join(".claude/skills/aside/SKILL.md");
    write(&own, &skill_md("aside", "Browse."));

    let listed = store(&fixture, Vec::new());
    let found = listed
        .unmanaged
        .iter()
        .find(|found| found.name == "aside")
        .unwrap();
    assert!(found.same);
    assert_eq!(found.copies.len(), 1);
    assert!(found.copies[0].agents.contains(&"claude-code".to_owned()));

    let adopted = store(
        &fixture,
        vec![StoreChange::Adopt {
            confirmed: None,
            kind: ItemKind::Skill,
            name: "aside".to_owned(),
            from: None,
        }],
    );
    assert_eq!(cells(&adopted, "skill:aside"), all(CellState::On));
    assert!(adopted.unmanaged.iter().all(|found| found.name != "aside"));
    assert!(matches!(adopted.items[0].source, Source::Copy { .. }));
    for folder in folders(&fixture) {
        assert_eq!(
            std::fs::read_to_string(folder.join("aside/SKILL.md")).unwrap(),
            skill_md("aside", "Browse.")
        );
    }

    // Hide manages Claude's own copy now: deleting takes it out too (D-17).
    store(
        &fixture,
        vec![StoreChange::Delete {
            item: "skill:aside".to_owned(),
        }],
    );
    assert!(!own.exists());
}

#[test]
fn differing_copies_are_adopted_only_from_the_one_picked_and_a_link_is_followed_to_its_folder() {
    let fixture = every_agent();
    let claude = fixture.home().join(".claude/skills/style");
    let shared = fixture.home().join(".agents/skills/style");
    write(&claude.join("SKILL.md"), &skill_md("style", "One."));
    write(&shared.join("SKILL.md"), &skill_md("style", "Two."));

    let refused = crate::apply_store(
        &fixture.target,
        vec![StoreChange::Adopt {
            confirmed: None,
            kind: ItemKind::Skill,
            name: "style".to_owned(),
            from: None,
        }],
    );
    assert_eq!(
        refused.outcomes[0]
            .refused
            .as_ref()
            .map(|refusal| refusal.cause),
        Some(StoreRefusalCause::Differs)
    );
    let found = refused
        .unmanaged
        .iter()
        .find(|found| found.name == "style")
        .unwrap();
    assert!(!found.same);
    let claude_place = found
        .copies
        .iter()
        .find(|copy| copy.agents.contains(&"claude-code".to_owned()))
        .unwrap()
        .place
        .clone();

    let adopted = store(
        &fixture,
        vec![StoreChange::Adopt {
            confirmed: None,
            kind: ItemKind::Skill,
            name: "style".to_owned(),
            from: Some(claude_place),
        }],
    );
    assert_eq!(cells(&adopted, "skill:style"), all(CellState::On));
    assert_eq!(
        std::fs::read_to_string(shared.join("SKILL.md")).unwrap(),
        skill_md("style", "One.")
    );
    assert!(
        adopted.trash.iter().any(
            |entry| entry.name == "style" && matches!(entry.cause, TrashCause::Replaced { .. })
        ),
        "the copy not picked is kept in the undo list"
    );

    let target = fixture.root.join("dotfiles/linked");
    write(&target.join("SKILL.md"), &skill_md("linked", "Linked."));
    let skills = fixture.home().join(".gemini/config/skills");
    std::fs::create_dir_all(&skills).unwrap();
    symlink(&target, &skills.join("linked"));
    let linked = store(
        &fixture,
        vec![StoreChange::Adopt {
            confirmed: None,
            kind: ItemKind::Skill,
            name: "linked".to_owned(),
            from: None,
        }],
    );
    let item = linked
        .items
        .iter()
        .find(|item| item.name == "linked")
        .unwrap();
    let real = hide_platform::fs::identity::canonical(&target).unwrap();
    assert_eq!(
        item.source,
        Source::Folder {
            path: real.display().to_string()
        }
    );
    assert_eq!(cells(&linked, "skill:linked"), all(CellState::On));
    assert!(
        !std::fs::symlink_metadata(skills.join("linked"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "Hide's copy replaces the link"
    );
    assert!(target.join("SKILL.md").is_file(), "the linked folder stays");
}

#[test]
fn an_agent_switched_on_later_and_a_new_profile_get_the_items_with_no_input() {
    let mut fixture = Fixture::new();
    with_git(&fixture);
    let source = fixture.root.join("mine/writer");
    write(&source.join("SKILL.md"), &skill_md("writer", "Write."));
    let report = install(
        &fixture,
        SourceSpec::Folder {
            path: source.display().to_string(),
        },
        &["writer"],
    );
    let cells_now = cells(&report, "skill:writer");
    assert_eq!(cells_now["claude"], CellState::On);
    // Codex and agy are on by default but not found here.
    assert_eq!(cells_now["agents"], CellState::Waiting);
    assert_eq!(cells_now["antigravity"], CellState::Waiting);
    assert!(!fixture.home().join(".agents/skills/writer").exists());

    installed(&fixture.home().join(".local/bin"), "pi");
    let switched = apply(&fixture.target, &Scope::agents(["pi"], []));
    let switched = switched.store.unwrap();
    assert_eq!(cells(&switched, "skill:writer")["agents"], CellState::On);
    assert!(
        fixture
            .home()
            .join(".agents/skills/writer/SKILL.md")
            .is_file()
    );

    let profile_skills = claude_profile(&mut fixture);
    let profiled = store(&fixture, Vec::new());
    assert_eq!(cells(&profiled, "skill:writer")["claude"], CellState::On);
    assert!(profile_skills.join("writer/SKILL.md").is_file());

    // A profile that goes takes Hide's copy with it.
    fixture.target.profiles = Some(ProfileSource::fixed(KitProfiles::default()));
    store(&fixture, Vec::new());
    assert!(!profile_skills.join("writer").exists());

    // Claude switched off in Settings › Agents loses its copy (B25).
    let off = apply(&fixture.target, &Scope::agents([], ["claude-code"]));
    assert_eq!(
        cells(off.store.as_ref().unwrap(), "skill:writer")["claude"],
        CellState::AgentOff
    );
    assert!(!fixture.home().join(".claude/skills/writer").exists());
}

#[test]
fn removing_the_machine_takes_hides_copies_and_the_store_out() {
    let fixture = every_agent();
    let (_, url) = repository(&fixture);
    install(&fixture, SourceSpec::Git { url }, &["archify"]);
    write(
        &fixture.home().join(".agents/skills/theirs/SKILL.md"),
        &skill_md("theirs", "Mine."),
    );

    let removed = remove(&fixture.target);

    assert!(
        removed
            .agents
            .iter()
            .any(|(name, outcome)| name == "store" && *outcome == RemoveOutcome::Removed),
        "{removed:?}"
    );
    for folder in folders(&fixture) {
        assert!(!folder.join("archify").exists(), "{}", folder.display());
    }
    assert!(
        fixture
            .home()
            .join(".agents/skills/theirs/SKILL.md")
            .is_file()
    );
    assert!(!crate::layout::store_dir(fixture.home()).exists());
}

/// The store holds the only original of a skill the operator had and Hide
/// took over, however the switches moved it since (B30).
#[test]
fn removing_the_machine_leaves_the_operators_adopted_skills_where_they_were() {
    let fixture = every_agent();
    let claude = fixture.home().join(".claude/skills");
    // Two plain skills and two the operator cloned, whose `.git` is no part
    // of Hide's copy.
    for name in ["mine", "left_off", "cloned", "forgotten"] {
        write(
            &claude.join(name).join("SKILL.md"),
            &skill_md(name, "Mine."),
        );
    }
    for name in ["cloned", "forgotten"] {
        write(
            &claude.join(name).join(".git/HEAD"),
            "ref: refs/heads/main\n",
        );
    }
    let adopt = |name: &str| StoreChange::Adopt {
        confirmed: None,
        kind: ItemKind::Skill,
        name: name.to_owned(),
        from: None,
    };
    let adopted = store(
        &fixture,
        ["mine", "left_off", "cloned", "forgotten"]
            .into_iter()
            .map(adopt)
            .collect(),
    );
    for name in ["mine", "left_off", "cloned", "forgotten"] {
        assert_eq!(
            cells(&adopted, &format!("skill:{name}")),
            all(CellState::On)
        );
    }
    // A clone went to the undo list whole, and Hide's copy stands there.
    assert!(!claude.join("cloned/.git").exists());
    let set_aside = |name: &str| {
        adopted
            .trash
            .iter()
            .find(|entry| entry.name == name && matches!(entry.cause, TrashCause::Replaced { .. }))
            .unwrap_or_else(|| panic!("{name} was not set aside: {:#?}", adopted.trash))
            .id
            .clone()
    };
    let column = |name: &str, on: bool| StoreChange::Set {
        item: format!("skill:{name}"),
        column: Some("claude".to_owned()),
        on,
    };
    let whole = |name: &str, on: bool| StoreChange::Set {
        item: format!("skill:{name}"),
        column: None,
        on,
    };
    // Claude's column off and on, then the item's own switch off and on:
    // Hide's copy goes and comes back each time.
    for name in ["mine", "cloned"] {
        store(&fixture, vec![column(name, false)]);
        assert!(!claude.join(name).exists());
        store(&fixture, vec![column(name, true), whole(name, false)]);
        assert!(!claude.join(name).exists());
        store(&fixture, vec![whole(name, true)]);
        assert!(claude.join(name).join("SKILL.md").is_file());
    }
    // One is left off, and one's clone is forgotten from the undo list
    // with its column left off: the store alone holds them.
    store(
        &fixture,
        vec![
            whole("left_off", false),
            StoreChange::Forget {
                entry: set_aside("forgotten"),
            },
            column("forgotten", false),
        ],
    );
    assert!(!claude.join("left_off").exists());
    assert!(!claude.join("forgotten").exists());

    let removed = remove(&fixture.target);

    // Each stands where the operator had it: kept, put back with its
    // `.git`, or written back from the store.
    for name in ["mine", "left_off", "cloned", "forgotten"] {
        assert_eq!(
            std::fs::read_to_string(claude.join(name).join("SKILL.md")).unwrap(),
            skill_md(name, "Mine."),
            "{name}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(claude.join("cloned/.git/HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
    for folder in &folders(&fixture)[1..] {
        for name in ["mine", "left_off", "cloned", "forgotten"] {
            assert!(!folder.join(name).exists(), "{}", folder.display());
        }
    }
    assert!(
        removed
            .agents
            .iter()
            .any(|(name, outcome)| name == "store" && *outcome == RemoveOutcome::Removed),
        "{removed:?}"
    );
    assert!(!crate::layout::store_dir(fixture.home()).exists());
}

#[test]
fn a_skill_folder_too_large_to_read_is_listed_with_why_and_the_others_stay_adoptable() {
    let fixture = every_agent();
    let claude = fixture.home().join(".claude/skills");
    write(&claude.join("small/SKILL.md"), &skill_md("small", "Small."));
    write(&claude.join("big/SKILL.md"), &skill_md("big", "Big."));
    for index in 0..crate::store::SKILL_FILE_LIMIT {
        write(&claude.join(format!("big/node_modules/{index}.js")), "x\n");
    }

    let listed = store(&fixture, Vec::new());

    let found = |name: &str| {
        listed
            .unmanaged
            .iter()
            .find(|found| found.name == name)
            .unwrap_or_else(|| panic!("no {name} in {listed:#?}"))
    };
    assert!(found("small").same);
    assert_eq!(found("small").copies[0].refused, None);
    assert!(!found("big").same);
    let refused = found("big").copies[0].refused.as_deref().unwrap();
    assert!(refused.contains("1000 files"), "{refused}");
}

#[test]
fn a_shared_folder_linked_to_claudes_is_one_place_both_columns_switch() {
    let fixture = every_agent();
    let home = fixture.home();
    std::fs::create_dir_all(home.join(".claude/skills")).unwrap();
    std::fs::create_dir_all(home.join(".agents")).unwrap();
    super::symlink(&home.join(".claude/skills"), &home.join(".agents/skills"));
    let (_, url) = repository(&fixture);

    let installed = install(&fixture, SourceSpec::Git { url }, &["archify"]);

    assert_eq!(cells(&installed, "skill:archify"), all(CellState::On));
    let again = store(&fixture, Vec::new());
    assert_eq!(cells(&again, "skill:archify"), all(CellState::On));
    assert!(again.unmanaged.is_empty(), "{:#?}", again.unmanaged);

    // The shared column off: Claude's column still keeps it in the folder.
    let off = store(
        &fixture,
        vec![StoreChange::Set {
            item: "skill:archify".to_owned(),
            column: Some("agents".to_owned()),
            on: false,
        }],
    );
    let mut expected = all(CellState::On);
    expected.insert("agents".to_owned(), CellState::Off);
    assert_eq!(cells(&off, "skill:archify"), expected);
    assert!(home.join(".claude/skills/archify/SKILL.md").is_file());
}

#[test]
fn a_repository_without_a_skill_or_access_says_so_and_asks_nobody() {
    let fixture = every_agent();
    let empty = fixture.root.join("empty");
    write(&empty.join("README.md"), "nothing\n");
    git(&fixture, &fixture.root, &["init", "-q", "empty"]);
    git(&fixture, &empty, &["add", "."]);
    git(&fixture, &empty, &["commit", "-q", "-m", "first"]);
    let url = |path: &Path| {
        let text = path.display().to_string().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        }
    };
    let cause = |source: SourceSpec| {
        crate::apply_store(&fixture.target, vec![StoreChange::Inspect { source }]).outcomes[0]
            .refused
            .as_ref()
            .map(|refusal| refusal.cause)
    };
    assert_eq!(
        cause(SourceSpec::Git { url: url(&empty) }),
        Some(StoreRefusalCause::NoSkills)
    );
    assert_eq!(
        cause(SourceSpec::Git {
            url: url(&fixture.root.join("nowhere"))
        }),
        Some(StoreRefusalCause::NoAccess)
    );
    let staging = crate::layout::store_dir(fixture.home()).join("staging");
    assert_eq!(
        std::fs::read_dir(&staging)
            .map(|entries| entries.count())
            .unwrap_or(0),
        0,
        "a failed read leaves nothing staged"
    );
}

#[test]
fn a_copy_carries_each_skill_with_its_files_to_another_machines_store_once() {
    let here = every_agent();
    let source = here.root.join("mine/writer");
    write(&source.join("SKILL.md"), &skill_md("writer", "Write."));
    let script = source.join("scripts/run.sh");
    write(&script, "#!/bin/sh\necho hi\n");
    hide_platform::fs::permissions::mark_executable(&script).unwrap();
    install(
        &here,
        SourceSpec::Folder {
            path: source.display().to_string(),
        },
        &["writer"],
    );
    store(
        &here,
        vec![StoreChange::Set {
            item: "skill:writer".to_owned(),
            column: Some("antigravity".to_owned()),
            on: false,
        }],
    );

    let export = crate::export_store(&here.target);
    assert_eq!(export.refused, None, "{:?}", export.refused);
    assert_eq!(export.items.len(), 1);

    let there = every_agent();
    let received = StoreChange::Receive {
        item: Box::new(export.items[0].clone()),
    };
    let report = store(&there, vec![received.clone()]);
    let StoreChange::Receive { item } = &report.outcomes[0].change else {
        panic!("{report:#?}");
    };
    assert!(
        matches!(&item.content, Carried::Skill { files }
            if !files.is_empty() && files.iter().all(|file| file.data.is_empty())),
        "an outcome names the files and holds none of their bytes"
    );
    let row = report
        .items
        .iter()
        .find(|item| item.id == "skill:writer")
        .unwrap();
    assert_eq!(
        row.version,
        Version::Copy,
        "a folder this machine cannot follow arrives as a copy"
    );
    assert!(!report.follows);
    let mut expected = all(CellState::On);
    expected.insert("antigravity".to_owned(), CellState::Off);
    assert_eq!(cells(&report, "skill:writer"), expected);
    for folder in &folders(&there)[..2] {
        assert_eq!(
            std::fs::read_to_string(folder.join("writer/SKILL.md")).unwrap(),
            skill_md("writer", "Write.")
        );
        let run = folder.join("writer/scripts/run.sh");
        assert_eq!(std::fs::read(&run).unwrap(), b"#!/bin/sh\necho hi\n");
        // Runnable where the system keeps a bit; Windows reads the extension.
        #[cfg(unix)]
        assert!(hide_platform::fs::permissions::is_executable(&run).unwrap());
    }
    assert!(!there.home().join(".gemini/config/skills/writer").exists());

    // The same copy again changes nothing.
    let skills = there.home().join(".claude/skills");
    let before = tree(&skills);
    let again = store(&there, vec![received]);
    assert!(again.trash.is_empty());
    assert_eq!(tree(&skills), before);

    // A carried path that climbs out of the skill is refused, and nothing is
    // written outside it.
    let mut climbing = export.items[0].clone();
    let Carried::Skill { files } = &mut climbing.content else {
        unreachable!()
    };
    files[0].path = "../escape".to_owned();
    let refused = crate::apply_store(
        &there.target,
        vec![StoreChange::Receive {
            item: Box::new(climbing),
        }],
    );
    assert_eq!(
        refused.outcomes[0]
            .refused
            .as_ref()
            .map(|refusal| refusal.cause),
        Some(StoreRefusalCause::Failed)
    );
    let staging = there.home().join(".hide/store/staging");
    assert!(
        std::fs::read_dir(&staging)
            .map(|entries| entries.count())
            .unwrap_or(0)
            == 0,
        "nothing stays in {}",
        staging.display()
    );
}

#[path = "store_mcp_cases.rs"]
mod mcp;

#[test]
fn a_take_over_refuses_skill_content_or_executable_changes_until_confirmed_again() {
    use crate::store::AdoptConfirmation;
    for permission_only in [false, true] {
        if permission_only && cfg!(windows) {
            continue;
        }
        let fixture = every_agent();
        let (_, url) = repository(&fixture);
        let installed = install(&fixture, SourceSpec::Git { url }, &["archify"]);
        let own = fixture.home().join(".claude/skills/archify");
        write(&own.join("SKILL.md"), &skill_md("archify", "Shown copy."));
        let inspected = store(
            &fixture,
            vec![StoreChange::Inspect {
                source: SourceSpec::Folder {
                    path: own.display().to_string(),
                },
            }],
        );
        let preview = inspected.previews.last().unwrap();
        let digest = preview.skills[0].digest.clone();
        let place = inspected.items[0]
            .cells
            .iter()
            .find(|cell| cell.column == "claude")
            .unwrap()
            .place
            .clone()
            .unwrap();
        if permission_only {
            hide_platform::fs::permissions::mark_executable(&own.join("SKILL.md")).unwrap();
        } else {
            write(&own.join("SKILL.md"), &skill_md("archify", "Unseen copy."));
        }
        let before = folders(&fixture).map(|folder| tree(&folder));
        let root = crate::layout::store_dir(fixture.home());
        let stored = tree(&root.join("items"));
        let staging = tree(&root.join("staging"));
        let refused = crate::apply_store(
            &fixture.target,
            vec![StoreChange::Adopt {
                kind: ItemKind::Skill,
                name: "archify".to_owned(),
                from: Some(place.clone()),
                confirmed: Some(AdoptConfirmation::Skill { digest }),
            }],
        );
        assert_eq!(
            refused.outcomes[0].refused.as_ref().unwrap().cause,
            StoreRefusalCause::Differs
        );
        assert_eq!(refused.items[0].source, installed.items[0].source);
        assert!(refused.trash.is_empty());
        assert_eq!(folders(&fixture).map(|folder| tree(&folder)), before);
        assert_eq!(tree(&root.join("items")), stored);
        // A failed attempt removes only its own staging, not the still-open preview.
        assert_eq!(tree(&root.join("staging")), staging);
        let read = store(
            &fixture,
            vec![StoreChange::Inspect {
                source: SourceSpec::Folder {
                    path: own.display().to_string(),
                },
            }],
        );
        let digest = read.previews.last().unwrap().skills[0].digest.clone();
        let adopted = store(
            &fixture,
            vec![StoreChange::Adopt {
                kind: ItemKind::Skill,
                name: "archify".to_owned(),
                from: Some(place),
                confirmed: Some(AdoptConfirmation::Skill { digest }),
            }],
        );
        assert!(adopted.outcomes[0].refused.is_none(), "{adopted:#?}");
        for folder in folders(&fixture) {
            assert_eq!(
                std::fs::read(folder.join("archify/SKILL.md")).unwrap(),
                std::fs::read(own.join("SKILL.md")).unwrap()
            );
        }
    }
}
