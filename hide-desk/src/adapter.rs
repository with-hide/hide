//! The agents Desk can talk to and how each is held in its ask mode (PRD
//! Desk D-53, D-79).
//!
//! The list is Hide's catalog (`hide_agent_adapter::ADAPTERS`, the agents
//! Hide can start), each named by its Herdr name, so an agent the catalog
//! gains is Desk's once its ACP route and ask mode are declared here; the
//! match below is exhaustive, so a new agent cannot be added without that
//! decision. Each speaks ACP one of two ways:
//! - its vendor's adapter, an npm package pinned with every dependency's
//!   integrity in `contracts/desk-adapters/<id>/` and run by the account's
//!   Node (Claude Code, Codex, Pi), installed when Desk is turned on;
//! - its own CLI's ACP mode (`opencode acp`, `omp --mode acp`, `agy acp`,
//!   `grok agent stdio`), which needs nothing installed.
//!
//! None reads the operator's automatic approvals:
//! - Claude starts with no settings sources (the operator's allow rules,
//!   default mode and hooks are not read; the sign-in lives in the system
//!   keychain, not in settings), bypass refused, every Bash call held by an
//!   ask rule, and the session put in `default` mode before any prompt.
//! - Codex starts its session in `read-only`, where its sandbox lets reads
//!   run and asks before every write, network use or command outside it,
//!   with Desk's own `CODEX_HOME` ([`crate::codex`]): none of the operator's
//!   Codex setup, the operator's own sign-in linked in, and the `hide`
//!   commands Desk frees allowed out of the sandbox.
//! - Pi asks for nothing on its own, so Desk loads its own extension
//!   (`pi_desk.js`) into the Pi process the adapter starts: every tool call
//!   that is not a read waits for Hide. The extension writes a marker naming
//!   its process when it loads, which is Hide's proof the gate is there.
//! - OpenCode reads an inline configuration above the operator's own
//!   (`OPENCODE_CONFIG_CONTENT`), which Desk sets to ask before every tool
//!   but reading and searching files; measured on OpenCode 2.0.29 over an
//!   operator configuration that allows everything.
//! - omp runs with `--approval-mode always-ask`, which approves read-only
//!   tools alone, and without discovered extensions, skills or rules;
//!   measured on omp 18.7.0.
//! - Antigravity's ACP server asks before every command and change on its
//!   own; measured on agy 1.4.12.
//! - Grok has none Desk can hold: its ACP mode starts in the automatic mode
//!   the operator's own setup chooses, where Grok's model approves commands
//!   itself (measured on Grok 1.0.70: a project's `ask` rules, its default
//!   mode variable and an agent profile left it running a command unasked),
//!   and only a Grok home of Desk's own changes that, which would leave
//!   Grok's sign-in apart from the operator's (D-81).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hide_agent_adapter::{ADAPTERS, AgentAdapter, AgentId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

/// One of the catalog's agents Desk speaks ACP with, named on the wire and in
/// Desk's store by its Herdr name (`claude`, `codex`, `pi`, `opencode`,
/// `omp`, `agy`, `grok`).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AdapterId(AgentId);

/// The confirm title Desk's Pi extension sends, which Hide reads as its own.
pub const PI_CONFIRM_TITLE: &str = "hide-desk.permission";
/// Desk's Pi extension.
pub const PI_EXTENSION: &str = include_str!("pi_desk.js");
/// What Pi loads from the operator's own setup and Desk keeps out: skills,
/// `AGENTS.md`/`CLAUDE.md`, discovered extensions, prompt templates and MCP
/// servers. Desk's extension, named with `-e`, still loads.
const PI_ISOLATION: &str =
    "--no-skills --no-context-files --no-extensions --no-prompt-templates --no-mcp";

/// OpenCode's permissions in Desk: every tool asks but reading, listing and
/// searching files, whatever the operator's own configuration allows.
const OPENCODE_PERMISSIONS: &str = r#"{"permission":{"*":"ask","read":"allow","list":"allow","glob":"allow","grep":"allow"}}"#;

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

