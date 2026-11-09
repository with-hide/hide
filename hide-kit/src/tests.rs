//! The kit against a HOME fixture. Nothing here
//! reads or writes the account running the tests.
//!
//! The fixture runs on every system, and so do the cases of the accounts
//! profiles run in (`profile_cases`). The rest drive the kit against a
//! stand-in daemon on a Unix socket and with shell scripts for the programs
//! it installs and asks, so they run where those do.

#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::sync::Mutex;

use serde_json::Value;
#[cfg(unix)]
use serde_json::json;

use super::*;
#[cfg(unix)]
use stand_ins::program;

#[cfg(unix)]
mod agent_cases;
#[cfg(unix)]
mod codex_trust_cases;
#[cfg(unix)]
mod herdr_cases;
#[cfg(unix)]
mod kit_cases;
mod profile_cases;
#[cfg(unix)]
mod retired_cases;
#[cfg(unix)]
#[path = "../../hide-platform/tests/it/stand_ins.rs"]
mod stand_ins;
#[path = "../../hide-platform/tests/it/widen.rs"]
mod widen;

#[cfg(unix)]
type IndexReadHook = Box<dyn FnOnce(&Path)>;
#[cfg(unix)]
std::thread_local! {
    static SUPERVISOR_INDEX_READ_HOOK: std::cell::RefCell<Option<IndexReadHook>> =
        const { std::cell::RefCell::new(None) };
    static SYSTEM_ALIAS_FIXTURE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
// A private fixture cannot create a root-owned system alias. Inject only
// that kernel ownership answer; the alias, target walk and kit are real.
pub(crate) fn inspect_system_alias_fixture(
    path: &Path,
) -> Option<std::io::Result<hide_platform::fs::private::InspectionDirectory>> {
    SYSTEM_ALIAS_FIXTURE.with(|slot| {
        (slot.borrow().as_deref() == Some(path)).then(|| {
            std::fs::read_link(path)
                .map(hide_platform::fs::private::InspectionDirectory::SystemAlias)
        })
    })
}

#[cfg(unix)]
struct ArmedSystemAliasFixture;

#[cfg(unix)]
impl ArmedSystemAliasFixture {
    fn new(path: PathBuf) -> Self {
        SYSTEM_ALIAS_FIXTURE.with(|slot| {
            assert!(slot.borrow_mut().replace(path).is_none());
        });
        Self
    }
}

#[cfg(unix)]
impl Drop for ArmedSystemAliasFixture {
    fn drop(&mut self) {
        SYSTEM_ALIAS_FIXTURE.with(|slot| slot.borrow_mut().take());
    }
}

#[cfg(unix)]
pub(crate) fn before_supervisor_index_read(path: &Path) {
    if let Some(hook) = SUPERVISOR_INDEX_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(path);
    }
}

#[cfg(unix)]
struct ArmedIndexReadHook;

#[cfg(unix)]
impl ArmedIndexReadHook {
    fn new(hook: impl FnOnce(&Path) + 'static) -> Self {
        SUPERVISOR_INDEX_READ_HOOK.with(|slot| {
            assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
        });
        Self
    }
}

#[cfg(unix)]
impl Drop for ArmedIndexReadHook {
    fn drop(&mut self) {
        SUPERVISOR_INDEX_READ_HOOK.with(|slot| slot.borrow_mut().take());
    }
}

#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A program named `name` in `folder` that does nothing, spelled as the
/// system finds it by that name: a shell script on Unix, a batch file on
/// Windows, where `PATHEXT` finds `<name>.cmd`.
fn installed(folder: &Path, name: &str) {
    #[cfg(unix)]
    executable(&folder.join(name), "#!/bin/sh\n");
    #[cfg(windows)]
    {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join(format!("{name}.cmd")), "@exit /b 0\r\n").unwrap();
    }
}

/// A link at `link` to `target`, made as an operator makes one on the
/// system: on Windows a folder's link or a file's, as `target` is (one to
/// nothing yet is a file's).
fn symlink(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).unwrap();
    } else {
        std::os::windows::fs::symlink_file(target, link).unwrap();
    }
}

