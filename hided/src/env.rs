//! Single registry for every environment key this crate reads.


use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use hide_platform::host;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvKey {
    pub key: &'static str,
    pub required: bool,
    pub format: &'static str,
    pub absent_behavior: &'static str,
}

pub const PATH: &str = "PATH";
pub const XDG_STATE_HOME: &str = "XDG_STATE_HOME";
pub const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
pub const APPDATA: &str = "APPDATA";
pub const PATHEXT: &str = "PATHEXT";
pub const HIDE_STATE_DIR: &str = "HIDE_STATE_DIR";
pub const HIDE_KEEP_ALIVE: &str = "HIDE_KEEP_ALIVE";
pub const HIDE_VITE_ORIGIN: &str = "HIDE_VITE_ORIGIN";
pub const HIDE_PORT: &str = "HIDE_PORT";
pub const HIDE_IDLE_SECS: &str = "HIDE_IDLE_SECS";
/// Optional, public build input; never read from the operator's runtime environment.
pub const HIDE_POSTHOG_PROJECT_TOKEN: &str = "HIDE_POSTHOG_PROJECT_TOKEN";
/// `HOME`, or `USERPROFILE` on Windows.
pub const HOME: &str = host::HOME_VARIABLE;
pub const HIDE_OPEN_COMMAND: &str = "HIDE_OPEN_COMMAND";
pub const HIDE_HOST_HELPER_ROOT: &str = "HIDE_HOST_HELPER_ROOT";
pub const HIDE_HOST_CLI_DIR: &str = "HIDE_HOST_CLI_DIR";
/// The pane a process runs in, which the engine sets in every pane it runs
/// (`hide_agent_hooks::pane`).
pub const HIDE_PANE_ID: &str = hide_agent_hooks::pane::HIDE_PANE_ID;
pub const HIDE_CAP_REF: &str = "HIDE_CAP_REF";
pub const HIDE_TAILSCALE_BIN: &str = "HIDE_TAILSCALE_BIN";
pub const HIDE_RELAY_URL: &str = "HIDE_RELAY_URL";
pub const RELAY_URL: &str = "wss://relay.withhide.dev/ws";
pub const HIDE_MACHINE_ID: &str = host::MACHINE_ID_VARIABLE;
/// Test-only: how a core move starts the core on the machine taking it.
pub const HIDE_CORE_STARTER: &str = "HIDE_CORE_STARTER";
/// Test-only: `hang` makes the core role's stop never return, so a journey
/// proves the stop's bound.
pub const HIDE_FIXTURE_CORE_STOP: &str = "HIDE_FIXTURE_CORE_STOP";
/// Test-only: a file in a fixture HOME naming a build whose daemon exits at
/// its start, so a journey proves a core update's way back to the previous
/// build; a file, since the fixture's SSH sessions keep the environment
/// they started with.
pub const FIXTURE_FAIL_BUILD_FILE: &str = ".hide-e2e-fail-build";
/// Test-only: a folder of stand-in `launchctl` and `pmset` that a move's
/// checks run instead of the system's.
pub const HIDE_PREFLIGHT_PROGRAMS: &str = "HIDE_PREFLIGHT_PROGRAMS";
/// Test-only: the version this build presents in a link's hello, so a
/// journey shows one build as newer or older than another.
pub const HIDE_BUILD_VERSION_OVERRIDE: &str = "HIDE_BUILD_VERSION_OVERRIDE";
/// Test-only: the build order this build presents in a link's hello.
pub const HIDE_BUILD_ORDER_OVERRIDE: &str = "HIDE_BUILD_ORDER_OVERRIDE";
/// The file that declares a HOME a test fixture's, beside every test-only
/// key's other conditions ([`fixture_home`]).
pub const FIXTURE_HOME_MARKER: &str = hide_kit::layout::FIXTURE_HOME_MARKER;
/// Development only: the home every SSH device's node runs with in place of
/// its account's, so a candidate on a shared device keeps its state, sessions
/// and pane holders apart from the account's own Hide there.
pub const HIDE_DEV_DEVICE_HOME: &str = hide_node::ssh::DEVICE_HOME_VARIABLE;

