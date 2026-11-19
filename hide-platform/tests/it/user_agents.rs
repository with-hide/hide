#[cfg(unix)]
use hide_platform::user_agents::Login;
use hide_platform::user_agents::{LoginAgent, NOT_THE_ACCOUNT_HOME, UserAgents, account_home};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::time::Duration;

#[test]
fn the_account_home_is_an_absolute_path() {
    assert!(account_home().unwrap().is_absolute());
}

/// A system with no login agents answers each question that needs one
/// `Unsupported` with its reason, holds no agent to read, unload, remove or
/// forget, and runs nothing: a HOME that is not the account's own gets this
/// on every system. `reason` is why it has none.
fn has_no_login_agents(agents: &UserAgents, reason: &str) {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let stop = AtomicBool::new(false);
    let unsupported = |result: std::io::Result<()>, reason: &str| {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
        assert_eq!(error.to_string(), reason);
    };
    let agent = LoginAgent {
        label: "example.agent",
        program: Path::new("/bin/sleep"),
        arguments: &[],
        environment: &[],
        log: &home.join("agent.log"),
    };
    unsupported(agents.supported(), reason);
    unsupported(agents.login(home, &stop).map(drop), reason);
    unsupported(
        agents.sleeps_on_power(home, &stop).map(drop),
        "this system has no power settings to read",
    );
    unsupported(
        agents.install(&agent, &home.join("marker"), home, &stop),
        reason,
    );
    unsupported(agents.linger(home, &stop).map(drop), reason);
    unsupported(agents.start_once(&agent, home, home, &stop), reason);
    unsupported(agents.start("example.agent", home, &stop), reason);
    assert_eq!(
        agents.last_exit("example.agent", home, &stop).unwrap(),
        None
    );
    assert!(!agents.is_loaded("example.agent", home, &stop).unwrap());
    assert!(!agents.is_installed("example.agent", home).unwrap());
    agents.unload("example.agent", home, &stop).unwrap();
    agents.remove("example.agent", home, &stop).unwrap();
    agents.forget("example.agent", home, &stop).unwrap();
    assert_eq!(std::fs::read_dir(home).unwrap().count(), 0);
}

#[test]
fn none_has_no_login_agents() {
    has_no_login_agents(&UserAgents::none(), "this system has no login agents");
}

/// The system's login agents are the account's whatever HOME says, so only
/// a program whose HOME is the account's own home gets them: any other
/// HOME, a test's or an isolated run's, has none and says why.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn only_the_accounts_own_home_has_the_systems_login_agents() {
    let own = account_home().unwrap();
    UserAgents::of_home(&own).supported().unwrap();
    // The same folder spelled another way is still the account's.
    UserAgents::of_home(&own.join(".")).supported().unwrap();

    let other = tempfile::tempdir().unwrap();
    has_no_login_agents(&UserAgents::of_home(other.path()), NOT_THE_ACCOUNT_HOME);
    // A folder inside the account's home is not its home, nor is one that
    // is not there.
    has_no_login_agents(
        &UserAgents::of_home(&own.join("no-such-folder-of-hide")),
        NOT_THE_ACCOUNT_HOME,
    );
}

/// Each desktop system keeps login agents: launchd, systemd's user manager,
/// Task Scheduler.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn every_desktop_system_has_login_agents() {
    UserAgents::current().supported().unwrap();
}

#[cfg(unix)]
#[test]
fn fixture_inspection_failure_does_not_attempt_a_bootout() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(
        &directory.path().join("launchctl"),
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"$HOME/calls\"\nexit 5\n",
    );
    let boundary = UserAgents::fixture(directory.path());
    assert!(
        boundary
            .unload("example.agent", directory.path(), &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("calls")).unwrap(),
        "print\n"
    );
}

