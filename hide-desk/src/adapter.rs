//! The agents Desk can talk to and how each starts (PRD Desk D-79, D-92).
//!
//! The list is Hide's catalog (`hide_agent_adapter::ADAPTERS`, the agents
//! Hide can start), each named by its agent name, so an agent the catalog
//! gains is Desk's once its ACP route is declared here; the match below is
//! exhaustive, so a new agent cannot be added without that decision. Each
//! speaks ACP one of two ways:
//! - its vendor's adapter, an npm package pinned with every dependency's
//!   integrity in `contracts/desk-adapters/<id>/` and run by the account's
//!   Node (Claude Code, Codex, Pi), installed when Desk is turned on;
//! - its own CLI's ACP mode (`opencode acp`, `omp --mode acp`, `agy acp`,
//!   `grok agent stdio`), which needs nothing installed.
//!
//! Each starts as it does in the operator's terminal (D-92): in its own
//! default mode, with the operator's own settings, sign-in, instructions and
//! extensions, and nothing Desk adds to make it ask. What it asks reaches
//! Hide as a permission request, which the operator's choice under Desk's
//! box answers ([`crate::permission`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hide_agent_adapter::{ADAPTERS, AgentAdapter, AgentId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// One of the catalog's agents Desk speaks ACP with, named on the wire and in
/// Desk's store by its agent name (`claude`, `codex`, `pi`, `opencode`,
/// `omp`, `agy`, `grok`).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AdapterId(AgentId);

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
        self.catalog().engine_kind
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

    /// Where the adapter tells a compaction's end, when it tells it outside
    /// the reply ([`CompactionMark`]).
    pub fn compaction_mark(self) -> Option<CompactionMark> {
        match self.0 {
            AgentId::OpenCode => Some(CompactionMark {
                pointer: "/_meta/opencode~1compaction",
            }),
            _ => None,
        }
    }

    /// The commands Desk keeps out of the agent's own list, as its definition
    /// file names them (`[desk] left_out`, [`crate::commands`]).
    pub fn left_out(self) -> &'static [String] {
        hide_agent_adapter::definition::builtin(self.catalog().id)
            .map_or(&[], |definition| &definition.desk.left_out)
    }

    /// How the adapter marks a turn that failed when it still answers the
    /// prompt as ended: codex-acp passes Codex's own thread status on, which
    /// is `systemError` only after a failed turn, and then writes the
    /// model's error as reply text and answers `end_turn` (letter-47671).
    /// An adapter that refuses the prompt itself, as Claude's does, needs
    /// none.
    pub fn failed_turn_mark(self) -> Option<FailedTurnMark> {
        match self.0 {
            AgentId::Codex => Some(FailedTurnMark {
                pointer: "/_meta/codex/threadStatus/type",
                value: "systemError",
            }),
            _ => None,
        }
    }
}

/// Where an adapter tells a compaction's end only in a `session_info_update`
/// field rather than in reply text, so a `/compact` turn would read empty:
/// OpenCode 2.0.29 sets `_meta["opencode/compaction"]` to `{status, error}`,
/// `status` `completed` or `failed` with the reason in `error.message`
/// (ACP v2's `compaction_update` is the standard form, which Desk, on v1,
/// is not sent).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactionMark {
    pub pointer: &'static str,
}

/// How a compaction ended, as its adapter told it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Compaction {
    Done,
    /// With the agent's own reason, when it gave one.
    Failed(Option<String>),
}

impl CompactionMark {
    /// The compaction's end the update tells, if it tells one.
    pub fn read(self, update: &Value) -> Option<Compaction> {
        let field = update.pointer(self.pointer)?;
        match field.get("status").and_then(Value::as_str)? {
            "completed" => Some(Compaction::Done),
            "failed" => Some(Compaction::Failed(
                field
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .and_then(|reason| reason.lines().next())
                    .map(str::trim)
                    .filter(|reason| !reason.is_empty())
                    .map(|reason| reason.chars().take(200).collect()),
            )),
            _ => None,
        }
    }
}

