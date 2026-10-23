use super::*;

/// An absolute folder on this system: `/work/<name>`, or `C:\work\<name>`.
fn folder(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"C:\work\{name}"))
    } else {
        PathBuf::from(format!("/work/{name}"))
    }
}

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".gemini/antigravity-cli")).unwrap();
    home
}

fn none() -> Tools<'static> {
    Tools::none()
}

/// A repository's main checkout and one linked worktree of it, in the files
/// Git writes for them.
fn linked_worktree(root: &Path) -> (PathBuf, PathBuf) {
    linked_worktree_named(root, "task")
}

/// [`linked_worktree`] named `repo-<name>`; another name adds another linked
/// worktree to the same repository.
fn linked_worktree_named(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let main = root.join("repo");
    let linked = root.join(format!("repo-{name}"));
    let admin = main.join(".git/worktrees").join(name);
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    std::fs::write(admin.join("HEAD"), format!("ref: refs/heads/{name}\n")).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", linked.join(".git").display()),
    )
    .unwrap();
    (main, linked)
}

/// The folders Hide's record of `agent` holds, in order.
fn recorded(agent: Agent, home: &Path) -> Vec<String> {
    read_record(agent, home)
        .unwrap()
        .iter()
        .map(|recorded| recorded.folder().to_owned())
        .collect()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A folder that exists under `root`, by its resolved path, as Hide records
/// it; a recorded folder that is gone leaves with the next trust.
fn real(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).unwrap();
    hide_platform::fs::identity::canonical(&path).unwrap()
}

#[test]
fn hide_takes_out_only_the_agy_trust_it_added_and_keeps_the_rest_byte_for_byte() {
    let home = home();
    let settings = Agent::Antigravity.settings_path(home.path());
    let (mine, tree) = (folder("me"), folder("tree-1"));
    let original = format!(
        "{{\n  \"theme\": \"dark\",\n  \"trustedWorkspaces\": [{}]\n}}\n",
        serde_json::to_string(mine.to_str().unwrap()).unwrap()
    );
    std::fs::write(&settings, &original).unwrap();
    trust(Agent::Antigravity, home.path(), &tree, none()).unwrap();
    trust(Agent::Antigravity, home.path(), &tree, none()).unwrap();
    trust(Agent::Antigravity, home.path(), &mine, none()).unwrap();
    assert_eq!(
        read_json(&settings)["trustedWorkspaces"],
        serde_json::json!([mine.to_str().unwrap(), tree.to_str().unwrap()])
    );
    // The operator's own folder is never Hide's to take out.
    forget(Agent::Antigravity, home.path(), &mine, none()).unwrap();
    forget(Agent::Antigravity, home.path(), &tree, none()).unwrap();
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
}

#[test]
fn switching_an_agent_off_takes_out_every_folder_hide_added_for_it_alone() {
    let home = home();
    let settings = Agent::Antigravity.settings_path(home.path());
    let (a, b) = (real(home.path(), "a"), real(home.path(), "b"));
    trust(Agent::Antigravity, home.path(), &a, none()).unwrap();
    trust(Agent::Antigravity, home.path(), &b, none()).unwrap();
    trust(Agent::ClaudeCode, home.path(), &a, none()).unwrap();
    assert_eq!(
        forget_all(Agent::Antigravity, home.path(), none()).unwrap(),
        2
    );
    assert_eq!(
        read_json(&settings)["trustedWorkspaces"],
        serde_json::json!([])
    );
    assert!(
        read_record(Agent::Antigravity, home.path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        read_record(Agent::ClaudeCode, home.path()).unwrap().len(),
        1
    );
}

#[test]
fn removing_where_hide_never_trusted_a_folder_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    for agent in Agent::ALL {
        assert_eq!(forget(agent, home.path(), &folder("a"), none()).unwrap(), 0);
        assert_eq!(forget_all(agent, home.path(), none()).unwrap(), 0);
        assert!(!agent.settings_path(home.path()).exists());
    }
    assert!(!home.path().join(".hide").exists());
}