/// How an agent speaks ACP.
pub enum Route {
    /// Its vendor's adapter, a pinned npm package Hide installs.
    Package(Pin),
    /// Its own CLI (the catalog's executable), with these arguments.
    Native(&'static [&'static str]),
}

/// How an adapter's session is put in its ask mode once it exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AskMode {
    /// `session/set_mode` to this mode, and the answer must not be an error.
    SetMode(&'static str),
    /// Desk's Pi extension, proven by its marker.
    PiExtension,
    /// The process starts in it: the arguments or configuration Desk starts
    /// it with ([`launch`]).
    AtStart,
}

impl AdapterId {
    pub const CLAUDE: Self = Self(AgentId::ClaudeCode);
    pub const CODEX: Self = Self(AgentId::Codex);
    pub const PI: Self = Self(AgentId::Pi);
    pub const OPENCODE: Self = Self(AgentId::OpenCode);
    pub const OMP: Self = Self(AgentId::Omp);
    pub const ANTIGRAVITY: Self = Self(AgentId::Antigravity);
    pub const GROK: Self = Self(AgentId::Grok);

    /// Every agent of the catalog Desk speaks ACP with, in the catalog's
    /// order.
    pub fn all() -> impl Iterator<Item = AdapterId> {
        ADAPTERS
            .iter()
            .filter(|row| row.start.is_some())
            .filter_map(|row| Self::from_key(row.key))
    }

    /// The agent `key` names, when Desk speaks ACP with it.
    pub fn from_key(key: AgentId) -> Option<Self> {
        route(key).map(|_| Self(key))
    }

    pub fn key(self) -> AgentId {
        self.0
    }

    /// Its row in the catalog.
    pub fn catalog(self) -> &'static AgentAdapter {
        self.0.adapter()
    }

    pub fn as_str(self) -> &'static str {
        self.catalog().herdr.name
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::all().find(|adapter| adapter.as_str() == id)
    }

    pub fn label(self) -> &'static str {
        self.catalog().label
    }

    pub fn route(self) -> Route {
        route(self.0).expect("an AdapterId has a route")
    }

    /// The pinned package Hide installs, for an agent spoken to through its
    /// vendor's adapter.
    pub fn pin(self) -> Option<Pin> {
        match self.route() {
            Route::Package(pin) => Some(pin),
            Route::Native(_) => None,
        }
    }

    /// How the session is held in its ask mode; `None` for an agent Desk
    /// cannot hold in one, which is listed and never started (D-53).
    pub fn ask_mode(self) -> Option<AskMode> {
        match self.0 {
            AgentId::ClaudeCode => Some(AskMode::SetMode("default")),
            AgentId::Codex => Some(AskMode::SetMode("read-only")),
            AgentId::Pi => Some(AskMode::PiExtension),
            AgentId::OpenCode | AgentId::Omp | AgentId::Antigravity => Some(AskMode::AtStart),
            AgentId::Grok | AgentId::Cursor => None,
        }
    }

    /// The agent's own CLI the process starts, which the operator installs:
    /// the program itself for an agent with its own ACP mode, and Pi's,
    /// which its adapter starts. Claude's and Codex's adapters carry theirs.
    pub fn own_cli(self) -> Option<&'static str> {
        match (self.route(), self.0) {
            (Route::Native(_), _) | (Route::Package(_), AgentId::Pi) => {
                self.catalog().executables.first().copied()
            }
            (Route::Package(_), _) => None,
        }
    }

    /// What the operator runs in a terminal to sign in (B6): the agent's own
    /// sign-in, which Desk uses as it is (D-81).
    pub fn login_command(self) -> &'static str {
        match self.0 {
            AgentId::ClaudeCode => "claude /login",
            AgentId::Codex => "codex login",
            AgentId::Pi => "pi /login",
            AgentId::OpenCode => "opencode auth login",
            AgentId::Omp => "omp login",
            AgentId::Antigravity => "agy",
            AgentId::Grok => "grok login",
            AgentId::Cursor => "cursor-agent login",
        }
    }

    /// `_meta` for `session/new` and `session/load`. Claude's own plan mode
    /// is denied: entering it needs no permission and leaves the ask mode,
    /// which ends the session as one that cannot ask (D-53), and leaving it
    /// offers modes that ask less.
    pub fn session_meta(self) -> Option<Value> {
        match self.0 {
            AgentId::ClaudeCode => Some(json!({"claudeCode": {"options": {
                "settingSources": [],
                "allowDangerouslySkipPermissions": false,
                "settings": {"permissions": {"ask": ["Bash"], "deny": ["EnterPlanMode"]}},
            }}})),
            _ => None,
        }
    }
}

