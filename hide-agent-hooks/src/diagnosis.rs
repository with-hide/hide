//! Why a pane's children are visible, or why they are not.
//!
//! Two screens depend on this being one function rather than a guess made at
//! each surface: the tooltip on an uninstrumented mark, and the Settings
//! diagnosis. The order the reasons are tried in is fixed, and the first one
//! that matches is the one shown; nothing falls through to an empty value or
//! an invented cause (PRD B21, B32, D-64).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_platform::process::run_to_end;
use serde::{Deserialize, Serialize};

use crate::install::{HookStatus, InstallFailure, status};
use crate::report::{ReportFailure, last_failure};
use crate::runtime::{AgentRuntime, HOOK_VERSION};

/// Why Hide cannot say what a pane's agent has spawned.
///
/// The variant order is the resolution order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UninstrumentedReason {
    /// The runtime's configuration file could not be read or parsed, so Hide
    /// never installed anything into it.
    ConfigUnreadable,
    /// The runtime is here and carries no hook of Hide's.
    HooksNotInstalled,
    /// The operator switched this agent off in Settings, Agents.
    HooksSwitchedOff,
    /// The hook is installed, but this session was already running when it
    /// was, so it never fired. Restarting the agent instruments it.
    SessionPredatesInstall,
    /// The session is reporting through an older hook than this Hide writes.
    HookOutdated,
    /// None of the above. The cause is genuinely unknown and is said to be,
    /// rather than guessed at or drawn as a zero.
    Unknown,
}

impl UninstrumentedReason {
    /// The stable name every surface keys on, so no screen has to compare
    /// the operator-facing sentence against a literal of its own.
    pub fn code(self) -> &'static str {
        match self {
            Self::ConfigUnreadable => "config_unreadable",
            Self::HooksNotInstalled => "hooks_not_installed",
            Self::HooksSwitchedOff => "hooks_switched_off",
            Self::SessionPredatesInstall => "session_predates_install",
            Self::HookOutdated => "hook_outdated",
            Self::Unknown => "unknown",
        }
    }

    /// Reads a code back, from the same table [`Self::code`] writes. A
    /// caller holding a projected reason can recover the resolution order
    /// without a second list of names.
    pub fn from_code(code: &str) -> Option<Self> {
        [
            Self::ConfigUnreadable,
            Self::HooksNotInstalled,
            Self::HooksSwitchedOff,
            Self::SessionPredatesInstall,
            Self::HookOutdated,
            Self::Unknown,
        ]
        .into_iter()
        .find(|reason| reason.code() == code)
    }

    /// The sentence the tooltip shows. It is the whole explanation: the
    /// operator must be able to tell "installed but this session is old" from
    /// "never installed" without opening anything.
    pub fn message(self) -> &'static str {
        match self {
            Self::ConfigUnreadable => {
                "Hide could not read this runtime's settings file, so its hook is not installed."
            }
            Self::HooksNotInstalled => "This runtime's Hide hook is not installed.",
            Self::HooksSwitchedOff => {
                "This agent is switched off in Settings, Agents, so Hide's hook is not installed."
            }
            Self::SessionPredatesInstall => {
                "This session started before the Hide hook was installed. Restart the agent to instrument it."
            }
            Self::HookOutdated => "This session is reporting through an older Hide hook.",
            Self::Unknown => "Child information is unavailable for this pane.",
        }
    }

    /// The short mark's accessible name, so the symbol never carries the
    /// meaning alone (PRD B37).
    pub fn accessibility_label(self) -> &'static str {
        "Children unknown"
    }
}

/// What the core observed about one pane, in the vocabulary this judgement
/// needs. Everything here comes from the snapshot the core already has. A
/// pane on a device is observed the same way, against that device's hooks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneObservation {
    /// The hook dialect of the agent Herdr detected in this pane, when Hide
    /// instruments it (Claude Code's and Codex's hooks, OpenCode's plugin).
    /// `None` is an agent Hide cannot instrument.
    pub dialect: Option<hide_agent_adapter::HookDialect>,
    /// The version carried by the pane's `hide_hooks` token, when the hook
    /// reported at all.
    pub token_version: Option<u32>,
    pub working: Option<u32>,
    pub done: Option<u32>,
    pub blocked: Option<u32>,
}