pub const REGISTRY: &[EnvKey] = &[
    EnvKey {
        key: HIDE_POSTHOG_PROJECT_TOKEN,
        required: false,
        format: "build-time public PostHog project token: phc_ and ASCII letters, digits or underscores, 20..128 bytes; empty disables transport",
        absent_behavior: "No external product analytics transport",
    },
    EnvKey {
        key: PATH,
        required: false,
        format: "the system's executable search path (colon-separated, semicolons on Windows)",
        absent_behavior: "Programs are found on the login shell's PATH and the usual install folders alone",
    },
    EnvKey {
        key: PATHEXT,
        required: false,
        format: "Windows only: semicolon-separated extensions a program on PATH may have",
        absent_behavior: "A program on PATH is found as .COM, .EXE, .BAT or .CMD",
    },
    EnvKey {
        key: XDG_STATE_HOME,
        required: false,
        format: "absolute directory path",
        absent_behavior: "State lives under $HOME/.hide/state; a set value keeps $XDG_STATE_HOME/hide and moves nothing",
    },
    EnvKey {
        key: XDG_CONFIG_HOME,
        required: false,
        format: "absolute directory path",
        absent_behavior: "The Herdr session a move into Hide's engine reads is under ~/.config/herdr (%APPDATA%\\herdr on Windows), as Herdr itself resolves it",
    },
    EnvKey {
        key: APPDATA,
        required: false,
        format: "Windows only: absolute directory path",
        absent_behavior: "The Herdr session a move into Hide's engine reads on Windows is under %USERPROFILE%\\AppData\\Roaming\\herdr",
    },
    EnvKey {
        key: HIDE_STATE_DIR,
        required: false,
        format: "absolute directory path",
        absent_behavior: "State lives under $XDG_STATE_HOME/hide when that is set, otherwise $HOME/.hide/state, where `hide connect` moves a legacy $HOME/.local/state/hide once",
    },
    EnvKey {
        key: HIDE_KEEP_ALIVE,
        required: false,
        format: "1 or true",
        absent_behavior: "Daemon exits 10 minutes after the last WebSocket client disconnects",
    },
    EnvKey {
        key: HIDE_VITE_ORIGIN,
        required: false,
        format: "http://127.0.0.1:<port>",
        absent_behavior: "Only the daemon origin is allowed on the WebSocket",
    },
    EnvKey {
        key: HIDE_PORT,
        required: false,
        format: "TCP port 1-65535",
        absent_behavior: "Bind 127.0.0.1:0 and write the chosen port to the state file",
    },
    EnvKey {
        key: HIDE_IDLE_SECS,
        required: false,
        format: "positive integer seconds",
        absent_behavior: "Idle timeout is 600 seconds",
    },
    EnvKey {
        key: HIDE_OPEN_COMMAND,
        required: false,
        format: "absolute path of an executable CLI helper whose first argument is the file to open",
        absent_behavior: "The host OS handler opens it (macOS `open`, Windows ShellExecuteW association, Linux `xdg-open`)",
    },
    EnvKey {
        key: HIDE_HOST_HELPER_ROOT,
        required: false,
        format: "absolute path, or a path under the device's home spelled `~/...`, with no `.` or `..` segment",
        absent_behavior: "The device helper installs under ~/.hide/host-helper on each consented device (a consent for the legacy ~/.local/share/hide/host-helper is carried there without asking); a different value is a different consent scope, so every device asks again (isolated verification sets it to a temporary folder)",
    },
    EnvKey {
        key: HIDE_HOST_CLI_DIR,
        required: false,
        format: "absolute path, or a path under the device's home spelled `~/...`, with no `.` or `..` segment",
        absent_behavior: "Each consented device links `hide` in ~/.local/bin to the command installed beside its helper, when that name is free or already Hide's link; a different value is a different consent scope, so every device asks again (isolated verification sets it to a temporary folder so no test writes the account's own ~/.local/bin)",
    },
    EnvKey {
        key: HIDE_PANE_ID,
        required: false,
        format: "the pane id; the engine sets it in every pane it runs",
        absent_behavior: "Pane-scoped Workspace commands refuse because the caller cannot identify a pane",
    },
    EnvKey {
        key: HIDE_CAP_REF,
        required: false,
        format: "absolute path to a regular, owner-only pane credential reference",
        absent_behavior: "Pane callers try kernel peer bootstrap; detached agent tools require a session-scoped reference",
    },
    EnvKey {
        key: HIDE_TAILSCALE_BIN,
        required: false,
        format: "absolute path of the tailscale CLI Settings > Mobile runs; a path that does not exist reads as Tailscale not installed",
        absent_behavior: "The CLI the system's Tailscale app installs (/Applications/Tailscale.app/Contents/MacOS/Tailscale on macOS, %ProgramFiles%\\Tailscale\\tailscale.exe on Windows), then `tailscale` on PATH; isolated verification sets it so no test reaches the account's own Tailscale",
    },
    EnvKey {
        key: HIDE_RELAY_URL,
        required: false,
        format: "WebSocket URL for development or tests; wss, or ws on loopback only; no credentials, query or fragment",
        absent_behavior: "Mobile connects to wss://relay.withhide.dev/ws",
    },
    EnvKey {
        key: HIDE_MACHINE_ID,
        required: false,
        format: "a node id: lowercase letters, digits and '-'; only a test fixture sets it, so one host runs a node and the core it dials as two machines",
        absent_behavior: "The machine id the system reports (IOPlatformUUID on macOS, /etc/machine-id on Linux, MachineGuid on Windows) names this node",
    },
    EnvKey {
        key: HIDE_BUILD_VERSION_OVERRIDE,
        required: false,
        format: "a version such as 0.4.2; read only when HOME is a fixture HOME (under /tmp, carrying .hide-e2e-device-home, and not the account's own)",
        absent_behavior: "A link's hello carries the version this build was packaged with (HIDE_VERSION at build time)",
    },
    EnvKey {
        key: HIDE_BUILD_ORDER_OVERRIDE,
        required: false,
        format: "a commit count; read only when HOME is a fixture HOME (under /tmp, carrying .hide-e2e-device-home, and not the account's own)",
        absent_behavior: "A link's hello carries the build order this build was packaged with (HIDE_BUILD_ORDER at build time, 0 when no package numbered it)",
    },
    EnvKey {
        key: HIDE_CORE_STARTER,
        required: false,
        format: "`fixture`; read only when HOME is a fixture HOME (under /tmp, carrying .hide-e2e-device-home, and not the account's own)",
        absent_behavior: "A core move starts the core on the machine taking it through the account's login item (launchd); a fixture HOME with `fixture` starts it as a detached process instead, and a fixture HOME without it is refused, so no test reaches launchd",
    },
    EnvKey {
        key: HIDE_FIXTURE_CORE_STOP,
        required: false,
        format: "`hang`; read only when HOME is a fixture HOME (under /tmp, carrying .hide-e2e-device-home, and not the account's own)",
        absent_behavior: "The core role stops when asked; a fixture HOME with `hang` makes that stop never return, so the process ends itself past its bound",
    },
    EnvKey {
        key: HIDE_PREFLIGHT_PROGRAMS,
        required: false,
        format: "absolute path of a folder holding stand-in `gh`, `launchctl` and `pmset`; read only when HOME is a fixture HOME (under /tmp, carrying .hide-e2e-device-home, and not the account's own)",
        absent_behavior: "A core move's checks on the machine taking the core run the account's `gh`, /bin/launchctl and /usr/bin/pmset; a fixture HOME names stand-ins, so a test passes or fails each check without the machine's own GitHub login, session and power settings, and a fixture HOME without it is refused rather than reaching the account's launchd",
    },
    EnvKey {
        key: HIDE_DEV_DEVICE_HOME,
        required: false,
        format: "`~/` and a path below the device account's home, or an absolute path on the device (`C:/...` on Windows), with no empty, `.` or `..` name; development only",
        absent_behavior: "Each SSH device's node runs with its account's own home (HOME, or USERPROFILE on Windows); isolated verification sets it, beside HIDE_HOST_HELPER_ROOT, so a candidate's state, sessions and pane holders on a shared device never meet the account's own Hide there",
    },
    EnvKey {
        key: HOME,
        required: true,
        format: "absolute home-directory path",
        absent_behavior: "Boot fails; the state directory cannot be resolved",
    },
];