#[test]
fn settings_hide_cannot_read_stay_untouched_and_the_trust_fails() {
    let home = home();
    for agent in [Agent::Antigravity, Agent::ClaudeCode] {
        let settings = agent.settings_path(home.path());
        std::fs::write(&settings, "{ not json").unwrap();
        assert!(matches!(
            trust(agent, home.path(), &folder("a"), none()),
            Err(InstallFailure::Unparsable { .. })
        ));
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
        assert!(read_record(agent, home.path()).unwrap().is_empty());
        assert!(trust(agent, home.path(), Path::new("relative"), none()).is_err());
    }
}

#[test]
fn only_a_git_worktree_below_home_is_trusted_as_a_worktree() {
    let home = home();
    let tree = home.path().join("repo-tree");
    std::fs::create_dir_all(tree.join(".git")).unwrap();
    let plain = home.path().join("downloads");
    std::fs::create_dir_all(&plain).unwrap();

    for agent in [Agent::Antigravity, Agent::ClaudeCode] {
        for refused in [
            plain.clone(),
            home.path().to_path_buf(),
            home.path().parent().unwrap().to_path_buf(),
            tree.join("..").join("repo-tree"),
            home.path().join(".").join("repo-tree"),
            tree.join("."),
            home.path().join("missing"),
        ] {
            assert!(
                trust_worktree(agent, home.path(), &refused, none()).is_err(),
                "{} {}",
                agent.label(),
                refused.display()
            );
        }
        assert!(read_record(agent, home.path()).unwrap().is_empty());
    }
    assert!(!Agent::ClaudeCode.settings_path(home.path()).exists());

    trust_worktree(Agent::Antigravity, home.path(), &tree, none()).unwrap();
    let resolved = hide_platform::fs::identity::canonical(&tree).unwrap();
    assert_eq!(
        recorded(Agent::Antigravity, home.path()),
        [resolved.to_str().unwrap()],
        "the entry is the folder's resolved path"
    );
    assert_eq!(
        forget(Agent::Antigravity, home.path(), &tree, none()).unwrap(),
        1,
        "undone by its own spelling"
    );

    // Named through a link, the entry is still the worktree itself, so
    // pointing the link elsewhere afterwards trusts nothing new.
    #[cfg(unix)]
    {
        let link = home.path().join("tree-link");
        std::os::unix::fs::symlink(&tree, &link).unwrap();
        trust_worktree(Agent::Antigravity, home.path(), &link, none()).unwrap();
        assert_eq!(
            recorded(Agent::Antigravity, home.path()),
            [resolved.to_str().unwrap()]
        );
        assert_eq!(
            forget(Agent::Antigravity, home.path(), &link, none()).unwrap(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn a_trust_whose_settings_write_fails_leaves_no_record_of_it() {
    use std::os::unix::fs::PermissionsExt;
    let home = home();
    let settings = Agent::Antigravity.settings_path(home.path());
    std::fs::write(&settings, "{}").unwrap();
    let folder_of_settings = settings.parent().unwrap();
    std::fs::set_permissions(folder_of_settings, std::fs::Permissions::from_mode(0o555)).unwrap();

    let trusted = trust(Agent::Antigravity, home.path(), &folder("tree-1"), none());

    std::fs::set_permissions(folder_of_settings, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(trusted.is_err());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
    assert!(
        read_record(Agent::Antigravity, home.path())
            .unwrap()
            .is_empty()
    );
    // So a trust the operator gives agy later stays theirs.
    assert_eq!(
        forget(Agent::Antigravity, home.path(), &folder("tree-1"), none()).unwrap(),
        0
    );
}

#[test]
fn claude_trusts_the_worktree_itself_and_hide_takes_out_only_its_own_flag() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude.json");
    let main = folder("repo");
    let kept = real(home.path(), "kept");
    let tree = real(home.path(), "repo-task");
    let key = |path: &Path| Agent::ClaudeCode.spelling(path).unwrap();
    // Claude's own file: other projects, an entry for the worktree Claude
    // filled before (trust off, other fields of its own), unknown keys.
    let original = format!(
        "{{\n  \"numStartups\": 3,\n  \"projects\": {{\n    {}: {{\n      \"hasTrustDialogAccepted\": true\n    }},\n    {}: {{\n      \"allowedTools\": [],\n      \"hasTrustDialogAccepted\": false\n    }}\n  }},\n  \"userID\": \"x\"\n}}\n",
        serde_json::to_string(&key(&main)).unwrap(),
        serde_json::to_string(&key(&kept)).unwrap(),
    );
    std::fs::write(&config, &original).unwrap();

    trust(Agent::ClaudeCode, home.path(), &tree, none()).unwrap();
    trust(Agent::ClaudeCode, home.path(), &kept, none()).unwrap();
    let written = read_json(&config);
    assert_eq!(
        written["projects"][key(&tree)],
        serde_json::json!({"hasTrustDialogAccepted": true})
    );
    assert_eq!(
        written["projects"][key(&kept)],
        serde_json::json!({"allowedTools": [], "hasTrustDialogAccepted": true})
    );
    // The repository's main checkout is the operator's and stays as it was.
    assert_eq!(
        written["projects"][key(&main)],
        serde_json::json!({"hasTrustDialogAccepted": true})
    );
    // A folder Claude already trusts stays the operator's.
    trust(Agent::ClaudeCode, home.path(), &main, none()).unwrap();
    assert_eq!(
        recorded(Agent::ClaudeCode, home.path()),
        [key(&tree), key(&kept)]
    );

    assert_eq!(
        forget(Agent::ClaudeCode, home.path(), &main, none()).unwrap(),
        0
    );
    assert_eq!(
        forget_all(Agent::ClaudeCode, home.path(), none()).unwrap(),
        2
    );
    let restored = read_json(&config);
    assert!(restored["projects"].get(key(&tree)).is_none());
    assert_eq!(
        restored["projects"][key(&kept)],
        serde_json::json!({"allowedTools": []}),
        "only the flag Hide set goes; the fields Claude wrote stay"
    );
    assert_eq!(
        restored["projects"][key(&main)],
        serde_json::json!({"hasTrustDialogAccepted": true})
    );
    assert_eq!(restored["userID"], "x");
}

#[test]
fn claude_s_file_is_made_when_claude_has_none_and_its_older_file_is_followed() {
    let home = tempfile::tempdir().unwrap();
    trust(Agent::ClaudeCode, home.path(), &folder("tree"), none()).unwrap();
    assert_eq!(
        read_json(&home.path().join(".claude.json"))["projects"]
            [Agent::ClaudeCode.spelling(&folder("tree")).unwrap()]["hasTrustDialogAccepted"],
        true
    );

    // Claude Code reads `~/.claude/.config.json` instead while it has content.
    let older = tempfile::tempdir().unwrap();
    let legacy = older.path().join(".claude/.config.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    assert_eq!(Agent::ClaudeCode.settings_path(older.path()), legacy);
    trust(Agent::ClaudeCode, older.path(), &folder("tree"), none()).unwrap();
    assert!(read_json(&legacy)["projects"].is_object());
    assert!(!older.path().join(".claude.json").exists());
}

#[test]
fn claude_s_own_lock_is_waited_for_and_released() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude.json");
    std::fs::write(&config, "{}").unwrap();
    let lock = home.path().join(".claude.json.lock");
    // A Claude Code saving its config holds the lock folder.
    std::fs::create_dir(&lock).unwrap();
    let refused = trust(Agent::ClaudeCode, home.path(), &folder("tree"), none()).unwrap_err();
    assert!(
        refused.message().contains("held its config lock"),
        "{}",
        refused.message()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{}");
    assert!(
        read_record(Agent::ClaudeCode, home.path())
            .unwrap()
            .is_empty()
    );
    assert!(
        lock.is_dir(),
        "a lock Hide did not take is not Hide's to remove"
    );

    std::fs::remove_dir(&lock).unwrap();
    trust(Agent::ClaudeCode, home.path(), &folder("tree"), none()).unwrap();
    assert!(!lock.exists(), "Hide releases the lock it took");
}

#[cfg(unix)]
#[test]
fn a_claude_lock_left_by_a_process_that_ended_is_taken_over() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
    let lock = home.path().join(".claude.json.lock");
    std::fs::create_dir(&lock).unwrap();
    let long_ago = std::time::SystemTime::now() - Duration::from_secs(60);
    std::fs::File::open(&lock)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(long_ago))
        .unwrap();
    trust(Agent::ClaudeCode, home.path(), &folder("tree"), none()).unwrap();
    assert!(!lock.exists());
}

#[test]
fn a_folder_that_is_gone_leaves_with_the_next_trust_hide_records() {
    let home = home();
    for agent in [Agent::Antigravity, Agent::ClaudeCode] {
        let (gone, live, next) = (
            real(home.path(), "gone"),
            real(home.path(), "live"),
            real(home.path(), "next"),
        );
        let key = |path: &Path| agent.spelling(path).unwrap();
        trust(agent, home.path(), &gone, none()).unwrap();
        trust(agent, home.path(), &live, none()).unwrap();
        // Removed outside Hide, so no removal of Hide's took it out.
        std::fs::remove_dir(&gone).unwrap();

        trust(agent, home.path(), &next, none()).unwrap();
        assert_eq!(
            recorded(agent, home.path()),
            [key(&live), key(&next)],
            "{}",
            agent.label()
        );
        let settings = read_json(&agent.settings_path(home.path()));
        let trusted = match agent {
            Agent::Antigravity => settings["trustedWorkspaces"].clone(),
            _ => serde_json::json!(
                settings["projects"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .collect::<Vec<_>>()
            ),
        };
        assert_eq!(
            trusted,
            serde_json::json!([key(&live), key(&next)]),
            "{}",
            agent.label()
        );
        forget_all(agent, home.path(), none()).unwrap();
        std::fs::remove_dir(&live).unwrap();
        std::fs::remove_dir(&next).unwrap();
    }
}

#[test]
fn a_trust_past_the_record_s_cap_is_refused_and_folders_that_are_gone_do_not_count() {
    let home = home();
    let gone = real(home.path(), "gone");
    let mut record: Vec<Recorded> = (0..RECORD_LIMIT - 1)
        .map(|n| {
            Recorded::Folder(
                real(home.path(), &format!("live-{n}"))
                    .to_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    record.push(Recorded::Folder(gone.to_str().unwrap().to_owned()));
    std::fs::create_dir_all(agent_hooks_dir(home.path())).unwrap();
    write_record(Agent::Antigravity, home.path(), &record).unwrap();
    std::fs::remove_dir(&gone).unwrap();
    // The folder that is gone makes room for one more.
    trust(
        Agent::Antigravity,
        home.path(),
        &real(home.path(), "one-more"),
        none(),
    )
    .unwrap();
    let refused = trust(
        Agent::Antigravity,
        home.path(),
        &real(home.path(), "past"),
        none(),
    )
    .unwrap_err()
    .message();
    assert!(refused.contains("4096 folders that exist"), "{refused}");
}

#[test]
fn a_removal_after_the_folder_is_gone_finds_it_by_the_key_hide_wrote() {
    let home = home();
    for agent in [Agent::Antigravity, Agent::ClaudeCode] {
        // Named as the caller does, not resolved: on macOS the temporary
        // folder is reached through `/var`, which resolves to `/private/var`.
        let tree = home.path().join("repo-task");
        std::fs::create_dir_all(tree.join(".git")).unwrap();
        trust_worktree(agent, home.path(), &tree, none()).unwrap();
        std::fs::remove_dir_all(&tree).unwrap();
        // As Git lists it, with `/` between names on every system.
        let listed = PathBuf::from(tree.to_str().unwrap().replace('\\', "/"));
        assert_eq!(
            forget(agent, home.path(), &listed, none()).unwrap(),
            1,
            "{}",
            agent.label()
        );
        assert!(read_record(agent, home.path()).unwrap().is_empty());
    }

    // Through a link in a folder above it, resolved when it was trusted.
    #[cfg(unix)]
    {
        let real_parent = home.path().join("disk");
        std::fs::create_dir_all(real_parent.join("repo-task/.git")).unwrap();
        let link = home.path().join("projects");
        std::os::unix::fs::symlink(&real_parent, &link).unwrap();
        trust_worktree(
            Agent::Antigravity,
            home.path(),
            &link.join("repo-task"),
            none(),
        )
        .unwrap();
        std::fs::remove_dir_all(real_parent.join("repo-task")).unwrap();
        assert_eq!(
            forget(
                Agent::Antigravity,
                home.path(),
                &link.join("repo-task"),
                none()
            )
            .unwrap(),
            1
        );
    }
}

#[test]
fn a_windows_folder_is_compared_without_case_and_with_either_separator() {
    let folder = Path::new(r"C:\Work\Repo-Task");
    if cfg!(windows) {
        for entry in [
            r"c:\work\repo-task",
            "C:/Work/Repo-Task",
            r"C:\Work\Repo-Task\",
        ] {
            assert!(same_folder(entry, folder), "{entry}");
        }
        assert!(same_folder(
            r"C:\Work\Repo-Task",
            Path::new("C:/work/repo-task/")
        ));
        assert!(!same_folder(r"C:\Work\Repo-Task-2", folder));
    } else {
        let folder = Path::new("/work/Repo-Task");
        assert!(same_folder("/work/Repo-Task", folder));
        assert!(same_folder(
            "/work/Repo-Task",
            Path::new("/work/Repo-Task/")
        ));
        assert!(!same_folder("/work/repo-task", folder));
    }
}

#[cfg(unix)]
#[test]
fn a_linked_claude_config_is_locked_where_claude_code_locks_it() {
    let home = tempfile::tempdir().unwrap();
    let dotfiles = home.path().join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let target = dotfiles.join("claude.json");
    std::fs::write(&target, "{}").unwrap();
    let config = home.path().join(".claude.json");
    std::os::unix::fs::symlink(&target, &config).unwrap();
    // A Claude Code saving its config holds the lock beside the real file.
    let lock = dotfiles.join("claude.json.lock");
    std::fs::create_dir(&lock).unwrap();
    let refused = trust(Agent::ClaudeCode, home.path(), &folder("tree"), none())
        .unwrap_err()
        .message();
    assert!(refused.contains("claude.json.lock"), "{refused}");
    assert!(
        refused.contains(dotfiles.file_name().unwrap().to_str().unwrap()),
        "{refused}"
    );
    assert!(!home.path().join(".claude.json.lock").exists());

    std::fs::remove_dir(&lock).unwrap();
    trust(Agent::ClaudeCode, home.path(), &folder("tree"), none()).unwrap();
    assert!(config.is_symlink(), "the link stays the operator's");
    assert!(read_json(&target)["projects"].is_object());
    assert!(!lock.exists());
}

#[test]
fn hide_leaves_a_claude_lock_it_no_longer_holds() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude.json");
    std::fs::write(&config, "{}").unwrap();
    let held = claude::ConfigLock::take(&config).unwrap();
    let lock = claude::ConfigLock::path(&config);
    assert!(held.held());
    // Claude Code took it over after Hide held it past the stale time.
    std::fs::remove_dir(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    assert!(!held.held());
    drop(held);
    assert!(lock.is_dir(), "the lock Claude Code holds stays");
}

#[test]
fn each_agent_spells_a_folder_as_it_compares_one() {
    let path = folder("Repo-Task");
    let native = path.to_str().unwrap().to_owned();
    assert_eq!(Agent::Antigravity.spelling(&path).unwrap(), native);
    if cfg!(windows) {
        assert_eq!(
            Agent::ClaudeCode.spelling(&path).unwrap(),
            "C:/work/Repo-Task"
        );
        assert_eq!(Agent::Codex.spelling(&path).unwrap(), r"c:\work\repo-task");
    } else {
        assert_eq!(Agent::ClaudeCode.spelling(&path).unwrap(), native);
        assert_eq!(Agent::Codex.spelling(&path).unwrap(), native);
    }
}

#[test]
fn a_linked_worktree_names_its_repository_s_main_checkout() {
    let root = tempfile::tempdir().unwrap();
    let (main, linked) = linked_worktree(root.path());
    assert_eq!(
        main_checkout(&linked),
        Some(hide_platform::fs::identity::canonical(&main).unwrap())
    );
    assert_eq!(
        main_checkout(&main),
        Some(hide_platform::fs::identity::canonical(&main).unwrap())
    );
}

#[test]
fn codex_trust_needs_a_codex_to_record_it_through() {
    let root = tempfile::tempdir().unwrap();
    let (_, linked) = linked_worktree(root.path());
    let refused = trust_worktree(Agent::Codex, root.path(), &linked, none()).unwrap_err();
    assert!(
        refused.message().contains("no codex program"),
        "{}",
        refused.message()
    );
    assert!(read_record(Agent::Codex, root.path()).unwrap().is_empty());
}

#[test]
fn only_claude_codex_agy_and_grok_starts_record_a_trust() {
    use hide_agent_adapter::LaunchDialect;
    for dialect in [
        LaunchDialect::Claude,
        LaunchDialect::Codex,
        LaunchDialect::Antigravity,
        LaunchDialect::Grok,
    ] {
        assert!(Agent::of(dialect).is_some(), "{dialect:?}");
    }
    for dialect in [
        LaunchDialect::OpenCode,
        LaunchDialect::Pi,
        LaunchDialect::Omp,
        LaunchDialect::Cursor,
    ] {
        assert!(Agent::of(dialect).is_none(), "{dialect:?}");
    }
}

/// Grok's trust file as Grok reads it: each folder it decided, and whether
/// it trusts it.
fn grok_decisions(home: &Path) -> Vec<(String, bool)> {
    let text = std::fs::read_to_string(Agent::Grok.settings_path(home)).unwrap();
    let document = text.parse::<toml_edit::DocumentMut>().unwrap();
    let Some(folders) = document
        .get("folders")
        .and_then(|item| item.as_table_like())
    else {
        return Vec::new();
    };
    folders
        .iter()
        .map(|(key, entry)| {
            let trusted = entry.get("trusted").and_then(|item| item.as_bool());
            (key.to_owned(), trusted == Some(true))
        })
        .collect()
}

#[test]
fn grok_trusts_a_worktree_through_its_main_checkout_while_any_worktree_of_hide_s_needs_it() {
    let home = tempfile::tempdir().unwrap();
    let settings = Agent::Grok.settings_path(home.path());
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let mine = folder("mine");
    let original = format!(
        "[folders.{}]\ntrusted = true\ndecided_at = 1789968913\n",
        serde_json::to_string(mine.to_str().unwrap()).unwrap()
    );
    std::fs::write(&settings, &original).unwrap();
    let (main, first) = linked_worktree_named(home.path(), "one");
    let (_, second) = linked_worktree_named(home.path(), "two");
    let main = hide_platform::fs::identity::canonical(&main)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

    trust_worktree(Agent::Grok, home.path(), &first, none()).unwrap();
    trust_worktree(Agent::Grok, home.path(), &second, none()).unwrap();
    let text = std::fs::read_to_string(&settings).unwrap();
    assert!(
        text.starts_with(&original),
        "the operator's entry keeps its bytes: {text}"
    );
    assert!(text.contains("decided_at = "), "{text}");
    assert_eq!(
        grok_decisions(home.path()),
        [
            (mine.to_str().unwrap().to_owned(), true),
            (main.clone(), true)
        ],
        "one entry, the main checkout's, for both worktrees"
    );
    assert_eq!(recorded(Agent::Grok, home.path()).len(), 2);

    // The entry stays while the other worktree still needs it, and leaves
    // with the last one, once the folder is gone, as Hide removes it.
    std::fs::remove_dir_all(&first).unwrap();
    assert_eq!(forget(Agent::Grok, home.path(), &first, none()).unwrap(), 0);
    assert!(grok_decisions(home.path()).contains(&(main.clone(), true)));
    std::fs::remove_dir_all(&second).unwrap();
    assert_eq!(
        forget(Agent::Grok, home.path(), &second, none()).unwrap(),
        1
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
    assert!(recorded(Agent::Grok, home.path()).is_empty());

    // Switching Grok off takes out every entry Hide added, and only those.
    let (_, third) = linked_worktree_named(home.path(), "three");
    trust_worktree(Agent::Grok, home.path(), &third, none()).unwrap();
    assert_eq!(forget_all(Agent::Grok, home.path(), none()).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
}

#[test]
fn grok_s_own_answer_for_the_repository_stays_the_operator_s() {
    let home = tempfile::tempdir().unwrap();
    let settings = Agent::Grok.settings_path(home.path());
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let (main, linked) = linked_worktree(home.path());
    let key = serde_json::to_string(
        hide_platform::fs::identity::canonical(&main)
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();

    // Trusted by the operator: nothing is written or recorded, so removing
    // the worktree leaves it.
    let trusted = format!("[folders.{key}]\ntrusted = true\n");
    std::fs::write(&settings, &trusted).unwrap();
    trust_worktree(Agent::Grok, home.path(), &linked, none()).unwrap();
    assert!(recorded(Agent::Grok, home.path()).is_empty());
    assert_eq!(
        forget(Agent::Grok, home.path(), &linked, none()).unwrap(),
        0
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), trusted);

    // Answered "No": the start is refused with what to do, never overridden.
    let untrusted = format!("[folders.{key}]\ntrusted = false\n");
    std::fs::write(&settings, &untrusted).unwrap();
    let refused = trust_worktree(Agent::Grok, home.path(), &linked, none())
        .unwrap_err()
        .message();
    assert!(refused.contains("Grok marks"), "{refused}");
    assert!(refused.contains("trust it in Grok"), "{refused}");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), untrusted);
    assert!(recorded(Agent::Grok, home.path()).is_empty());

    // A file Grok could not read either is left as it is.
    std::fs::write(&settings, "[folders\n").unwrap();
    assert!(trust_worktree(Agent::Grok, home.path(), &linked, none()).is_err());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "[folders\n");
    assert!(recorded(Agent::Grok, home.path()).is_empty());
}

#[test]
fn grok_s_own_lock_is_waited_for() {
    let home = tempfile::tempdir().unwrap();
    let settings = Agent::Grok.settings_path(home.path());
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let (_, linked) = linked_worktree(home.path());
    // A Grok saving its trust holds the lock file beside it.
    let held = std::fs::File::create(settings.with_extension("toml.lock")).unwrap();
    held.lock().unwrap();
    let refused = trust_worktree(Agent::Grok, home.path(), &linked, none())
        .unwrap_err()
        .message();
    assert!(refused.contains("held its trust lock"), "{refused}");
    assert!(!settings.exists());
    assert!(recorded(Agent::Grok, home.path()).is_empty());

    held.unlock().unwrap();
    trust_worktree(Agent::Grok, home.path(), &linked, none()).unwrap();
    assert_eq!(grok_decisions(home.path()).len(), 1);
}

#[test]
fn a_grok_worktree_of_a_repository_without_a_main_checkout_is_refused() {
    let home = tempfile::tempdir().unwrap();
    // A folder with a `.git` that names no repository Git can open.
    let tree = home.path().join("tree");
    std::fs::create_dir_all(tree.join(".git")).unwrap();
    let refused = trust_worktree(Agent::Grok, home.path(), &tree, none())
        .unwrap_err()
        .message();
    assert!(refused.contains("main checkout"), "{refused}");
    assert!(!Agent::Grok.settings_path(home.path()).exists());
}