/// How the agent `key` speaks ACP, or `None` when Desk does not speak it.
fn route(key: AgentId) -> Option<Route> {
    Some(match key {
        AgentId::ClaudeCode => Route::Package(Pin {
            package: "@agentclientprotocol/claude-agent-acp",
            version: "0.99.1",
            entry: "dist/index.js",
            manifest: include_str!("../../contracts/desk-adapters/claude/package.json"),
            lock: include_str!("../../contracts/desk-adapters/claude/package-lock.json"),
        }),
        AgentId::Codex => Route::Package(Pin {
            package: "@agentclientprotocol/codex-acp",
            version: "2.5.0",
            entry: "dist/index.js",
            manifest: include_str!("../../contracts/desk-adapters/codex/package.json"),
            lock: include_str!("../../contracts/desk-adapters/codex/package-lock.json"),
        }),
        AgentId::Pi => Route::Package(Pin {
            package: "pi-acp",
            version: "0.0.34",
            entry: "dist/index.js",
            manifest: include_str!("../../contracts/desk-adapters/pi/package.json"),
            lock: include_str!("../../contracts/desk-adapters/pi/package-lock.json"),
        }),
        AgentId::OpenCode => Route::Native(&["acp"]),
        AgentId::Omp => Route::Native(&[
            "--mode",
            "acp",
            "--approval-mode",
            "always-ask",
            "--no-extensions",
            "--no-skills",
            "--no-rules",
        ]),
        AgentId::Antigravity => Route::Native(&["acp"]),
        AgentId::Grok => Route::Native(&["agent", "--no-leader", "stdio"]),
        // Retired: Hide starts no Cursor.
        AgentId::Cursor => return None,
    })
}

impl Serialize for AdapterId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AdapterId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::parse(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("no Desk agent is named {name}")))
    }
}

/// Where the pieces of one Desk process live.
pub struct Paths<'a> {
    /// For an agent spoken to through its vendor's adapter: the installed
    /// package folder (`node_modules/<package>`) and the account's Node that
    /// runs it.
    pub package: Option<(&'a Path, &'a Path)>,
    /// The agent's own CLI when the process needs one ([`AdapterId::own_cli`]).
    pub own_cli: Option<&'a Path>,
    /// The folder `hide` is in, put first on the process's `PATH`.
    pub hide_dir: &'a Path,
    /// The account's `PATH` as the login shell gives it.
    pub search: Option<&'a OsString>,
    /// Desk's own folder for this process: its working folder and Pi's
    /// launcher, extension and marker.
    pub desk: &'a Path,
    /// The Workspace capability reference the `hide` CLI presents.
    pub capability: &'a Path,
    /// Desk's own `CODEX_HOME` (`hide_kit::layout::desk_codex_home`).
    pub codex_home: &'a Path,
    /// The operator's own Codex sign-in ([`crate::codex::operator_sign_in`]).
    pub codex_sign_in: &'a Path,
}

/// The program, arguments and environment one Desk process starts with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Variables set on top of the login variables.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
    /// Files to write before the start (Pi's launcher and extension,
    /// Codex's configuration and rules).
    pub files: Vec<(PathBuf, String, bool)>,
    /// Links to make before the start, each `(target, link)`: Codex's
    /// sign-in, which stays the operator's own.
    pub links: Vec<(PathBuf, PathBuf)>,
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