#[derive(Clone, Debug)]
pub struct Env {
    /// The filesystem boundary root for the web shell's listing and
    /// registration events (`boundary.rs`); read once, never configured.
    pub home: PathBuf,
    pub state_dir: PathBuf,
    /// The folder builds before `~/.hide` kept state in, when `state_dir` is
    /// the default one: `hide connect` moves it there once (PRD
    /// hide-home-layout D-05). `None` when HIDE_STATE_DIR or XDG_STATE_HOME
    /// chose another folder, which is never moved.
    pub legacy_state_dir: Option<PathBuf>,
    /// `state_dir` is the folder this account's hide picks with no
    /// HIDE_STATE_DIR: `~/.hide/state`, or `$XDG_STATE_HOME/hide` where that
    /// is set. Only a core there writes this machine's guardrails policy.
    pub account_state_folder: bool,
    pub keep_alive: bool,
    pub vite_origin: Option<String>,
    pub bind: SocketAddr,
    pub idle_secs: u64,
    /// This daemon's build (`build_id`), set by `hided serve` from its own
    /// executable; never read from the environment. A daemon started in
    /// process (tests) has none and reports none.
    pub build: Option<String>,
    /// The program a core's starter runs, set by `hided core-login` from its
    /// own executable (PRD core-host-node-move Q13); never read from the
    /// environment. `None` for a daemon `hide connect` started.
    pub starter_program: Option<PathBuf>,
    /// Optional test/operator-selected host opener, validated before serving.
    pub open_command: Option<PathBuf>,
    /// Where the device helper is installed on each SSH device; part of the
    /// consent scope the operator agrees to (PRD S5.5 D-23).
    pub host_helper_root: Option<String>,
    /// Where each SSH device links its `hide` command; part of the same
    /// consent scope.
    pub host_cli_dir: Option<String>,
    /// The pane a Workspace CLI command runs in; the daemon verifies its
    /// live membership and never trusts a caller-supplied Workspace.
    pub pane_id: Option<String>,
    /// The only tailscale CLI Mobile runs, when set (`HIDE_TAILSCALE_BIN`).
    pub tailscale_bin: Option<PathBuf>,
    pub relay_url: String,
    /// PATH as the daemon received it, searched for `tailscale` at each check.
    pub search_path: Option<String>,
}