/// Someone is logged in when the account's session answers, and an answer
/// the system's program cannot give is a failure, not a session.
#[cfg(unix)]
#[test]
fn the_login_is_what_the_session_answers() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let stop = AtomicBool::new(false);
    crate::stand_ins::program(
        &home.join("launchctl"),
        "#!/bin/sh\n[ -f \"$HOME/logged-in\" ] && exit 0\nexit 113\n",
    );
    let boundary = UserAgents::fixture(home);
    assert_eq!(boundary.login(home, &stop).unwrap(), Login::LoggedOut);
    std::fs::write(home.join("logged-in"), "").unwrap();
    assert_eq!(boundary.login(home, &stop).unwrap(), Login::LoggedIn);
    let gone = UserAgents::fixture(&home.join("missing"));
    let error = gone.login(home, &stop).unwrap_err();
    assert!(
        error.to_string().starts_with("launchctl did not answer: "),
        "{error}"
    );
}

/// The machine sleeps on its own on power after the power section's
/// `sleep` minutes, never at 0; a sleep setting only on battery, one that
/// is not a number, or a program that fails answers no sleep at all.
#[cfg(unix)]
#[test]
fn only_the_power_section_s_sleep_counts() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let stop = AtomicBool::new(false);
    crate::stand_ins::program(
        &home.join("pmset"),
        "#!/bin/sh\n[ \"$*\" = '-g custom' ] || exit 2\ncat \"$HOME/settings\"\n",
    );
    let boundary = UserAgents::fixture(home);
    let sleeps = |settings: &str| {
        std::fs::write(home.join("settings"), settings).unwrap();
        boundary
            .sleeps_on_power(home, &stop)
            .map_err(|error| error.to_string())
    };
    assert_eq!(
        sleeps(
            "Battery Power:\n lidwake              1\n sleep                1\n displaysleep         2\nAC Power:\n lidwake              1\n sleep                0 (sleep prevented by sharingd)\n displaysleep         10\n"
        ),
        Ok(None)
    );
    assert_eq!(
        sleeps("AC Power:\n displaysleep         10\n sleep                10\n"),
        Ok(Some(Duration::from_secs(600)))
    );
    let none = Err("pmset reports no sleep setting on power".to_owned());
    assert_eq!(sleeps("Battery Power:\n sleep                0\n"), none);
    assert_eq!(sleeps("AC Power:\n sleep                never\n"), none);
    std::fs::remove_file(home.join("settings")).unwrap();
    assert_eq!(
        boundary
            .sleeps_on_power(home, &stop)
            .unwrap_err()
            .to_string(),
        "pmset exited Some(1)"
    );
}

/// A bootout may take as long to answer as its job takes to end, which can be
/// launchd's whole exit allowance: one past the bound of every other command
/// still unloads it.
#[cfg(unix)]
#[test]
fn a_job_that_takes_its_time_to_stop_is_still_unloaded() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(
        &directory.path().join("launchctl"),
        "#!/bin/sh\ncase \"$1\" in\n  print) [ -f \"$HOME/loaded\" ] && exit 0; exit 113 ;;\n  bootout) sleep 6; rm -f \"$HOME/loaded\" ;;\nesac\nexit 0\n",
    );
    std::fs::write(directory.path().join("loaded"), "").unwrap();
    let boundary = UserAgents::fixture(directory.path());
    boundary
        .unload("example.agent", directory.path(), &AtomicBool::new(false))
        .unwrap();
    assert!(!directory.path().join("loaded").exists());
}

/// A bootout that answers before launchd has finished the teardown (0, 36
/// "Operation now in progress", or 37 "Operation already in progress" when
/// another bootout began it) leaves the job loaded while it exits: the
/// unload waits until launchd no longer knows it.
#[cfg(unix)]
#[test]
fn an_unload_waits_out_a_teardown_the_bootout_left_under_way() {
    for answer in [0, 36, 37] {
        let directory = tempfile::tempdir().unwrap();
        // `print` answers loaded twice more after the bootout, then gone.
        crate::stand_ins::program(
            &directory.path().join("launchctl"),
            &format!(
                "#!/bin/sh\ncase \"$1\" in\n  print)\n    [ -f \"$HOME/left\" ] || exit 0\n    left=$(cat \"$HOME/left\")\n    [ \"$left\" -eq 0 ] && exit 113\n    echo $((left - 1)) > \"$HOME/left\"\n    exit 0 ;;\n  bootout) echo 2 > \"$HOME/left\"; exit {answer} ;;\nesac\nexit 0\n"
            ),
        );
        let boundary = UserAgents::fixture(directory.path());
        boundary
            .unload("example.agent", directory.path(), &AtomicBool::new(false))
            .unwrap_or_else(|error| panic!("bootout answering {answer}: {error}"));
        assert_eq!(
            std::fs::read_to_string(directory.path().join("left")).unwrap(),
            "0\n",
            "bootout answering {answer}"
        );
    }
}

