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

/// Where a shell runs: in a Herdr pane, whose binary is a script that
/// records the marks reported, or outside one, on the core's own engine.
#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Herdr,
    Own,
}

/// A shell started with `args` and `env` from the integration, reading
/// `commands` on its input. Answers what the user's files, the mark and the
/// commands wrote, in order, then the marks reported.
fn run(
    bin: &Path,
    args: &[&str],
    home: &Path,
    env: &BTreeMap<String, String>,
    commands: &str,
) -> (String, String) {
    run_in(Pane::Herdr, bin, args, home, env, commands).0
}

/// [`run`] in `pane`, with what the shell wrote to its output.
#[allow(clippy::disallowed_methods)] // env_clear: the shell starts with only what the integration gives it (Unix only)
fn run_in(
    pane: Pane,
    bin: &Path,
    args: &[&str],
    home: &Path,
    env: &BTreeMap<String, String>,
    commands: &str,
) -> ((String, String), String) {
    let mark = home.join("mark.sh");
    std::fs::write(
        &mark,
        "#!/bin/sh\necho mark >> \"$HOME/log\"\necho \"$@\" >> \"$HOME/mark\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&mark, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut command = Command::new(bin);
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb");
    if pane == Pane::Herdr {
        command
            .env("HERDR_BIN_PATH", &mark)
            .env("HERDR_PANE_ID", "w1:p1");
    }
    let mut child = command
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
    let read = |name: &str| {
        let text = std::fs::read_to_string(home.join(name)).unwrap_or_default();
        let _ = std::fs::remove_file(home.join(name));
        text.replace(&home.display().to_string(), "~")
    };
    (
        (read("log"), read("mark")),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// The prompt mark a shell outside Herdr writes on its screen.
const SCREEN_MARK: &str = "\u{1b}]133;A\u{7}";

const MARK: &str = "pane report-metadata w1:p1 --source hide:shell --token hide_prompt=1\n";

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
            format!("echo \"{name} {seen}\" >> \"$HOME/log\"\n{declare} from_{variable}\nfrom_{variable}[a]=1\n"),
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
    let (log, mark) = run(&zsh, &["-i", "-l"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv unset\n.zprofile unset\n.zshrc unset\n.zlogin unset\nmark\nafter unset  0 association association\n"
    );
    assert_eq!(mark, MARK);

    // An interactive shell that is not a login shell, the pane's on Linux,
    // reads two.
    let (log, mark) = run(&zsh, &["-i"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv unset\n.zshrc unset\nmark\nafter unset  0 association association\n"
    );
    assert_eq!(mark, MARK);

    // A command reads only .zshenv, gets everything back, and marks nothing.
    let (log, mark) = run(
        &zsh,
        &["-c", ZSH_AFTER.trim_end_matches("exit\n")],
        &home,
        &env,
        "",
    );
    assert_eq!(log, ".zshenv unset\nafter unset  0 association \n");
    assert_eq!(mark, "");

    // A user whose own ZDOTDIR is set gets it back, exported, and their
    // files are read from it.
    let zdotdir = home.join("z");
    user_files(&zdotdir, &names, "${ZDOTDIR-unset}", "typeset -A");
    let env = install(&home.join("hide"), &user(&zsh, Some(&zdotdir)))
        .unwrap()
        .unwrap()
        .env;
    let (log, mark) = run(&zsh, &["-i", "-l"], &home, &env, ZSH_AFTER);
    assert_eq!(
        log,
        ".zshenv ~/z\n.zprofile ~/z\n.zshrc ~/z\n.zlogin ~/z\nmark\nafter ~/z scalar-export 0 association association\n"
    );
    assert_eq!(mark, MARK);
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
    // As Herdr runs it, interactive, with the init file the launch names.
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
    let (home, init) = folder("login.bash");
    user_files(
        home.path(),
        &[".bash_profile", ".bash_login", ".profile"],
        "login",
        "declare -a",
    );
    let (log, mark) = run_with(home.path(), &init);
    assert_eq!(log, ".bash_profile login\nmark\nafter unset  1 \n");
    assert_eq!(mark, MARK);
    std::fs::remove_file(home.path().join(".bash_profile")).unwrap();
    std::fs::remove_file(home.path().join(".bash_login")).unwrap();
    let (log, _) = run_with(home.path(), &init);
    assert_eq!(log, ".profile login\nmark\nafter unset   1\n");

    // Not a login shell (Linux): ~/.bashrc, whose PROMPT_COMMAND, set or
    // appended to, is the user's own.
    let (home, init) = folder("interactive.bash");
    user_files(home.path(), &[".bashrc"], "rc", "declare -a");
    std::fs::write(
        home.path().join(".bashrc"),
        std::fs::read_to_string(home.path().join(".bashrc")).unwrap()
            + "PROMPT_COMMAND=\"${PROMPT_COMMAND:+$PROMPT_COMMAND;}true\"\n",
    )
    .unwrap();
    let (log, mark) = run_with(home.path(), &init);
    assert_eq!(log, ".bashrc rc\nmark\nafter true 1  \n");
    assert_eq!(mark, MARK);
}

#[test]
#[ignore = "needs fish: set HIDE_TEST_FISH_BIN and run with --ignored"]
fn fish_marks_after_its_configuration() {
    let fish = shell("HIDE_TEST_FISH_BIN");
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let config = home.join(".config/fish");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.fish"), "echo config >> $HOME/log\n").unwrap();
    let launch = install(&home.join("hide"), &user(&fish, None))
        .unwrap()
        .unwrap();
    let mut args: Vec<&str> = launch.command[1..].iter().map(String::as_str).collect();
    args.push("-i");
    let (log, mark) = run(
        &fish,
        &args,
        &home,
        &launch.env,
        "echo after >> $HOME/log\nexit\n",
    );
    assert_eq!(log, "config\nmark\nafter\n");
    assert_eq!(mark, MARK);
}

/// Outside a Herdr pane, on the core's own engine, each shell marks its first
/// prompt on its screen once, after the user's files, and reports nothing to
/// Herdr; in a Herdr pane it writes no screen mark.
#[test]
#[ignore = "needs zsh, bash and fish: set HIDE_TEST_ZSH_BIN, HIDE_TEST_BASH_BIN and HIDE_TEST_FISH_BIN and run with --ignored"]
fn outside_herdr_each_shell_marks_its_first_prompt_on_its_screen_once() {
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
        let ((log, reported), screen) = run_in(Pane::Own, bin, &args, &home, &launch.env, after);
        assert_eq!(reported, "", "{bin:?} reports nothing outside Herdr");
        assert_eq!(log, "after\n", "{bin:?}");
        assert_eq!(
            screen.matches(SCREEN_MARK).count(),
            1,
            "{bin:?}: {screen:?}"
        );
        let ((_, reported), screen) = run_in(Pane::Herdr, bin, &args, &home, &launch.env, after);
        assert_eq!(reported, MARK, "{bin:?}");
        assert!(!screen.contains(SCREEN_MARK), "{bin:?}: {screen:?}");
    }
}

/// Outside Herdr each shell says its folder on its screen (OSC 7) as it
/// starts and after a `cd`, a `%` in a name spelled `%25`, for the pane's
/// holder; in a Herdr pane, which reads the folder from the process, it says
/// nothing of it.
#[test]
#[ignore = "needs zsh, bash and fish: set HIDE_TEST_ZSH_BIN, HIDE_TEST_BASH_BIN and HIDE_TEST_FISH_BIN and run with --ignored"]
fn outside_herdr_each_shell_says_its_folder_on_its_screen() {
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
        let (_, screen) = run_in(Pane::Own, &bin, &args, &home, &launch.env, after);
        assert!(
            screen.contains(&said(home.display().to_string())),
            "{variable}: {screen:?}"
        );
        assert!(
            screen.contains(&said(format!("{}/a b%25c", home.display()))),
            "{variable}: {screen:?}"
        );
        let (_, screen) = run_in(Pane::Herdr, &bin, &args, &home, &launch.env, after);
        assert!(!screen.contains("\u{1b}]7;"), "{variable}: {screen:?}");
    }
}

/// Bash 5.1 runs each element of a `PROMPT_COMMAND` array: the user's own
/// elements all still run, in their order, and the folder is said after
/// all of them, not inside the first.
#[test]
#[ignore = "needs bash 5.1 or later: set HIDE_TEST_BASH_BIN and run with --ignored"]
fn outside_herdr_bash_keeps_every_prompt_command_the_user_set() {
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
    let ((log, _), screen) = run_in(
        Pane::Own,
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
    assert!(two < said, "the folder was said before user_two: {screen:?}");
}