/// What one pane's chip row is allowed to say.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneInstrumentation {
    /// The counts below are true. False means the mark and its reason.
    pub instrumented: bool,
    pub reason: Option<UninstrumentedReason>,
    pub working: Option<u32>,
    pub done: Option<u32>,
    pub blocked: Option<u32>,
}

/// Resolves one pane against the runtime's install state.
///
/// `status` is the state of the pane's own runtime; a pane with no adapter
/// passes `None` and lands on the single fallback reason.
pub fn instrumentation(
    observation: PaneObservation,
    status: Option<&HookStatus>,
) -> PaneInstrumentation {
    let uninstrumented = |reason: UninstrumentedReason| PaneInstrumentation {
        instrumented: false,
        reason: Some(reason),
        working: None,
        done: None,
        blocked: None,
    };
    let Some(status) = status else {
        return uninstrumented(UninstrumentedReason::Unknown);
    };
    match status {
        HookStatus::Failed { .. } => return uninstrumented(UninstrumentedReason::ConfigUnreadable),
        HookStatus::RuntimeAbsent | HookStatus::NotInstalled => {
            return uninstrumented(UninstrumentedReason::HooksNotInstalled);
        }
        HookStatus::Off => return uninstrumented(UninstrumentedReason::HooksSwitchedOff),
        HookStatus::Installed { .. } | HookStatus::Outdated { .. } => {}
    }
    match observation.token_version {
        None => uninstrumented(UninstrumentedReason::SessionPredatesInstall),
        Some(version) if version < HOOK_VERSION => {
            uninstrumented(UninstrumentedReason::HookOutdated)
        }
        Some(_) => PaneInstrumentation {
            instrumented: true,
            reason: None,
            working: observation.working,
            done: observation.done,
            blocked: observation.blocked,
        },
    }
}

/// One runtime's row in the Settings diagnosis and the CLI output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDiagnosis {
    pub runtime: AgentRuntime,
    pub label: String,
    pub path: String,
    pub status: HookStatus,
    /// The version this Hide would install.
    pub current_version: u32,
    /// Whether this installed runtime supports the hook output shape used by
    /// Project Memory. This is deliberately separate from hook installation:
    /// updating Hide's entry cannot upgrade the operator's agent runtime.
    pub memory_compatibility: MemoryCompatibility,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MemoryCompatibility {
    Supported {
        version: String,
    },
    UpdateRequired {
        installed_version: Option<String>,
        minimum_version: String,
    },
}

impl MemoryCompatibility {
    pub fn supports_injection(&self) -> bool {
        matches!(self, Self::Supported { .. })
    }
}

impl RuntimeDiagnosis {
    /// The short word beside the runtime's name.
    pub fn headline(&self) -> String {
        if !matches!(self.status, HookStatus::RuntimeAbsent)
            && !self.memory_compatibility.supports_injection()
        {
            return "Update required".to_owned();
        }
        match &self.status {
            HookStatus::RuntimeAbsent => "Not on this Mac".to_owned(),
            HookStatus::Installed { version } => format!("Installed (v{version})"),
            HookStatus::Outdated { version } => {
                format!(
                    "Update required (v{version}, current v{})",
                    self.current_version
                )
            }
            HookStatus::NotInstalled => "Not installed".to_owned(),
            HookStatus::Off => "Off".to_owned(),
            HookStatus::Failed { reason } => reason.message(),
        }
    }

    /// Whether the operator can be offered a reinstall for this runtime
    /// (PRD B28).
    pub fn offers_install(&self) -> bool {
        self.memory_compatibility.supports_injection()
            && matches!(
                self.status,
                HookStatus::NotInstalled | HookStatus::Outdated { .. } | HookStatus::Failed { .. }
            )
    }

    /// Whether the operator can be offered a removal (PRD B29).
    ///
    /// A missing helper counts. Finding it is proof that Hide's own entries
    /// are in the file and readable - that is how the helper path was read in
    /// the first place - and those entries are exactly what the operator
    /// needs taken out when the binary they name is gone. The other failures
    /// do not count: Hide could not parse or read the file, so it has no idea
    /// what removing would touch.
    pub fn offers_removal(&self) -> bool {
        matches!(
            self.status,
            HookStatus::Installed { .. }
                | HookStatus::Outdated { .. }
                | HookStatus::Failed {
                    reason: InstallFailure::HelperMissing { .. }
                }
        )
    }
}