/// A program of the kit's own build, `name` in the kit's folder as the
/// package places it (`hide.exe` on Windows), which no test runs.
fn kit_program(kit: &Path, name: &str) {
    #[cfg(unix)]
    executable(&kit.join(name), "#!/bin/sh\n");
    #[cfg(windows)]
    {
        std::fs::create_dir_all(kit).unwrap();
        std::fs::write(kit.join(format!("{name}.exe")), "").unwrap();
    }
}

const OTHER_TOOL: &str = r#"{
  "model": "opus",
  "hooks": {
    "SubagentStart": [
      { "hooks": [ { "type": "command", "command": "/opt/other/notify.sh start" } ] }
    ]
  }
}
"#;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    target: KitTarget,
}

impl Fixture {
    /// A machine that already has a kit record, as every machine has after
    /// its first pass; [`Fixture::fresh`] is one that has never had one.
    fn new() -> Self {
        let fixture = Self::fresh();
        record::save(fixture.home(), &record::Record::default()).unwrap();
        fixture
    }

    fn fresh() -> Self {
        // The private HOME must leave room for the legacy daemon's nested
        // socket under SUN_LEN, even when the harness gives TMPDIR a long
        // spelling. TempDir owns this unique private short-root fixture.
        let dir = crate::short_tempdir();
        // Resolved, so paths compare equal to what the stand-in programs record.
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let kit = root.join("kit");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude/settings.json"), OTHER_TOOL).unwrap();
        // Claude Code is installed: its program is what says so.
        installed(&home.join(".local/bin"), "claude");
        kit_program(&kit, "hide");
        kit_program(&kit, "hide-agent-hooks");
        // launchd's, which the fixture's own domain of agents answers.
        #[cfg(unix)]
        program(&root.join("launchctl"), "#!/bin/sh\nexit 113\n");
        let target = KitTarget {
            home: home.clone(),
            install: hide_agent_hooks::route::APP_INSTALL.to_owned(),
            kit_dir: kit,
            cli_dir: home.join(".local/bin"),
            owned_roots: vec![root.join("helper-root")],
            codex: None,
            login_shell: None,
            legacy_coordination_home: None,
            user_agents: hide_platform::user_agents::UserAgents::fixture(&root),
            retirement_projects: Vec::new(),
            legacy: Vec::new(),
            stop: Arc::default(),
            profiles: None,
        };
        Self {
            _dir: dir,
            root,
            target,
        }
    }

    fn home(&self) -> &Path {
        &self.target.home
    }

    #[cfg(unix)]
    fn settings(&self) -> String {
        std::fs::read_to_string(self.home().join(".claude/settings.json")).unwrap()
    }
}

fn state(report: &KitReport, id: ComponentId) -> ComponentState {
    report.component(id).unwrap().state
}

#[cfg(unix)]
/// The pass stopped before changing anything, for `cause`, and judged no
/// part or agent.
fn refusal(report: &KitReport, cause: RefusalCause) -> &KitRefusal {
    let refusal = report
        .refused
        .as_deref()
        .unwrap_or_else(|| panic!("{report:?}"));
    assert_eq!(refusal.cause, cause, "{refusal:?}");
    assert!(
        report.components.is_empty() && report.agents.is_empty(),
        "{report:?}"
    );
    refusal
}

#[cfg(unix)]
fn other_tool_entry(settings: &str) -> Value {
    let document: Value = serde_json::from_str(settings).unwrap();
    document["hooks"]["SubagentStart"][0].clone()
}