#[derive(Debug)]
pub struct EnvError {
    pub key: &'static str,
    pub kind: &'static str,
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.kind)
    }
}

impl std::error::Error for EnvError {}

/// The keys that decide which state folder a command acts on: `HOME` (the
/// default folder is under it), `HIDE_STATE_DIR` and `XDG_STATE_HOME`.
/// `hide stop` and `hide status --json` read nothing else.
pub const STATE_FOLDER_KEYS: &[&str] = &[HOME, HIDE_STATE_DIR, XDG_STATE_HOME];

/// What `hide status` reads: the state folder, and `HIDE_IDLE_SECS`, which it
/// prints as the idle time when the daemon's `/health` gives none.
pub const STATUS_KEYS: &[&str] = &[HOME, HIDE_STATE_DIR, XDG_STATE_HOME, HIDE_IDLE_SECS];

/// Every key, checked: the daemon and every command that acts on its whole
/// configuration refuse on any invalid key, all reported at once.
pub fn load() -> Result<Env, Vec<EnvError>> {
    load_from(|key| std::env::var(key).ok())
}

/// Only `keys` are checked, for a command that reads nothing else (the sets
/// above). A key outside the set is not looked at, so the command must read
/// only the fields those keys fill; an invalid key inside the set refuses,
/// named, and is never replaced by its default.
pub fn load_for(keys: &[&str]) -> Result<Env, Vec<EnvError>> {
    load_for_from(keys, |key| std::env::var(key).ok())
}

pub fn load_for_from(
    keys: &[&str],
    read: impl FnMut(&str) -> Option<String>,
) -> Result<Env, Vec<EnvError>> {
    let (env, errors) = resolve(read);
    let errors: Vec<EnvError> = errors
        .into_iter()
        .filter(|error| keys.contains(&error.key))
        .collect();
    if errors.is_empty() {
        Ok(env)
    } else {
        Err(errors)
    }
}

pub fn load_from(read: impl FnMut(&str) -> Option<String>) -> Result<Env, Vec<EnvError>> {
    let (env, errors) = resolve(read);
    if errors.is_empty() {
        Ok(env)
    } else {
        Err(errors)
    }
}