/// The whole hook-install picture, in one value both the app and the CLI
/// render, so the terminal answer cannot drift from the app's (PRD B30).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub runtimes: Vec<RuntimeDiagnosis>,
    /// The last hook report that did not reach Herdr, if the most recent
    /// one failed. An installed hook whose reports are refused looks exactly
    /// like a session that predates the install, and this is what tells the
    /// two apart.
    #[serde(default)]
    pub last_report_failure: Option<ReportFailure>,
}

impl Diagnosis {
    /// Reads every runtime under `home`. It performs file reads and no
    /// writes, so it is safe to call from a background reader.
    pub fn read(home: &Path) -> Self {
        Self::read_with_probe(home, runtime_compatibility)
    }

    fn read_with_probe(
        home: &Path,
        probe: impl Fn(AgentRuntime, &Path) -> MemoryCompatibility,
    ) -> Self {
        Self {
            runtimes: AgentRuntime::ALL
                .into_iter()
                .map(|runtime| RuntimeDiagnosis {
                    runtime,
                    label: runtime.label().to_owned(),
                    path: runtime.config_path(home).display().to_string(),
                    status: status(runtime, home),
                    current_version: HOOK_VERSION,
                    memory_compatibility: probe(runtime, home),
                })
                .collect(),
            last_report_failure: last_failure(home),
        }
    }

    pub fn status_of(&self, runtime: AgentRuntime) -> Option<&HookStatus> {
        self.runtimes
            .iter()
            .find(|row| row.runtime == runtime)
            .map(|row| &row.status)
    }

    /// Plain text, one line per runtime, for a terminal.
    pub fn render(&self) -> String {
        let width = self
            .runtimes
            .iter()
            .map(|row| row.label.chars().count())
            .max()
            .unwrap_or(0);
        let mut lines = self
            .runtimes
            .iter()
            .map(|row| {
                format!(
                    "{:width$}  {}\n{:width$}  {}",
                    row.label,
                    row.headline(),
                    "",
                    row.path,
                    width = width
                )
            })
            .collect::<Vec<_>>();
        if let Some(failure) = &self.last_report_failure {
            lines.push(format!("Last report failed: {}", failure.message()));
        }
        lines.join("\n")
    }
}

const RUNTIME_VERSION_PROBE_TIMEOUT: Duration = Duration::from_millis(750);

/// Whether the runtime installed under `home` is new enough for the hook Hide
/// writes, read from its `--version` within a short bound.
pub fn runtime_compatibility(runtime: AgentRuntime, home: &Path) -> MemoryCompatibility {
    let minimum = minimum_memory_version(runtime);
    let Some(binary) = runtime_binary(runtime, home) else {
        return MemoryCompatibility::UpdateRequired {
            installed_version: None,
            minimum_version: minimum.to_owned(),
        };
    };
    let Some(output) = cli_path(home)
        .and_then(|path| version_output(&binary, &path, RUNTIME_VERSION_PROBE_TIMEOUT))
    else {
        return MemoryCompatibility::UpdateRequired {
            installed_version: None,
            minimum_version: minimum.to_owned(),
        };
    };
    let installed = parse_version(&output);
    match installed.as_deref() {
        Some(version) if version_at_least(version, minimum) => MemoryCompatibility::Supported {
            version: version.to_owned(),
        },
        _ => MemoryCompatibility::UpdateRequired {
            installed_version: installed,
            minimum_version: minimum.to_owned(),
        },
    }
}

fn minimum_memory_version(runtime: AgentRuntime) -> &'static str {
    match runtime {
        AgentRuntime::ClaudeCode => "2.1.278",
        AgentRuntime::Codex => "0.155.1",
    }
}

pub(crate) fn runtime_binary(runtime: AgentRuntime, home: &Path) -> Option<PathBuf> {
    let name = match runtime {
        AgentRuntime::ClaudeCode => "claude",
        AgentRuntime::Codex => "codex",
    };
    find_binary(name, home)
}

