//! The agents Desk can talk to and how each is held in its ask mode (PRD
//! Desk D-17, D-53).
//!
//! Each is its vendor's ACP adapter, an npm package pinned with every
//! dependency's integrity in `contracts/desk-adapters/<id>/`, run by the
//! account's Node. None reads the operator's automatic approvals:
//! - Claude starts with no settings sources (the operator's allow rules,
//!   default mode and hooks are not read; the sign-in lives in the system
//!   keychain, not in settings), bypass refused, every Bash call held by an
//!   ask rule, and the session put in `default` mode before any prompt.
//! - Codex starts its session in `read-only`, where its sandbox lets reads
//!   run and asks before every write, network use or command outside it.
//! - Pi asks for nothing on its own, so Desk loads its own extension
//!   (`pi_desk.js`) into the Pi process the adapter starts: every tool call
//!   that is not a read waits for Hide. The extension writes a marker naming
//!   its process when it loads, which is Hide's proof the gate is there.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterId {
    Claude,
    Codex,
    Pi,
}

/// The confirm title Desk's Pi extension sends, which Hide reads as its own.
pub const PI_CONFIRM_TITLE: &str = "hide-desk.permission";
/// Desk's Pi extension.
pub const PI_EXTENSION: &str = include_str!("pi_desk.js");
/// What Pi loads from the operator's own setup and Desk keeps out: skills,
/// `AGENTS.md`/`CLAUDE.md`, discovered extensions, prompt templates and MCP
/// servers. Desk's extension, named with `-e`, still loads.
const PI_ISOLATION: &str =
    "--no-skills --no-context-files --no-extensions --no-prompt-templates --no-mcp";

/// One adapter's pinned package.
pub struct Pin {
    pub package: &'static str,
    pub version: &'static str,
    /// The program's script inside the package folder.
    pub entry: &'static str,
    /// The package.json and package-lock.json `npm ci` installs from.
    pub manifest: &'static str,
    pub lock: &'static str,
}

/// How an adapter's session is put in its ask mode once it exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AskMode {
    /// `session/set_mode` to this mode, and the answer must not be an error.
    SetMode(&'static str),
    /// Desk's Pi extension, proven by its marker.
    PiExtension,
}

impl AdapterId {
    pub const ALL: [AdapterId; 3] = [AdapterId::Claude, AdapterId::Codex, AdapterId::Pi];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|adapter| adapter.as_str() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
        }
    }

    pub fn pin(self) -> Pin {
        match self {
            Self::Claude => Pin {
                package: "@agentclientprotocol/claude-agent-acp",
                version: "0.99.1",
                entry: "dist/index.js",
                manifest: include_str!("../../contracts/desk-adapters/claude/package.json"),
                lock: include_str!("../../contracts/desk-adapters/claude/package-lock.json"),
            },
            Self::Codex => Pin {
                package: "@agentclientprotocol/codex-acp",
                version: "2.5.0",
                entry: "dist/index.js",
                manifest: include_str!("../../contracts/desk-adapters/codex/package.json"),
                lock: include_str!("../../contracts/desk-adapters/codex/package-lock.json"),
            },
            Self::Pi => Pin {
                package: "pi-acp",
                version: "0.0.34",
                entry: "dist/index.js",
                manifest: include_str!("../../contracts/desk-adapters/pi/package.json"),
                lock: include_str!("../../contracts/desk-adapters/pi/package-lock.json"),
            },
        }
    }

    pub fn ask_mode(self) -> AskMode {
        match self {
            Self::Claude => AskMode::SetMode("default"),
            Self::Codex => AskMode::SetMode("read-only"),
            Self::Pi => AskMode::PiExtension,
        }
    }

    /// The agent's own CLI the adapter starts, which the operator installs;
    /// Claude's and Codex's adapters carry theirs.
    pub fn own_cli(self) -> Option<&'static str> {
        match self {
            Self::Pi => Some("pi"),
            Self::Claude | Self::Codex => None,
        }
    }

    /// What the operator runs in a terminal to sign in (B6).
    pub fn login_command(self) -> &'static str {
        match self {
            Self::Claude => "claude /login",
            Self::Codex => "codex login",
            Self::Pi => "pi /login",
        }
    }

    /// `_meta` for `session/new` and `session/load`.
    pub fn session_meta(self) -> Option<Value> {
        match self {
            Self::Claude => Some(json!({"claudeCode": {"options": {
                "settingSources": [],
                "allowDangerouslySkipPermissions": false,
                "settings": {"permissions": {"ask": ["Bash"]}},
            }}})),
            Self::Codex | Self::Pi => None,
        }
    }
}

/// Where the pieces of one Desk process live.
pub struct Paths<'a> {
    /// The adapter's installed package folder (`node_modules/<package>`).
    pub package: &'a Path,
    /// Node, from the account's `PATH`.
    pub node: &'a Path,
    /// The folder `hide` is in, put first on the process's `PATH`.
    pub hide_dir: &'a Path,
    /// The account's `PATH` as the login shell gives it.
    pub search: Option<&'a OsString>,
    /// Desk's own folder for this process: its working folder and Pi's
    /// launcher, extension and marker.
    pub desk: &'a Path,
    /// The Workspace capability reference the `hide` CLI presents.
    pub capability: &'a Path,
    /// The agent's own CLI when the adapter needs one ([`AdapterId::own_cli`]).
    pub own_cli: Option<&'a Path>,
}

/// The program, arguments and environment one Desk process starts with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Variables set on top of the login variables.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
    /// Files to write before the start (Pi's launcher and extension).
    pub files: Vec<(PathBuf, String, bool)>,
    /// The marker Pi's extension writes when it loads.
    pub pi_marker: Option<PathBuf>,
}

