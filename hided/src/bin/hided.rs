use hided::role::Role;

/// How long the process waits, once its roles ended, for blocking work
/// still running before it exits.
const EXIT_WITHIN: std::time::Duration = std::time::Duration::from_secs(1);

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    // Read before anything runs: an argument no role names is refused here,
    // before an owner watch, a state folder or a log is touched.
    let role = match hided::role::parse(&args) {
        // A pane's entry point becomes its shell in this same process. In
        // particular it must not acquire a daemon's owner watch or load
        // state.
        Ok(Role::ShellExec(words)) => match hided::shell_exec::run(words) {
            Ok(never) => match never {},
            Err(error) => {
                eprintln!("shell-exec: {error}");
                std::process::exit(2);
            }
        },
        Ok(role) => role,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    // Taken before daemon-role work, as every cooperative child takes it: a
    // starter that passes an owner channel (the web e2e fixture) ends this
    // daemon and what it started by ending itself, even when it is killed.
    // `hide connect` starts the daemon without one, through `spawn_owned`,
    // because the daemon outlives the app that started it.
    let owner_watch = match hide_platform::process::OwnerWatch::from_launch() {
        Ok(watch) => watch,
        Err(error) => {
            eprintln!("owner watch acquisition failed: {error}");
            std::process::exit(2);
        }
    };
    let result = run(role);
    // Before every exit below, so the records queued last reach the log.
    hide_core::diagnostics::drain();
    // After `run` has dropped the runtime, and with it the core.
    if let Some(watch) = owner_watch
        && let Err(error) = watch.close()
    {
        eprintln!("owner watch cleanup failed: {error}");
        std::process::exit(2);
    }
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(role: Role<'_>) -> Result<(), String> {
    match role {
        // Answered before the environment is read: neither opens a state
        // folder, on any machine.
        Role::Version => {
            println!("{}", hided::role::version_line());
            return Ok(());
        }
        Role::Help => {
            println!("{}", hided::role::USAGE);
            return Ok(());
        }
        // `main` hands it its shell before the owner watch.
        Role::ShellExec(_) => {
            return Err("shell-exec runs before the daemon's roles".into());
        }
        // The node role on a device reads no daemon environment and opens no
        // state folder: it answers its link until the channel closes.
        Role::Node(words) => return hided::node_cli::run(words),
        // The attach role a node on another machine runs here over SSH: it
        // pipes the node's channel to the core already running on this
        // machine and starts none.
        Role::Attach(words) => return hided::attach::run(words),
        // A step of a core move run here over SSH by the machine driving it.
        Role::CoreMove(words) => return hided::core_move::target::run(words),
        // The update of the core here to this build, run over SSH by a node
        // of this build.
        Role::CoreUpdate(words) => return hided::core_update::run(words),
        Role::OpenHelper(words) => {
            // Only Unix supervises a file opener through this mode.
            #[cfg(unix)]
            return hide_node::opener::run_opener_helper(words);
            #[cfg(not(unix))]
            {
                let _ = words;
                return Err("--open-helper is the Unix opener supervisor's mode".into());
            }
        }
        // What the login item of a core moved here runs: its log kept within
        // its limit first, then the daemon below.
        Role::CoreLogin => hided::login_item::before_core_at_login(),
        Role::Daemon => {}
    }
    let mut env = hided::env::load().map_err(|errors| {
        errors
            .iter()
            .map(|error| format!("{}: {}", error.key, error.kind))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    // Hashed before anything else: an app update replaces the file under the
    // same path, and a later read would report the new build as this one.
    let build = hided::build_id::of_current_exe()?;
    if hided::env::fixture_fails_build(&build) {
        return Err(format!(
            "this build ({build}) is one a fixture fails at its start"
        ));
    }
    env.build = Some(build);
    if role == Role::CoreLogin {
        env.starter_program = Some(
            std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?,
        );
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("hided")
        .build()
        .map_err(|error| error.to_string())?;
    // The programs the daemon starts by name run on the account's search;
    // its first read starts now, so the daemon's first command seldom waits
    // for it.
    hide_platform::programs::warm_account_path();
    let ran = runtime.block_on(hided::run_daemon(env));
    // The roles have ended and saved; a step a stop abandoned (an SSH step
    // to a peer that stalls) is not waited for, so `hide stop` sees the
    // process end rather than kill it.
    runtime.shutdown_timeout(EXIT_WITHIN);
    ran
}
