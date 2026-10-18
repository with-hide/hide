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
/// `commands` on its input. Answers what the user's files, the mark and the
/// commands wrote, in order, then the marks reported.
#[allow(clippy::disallowed_methods)] // env_clear: the shell starts with only what the integration gives it (Unix only)
fn run(
    bin: &Path,
    args: &[&str],
    home: &Path,
    env: &BTreeMap<String, String>,
    commands: &str,
) -> (String, String) {
    let mark = home.join("mark.sh");
    std::fs::write(
        &mark,
        "#!/bin/sh\necho mark >> \"$HOME/log\"\necho \"$@\" >> \"$HOME/mark\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&mark, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut child = Command::new(bin)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb")
        .env("HERDR_BIN_PATH", &mark)
        .env("HERDR_PANE_ID", "w1:p1")
        .envs(env)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(commands.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let read = |name: &str| {
        let text = std::fs::read_to_string(home.join(name)).unwrap_or_default();
        let _ = std::fs::remove_file(home.join(name));
        text.replace(&home.display().to_string(), "~")
    };
    (read("log"), read("mark"))
}

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