/// Whether Pi's Desk extension loaded in the Desk process's tree: its
/// marker names a pid under `host`, the process Desk started.
pub fn pi_extension_loaded(marker: Option<&Path>, host: u32) -> Result<(), String> {
    let marker = marker.ok_or("no marker was asked for")?;
    let text = std::fs::read_to_string(marker)
        .map_err(|error| format!("the extension's marker is missing: {error}"))?;
    let pid: u32 = text
        .trim()
        .parse()
        .map_err(|_| "the extension's marker names no process".to_owned())?;
    if hide_platform::process::descends_from(pid, host) {
        Ok(())
    } else {
        Err("the extension loaded in another process".into())
    }
}

/// Desk's working folder: no checkout, so no project's settings or
/// instructions load into the Desk agent.
pub fn work_dir(desk: &Path) -> PathBuf {
    desk.join("work")
}

pub fn launch(adapter: AdapterId, paths: &Paths<'_>, nonce: &str) -> Launch {
    let mut search = OsString::from(paths.hide_dir.as_os_str());
    if let Some(account) = paths.search.filter(|path| !path.is_empty()) {
        search.push(if cfg!(windows) { ";" } else { ":" });
        search.push(account);
    }
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), search),
        ("HIDE_CAP_REF".into(), paths.capability.into()),
        ("HIDE_DESK".into(), "1".into()),
    ];
    let mut files = Vec::new();
    let mut pi_marker = None;
    match adapter {
        AdapterId::Claude => {}
        AdapterId::Codex => env.push(("INITIAL_AGENT_MODE".into(), "read-only".into())),
        AdapterId::Pi => {
            let extension = paths.desk.join("pi-desk.js");
            let marker = paths.desk.join(format!("pi-ready-{nonce}"));
            let (launcher, body) = if cfg!(windows) {
                (
                    paths.desk.join("pi-desk.cmd"),
                    format!(
                        "@\"%HIDE_DESK_PI%\" -e \"%HIDE_DESK_PI_EXTENSION%\" {PI_ISOLATION} %*\r\n"
                    ),
                )
            } else {
                (
                    paths.desk.join("pi-desk"),
                    format!(
                        "#!/bin/sh\nexec \"$HIDE_DESK_PI\" -e \"$HIDE_DESK_PI_EXTENSION\" {PI_ISOLATION} \"$@\"\n"
                    ),
                )
            };
            files.push((extension.clone(), PI_EXTENSION.to_owned(), false));
            files.push((launcher.clone(), body, true));
            env.push(("PI_ACP_PI_COMMAND".into(), launcher.into()));
            env.push(("HIDE_DESK_PI_EXTENSION".into(), extension.into()));
            env.push(("HIDE_DESK_PI_READY".into(), marker.clone().into()));
            if let Some(pi) = paths.own_cli {
                env.push(("HIDE_DESK_PI".into(), pi.into()));
            }
            pi_marker = Some(marker);
        }
    }
    Launch {
        program: paths.node.to_path_buf(),
        args: vec![paths.package.join(adapter.pin().entry).into()],
        env,
        cwd: work_dir(paths.desk),
        files,
        pi_marker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pin_names_its_package_at_the_locked_version_and_integrity() {
        for adapter in AdapterId::ALL {
            let pin = adapter.pin();
            let manifest: Value = serde_json::from_str(pin.manifest).unwrap();
            assert_eq!(
                manifest["dependencies"][pin.package], pin.version,
                "{adapter:?}"
            );
            let lock: Value = serde_json::from_str(pin.lock).unwrap();
            let entry = &lock["packages"][format!("node_modules/{}", pin.package)];
            assert_eq!(entry["version"], pin.version, "{adapter:?}");
            assert!(
                entry["integrity"].as_str().unwrap().starts_with("sha512-"),
                "{adapter:?}"
            );
            // Every package npm installs is pinned by its integrity.
            for (path, package) in lock["packages"].as_object().unwrap() {
                if !path.is_empty() && package.get("link").is_none() {
                    assert!(package.get("integrity").is_some(), "{adapter:?} {path}");
                }
            }
        }
    }

    #[test]
    fn claude_starts_without_the_operator_s_settings_and_with_bash_held() {
        let meta = AdapterId::Claude.session_meta().unwrap();
        let options = &meta["claudeCode"]["options"];
        assert_eq!(options["settingSources"], json!([]));
        assert_eq!(options["allowDangerouslySkipPermissions"], json!(false));
        assert_eq!(options["settings"]["permissions"]["ask"], json!(["Bash"]));
    }

    #[test]
    fn hide_comes_first_on_the_path_and_no_pane_is_named() {
        let desk = Path::new("/state/desk");
        let account = OsString::from("/usr/bin:/bin");
        let launch = launch(
            AdapterId::Pi,
            &Paths {
                package: Path::new("/a/node_modules/pi-acp"),
                node: Path::new("/usr/local/bin/node"),
                hide_dir: Path::new("/Applications/hide.app/Contents/Resources"),
                search: Some(&account),
                desk,
                capability: Path::new("/state/pane-capabilities/desk.json"),
                own_cli: Some(Path::new("/usr/local/bin/pi")),
            },
            "n1",
        );
        let value = |key: &str| {
            launch
                .env
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            value("PATH").unwrap(),
            OsString::from(format!(
                "/Applications/hide.app/Contents/Resources{separator}/usr/bin:/bin"
            ))
        );
        assert!(value("HERDR_PANE_ID").is_none());
        assert_eq!(launch.cwd, desk.join("work"));
        assert_eq!(launch.pi_marker, Some(desk.join("pi-ready-n1")));
        assert_eq!(
            launch.args,
            vec![OsString::from(
                Path::new("/a/node_modules/pi-acp").join("dist/index.js")
            )]
        );
    }
}