/// A bootout that fails with the job still loaded is reported at once, not
/// waited out.
#[cfg(unix)]
#[test]
fn a_failed_bootout_with_the_job_still_loaded_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(
        &directory.path().join("launchctl"),
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"$HOME/calls\"\n[ \"$1\" = bootout ] && exit 5\nexit 0\n",
    );
    let boundary = UserAgents::fixture(directory.path());
    let error = boundary
        .unload("example.agent", directory.path(), &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(error.to_string(), "bootout failed (exit Some(5))");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("calls")).unwrap(),
        "print\nbootout\nprint\n"
    );
}

/// `print` answers loaded once a list is bootstrapped and until a bootout;
/// the list each bootstrap loads is kept as `list`.
#[cfg(unix)]
const RECORDING_LAUNCHCTL: &str = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/calls\"\ncase \"$1\" in\n  print) [ -f \"$HOME/loaded\" ] && exit 0; exit 113 ;;\n  bootstrap) cp \"$3\" \"$HOME/list\"; touch \"$HOME/loaded\" ;;\n  bootout) rm -f \"$HOME/loaded\" ;;\nesac\nexit 0\n";

/// The changes a recording `launchctl` was asked for, as the command and
/// its domain or job.
#[cfg(unix)]
fn changes(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join("calls"))
        .unwrap()
        .lines()
        .filter(|call| !call.starts_with("print "))
        .map(|call| call.split(' ').take(2).collect::<Vec<_>>().join(" "))
        .collect()
}

#[cfg(target_os = "macos")]
fn lint(list: &Path) {
    let lint = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(list)
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stdout)
    );
}

/// An install replaces what is loaded, then loads its definition for the
/// home it was given, kept alive while its marker exists; a removal unloads
/// before it uninstalls, and a forgotten agent stays loaded but is no longer
/// installed.
#[cfg(unix)]
#[test]
fn an_install_loads_its_definition_and_a_removal_unloads_it_first() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(&directory.path().join("launchctl"), RECORDING_LAUNCHCTL);
    let boundary = UserAgents::fixture(directory.path());
    let home = directory.path();
    let stop = AtomicBool::new(false);
    let agent = LoginAgent {
        label: "example.agent",
        program: Path::new("/bin/sleep"),
        arguments: &["600"],
        environment: &[("HOME", "/private/tmp/a&b")],
        log: &home.join("agent.log"),
    };
    let marker = Path::new("/private/tmp/a&b/keep-alive");
    boundary.install(&agent, marker, home, &stop).unwrap();
    let written = std::fs::read_to_string(home.join("list")).unwrap();
    assert!(
        written.contains("<string>/private/tmp/a&amp;b</string>"),
        "{written}"
    );
    assert!(
        written.contains(
            "<key>KeepAlive</key><dict><key>PathState</key><dict><key>/private/tmp/a&amp;b/keep-alive</key><true/></dict></dict>"
        ),
        "{written}"
    );
    assert!(written.contains("<key>RunAtLoad</key><true/>"), "{written}");
    #[cfg(target_os = "macos")]
    lint(&home.join("list"));
    assert!(boundary.is_loaded("example.agent", home, &stop).unwrap());
    assert!(boundary.is_installed("example.agent", home).unwrap());
    let other = tempfile::tempdir().unwrap();
    assert!(
        !boundary
            .is_installed("example.agent", other.path())
            .unwrap()
    );
    boundary.install(&agent, marker, home, &stop).unwrap();
    boundary.remove("example.agent", home, &stop).unwrap();
    assert!(!boundary.is_installed("example.agent", home).unwrap());
    assert!(!boundary.is_loaded("example.agent", home, &stop).unwrap());
    assert_eq!(
        changes(home),
        [
            "bootstrap fixture",
            "bootout fixture/example.agent",
            "bootstrap fixture",
            "bootout fixture/example.agent",
        ]
    );

    boundary.install(&agent, marker, home, &stop).unwrap();
    boundary.forget("example.agent", home, &stop).unwrap();
    assert!(!boundary.is_installed("example.agent", home).unwrap());
    assert!(boundary.is_loaded("example.agent", home, &stop).unwrap());
    boundary.forget("example.agent", home, &stop).unwrap();
    boundary.unload("example.agent", home, &stop).unwrap();
}