/// The program `name` as a hook's daemon would find it on [`cli_path`],
/// without the login shell's folders, which only the install kit and Hide AI
/// read (`hide_platform::programs`).
pub fn find_binary(name: &str, home: &Path) -> Option<PathBuf> {
    hide_platform::host::find_program(&cli_path(home)?, name)
}

/// The `PATH` an agent's CLI is found on and run with: the daemon's own
/// `PATH`, then the folders installers put a CLI in that such a `PATH` may
/// not reach. A device helper started over SSH has only the system folders.
/// A CLI installed as a script starts its interpreter from the same `PATH`
/// (pnpm's `codex` is a shell script that runs `node`), so a CLI found here
/// is run with this value, never with the daemon's `PATH` alone.
pub fn cli_path(home: &Path) -> Option<OsString> {
    hide_platform::programs::cli_path_with(home, None)
}

fn version_output(binary: &Path, path: &OsStr, timeout: Duration) -> Option<String> {
    let mut command = Command::new(binary);
    command.arg("--version").env("PATH", path);
    let finished = run_to_end(&mut command, timeout, &AtomicBool::new(false)).ok()?;
    let output = if finished.stdout.trim().is_empty() {
        finished.stderr
    } else {
        finished.stdout
    };
    finished
        .code
        .is_some_and(|code| code == 0)
        .then(|| output.trim().to_owned())
}

fn parse_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|word| {
        let trimmed =
            word.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
        let mut parts = trimmed.split('.');
        (parts.clone().count() == 3
            && parts.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())))
        .then(|| trimmed.to_owned())
    })
}