#[cfg(unix)]
/// A `codex` that keeps `daemon_auto_start` in `$CODEX_HOME/daemon` the way
/// `codex features` reports it, logs every call, and is told by files in
/// HOME to be an old Codex (`codex-old`), to fail (`codex-fails`), to refuse
/// only the turn-off (`disable-fails`) or to have a daemon answering
/// (`daemon-running`).
fn fake_codex(fixture: &mut Fixture, daemon: &str) -> PathBuf {
    let codex = fixture.root.join("bin/codex");
    program(
        &codex,
        concat!(
            "#!/bin/sh\n",
            "echo \"$@\" >> \"$HOME/codex.log\"\n",
            "if [ -e \"$HOME/codex-fails\" ]; then echo 'Error: config.toml is locked' >&2; exit 1; fi\n",
            "case \"$1 $2\" in\n",
            "  'features list')\n",
            "    echo 'apps                 stable  true'\n",
            "    [ -e \"$HOME/codex-old\" ] || echo \"daemon_auto_start    stable  $(cat \"$CODEX_HOME/daemon\" 2>/dev/null || echo true)\" ;;\n",
            "  'features disable')\n",
            "    if [ -e \"$HOME/disable-fails\" ]; then echo 'Error: config.toml is locked' >&2; exit 1; fi\n",
            "    echo false > \"$CODEX_HOME/daemon\" ;;\n",
            "  'features enable') echo true > \"$CODEX_HOME/daemon\" ;;\n",
            "  'app-server daemon')\n",
            "    case \"$3\" in\n",
            "      version)\n",
            "        [ -e \"$HOME/daemon-running\" ] || { echo 'Error: failed to connect' >&2; exit 1; }\n",
            "        cat \"$HOME/daemon-answer\" 2>/dev/null || echo '{\"status\":\"running\"}' ;;\n",
            "      stop)\n",
            "        if [ -e \"$HOME/daemon-stop-fails\" ]; then echo 'Error: permission denied' >&2; exit 1; fi\n",
            "        [ -e \"$HOME/daemon-running\" ] || { echo '{\"status\":\"notRunning\"}'; exit 0; }\n",
            "        [ -e \"$HOME/daemon-comes-back\" ] || rm -f \"$HOME/daemon-running\"\n",
            "        echo '{\"status\":\"stopped\"}' ;;\n",
            "      *) exit 2 ;;\n",
            "    esac ;;\n",
            "  *) exit 2 ;;\n",
            "esac\n",
        ),
    );
    let folder = fixture.home().join(".codex");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("daemon"), format!("{daemon}\n")).unwrap();
    fixture.use_codex(&codex);
    codex
}

#[cfg(unix)]
impl Fixture {
    /// Gives the target `codex` as the Codex it found, run with this
    /// process's `PATH` and the usual install folders under the fixture home.
    fn use_codex(&mut self, codex: &Path) {
        self.target.codex = Some(hide_platform::programs::Program::named(
            codex.to_path_buf(),
            hide_platform::programs::cli_path_with(self.home(), None).unwrap(),
        ));
    }

    /// How many times the kit asked Codex to stop its shared daemon.
    fn daemon_stops(&self) -> usize {
        std::fs::read_to_string(self.home().join("codex.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == "app-server daemon stop")
            .count()
    }

    fn daemon_running(&self) -> bool {
        self.home().join("daemon-running").exists()
    }

    fn daemon_setting(&self) -> String {
        std::fs::read_to_string(self.home().join(".codex/daemon"))
            .unwrap()
            .trim()
            .to_owned()
    }

    /// The `codex features enable|disable` calls the kit made, in order.
    fn codex_writes(&self) -> Vec<String> {
        std::fs::read_to_string(self.home().join("codex.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| {
                line.starts_with("features enable") || line.starts_with("features disable")
            })
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(unix)]
fn home_tree(home: &Path) -> std::collections::BTreeMap<PathBuf, (u32, Option<Vec<u8>>)> {
    let mut result = std::collections::BTreeMap::new();
    let mut pending = vec![home.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let metadata = std::fs::symlink_metadata(entry.path()).unwrap();
            let contents = if metadata.is_file() {
                Some(std::fs::read(entry.path()).unwrap())
            } else {
                None
            };
            result.insert(
                entry.path().strip_prefix(home).unwrap().to_path_buf(),
                (metadata.permissions().mode(), contents),
            );
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    result
}