/// A recording `systemctl`: `show` answers active once a unit was started
/// and until it is stopped, and a `daemon-reload` keeps a copy of each unit
/// the folder holds.
#[cfg(unix)]
const RECORDING_SYSTEMCTL: &str = "#!/bin/sh\nshift\nprintf '%s\\n' \"$*\" >> \"$HOME/calls\"\ncase \"$1\" in\n  show) [ -f \"$HOME/active\" ] && echo ActiveState=active && exit 0; echo ActiveState=inactive; exit 0 ;;\n  start) touch \"$HOME/active\" ;;\n  stop) rm -f \"$HOME/active\" ;;\n  enable) mkdir -p \"$HOME/.config/systemd/user/default.target.wants\"; ln -sf \"../$2\" \"$HOME/.config/systemd/user/default.target.wants/$2\" ;;\nesac\nexit 0\n";

/// An agent adopted over its own run is written and left running, never
/// unloaded: the process asking may be that run. One the system dropped is
/// loaded again from its definition, and one never installed is
/// `NotFound`.
#[cfg(unix)]
#[test]
fn an_adopted_agent_keeps_its_run_and_a_dropped_one_is_loaded_again() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(&directory.path().join("launchctl"), RECORDING_LAUNCHCTL);
    let agents = UserAgents::fixture(directory.path());
    let home = directory.path();
    let stop = AtomicBool::new(false);
    let agent = LoginAgent {
        label: "example.agent",
        program: Path::new("/bin/sleep"),
        arguments: &["600"],
        environment: &[],
        log: &home.join("agent.log"),
    };
    let marker = home.join("keep-alive");
    let missing = agents.load("example.agent", home, &stop).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");

    agents.install(&agent, &marker, home, &stop).unwrap();
    agents.forget("example.agent", home, &stop).unwrap();
    agents.adopt(&agent, &marker, home, &stop).unwrap();
    assert!(agents.is_installed("example.agent", home).unwrap());
    assert!(agents.is_loaded("example.agent", home, &stop).unwrap());
    assert_eq!(changes(home), ["bootstrap fixture"], "the run was unloaded");

    std::fs::remove_file(home.join("loaded")).unwrap();
    agents.load("example.agent", home, &stop).unwrap();
    agents.load("example.agent", home, &stop).unwrap();
    assert!(agents.is_loaded("example.agent", home, &stop).unwrap());
    assert_eq!(changes(home), ["bootstrap fixture", "bootstrap fixture"]);
}