/// Reads every key once: the environment, and what was wrong with each key
/// that was invalid (whose field then holds a placeholder no caller may use).
fn resolve(mut read: impl FnMut(&str) -> Option<String>) -> (Env, Vec<EnvError>) {
    let mut errors = Vec::new();
    let home = match read(HOME) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            errors.push(EnvError {
                key: HOME,
                kind: "missing",
            });
            PathBuf::new()
        }
    };
    let hide_state_dir = read(HIDE_STATE_DIR);
    if hide_state_dir.as_deref() == Some("") {
        errors.push(EnvError {
            key: HIDE_STATE_DIR,
            kind: "empty",
        });
    }
    let xdg_state_home = read(XDG_STATE_HOME);
    let state_dir =
        hide_kit::layout::state_dir(&home, hide_state_dir.as_deref(), xdg_state_home.as_deref());
    let account_state_folder =
        state_dir == hide_kit::layout::state_dir(&home, None, xdg_state_home.as_deref());
    // Judged by the folder, not by which variable named it: `hide connect`
    // hands the daemon it starts the default folder through HIDE_STATE_DIR.
    // Only a Mac ever had the old folder.
    let legacy_state_dir = (cfg!(unix) && state_dir == hide_kit::layout::default_state_dir(&home))
        .then(|| hide_kit::layout::legacy_state_dir(&home));
    let keep_alive = match read(HIDE_KEEP_ALIVE).as_deref() {
        None => false,
        Some("1" | "true" | "TRUE") => true,
        Some(_) => {
            errors.push(EnvError {
                key: HIDE_KEEP_ALIVE,
                kind: "invalid",
            });
            false
        }
    };
    let vite_origin = match read(HIDE_VITE_ORIGIN) {
        Some(value) if value.is_empty() => {
            errors.push(EnvError {
                key: HIDE_VITE_ORIGIN,
                kind: "empty",
            });
            None
        }
        Some(value)
            if !(value.starts_with("http://127.0.0.1:")
                || value.starts_with("http://localhost:")) =>
        {
            errors.push(EnvError {
                key: HIDE_VITE_ORIGIN,
                kind: "invalid",
            });
            None
        }
        other => other,
    };
    let port = match read(HIDE_PORT) {
        None => 0_u16,
        Some(value) => match value.parse::<u16>() {
            Ok(port) => port,
            Err(_) => {
                errors.push(EnvError {
                    key: HIDE_PORT,
                    kind: "invalid",
                });
                0
            }
        },
    };
    let idle_secs = match read(HIDE_IDLE_SECS) {
        None => 600,
        Some(value) => match value.parse::<u64>() {
            Ok(secs) if secs > 0 => secs,
            _ => {
                errors.push(EnvError {
                    key: HIDE_IDLE_SECS,
                    kind: "invalid",
                });
                600
            }
        },
    };
    let open_command = match read(HIDE_OPEN_COMMAND) {
        Some(value) if !valid_program_path(Path::new(&value)) => {
            errors.push(EnvError {
                key: HIDE_OPEN_COMMAND,
                kind: "invalid",
            });
            None
        }
        Some(value) => Some(PathBuf::from(value)),
        None => None,
    };
    let host_helper_root = match read(HIDE_HOST_HELPER_ROOT) {
        Some(value) if !valid_helper_root(&value) => {
            errors.push(EnvError {
                key: HIDE_HOST_HELPER_ROOT,
                kind: "invalid",
            });
            None
        }
        other => other,
    };
    if read(HIDE_DEV_DEVICE_HOME)
        .is_some_and(|value| hide_node::ssh::parse_device_home(&value).is_err())
    {
        errors.push(EnvError {
            key: HIDE_DEV_DEVICE_HOME,
            kind: "invalid",
        });
    }
    let host_cli_dir = match read(HIDE_HOST_CLI_DIR) {
        Some(value) if !valid_helper_root(&value) => {
            errors.push(EnvError {
                key: HIDE_HOST_CLI_DIR,
                kind: "invalid",
            });
            None
        }
        other => other,
    };
    let pane_id = match read(HIDE_PANE_ID) {
        Some(value) if value.is_empty() => {
            errors.push(EnvError {
                key: HIDE_PANE_ID,
                kind: "empty",
            });
            None
        }
        other => other,
    };
    let tailscale_bin = match read(HIDE_TAILSCALE_BIN) {
        Some(value)
            if !Path::new(&value).is_absolute() || value.bytes().any(|b| b.is_ascii_control()) =>
        {
            errors.push(EnvError {
                key: HIDE_TAILSCALE_BIN,
                kind: "invalid",
            });
            None
        }
        Some(value) => Some(PathBuf::from(value)),
        None => None,
    };
    let relay_url = match read(HIDE_RELAY_URL) {
        Some(value) if valid_relay_url(&value) => value,
        Some(_) => {
            errors.push(EnvError {
                key: HIDE_RELAY_URL,
                kind: "invalid",
            });
            String::new()
        }
        None => RELAY_URL.to_owned(),
    };
    let search_path = read(PATH).filter(|value| !value.is_empty());
    let env = Env {
        home,
        state_dir,
        legacy_state_dir,
        account_state_folder,
        keep_alive,
        vite_origin,
        bind: SocketAddr::from(([127, 0, 0, 1], port)),
        idle_secs,
        build: None,
        starter_program: None,
        open_command,
        host_helper_root,
        host_cli_dir,
        pane_id,
        tailscale_bin,
        relay_url,
        search_path,
    };
    (env, errors)
}

fn valid_relay_url(value: &str) -> bool {
    let Ok(uri) = value.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(host) = uri.host() else {
        return false;
    };
    !value.contains(['@', '#'])
        && uri.query().is_none()
        && (uri.scheme_str() == Some("wss")
            || (uri.scheme_str() == Some("ws")
                && matches!(host, "localhost" | "127.0.0.1" | "[::1]")))
}