/// How `adapter`'s Desk process starts. Refused for an agent Desk cannot
/// hold in an ask mode, and when a piece its route needs is not given.
pub fn launch(adapter: AdapterId, paths: &Paths<'_>, nonce: &str) -> Result<Launch, String> {
    if adapter.ask_mode().is_none() {
        return Err(format!("{} cannot be held in an ask mode", adapter.label()));
    }
    let mut search = OsString::from(paths.hide_dir.as_os_str());
    if let Some(account) = paths.search.filter(|path| !path.is_empty()) {
        search.push(if cfg!(windows) { ";" } else { ":" });
        search.push(account);
    }
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), search),
        ("HIDE_CAP_REF".into(), paths.capability.into()),
    ];
    let mut files = Vec::new();
    let mut links = Vec::new();
    let mut pi_marker = None;
    let own_cli = || {
        paths
            .own_cli
            .ok_or_else(|| format!("{} is not given", adapter.own_cli().unwrap_or("the CLI")))
    };
    match adapter.key() {
        AgentId::Codex => {
            env.push(("INITIAL_AGENT_MODE".into(), "read-only".into()));
            env.push(crate::codex::home_variable(paths.codex_home));
            files.extend(crate::codex::files(paths.codex_home));
            links.push(crate::codex::sign_in_link(
                paths.codex_sign_in,
                paths.codex_home,
            ));
        }
        AgentId::Pi => {
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
            env.push(("HIDE_DESK_PI".into(), own_cli()?.into()));
            pi_marker = Some(marker);
        }
        AgentId::OpenCode => {
            env.push(("OPENCODE_CONFIG_CONTENT".into(), OPENCODE_PERMISSIONS.into()));
        }
        _ => {}
    }
    let (program, args) = match adapter.route() {
        Route::Package(pin) => {
            let (package, node) = paths
                .package
                .ok_or_else(|| format!("{}'s adapter is not installed", adapter.label()))?;
            (node.to_path_buf(), vec![package.join(pin.entry).into()])
        }
        Route::Native(args) => (
            own_cli()?.to_path_buf(),
            args.iter().map(OsString::from).collect(),
        ),
    };
    Ok(Launch {
        program,
        args,
        env,
        cwd: work_dir(paths.desk),
        files,
        links,
        pi_marker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desk_speaks_with_every_agent_the_catalog_can_start_by_its_herdr_name() {
        let names: Vec<&str> = AdapterId::all().map(AdapterId::as_str).collect();
        assert_eq!(
            names,
            ["claude", "codex", "grok", "opencode", "pi", "omp", "agy"]
        );
        for adapter in AdapterId::all() {
            assert_eq!(AdapterId::parse(adapter.as_str()), Some(adapter));
            assert_eq!(
                serde_json::to_value(adapter).unwrap(),
                json!(adapter.as_str())
            );
        }
        // A value an earlier Desk stored still reads.
        assert_eq!(
            serde_json::from_value::<AdapterId>(json!("claude")).unwrap(),
            AdapterId::CLAUDE
        );
        assert!(serde_json::from_value::<AdapterId>(json!("cursor")).is_err());
    }

    #[test]
    fn each_pin_names_its_package_at_the_locked_version_and_integrity() {
        for (adapter, pin) in AdapterId::all().filter_map(|adapter| Some((adapter, adapter.pin()?))) {
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
        let meta = AdapterId::CLAUDE.session_meta().unwrap();
        let options = &meta["claudeCode"]["options"];
        assert_eq!(options["settingSources"], json!([]));
        assert_eq!(options["allowDangerouslySkipPermissions"], json!(false));
        assert_eq!(options["settings"]["permissions"]["ask"], json!(["Bash"]));
        assert_eq!(
            options["settings"]["permissions"]["deny"],
            json!(["EnterPlanMode"])
        );
    }

    fn paths<'a>(desk: &'a Path, home: &'a Path, account: Option<&'a OsString>) -> Paths<'a> {
        Paths {
            package: None,
            own_cli: None,
            hide_dir: Path::new("/Applications/hide.app/Contents/Resources"),
            search: account,
            desk,
            capability: Path::new("/state/pane-capabilities/desk.json"),
            codex_home: home,
            codex_sign_in: Path::new("/home/me/.codex/auth.json"),
        }
    }

    fn value(launch: &Launch, key: &str) -> Option<OsString> {
        launch
            .env
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }

    #[test]
    fn hide_comes_first_on_the_path_and_no_pane_is_named() {
        let desk = Path::new("/state/desk");
        let home = desk.join("codex");
        let account = OsString::from("/usr/bin:/bin");
        let package = Path::new("/a/node_modules/pi-acp");
        let launch = launch(
            AdapterId::PI,
            &Paths {
                package: Some((package, Path::new("/usr/local/bin/node"))),
                own_cli: Some(Path::new("/usr/local/bin/pi")),
                ..paths(desk, &home, Some(&account))
            },
            "n1",
        )
        .unwrap();
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            value(&launch, "PATH").unwrap(),
            OsString::from(format!(
                "/Applications/hide.app/Contents/Resources{separator}/usr/bin:/bin"
            ))
        );
        assert!(value(&launch, "HERDR_PANE_ID").is_none());
        assert_eq!(launch.cwd, desk.join("work"));
        assert_eq!(launch.pi_marker, Some(desk.join("pi-ready-n1")));
        assert_eq!(launch.program, Path::new("/usr/local/bin/node"));
        assert_eq!(
            launch.args,
            vec![OsString::from(package.join("dist/index.js"))]
        );
    }

    #[test]
    fn codex_runs_with_desk_s_own_codex_home_and_the_operator_s_sign_in() {
        let desk = Path::new("/state/desk");
        let home = desk.join("codex");
        let launch = launch(
            AdapterId::CODEX,
            &Paths {
                package: Some((
                    Path::new("/a/node_modules/@agentclientprotocol/codex-acp"),
                    Path::new("/usr/local/bin/node"),
                )),
                ..paths(desk, &home, None)
            },
            "n1",
        )
        .unwrap();
        assert!(
            launch
                .env
                .contains(&(OsString::from("CODEX_HOME"), home.clone().into_os_string()))
        );
        let written: Vec<&Path> = launch
            .files
            .iter()
            .map(|(path, ..)| path.as_path())
            .collect();
        assert_eq!(
            written,
            [
                home.join("config.toml").as_path(),
                home.join("rules").join("hide-desk.rules").as_path()
            ]
        );
        assert_eq!(
            launch.links,
            [(
                PathBuf::from("/home/me/.codex/auth.json"),
                home.join("auth.json")
            )]
        );
    }

    #[test]
    fn an_agent_with_its_own_acp_mode_starts_its_cli_held_to_ask() {
        let desk = Path::new("/state/desk");
        let home = desk.join("codex");
        let opencode = launch(
            AdapterId::OPENCODE,
            &Paths {
                own_cli: Some(Path::new("/opt/bin/opencode")),
                ..paths(desk, &home, None)
            },
            "n1",
        )
        .unwrap();
        assert_eq!(opencode.program, Path::new("/opt/bin/opencode"));
        assert_eq!(opencode.args, [OsString::from("acp")]);
        let config: Value = serde_json::from_str(
            value(&opencode, "OPENCODE_CONFIG_CONTENT")
                .unwrap()
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(config["permission"]["*"], "ask");
        assert_eq!(config["permission"]["read"], "allow");

        let omp = launch(
            AdapterId::OMP,
            &Paths {
                own_cli: Some(Path::new("/opt/bin/omp")),
                ..paths(desk, &home, None)
            },
            "n1",
        )
        .unwrap();
        let args: Vec<&str> = omp.args.iter().map(|arg| arg.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|pair| pair == ["--approval-mode", "always-ask"]));
        assert!(args.windows(2).any(|pair| pair == ["--mode", "acp"]));

        // Without its CLI the agent does not start.
        assert!(launch(AdapterId::ANTIGRAVITY, &paths(desk, &home, None), "n1").is_err());
    }

    #[test]
    fn an_agent_desk_cannot_hold_in_an_ask_mode_never_starts() {
        let desk = Path::new("/state/desk");
        let home = desk.join("codex");
        assert_eq!(AdapterId::GROK.ask_mode(), None);
        assert!(
            launch(
                AdapterId::GROK,
                &Paths {
                    own_cli: Some(Path::new("/opt/bin/grok")),
                    ..paths(desk, &home, None)
                },
                "n1",
            )
            .is_err()
        );
    }
}