/// systemd's agent is a unit the account's manager wants with
/// `default.target`, kept alive while its marker exists: an install stops
/// what runs, writes the unit, reloads, enables and starts it; a removal
/// stops it and takes the unit and its link away; a forgotten unit goes on
/// running.
#[cfg(unix)]
#[test]
fn a_systemd_agent_is_a_wanted_unit_kept_alive_by_its_marker() {
    use hide_platform::user_agents::Kind;
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    crate::stand_ins::program(&home.join("systemctl"), RECORDING_SYSTEMCTL);
    let agents = UserAgents::fixture_of(Kind::Systemd, home);
    let stop = AtomicBool::new(false);
    let agent = LoginAgent {
        label: "example.agent",
        program: Path::new("/opt/a b/hided"),
        arguments: &["core-login", "--state-dir", "/home/example/state"],
        environment: &[],
        log: &home.join("agent.log"),
    };
    let marker = home.join("keep-alive");
    agents.install(&agent, &marker, home, &stop).unwrap();
    let unit =
        std::fs::read_to_string(home.join(".config/systemd/user/example.agent.service")).unwrap();
    assert!(
        unit.contains(&format!("ConditionPathExists={}\n", marker.display())),
        "{unit}"
    );
    assert!(
        unit.contains(
            "ExecStart=\"/opt/a b/hided\" \"core-login\" \"--state-dir\" \"/home/example/state\"\n"
        ),
        "{unit}"
    );
    assert!(agents.is_installed("example.agent", home).unwrap());
    assert!(agents.is_loaded("example.agent", home, &stop).unwrap());
    agents.forget("example.agent", home, &stop).unwrap();
    assert!(!agents.is_installed("example.agent", home).unwrap());
    assert!(
        !home
            .join(".config/systemd/user/default.target.wants/example.agent.service")
            .exists()
    );
    assert!(agents.is_loaded("example.agent", home, &stop).unwrap());
    agents.install(&agent, &marker, home, &stop).unwrap();
    agents.remove("example.agent", home, &stop).unwrap();
    assert!(!agents.is_loaded("example.agent", home, &stop).unwrap());
    let calls: Vec<String> = std::fs::read_to_string(home.join("calls"))
        .unwrap()
        .lines()
        .filter(|call| !call.starts_with("show "))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        calls,
        [
            "daemon-reload",
            "enable example.agent.service",
            "start --no-block example.agent.service",
            "daemon-reload",
            "stop example.agent.service",
            "daemon-reload",
            "enable example.agent.service",
            "start --no-block example.agent.service",
            "stop example.agent.service",
            "daemon-reload",
        ]
    );
}

/// A systemd unit adopted over its own run is rewritten and enabled, never
/// stopped and not started again: the process asking may be that run. One
/// not running is started.
#[cfg(unix)]
#[test]
fn a_systemd_adopt_never_stops_a_running_unit() {
    use hide_platform::user_agents::Kind;
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    crate::stand_ins::program(&home.join("systemctl"), RECORDING_SYSTEMCTL);
    let agents = UserAgents::fixture_of(Kind::Systemd, home);
    let stop = AtomicBool::new(false);
    let agent = LoginAgent {
        label: "example.agent",
        program: Path::new("/opt/hide/hided"),
        arguments: &["core-login"],
        environment: &[],
        log: &home.join("agent.log"),
    };
    let marker = home.join("keep-alive");
    agents.install(&agent, &marker, home, &stop).unwrap();
    agents.forget("example.agent", home, &stop).unwrap();
    std::fs::remove_file(home.join("calls")).unwrap();

    agents.adopt(&agent, &marker, home, &stop).unwrap();
    assert!(agents.is_installed("example.agent", home).unwrap());
    assert!(agents.is_loaded("example.agent", home, &stop).unwrap());
    let calls = |home: &Path| -> Vec<String> {
        std::fs::read_to_string(home.join("calls"))
            .unwrap()
            .lines()
            .filter(|call| !call.starts_with("show "))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        calls(home),
        ["daemon-reload", "enable example.agent.service"],
        "the running unit was stopped or started again"
    );

    std::fs::remove_file(home.join("active")).unwrap();
    std::fs::remove_file(home.join("calls")).unwrap();
    agents.adopt(&agent, &marker, home, &stop).unwrap();
    assert_eq!(
        calls(home),
        [
            "daemon-reload",
            "enable example.agent.service",
            "start --no-block example.agent.service"
        ]
    );
}

/// The account lingers once `loginctl` says it does; a refusal names the
/// command an administrator runs.
#[cfg(unix)]
#[test]
fn a_linger_the_account_may_not_ask_for_names_the_command_that_turns_it_on() {
    use hide_platform::user_agents::{Kind, Linger};
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    crate::stand_ins::program(
        &home.join("loginctl"),
        "#!/bin/sh\ncase \"$1\" in\n  show-user) [ -f \"$HOME/lingers\" ] && echo yes && exit 0; echo no; exit 0 ;;\n  enable-linger) [ -f \"$HOME/allowed\" ] && touch \"$HOME/lingers\" && exit 0; echo 'Access denied' >&2; exit 1 ;;\nesac\nexit 2\n",
    );
    crate::stand_ins::program(&home.join("systemctl"), "#!/bin/sh\nexit 0\n");
    let agents = UserAgents::fixture_of(Kind::Systemd, home);
    let stop = AtomicBool::new(false);
    assert_eq!(agents.login(home, &stop).unwrap(), Login::LoggedIn);
    let Linger::NeedsAdministrator { command } = agents.linger(home, &stop).unwrap() else {
        panic!("a refused linger is on");
    };
    assert!(
        command.starts_with("sudo loginctl enable-linger "),
        "{command}"
    );
    std::fs::write(home.join("allowed"), "").unwrap();
    assert_eq!(agents.linger(home, &stop).unwrap(), Linger::On);
    assert_eq!(agents.login(home, &stop).unwrap(), Login::NotNeeded);
}