/// A helper root names a folder on another machine, so it is checked for
/// shape only: absolute or under that machine's home, one line, and no
/// segment that could climb out of what the operator agreed to.
fn valid_helper_root(value: &str) -> bool {
    let rest = if let Some(rest) = value.strip_prefix("~/") {
        rest
    } else if let Some(rest) = value.strip_prefix('/') {
        rest
    } else {
        return false;
    };
    !rest.is_empty()
        && !value.chars().any(char::is_control)
        && rest
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn valid_program_path(path: &Path) -> bool {
    if !path.is_absolute() || !path.is_file() {
        return false;
    }
    // What Windows starts as a program: an executable, or a batch file that
    // runs through `cmd.exe`.
    #[cfg(windows)]
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["exe", "com", "bat", "cmd"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path
            .metadata()
            .map_or(true, |meta| meta.permissions().mode() & 0o111 == 0)
        {
            return false;
        }
    }
    true
}

/// Whether `home` is a test fixture's HOME, where a test-only key is read:
/// under the system's temporary folder, carrying [`FIXTURE_HOME_MARKER`],
/// and not this account's own home by the account database (`HOME` is the
/// fixture's, so it cannot tell). Anything else reads every test-only key as
/// unset.
pub fn fixture_home(home: &std::path::Path) -> bool {
    hide_kit::layout::fixture_home(home)
}

/// The version and order a fixture has this build present
/// ([`HIDE_BUILD_VERSION_OVERRIDE`], [`HIDE_BUILD_ORDER_OVERRIDE`]); each
/// is read only under a fixture HOME.
pub struct BuildOverrides {
    pub version: Option<String>,
    pub order: Option<u64>,
}

pub fn fixture_build_overrides() -> BuildOverrides {
    if !hide_platform::host::home_dir().is_ok_and(|home| fixture_home(&home)) {
        return BuildOverrides {
            version: None,
            order: None,
        };
    }
    BuildOverrides {
        version: std::env::var(HIDE_BUILD_VERSION_OVERRIDE)
            .ok()
            .filter(|version| !version.is_empty()),
        // Only a fixture sets it, and one that misspells it must not test
        // the build it did not mean.
        order: std::env::var(HIDE_BUILD_ORDER_OVERRIDE).ok().map(|order| {
            order
                .parse()
                .unwrap_or_else(|_| panic!("{HIDE_BUILD_ORDER_OVERRIDE} is not a count: {order:?}"))
        }),
    }
}

/// Whether a fixture HOME names `build` as one whose daemon fails at its
/// start ([`FIXTURE_FAIL_BUILD_FILE`]).
pub fn fixture_fails_build(build: &str) -> bool {
    hide_platform::host::home_dir().is_ok_and(|home| {
        fixture_home(&home)
            && std::fs::read_to_string(home.join(FIXTURE_FAIL_BUILD_FILE))
                .is_ok_and(|named| named.trim() == build)
    })
}

/// Whether a fixture asked the core role's stop never to return
/// ([`HIDE_FIXTURE_CORE_STOP`]).
pub fn fixture_core_stop_hangs() -> bool {
    std::env::var_os(HIDE_FIXTURE_CORE_STOP).as_deref() == Some(std::ffi::OsStr::new("hang"))
        && hide_platform::host::home_dir().is_ok_and(|home| fixture_home(&home))
}

/// The home every SSH device's node runs with ([`HIDE_DEV_DEVICE_HOME`]),
/// `None` when it is not set; a value it does not take is an error saying
/// what it takes.
pub(crate) fn device_home() -> Result<Option<String>, String> {
    match std::env::var_os(HIDE_DEV_DEVICE_HOME) {
        None => Ok(None),
        Some(value) => {
            let value = value
                .into_string()
                .map_err(|_| format!("{HIDE_DEV_DEVICE_HOME} is not Unicode"))?;
            hide_node::ssh::parse_device_home(&value).map(Some)
        }
    }
}

/// Public ingestion token embedded by build.rs; runtime cannot replace it.
pub(crate) fn product_analytics_token() -> Option<String> {
    let token = env!("HIDE_POSTHOG_PROJECT_TOKEN");
    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn from_map(pairs: &[(&str, &str)]) -> Result<Env, Vec<EnvError>> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        load_from(move |key| map.get(key).cloned())
    }

    fn scoped(keys: &[&str], pairs: &[(&str, &str)]) -> Result<Env, Vec<EnvError>> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        load_for_from(keys, move |key| map.get(key).cloned())
    }

    /// An invalid value for every key outside the state folder.
    const UNRELATED_INVALID: &[(&str, &str)] = &[
        (HIDE_PORT, "abc"),
        (HIDE_OPEN_COMMAND, "relative-opener"),
        (HIDE_KEEP_ALIVE, "maybe"),
        (HIDE_VITE_ORIGIN, "http://example.com:1"),
        (HIDE_PANE_ID, ""),
        (HIDE_HOST_HELPER_ROOT, "relative"),
        (HIDE_TAILSCALE_BIN, "relative"),
        (HIDE_DEV_DEVICE_HOME, "relative/home"),
    ];

    #[test]
    fn the_state_folder_is_all_stop_and_status_json_ask_for() {
        let mut pairs = vec![(HOME, "/Users/example"), (HIDE_IDLE_SECS, "0")];
        pairs.extend_from_slice(UNRELATED_INVALID);
        assert!(from_map(&pairs).is_err(), "the whole set is invalid");
        let env = scoped(STATE_FOLDER_KEYS, &pairs).unwrap();
        assert_eq!(env.state_dir, PathBuf::from("/Users/example/.hide/state"));
    }

    #[test]
    fn an_invalid_state_folder_key_is_refused_by_name_not_defaulted() {
        let err = scoped(
            STATE_FOLDER_KEYS,
            &[(HOME, "/Users/example"), (HIDE_STATE_DIR, "")],
        )
        .unwrap_err();
        assert_eq!(
            err.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [format!("{HIDE_STATE_DIR}: empty")]
        );
        let err = scoped(STATE_FOLDER_KEYS, UNRELATED_INVALID).unwrap_err();
        assert_eq!(
            err.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [format!("{HOME}: missing")],
            "only the key the command reads is named"
        );
    }

    #[test]
    fn status_reads_the_idle_time_as_well() {
        let mut pairs = vec![(HOME, "/Users/example")];
        pairs.extend_from_slice(UNRELATED_INVALID);
        let env = scoped(STATUS_KEYS, &pairs).unwrap();
        assert_eq!(env.idle_secs, 600);
        pairs.push((HIDE_IDLE_SECS, "0"));
        let err = scoped(STATUS_KEYS, &pairs).unwrap_err();
        assert_eq!(
            err.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [format!("{HIDE_IDLE_SECS}: invalid")]
        );
    }

    #[test]
    fn a_command_set_names_only_registered_keys() {
        for key in STATE_FOLDER_KEYS.iter().chain(STATUS_KEYS) {
            assert!(
                REGISTRY.iter().any(|entry| entry.key == *key),
                "{key} is not in REGISTRY"
            );
        }
    }

    #[test]
    fn the_daemon_check_still_reports_every_invalid_key_at_once() {
        let mut pairs = vec![(HOME, "/Users/example")];
        pairs.extend_from_slice(UNRELATED_INVALID);
        let err = from_map(&pairs).unwrap_err();
        for (key, _) in UNRELATED_INVALID {
            assert!(
                err.iter().any(|error| error.key == *key),
                "{key} unreported"
            );
        }
    }

    #[test]
    fn missing_home_fails_boot() {
        let err = from_map(&[]).unwrap_err();
        assert_eq!(err[0].key, HOME);
        assert_eq!(err[0].kind, "missing");
    }

    #[test]
    fn defaults_state_dir_under_home() {
        let env = from_map(&[(HOME, "/Users/example")]).unwrap();
        assert_eq!(env.state_dir, PathBuf::from("/Users/example/.hide/state"));
        assert_eq!(
            env.legacy_state_dir.as_deref(),
            cfg!(unix).then_some(Path::new("/Users/example/.local/state/hide"))
        );
        for relocated in [
            ("HIDE_STATE_DIR", "/isolated/state"),
            ("XDG_STATE_HOME", "/xdg"),
        ] {
            let env = from_map(&[("HOME", "/Users/example"), relocated]).unwrap();
            assert_eq!(env.legacy_state_dir, None, "{relocated:?}");
        }
        // The account's own folder, named or not, writes the hooks' policy;
        // a relocated one does not.
        for (vars, own) in [
            (&[("HOME", "/Users/example")][..], true),
            (
                &[("HOME", "/Users/example"), ("XDG_STATE_HOME", "/xdg")][..],
                true,
            ),
            (
                &[
                    ("HOME", "/Users/example"),
                    ("HIDE_STATE_DIR", "/Users/example/.hide/state"),
                ][..],
                true,
            ),
            (
                &[
                    ("HOME", "/Users/example"),
                    ("XDG_STATE_HOME", "/xdg"),
                    ("HIDE_STATE_DIR", "/xdg/hide"),
                ][..],
                true,
            ),
            (
                &[
                    ("HOME", "/Users/example"),
                    ("HIDE_STATE_DIR", "/isolated/state"),
                ][..],
                false,
            ),
            (
                &[
                    ("HOME", "/Users/example"),
                    ("XDG_STATE_HOME", "/xdg"),
                    ("HIDE_STATE_DIR", "/Users/example/.hide/state"),
                ][..],
                false,
            ),
        ] {
            assert_eq!(
                from_map(vars).unwrap().account_state_folder,
                own,
                "{vars:?}"
            );
        }
        assert_eq!(
            from_map(&[("HOME", "/Users/example"), ("XDG_STATE_HOME", "/xdg")])
                .unwrap()
                .state_dir,
            PathBuf::from("/xdg/hide")
        );
        assert_eq!(env.idle_secs, 600);
        assert!(!env.keep_alive);
    }

    #[test]
    fn vite_origin_must_be_loopback() {
        let err = from_map(&[
            (HOME, "/Users/example"),
            ("HIDE_VITE_ORIGIN", "http://example.com"),
        ])
        .unwrap_err();
        assert_eq!(err[0].key, HIDE_VITE_ORIGIN);
    }

    #[test]
    fn helper_root_is_absolute_or_under_home_and_never_climbs() {
        for bad in [
            "relative/dir",
            "~",
            "~/",
            "/",
            "/a/../b",
            "~/a/./b",
            "/a//b",
            "/a\nb",
        ] {
            let err =
                from_map(&[(HOME, "/Users/example"), (HIDE_HOST_HELPER_ROOT, bad)]).unwrap_err();
            assert_eq!(err[0].key, HIDE_HOST_HELPER_ROOT, "{bad}");
        }
        for good in ["~/.cache/hide-test/helper", "/tmp/hide-verify/helper"] {
            let env = from_map(&[(HOME, "/Users/example"), (HIDE_HOST_HELPER_ROOT, good)]).unwrap();
            assert_eq!(env.host_helper_root.as_deref(), Some(good));
            let env = from_map(&[(HOME, "/Users/example"), (HIDE_HOST_CLI_DIR, good)]).unwrap();
            assert_eq!(env.host_cli_dir.as_deref(), Some(good));
        }
        let err = from_map(&[(HOME, "/Users/example"), (HIDE_HOST_CLI_DIR, "bin")]).unwrap_err();
        assert_eq!(err[0].key, HIDE_HOST_CLI_DIR);
        assert_eq!(
            from_map(&[(HOME, "/Users/example")])
                .unwrap()
                .host_helper_root,
            None
        );
    }

    #[test]
    fn relay_override_requires_tls_except_on_loopback() {
        for value in [
            "ws://example.test/ws",
            "https://example.test/ws",
            "wss://user@example.test/ws",
            "wss://example.test/ws?token=secret",
            "wss://example.test/#secret",
        ] {
            assert_eq!(
                from_map(&[(HOME, "/Users/example"), (HIDE_RELAY_URL, value)]).unwrap_err()[0].key,
                HIDE_RELAY_URL
            );
        }
        for value in [
            "wss://relay.example.test/ws",
            "ws://127.0.0.1:8787/ws",
            "ws://[::1]:8787/ws",
        ] {
            assert_eq!(
                from_map(&[(HOME, "/Users/example"), (HIDE_RELAY_URL, value)])
                    .unwrap()
                    .relay_url,
                value
            );
        }
        assert_eq!(
            from_map(&[(HOME, "/Users/example")]).unwrap().relay_url,
            RELAY_URL
        );
    }

    #[test]
    fn open_command_is_validated_at_boot() {
        let err = from_map(&[
            (HOME, "/Users/example"),
            (HIDE_OPEN_COMMAND, "relative-opener"),
        ])
        .unwrap_err();
        assert_eq!(err[0].key, HIDE_OPEN_COMMAND);
        let executable = std::env::current_exe().unwrap();
        let env = from_map(&[
            (HOME, "/Users/example"),
            (HIDE_OPEN_COMMAND, executable.to_str().unwrap()),
        ])
        .unwrap();
        assert_eq!(env.open_command.as_deref(), Some(executable.as_path()));
        // A file the system cannot start as a program: no execute bit on
        // Unix, no program extension on Windows.
        let directory = tempfile::tempdir().unwrap();
        let text = directory.path().join("opener.txt");
        std::fs::write(&text, "not a program").unwrap();
        let err = from_map(&[
            (HOME, "/Users/example"),
            (HIDE_OPEN_COMMAND, text.to_str().unwrap()),
        ])
        .unwrap_err();
        assert_eq!(err[0].key, HIDE_OPEN_COMMAND);
    }
}