/// A `session_info_update` field that says the running turn failed: the
/// value at `pointer` in the update is `value`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailedTurnMark {
    pub pointer: &'static str,
    pub value: &'static str,
}

impl FailedTurnMark {
    pub fn marks(self, update: &Value) -> bool {
        update.pointer(self.pointer).and_then(Value::as_str) == Some(self.value)
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
        AgentId::Omp => Route::Native(&["--mode", "acp"]),
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
    /// The environment a new terminal of the account starts with
    /// (`hide_platform::programs::account_environment`), which an agent the
    /// operator starts in a pane has: a provider key its own configuration
    /// reads (OpenCode's `{env:…}`, `ANTHROPIC_API_KEY`), its settings
    /// folder (`CODEX_HOME`). A pane's identity and Hide's own variables in
    /// it are left out ([`crate::host::from_account`]), and Desk's own
    /// variables win over it.
    pub account: Option<&'a [(OsString, OsString)]>,
    /// Desk's own folder, which holds the process's working folder.
    pub desk: &'a Path,
    /// The Workspace capability reference the `hide` CLI presents.
    pub capability: &'a Path,
}

/// The program, arguments and environment one Desk process starts with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Variables set over what the process keeps from hided: the account's
    /// environment, then Desk's own, each named once.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
}

/// Desk's working folder: no checkout, so no project's settings or
/// instructions load into the Desk agent.
pub fn work_dir(desk: &Path) -> PathBuf {
    desk.join("work")
}