/// Task Scheduler's agent is a task under `\\Hide\\` created from its
/// definition and run at once; it carries no environment, and a forgotten
/// one is deleted.
#[cfg(unix)]
#[test]
fn a_task_scheduler_agent_is_a_task_created_from_its_definition() {
    use hide_platform::user_agents::Kind;
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    crate::stand_ins::program(
        &home.join("schtasks"),
        "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$3\" >> \"$HOME/calls\"\n[ \"$1\" = /Create ] && cp \"$5\" \"$HOME/task.xml\"\nexit 0\n",
    );
    let agents = UserAgents::fixture_of(Kind::TaskScheduler, home);
    let stop = AtomicBool::new(false);
    let marker = home.join("keep-alive");
    let mut agent = LoginAgent {
        label: "example.agent",
        program: Path::new("C:\\hide\\hided.exe"),
        arguments: &["core-login"],
        environment: &[("HOME", "C:\\x")],
        log: &home.join("agent.log"),
    };
    let refused = agents.install(&agent, &marker, home, &stop).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::Unsupported, "{refused}");
    agent.environment = &[];
    agents.install(&agent, &marker, home, &stop).unwrap();
    assert!(agents.is_installed("example.agent", home).unwrap());
    let task = std::fs::read(home.join("task.xml")).unwrap();
    assert_eq!(&task[..2], &[0xFF, 0xFE], "UTF-16 with its byte order mark");
    agents.remove("example.agent", home, &stop).unwrap();
    assert!(!agents.is_installed("example.agent", home).unwrap());
    assert_eq!(
        std::fs::read_to_string(home.join("calls")).unwrap(),
        "/Create \\Hide\\example.agent\n/Run \\Hide\\example.agent\n/End \\Hide\\example.agent\n/Delete \\Hide\\example.agent\n"
    );
}