/// Whether `installed` is at least `minimum`, both `x.y.z`.
pub fn version_at_least(installed: &str, minimum: &str) -> bool {
    let tuple = |version: &str| {
        let parts = version
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]))
    };
    matches!((tuple(installed), tuple(minimum)), (Some(found), Some(required)) if found >= required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::InstallFailure;

    fn compatible(_runtime: AgentRuntime, _home: &Path) -> MemoryCompatibility {
        MemoryCompatibility::Supported {
            version: "999.0.0".to_owned(),
        }
    }

    fn observation(token_version: Option<u32>) -> PaneObservation {
        PaneObservation {
            dialect: Some(hide_agent_adapter::HookDialect::ClaudeCode),
            token_version,
            working: Some(2),
            done: Some(3),
            blocked: None,
        }
    }

    #[test]
    fn the_first_matching_reason_wins_in_the_documented_order() {
        let failed = HookStatus::Failed {
            reason: InstallFailure::Unparsable {
                path: "/Users/example/.claude/settings.json".to_owned(),
                detail: "expected value".to_owned(),
            },
        };
        assert_eq!(
            instrumentation(observation(Some(HOOK_VERSION)), Some(&failed)).reason,
            Some(UninstrumentedReason::ConfigUnreadable)
        );
        assert_eq!(
            instrumentation(
                observation(Some(HOOK_VERSION)),
                Some(&HookStatus::NotInstalled)
            )
            .reason,
            Some(UninstrumentedReason::HooksNotInstalled)
        );
        assert_eq!(
            instrumentation(
                observation(None),
                Some(&HookStatus::Installed {
                    version: HOOK_VERSION
                })
            )
            .reason,
            Some(UninstrumentedReason::SessionPredatesInstall)
        );
        assert_eq!(
            instrumentation(
                observation(Some(HOOK_VERSION - 1)),
                Some(&HookStatus::Installed {
                    version: HOOK_VERSION
                })
            )
            .reason,
            Some(UninstrumentedReason::HookOutdated)
        );
    }

    #[test]
    fn outdated_runtime_uses_the_required_update_copy() {
        let row = RuntimeDiagnosis {
            runtime: AgentRuntime::Codex,
            label: "Codex".to_owned(),
            path: "/tmp/hooks.json".to_owned(),
            status: HookStatus::Outdated { version: 2 },
            current_version: HOOK_VERSION,
            memory_compatibility: compatible(AgentRuntime::Codex, Path::new("/tmp")),
        };
        assert_eq!(row.headline(), "Update required (v2, current v9)");
        assert!(row.offers_install());
    }

    #[test]
    fn an_agent_with_no_adapter_says_so_rather_than_guessing() {
        let mut unknown = observation(Some(HOOK_VERSION));
        unknown.dialect = None;
        let result = instrumentation(unknown, None);
        assert_eq!(result.reason, Some(UninstrumentedReason::Unknown));
        assert!(result.working.is_none(), "an unknown count is never a zero");
    }

    #[test]
    fn an_instrumented_session_reports_its_counts_and_no_reason() {
        let result = instrumentation(
            observation(Some(HOOK_VERSION)),
            Some(&HookStatus::Installed {
                version: HOOK_VERSION,
            }),
        );
        assert!(result.instrumented);
        assert_eq!(result.reason, None);
        assert_eq!((result.working, result.done), (Some(2), Some(3)));
        assert_eq!(
            result.blocked, None,
            "no adapter observes a blocked subagent"
        );
    }

    #[test]
    fn an_agent_switched_off_is_uninstrumented_for_that_reason_and_offers_no_install() {
        let result = instrumentation(PaneObservation::default(), Some(&HookStatus::Off));
        assert_eq!(result.reason, Some(UninstrumentedReason::HooksSwitchedOff));
        assert_eq!(
            UninstrumentedReason::from_code("hooks_switched_off"),
            Some(UninstrumentedReason::HooksSwitchedOff)
        );
    }

    #[test]
    fn every_reason_carries_a_distinct_sentence() {
        let reasons = [
            UninstrumentedReason::ConfigUnreadable,
            UninstrumentedReason::HooksNotInstalled,
            UninstrumentedReason::HooksSwitchedOff,
            UninstrumentedReason::SessionPredatesInstall,
            UninstrumentedReason::HookOutdated,
            UninstrumentedReason::Unknown,
        ];
        let messages: std::collections::BTreeSet<_> =
            reasons.iter().map(|reason| reason.message()).collect();
        assert_eq!(messages.len(), reasons.len());
        assert!(reasons.iter().all(|reason| !reason.message().is_empty()));
    }

    #[test]
    fn the_diagnosis_names_each_runtime_its_file_and_what_it_offers() {
        let home = std::env::temp_dir().join(format!("hide-diagnosis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let diagnosis = Diagnosis::read_with_probe(&home, compatible);
        assert_eq!(diagnosis.runtimes.len(), 2);
        let claude = &diagnosis.runtimes[0];
        assert_eq!(claude.status, HookStatus::NotInstalled);
        assert!(claude.offers_install() && !claude.offers_removal());
        let codex = &diagnosis.runtimes[1];
        assert_eq!(codex.status, HookStatus::RuntimeAbsent);
        assert!(!codex.offers_install() && !codex.offers_removal());
        let settings = Path::new(".claude").join("settings.json");
        assert!(diagnosis.render().contains(&settings.display().to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn runtime_versions_gate_only_the_runtime_with_an_unsupported_hook_schema() {
        assert_eq!(
            parse_version("codex-cli 0.155.1"),
            Some("0.155.1".to_owned())
        );
        assert_eq!(
            parse_version("2.1.278 (Claude Code)"),
            Some("2.1.278".to_owned())
        );
        assert!(version_at_least("0.155.2", "0.155.1"));
        assert!(!version_at_least("0.154.9", "0.155.1"));

        let supported = RuntimeDiagnosis {
            runtime: AgentRuntime::Codex,
            label: "Codex".to_owned(),
            path: "/tmp/hooks.json".to_owned(),
            status: HookStatus::Installed {
                version: HOOK_VERSION,
            },
            current_version: HOOK_VERSION,
            memory_compatibility: MemoryCompatibility::Supported {
                version: "0.155.1".to_owned(),
            },
        };
        let unsupported = RuntimeDiagnosis {
            runtime: AgentRuntime::ClaudeCode,
            label: "Claude Code".to_owned(),
            path: "/tmp/settings.json".to_owned(),
            status: HookStatus::Installed {
                version: HOOK_VERSION,
            },
            current_version: HOOK_VERSION,
            memory_compatibility: MemoryCompatibility::UpdateRequired {
                installed_version: Some("2.1.277".to_owned()),
                minimum_version: "2.1.278".to_owned(),
            },
        };
        assert_eq!(supported.headline(), "Installed (v9)");
        assert_eq!(unsupported.headline(), "Update required");
        assert!(!unsupported.offers_install());
    }
}
