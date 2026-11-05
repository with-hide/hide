//! The integration run by the shells themselves: the user's own files are
//! read in the shell's order, zsh's see their own `ZDOTDIR` and the shell
//! starts with it as the user had it, unset included, and the mark comes
//! after every one of them. They need the shells, named in
//! `HIDE_TEST_ZSH_BIN`, `HIDE_TEST_BASH_BIN` and `HIDE_TEST_FISH_BIN`, so they
//! are ignored by default; the rust lane's shell step runs them with
//! `--ignored`.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use super::*;

fn shell(variable: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} names the shell")),
    )
}

/// A shell started with `args` and `env` from the integration, reading
/// `commands` on its input. Answers what the user's files and the commands
/// wrote to the log, in order, and what the shell wrote to its screen.
#[allow(clippy::disallowed_methods)] // env_clear: the shell starts with only what the integration gives it (Unix only)
fn run(
    bin: &Path,
    args: &[&str],
    home: &Path,
    env: &BTreeMap<String, String>,
    commands: &str,
) -> (String, String) {
    let mut child = Command::new(bin)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb")
        .envs(env)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(commands.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let log = std::fs::read_to_string(home.join("log")).unwrap_or_default();
    let _ = std::fs::remove_file(home.join("log"));
    (
        log.replace(&home.display().to_string(), "~"),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// The prompt mark a shell writes on its screen.
const SCREEN_MARK: &str = "\u{1b}]133;A\u{7}";

/// What each of the user's files writes on the screen as it runs.
const FILE_RAN: &str = "user-file-ran";

/// Asserts the shell marked its first prompt on its screen exactly once,
/// after every one of the user's files ran.
fn assert_marked_after_files(screen: &str) {
    assert_eq!(screen.matches(SCREEN_MARK).count(), 1, "{screen:?}");
    let mark = screen.find(SCREEN_MARK).unwrap();
    assert!(
        screen.rfind(FILE_RAN).is_none_or(|ran| ran < mark),
        "the mark came before a file of the user's: {screen:?}"
    );
}

fn user(shell: &Path, zdotdir: Option<&Path>) -> UserShell {
    UserShell {
        shell: Some(shell.display().to_string()),
        zdotdir: zdotdir.map(|path| path.display().to_string()),
    }
}

/// Each of the user's files says it ran, with what `seen` expands to there,
/// and declares something that must outlive it.
fn user_files(folder: &Path, names: &[&str], seen: &str, declare: &str) {
    std::fs::create_dir_all(folder).unwrap();
    for name in names {
        let variable = name.trim_start_matches('.').replace('.', "_");
        std::fs::write(
            folder.join(name),
            format!("echo \"{name} {seen}\" >> \"$HOME/log\"\necho {FILE_RAN}\n{declare} from_{variable}\nfrom_{variable}[a]=1\n"),
        )
        .unwrap();
    }
}

const ZSH_AFTER: &str = "print -r -- \"after ${ZDOTDIR-unset} ${(t)ZDOTDIR} ${+HIDE_SHELL_ZDOTDIR} ${(t)from_zshenv} ${(t)from_zshrc}\" >> $HOME/log\nexit\n";

#[test]
#[ignore = "needs zsh: set HIDE_TEST_ZSH_BIN and run with --ignored"]
fn zsh_reads_the_users_files_in_its_own_order_and_gives_their_environment_back() {
    let zsh = shell("HIDE_TEST_ZSH_BIN");
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let names = [".zshenv", ".zprofile", ".zshrc", ".zlogin"];
    user_files(&home, &names, "${ZDOTDIR-unset}", "typeset -A");
    let env = install(&home.join("hide"), &user(&zsh, None))
        .unwrap()
        .unwrap()
        .env;

    // A login shell, the pane's on macOS: all four, in zsh's order, then the
    // mark.
    let (log, screen) = run(&zsh, &["-i", "-l"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv unset\n.zprofile unset\n.zshrc unset\n.zlogin unset\nafter unset  0 association association\n"
    );
    assert_marked_after_files(&screen);

    // An interactive shell that is not a login shell, the pane's on Linux,
    // reads two.
    let (log, screen) = run(&zsh, &["-i"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv unset\n.zshrc unset\nafter unset  0 association association\n"
    );
    assert_marked_after_files(&screen);

    // A command reads only .zshenv, gets everything back, and marks nothing.
    let (log, screen) = run(
        &zsh,
        &["-c", ZSH_AFTER.trim_end_matches("exit\n")],
        &home,
        &env,
        "",
    );
    assert_eq!(log, ".zshenv unset\nafter unset  0 association \n");
    assert!(!screen.contains(SCREEN_MARK), "{screen:?}");

    // A user whose own ZDOTDIR is set gets it back, exported, and their
    // files are read from it.
    let zdotdir = home.join("z");
    user_files(&zdotdir, &names, "${ZDOTDIR-unset}", "typeset -A");
    let env = install(&home.join("hide"), &user(&zsh, Some(&zdotdir)))
        .unwrap()
        .unwrap()
        .env;
    let (log, screen) = run(&zsh, &["-i", "-l"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv ~/z\n.zprofile ~/z\n.zshrc ~/z\n.zlogin ~/z\nafter ~/z scalar-export 0 association association\n"
    );
    assert_marked_after_files(&screen);
}

const BASH_AFTER: &str = "echo \"after ${PROMPT_COMMAND-unset} ${from_bashrc[a]-} ${from_bash_profile[a]-} ${from_profile[a]-}\" >> \"$HOME/log\"\nexit\n";

#[test]
#[ignore = "needs bash: set HIDE_TEST_BASH_BIN and run with --ignored"]
fn bash_reads_the_users_files_in_its_own_order_then_marks() {
    let bash = shell("HIDE_TEST_BASH_BIN");
    let folder = |file: &str| {
        let home = tempfile::tempdir().unwrap();
        let path = install(home.path(), &user(&bash, None))
            .unwrap()
            .unwrap()
            .command;
        let init = Path::new(&path[2]);
        let init = init.parent().unwrap().join(file);
        (home, init)
    };
    // Interactive, with the init file the launch names.
    let run_with = |home: &Path, init: &Path| {
        run(
            &bash,
            &["--init-file", &init.display().to_string(), "-i"],
            home,
            &BTreeMap::new(),
            BASH_AFTER,
        )
    };

    // Standing for a login shell (macOS): the first of the user's login
    // files, after /etc/profile, then the mark; ~/.bashrc only through it.
    // Hide's folder report is the one PROMPT_COMMAND.
    let (home, init) = folder("login.bash");
    user_files(
        home.path(),
        &[".bash_profile", ".bash_login", ".profile"],
        "login",
        "declare -a",
    );
    let (log, screen) = run_with(home.path(), &init);
    assert_eq!(log, ".bash_profile login\nafter __hide_cwd  1 \n");
    assert_marked_after_files(&screen);
    std::fs::remove_file(home.path().join(".bash_profile")).unwrap();
    std::fs::remove_file(home.path().join(".bash_login")).unwrap();
    let (log, screen) = run_with(home.path(), &init);
    assert_eq!(log, ".profile login\nafter __hide_cwd   1\n");
    assert_marked_after_files(&screen);

    // Not a login shell (Linux): ~/.bashrc, whose PROMPT_COMMAND, set or
    // appended to, stays the user's first.
    let (home, init) = folder("interactive.bash");
    user_files(home.path(), &[".bashrc"], "rc", "declare -a");
    std::fs::write(
        home.path().join(".bashrc"),
        std::fs::read_to_string(home.path().join(".bashrc")).unwrap()
            + "PROMPT_COMMAND=\"${PROMPT_COMMAND:+$PROMPT_COMMAND;}true\"\n",
    )
    .unwrap();
    let (log, screen) = run_with(home.path(), &init);
    assert_eq!(log, ".bashrc rc\nafter true 1  \n");
    assert_marked_after_files(&screen);
}

#[test]
#[ignore = "needs fish: set HIDE_TEST_FISH_BIN and run with --ignored"]
fn fish_marks_after_its_configuration() {
    let fish = shell("HIDE_TEST_FISH_BIN");
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let config = home.join(".config/fish");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.fish"),
        format!("echo config >> $HOME/log\necho {FILE_RAN}\n"),
    )
    .unwrap();
    let launch = install(&home.join("hide"), &user(&fish, None))
        .unwrap()
        .unwrap();
    let mut args: Vec<&str> = launch.command[1..].iter().map(String::as_str).collect();
    args.push("-i");
    let (log, screen) = run(
        &fish,
        &args,
        &home,
        &launch.env,
        "echo after >> $HOME/log\nexit\n",
    );
    assert_eq!(log, "config\nafter\n");
    assert_marked_after_files(&screen);
}

/// Each shell marks its first prompt on its screen once, after the user's
/// files, and not again at later prompts.
#[test]
#[ignore = "needs zsh, bash and fish: set HIDE_TEST_ZSH_BIN, HIDE_TEST_BASH_BIN and HIDE_TEST_FISH_BIN and run with --ignored"]
fn each_shell_marks_its_first_prompt_on_its_screen_once() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let launches = [
        "HIDE_TEST_ZSH_BIN",
        "HIDE_TEST_BASH_BIN",
        "HIDE_TEST_FISH_BIN",
    ]
    .map(|variable| {
        let bin = shell(variable);
        let launch = install(&home.join("hide"), &user(&bin, None))
            .unwrap()
            .unwrap();
        (bin, launch)
    });
    for (bin, launch) in &launches {
        let mut args: Vec<&str> = launch.command[1..].iter().map(String::as_str).collect();
        args.push("-i");
        let after = "echo after >> \"$HOME/log\"\nexit\n";
        let (log, screen) = run(bin, &args, &home, &launch.env, after);
        assert_eq!(log, "after\n", "{bin:?}");
        assert_eq!(
            screen.matches(SCREEN_MARK).count(),
            1,
            "{bin:?}: {screen:?}"
        );
    }
}

/// Each shell says its folder on its screen (OSC 7) as it starts and after a
/// `cd`, a `%` in a name spelled `%25`, for the pane's holder.
#[test]
#[ignore = "needs zsh, bash and fish: set HIDE_TEST_ZSH_BIN, HIDE_TEST_BASH_BIN and HIDE_TEST_FISH_BIN and run with --ignored"]
fn each_shell_says_its_folder_on_its_screen() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    std::fs::create_dir(home.join("a b%c")).unwrap();
    let said = |folder: String| format!("\u{1b}]7;file://{folder}\u{7}");
    for variable in [
        "HIDE_TEST_ZSH_BIN",
        "HIDE_TEST_BASH_BIN",
        "HIDE_TEST_FISH_BIN",
    ] {
        let bin = shell(variable);
        let launch = install(&home.join("hide"), &user(&bin, None))
            .unwrap()
            .unwrap();
        let mut args: Vec<&str> = launch.command[1..].iter().map(String::as_str).collect();
        args.push("-i");
        let after = "cd 'a b%c'\necho after >> \"$HOME/log\"\nexit\n";
        let (_, screen) = run(&bin, &args, &home, &launch.env, after);
        assert!(
            screen.contains(&said(home.display().to_string())),
            "{variable}: {screen:?}"
        );
        assert!(
            screen.contains(&said(format!("{}/a b%25c", home.display()))),
            "{variable}: {screen:?}"
        );
    }
}

/// Bash 5.1 runs each element of a `PROMPT_COMMAND` array: the user's own
/// elements all still run, in their order, and the folder is said after
/// all of them, not inside the first.
#[test]
#[ignore = "needs bash 5.1 or later: set HIDE_TEST_BASH_BIN and run with --ignored"]
fn bash_keeps_every_prompt_command_the_user_set() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let rc = "user_one() { echo one >> \"$HOME/log\"; }\nuser_two() { echo two >> \"$HOME/log\"; echo user-two-ran; }\nPROMPT_COMMAND=(user_one user_two)\n";
    for name in [".bashrc", ".bash_profile"] {
        std::fs::write(home.join(name), rc).unwrap();
    }
    let bin = shell("HIDE_TEST_BASH_BIN");
    let launch = install(&home.join("hide"), &user(&bin, None))
        .unwrap()
        .unwrap();
    let mut args: Vec<&str> = launch.command[1..].iter().map(String::as_str).collect();
    args.push("-i");
    let (log, screen) = run(
        &bin,
        &args,
        &home,
        &launch.env,
        "echo after >> \"$HOME/log\"\nexit\n",
    );
    assert!(log.starts_with("one\ntwo\nafter\none\ntwo\n"), "{log:?}");
    let said = screen
        .find(&format!("\u{1b}]7;file://{}\u{7}", home.display()))
        .unwrap_or_else(|| panic!("the folder was not said: {screen:?}"));
    let two = screen.find("user-two-ran").expect("user_two ran");
    assert!(
        two < said,
        "the folder was said before user_two: {screen:?}"
    );
}