/// The real launchd of a disposable macOS runner (PRD core-host-node-move
/// D-16, device-connect-iroh D-41): a login agent installed this way starts,
/// comes back after it is killed while its marker exists, stays stopped
/// after an exit once the marker is gone, starts again when the marker comes
/// back, is replaced by the next install when it was only forgotten (a core
/// that gives its copy back leaves itself loaded, B9), and is gone with its
/// process when removed. It
/// refuses to run anywhere but a hosted CI runner, since `gui/<uid>` is the
/// account's real session whatever HOME is.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "real launchd: only on a disposable macOS CI runner"]
fn a_login_agent_restarts_while_its_marker_exists_and_ends_when_removed() {
    use std::time::Instant;
    assert!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "this test reaches the account's real launchd; it runs only on a hosted CI runner"
    );
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let program = home.join("agent");
    // Records each start; a TERM ends it successfully. The store's own first
    // start (`--help`) ends at once, as every stand-in's must.
    crate::stand_ins::program(
        &program,
        "#!/bin/sh\n[ \"$1\" = --help ] && exit 0\ntrap 'exit 0' TERM\necho $$ >> \"$HOME/starts\"\nwhile :; do sleep 1; done\n",
    );
    let label = format!("dev.withhide.contract.{}", std::process::id());
    let home_value = home.to_string_lossy().into_owned();
    let environment = [("HOME", home_value.as_str())];
    let agent = LoginAgent {
        label: &label,
        program: &program,
        arguments: &[],
        environment: &environment,
        log: &home.join("agent.log"),
    };
    let agents = UserAgents::current();
    let stop = AtomicBool::new(false);
    let starts = || -> Vec<u32> {
        std::fs::read_to_string(home.join("starts"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    };
    #[allow(clippy::disallowed_methods)] // a bounded poll of another process
    let wait = |what: &str, within: Duration, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + within;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    let alive = |pid: u32| hide_platform::process::is_alive(pid);
    assert_eq!(
        agents.login(home, &stop).unwrap(),
        Login::LoggedIn,
        "no GUI session"
    );
    let marker = home.join("keep-alive");
    std::fs::write(&marker, "").unwrap();
    agents.install(&agent, &marker, home, &stop).unwrap();
    let removed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait("the first start", Duration::from_secs(20), &|| {
            starts().len() == 1
        });
        let first = starts()[0];
        assert!(alive(first));
        hide_platform::process::kill_tree(first).unwrap();
        wait("the restart after a kill", Duration::from_secs(30), &|| {
            starts().len() == 2
        });
        let second = starts()[1];
        assert!(alive(second));
        std::fs::remove_file(&marker).unwrap();
        hide_platform::process::terminate(second).unwrap();
        wait("the exit", Duration::from_secs(10), &|| !alive(second));
        #[allow(clippy::disallowed_methods)] // past launchd's ten-second throttle
        std::thread::sleep(Duration::from_secs(15));
        assert_eq!(starts().len(), 2, "launchd restarted with the marker gone");
        std::fs::write(&marker, "").unwrap();
        wait(
            "the start the marker asked for",
            Duration::from_secs(30),
            &|| starts().len() == 3,
        );
        let third = starts()[2];
        // Only the installation goes; the job stays loaded and running.
        agents.forget(&label, home, &stop).unwrap();
        assert!(agents.is_loaded(&label, home, &stop).unwrap());
        agents.install(&agent, &marker, home, &stop).unwrap();
        wait(
            "the start of the new install",
            Duration::from_secs(20),
            &|| starts().len() == 4,
        );
        wait(
            "the end of what was left loaded",
            Duration::from_secs(10),
            &|| !alive(third),
        );
        // A start asked for right after an exit, while launchd holds the
        // job's restart back for its throttle: `hide stop` then `hide
        // connect` does this, and the start waits the throttle out.
        let fourth = starts()[3];
        std::fs::remove_file(&marker).unwrap();
        hide_platform::process::terminate(fourth).unwrap();
        wait("the fourth's exit", Duration::from_secs(10), &|| {
            !alive(fourth)
        });
        std::fs::write(&marker, "").unwrap();
        agents.start(&label, home, &stop).unwrap();
        wait("the asked-for start", Duration::from_secs(5), &|| {
            starts().len() == 5
        });
        starts()[4]
    }));
    let removal = agents.remove(&label, home, &stop);
    let running = removed.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    removal.unwrap();
    assert!(
        !agents.is_loaded(&label, home, &stop).unwrap(),
        "still loaded"
    );
    wait("the removed agent's end", Duration::from_secs(10), &|| {
        !alive(running)
    });
    assert!(!agents.is_installed(&label, home).unwrap());
}

/// A job's exit is read only once it is not running and has exited, and
/// only while the system holds it.
#[cfg(unix)]
#[test]
fn a_job_s_exit_is_read_only_once_it_ran_and_stopped() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let stop = AtomicBool::new(false);
    crate::stand_ins::program(
        &home.join("launchctl"),
        "#!/bin/sh\n[ -f \"$HOME/printed\" ] || exit 113\ncat \"$HOME/printed\"\n",
    );
    let boundary = UserAgents::fixture(home);
    let last_exit = |printed: &str| {
        std::fs::write(home.join("printed"), printed).unwrap();
        boundary.last_exit("example.job", home, &stop).unwrap()
    };
    let job = |state: &str, code: &str| {
        format!(
            "gui/501/example.job = {{\n\tactive count = 0\n\tstate = {state}\n\truns = 1\n\tlast exit code = {code}\n}}\n"
        )
    };
    assert_eq!(last_exit(&job("not running", "3")).as_deref(), Some("3"));
    assert_eq!(last_exit(&job("running", "3")), None);
    assert_eq!(last_exit(&job("not running", "(never exited)")), None);
    assert_eq!(last_exit("gui/501/example.job = {\n}\n"), None);
    std::fs::remove_file(home.join("printed")).unwrap();
    assert_eq!(
        boundary.last_exit("example.job", home, &stop).unwrap(),
        None
    );
}