/// How `adapter`'s Desk process starts. Refused when a piece its route needs
/// is not given.
pub fn launch(adapter: AdapterId, paths: &Paths<'_>) -> Result<Launch, String> {
    let mut search = OsString::from(paths.hide_dir.as_os_str());
    if let Some(account) = paths.search.filter(|path| !path.is_empty()) {
        search.push(if cfg!(windows) { ";" } else { ":" });
        search.push(account);
    }
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), search),
        ("HIDE_CAP_REF".into(), paths.capability.into()),
    ];
    let own_cli = || {
        paths
            .own_cli
            .ok_or_else(|| format!("{} is not given", adapter.own_cli().unwrap_or("the CLI")))
    };
    if adapter.key() == AgentId::Pi {
        // The Pi Hide found on the account's `PATH`, which pi-acp starts.
        env.push(("PI_ACP_PI_COMMAND".into(), own_cli()?.into()));
    }
    if adapter.key() == AgentId::Codex {
        // codex-acp starts in a preset of its own, never in what the
        // account's `config.toml` sets; `workspace-write` is the Codex CLI's
        // own default (asks before leaving the workspace or using the
        // network, the operator answering), where codex-acp's default has a
        // model answer instead (docs/INSTALL.md, Desk's Codex).
        env.push(("INITIAL_AGENT_MODE".into(), "workspace-write".into()));
    }
    let mut account: Vec<(OsString, OsString)> = paths
        .account
        .unwrap_or_default()
        .iter()
        .filter(|(name, _)| {
            crate::host::from_account(name)
                && !env.iter().any(|(own, _)| crate::host::same_name(own, name))
        })
        .cloned()
        .collect();
    account.extend(env);
    let env = account;
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desk_speaks_with_every_agent_the_catalog_can_start_by_its_engine_kind() {
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
        for (adapter, pin) in AdapterId::all().filter_map(|adapter| Some((adapter, adapter.pin()?)))
        {
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

    fn paths<'a>(desk: &'a Path, account: Option<&'a OsString>) -> Paths<'a> {
        Paths {
            package: None,
            own_cli: None,
            hide_dir: Path::new("/Applications/hide.app/Contents/Resources"),
            search: account,
            account: None,
            desk,
            capability: Path::new("/state/pane-capabilities/desk.json"),
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
    fn opencode_s_compaction_end_reads_from_its_meta_field() {
        let mark = AdapterId::OPENCODE.compaction_mark().unwrap();
        // The updates OpenCode 2.0.29 sent for `/compact` (mission-101 slash-research probe).
        let update = |status: Value| json!({"sessionUpdate": "session_info_update", "_meta": {"opencode/compaction": status}});
        assert_eq!(
            mark.read(&update(json!({"status": "started", "reason": "manual"}))),
            None
        );
        assert_eq!(
            mark.read(&update(json!({"status": "failed", "reason": "manual",
                "error": {"type": "compaction.unavailable", "message": "Nothing to compact yet"}}))),
            Some(Compaction::Failed(Some("Nothing to compact yet".into())))
        );
        assert_eq!(
            mark.read(&update(json!({"status": "completed"}))),
            Some(Compaction::Done)
        );
        assert_eq!(
            mark.read(&json!({"sessionUpdate": "session_info_update", "title": "x"})),
            None
        );
    }

    #[test]
    fn hide_comes_first_on_the_path_and_pi_acp_starts_the_pi_hide_found() {
        let desk = Path::new("/state/desk");
        let account = OsString::from("/usr/bin:/bin");
        let package = Path::new("/a/node_modules/pi-acp");
        let launch = launch(
            AdapterId::PI,
            &Paths {
                package: Some((package, Path::new("/usr/local/bin/node"))),
                own_cli: Some(Path::new("/usr/local/bin/pi")),
                ..paths(desk, Some(&account))
            },
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
        assert_eq!(
            value(&launch, "PI_ACP_PI_COMMAND").unwrap(),
            OsString::from("/usr/local/bin/pi")
        );
        assert_eq!(launch.cwd, desk.join("work"));
        assert_eq!(launch.program, Path::new("/usr/local/bin/node"));
        assert_eq!(
            launch.args,
            vec![OsString::from(package.join("dist/index.js"))]
        );
    }

    /// D-92: nothing Desk adds makes an agent ask, so each starts as the
    /// operator's terminal starts it, with the account's own settings.
    #[test]
    fn every_agent_starts_with_the_account_s_settings_and_codex_in_its_cli_default() {
        let desk = Path::new("/state/desk");
        let account = [
            (
                OsString::from("CODEX_HOME"),
                OsString::from("/home/me/.codex"),
            ),
            (OsString::from("HIDE_PANE_ID"), OsString::from("p1")),
        ];
        for (adapter, cli, args) in [
            (AdapterId::OPENCODE, "/opt/bin/opencode", &["acp"][..]),
            (AdapterId::OMP, "/opt/bin/omp", &["--mode", "acp"][..]),
            (AdapterId::ANTIGRAVITY, "/opt/bin/agy", &["acp"][..]),
            (
                AdapterId::GROK,
                "/opt/bin/grok",
                &["agent", "--no-leader", "stdio"][..],
            ),
        ] {
            let launched = launch(
                adapter,
                &Paths {
                    own_cli: Some(Path::new(cli)),
                    account: Some(&account),
                    ..paths(desk, None)
                },
            )
            .unwrap();
            assert_eq!(launched.program, Path::new(cli), "{adapter:?}");
            assert_eq!(launched.args, args, "{adapter:?}");
            let names: Vec<&OsString> = launched.env.iter().map(|(name, _)| name).collect();
            assert_eq!(names, ["CODEX_HOME", "PATH", "HIDE_CAP_REF"], "{adapter:?}");
        }
        let codex = launch(
            AdapterId::CODEX,
            &Paths {
                package: Some((
                    Path::new("/a/node_modules/@agentclientprotocol/codex-acp"),
                    Path::new("/usr/local/bin/node"),
                )),
                account: Some(&account),
                ..paths(desk, None)
            },
        )
        .unwrap();
        assert_eq!(
            value(&codex, "CODEX_HOME").unwrap(),
            OsString::from("/home/me/.codex")
        );
        assert_eq!(
            value(&codex, "INITIAL_AGENT_MODE").unwrap(),
            OsString::from("workspace-write")
        );

        // Without its CLI the agent does not start.
        assert!(launch(AdapterId::ANTIGRAVITY, &paths(desk, None)).is_err());
    }
}