/// A one-shot job is loaded from the caller's folder, never installed, so
/// no later login starts it, and launchd never restarts it.
#[cfg(unix)]
#[test]
fn a_one_shot_job_is_loaded_from_the_caller_s_folder_and_never_restarted() {
    let directory = tempfile::tempdir().unwrap();
    crate::stand_ins::program(&directory.path().join("launchctl"), RECORDING_LAUNCHCTL);
    let boundary = UserAgents::fixture(directory.path());
    let home = directory.path();
    let stop = AtomicBool::new(false);
    let folder = home.join("job");
    std::fs::create_dir(&folder).unwrap();
    let agent = LoginAgent {
        label: "example.job",
        program: Path::new("/usr/bin/true"),
        arguments: &[],
        environment: &[],
        log: &home.join("job.log"),
    };
    boundary.start_once(&agent, &folder, home, &stop).unwrap();
    let written = std::fs::read_to_string(home.join("list")).unwrap();
    assert!(
        written.contains("<key>KeepAlive</key><false/>"),
        "{written}"
    );
    assert!(!written.contains("ThrottleInterval"), "{written}");
    #[cfg(target_os = "macos")]
    lint(&home.join("list"));
    assert!(!boundary.is_installed("example.job", home).unwrap());
    let calls = std::fs::read_to_string(home.join("calls")).unwrap();
    assert!(
        calls.contains(&format!("bootstrap fixture {}/", folder.display())),
        "{calls}"
    );
    boundary.unload("example.job", home, &stop).unwrap();
    assert_eq!(
        changes(home),
        ["bootstrap fixture", "bootout fixture/example.job"]
    );
}

/// The real launchd of a disposable macOS runner (PRD core-host-node-move
/// B3): a one-shot job runs once in the GUI session, is not started again
/// after it fails, and is gone when unloaded. It refuses to run anywhere
/// but a hosted CI runner, like the login agent's test.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "real launchd: only on a disposable macOS CI runner"]
fn a_one_shot_job_runs_once_in_the_gui_session_even_when_it_fails() {
    use std::time::Instant;
    assert!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "this test reaches the account's real launchd; it runs only on a hosted CI runner"
    );
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let program = home.join("job");
    // Records each start and fails, which a login agent would restart.
    crate::stand_ins::program(&program, "#!/bin/sh\necho $$ >> \"$HOME/starts\"\nexit 3\n");
    let label = format!("dev.withhide.contract.once.{}", std::process::id());
    let home_value = home.to_string_lossy().into_owned();
    let environment = [("HOME", home_value.as_str())];
    let agent = LoginAgent {
        label: &label,
        program: &program,
        arguments: &[],
        environment: &environment,
        log: &home.join("job.log"),
    };
    let agents = UserAgents::current();
    let stop = AtomicBool::new(false);
    let starts = || {
        std::fs::read_to_string(home.join("starts"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    agents.start_once(&agent, home, home, &stop).unwrap();
    let ran = std::panic::catch_unwind(|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while starts() == 0 {
            assert!(Instant::now() < deadline, "the job never ran");
            #[allow(clippy::disallowed_methods)] // a bounded poll of another process
            std::thread::sleep(Duration::from_millis(200));
        }
        #[allow(clippy::disallowed_methods)] // past launchd's ten-second throttle
        std::thread::sleep(Duration::from_secs(15));
        assert_eq!(starts(), 1, "launchd started a one-shot job again");
        assert_eq!(
            agents.last_exit(&label, home, &stop).unwrap().as_deref(),
            Some("3")
        );
    });
    let unloaded = agents.unload(&label, home, &stop);
    ran.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    unloaded.unwrap();
    assert!(!agents.is_loaded(&label, home, &stop).unwrap());
    assert!(!agents.is_installed(&label, home).unwrap());
}
