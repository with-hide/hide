//! What an agent whose CLI is older than its minimum needs: the version the
//! copy answered, the one it needs, and the one command that updates that
//! copy, chosen from the way the vendor's documentation installed it as the
//! program's path shows (`hide_agent_adapter::UpdateRoute`; docs/agent-hooks.md,
//! Updating an agent that is too old). A copy no route recognises carries no
//! command, and the screen offers the agent's install page instead.

use std::path::{Component, Path, PathBuf};

use hide_agent_adapter::{InstallMethod, UpdateRoute};
use hide_platform::programs::Program;
use serde::{Deserialize, Serialize};

use crate::agents::AgentAdapter;

/// The update an agent's found copy needs before Hide puts its pieces in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentUpdate {
    /// The version the program answered.
    pub installed: String,
    /// The agent's minimum (`AgentAdapter::minimum`).
    pub minimum: String,
    /// The program that was asked, the copy the operator's shell runs.
    pub path: String,
    /// The vendor's command for how that copy was installed; absent when
    /// Hide cannot tell, and never guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// The update `program` needs, or `None` when it answers the agent's
/// minimum or newer, or answers no version Hide can read (`agents::too_old`
/// judges the same answer, which `program_version` keeps per file).
pub(crate) fn needed(
    adapter: &AgentAdapter,
    program: &Program,
    home: &Path,
) -> Option<AgentUpdate> {
    let installed = hide_agent_hooks::program_version(program)?;
    if hide_agent_hooks::version_at_least(&installed, adapter.minimum) {
        return None;
    }
    Some(AgentUpdate {
        installed,
        minimum: adapter.minimum.to_owned(),
        path: program.path().display().to_string(),
        command: route(adapter.updates(), program.path(), home)
            .map(|route| route.command.to_owned()),
    })
}

/// The route whose way of installing put `program` there, among the ones
/// documented for this system. The kinds are tried from the most specific
/// evidence to the least: where the file really is (a Homebrew keg, a WinGet
/// package, the vendor's own folder, bun's global folder), then npm's
/// folders beside the program, so a stale npm package next to a Homebrew
/// link never wins over the link.
pub(crate) fn route<'a>(
    routes: &'a [UpdateRoute],
    program: &Path,
    home: &Path,
) -> Option<&'a UpdateRoute> {
    let resolved =
        hide_platform::fs::identity::canonical(program).unwrap_or_else(|_| program.to_owned());
    let home = hide_platform::fs::identity::canonical(home).unwrap_or_else(|_| home.to_owned());
    let names = names(&resolved);
    let folder = program.parent();
    let documented = || {
        routes
            .iter()
            .filter(|route| route.systems.include_this_one())
    };
    let homebrew = documented().find(|route| match route.method {
        InstallMethod::Homebrew(name) => {
            holds_run(&names, &["Cellar", name]) || holds_run(&names, &["Caskroom", name])
        }
        _ => false,
    });
    let winget = || {
        documented().find(|route| match route.method {
            InstallMethod::WinGet(id) => names.windows(3).any(|run| {
                same(&run[0], "WinGet")
                    && same(&run[1], "Packages")
                    && starts_with(&run[2], &format!("{id}_"))
            }),
            _ => false,
        })
    };
    let installer = || {
        documented().find(|route| match route.method {
            InstallMethod::Installer(paths) => paths.iter().any(|path| {
                let at = path.split('/').fold(home.clone(), |at, name| at.join(name));
                resolved == at || resolved.starts_with(&at)
            }),
            _ => false,
        })
    };
    let bun = || {
        documented().find(|route| match route.method {
            InstallMethod::Bun(package) => holds_run(&names, &bun_folder(package)),
            _ => false,
        })
    };
    let npm = || {
        documented().find(|route| match route.method {
            InstallMethod::Npm(package) => {
                let package_names = package.split('/').collect::<Vec<_>>();
                let within = |base: PathBuf| {
                    package_names
                        .iter()
                        .fold(base, |at, name| at.join(name))
                        .is_dir()
                };
                (holds_run(
                    &names,
                    &[&["node_modules"], package_names.as_slice()].concat(),
                ) && !holds_run(&names, &[".bun", "install", "global", "node_modules"]))
                    || folder.is_some_and(|folder| {
                        // A Unix prefix (`<prefix>/bin` beside
                        // `<prefix>/lib/node_modules`: nvm, fnm, Homebrew's
                        // Node, `~/.npm-global`), a Windows prefix
                        // (`%AppData%\npm` holding `node_modules`), and Volta,
                        // whose shims are in `bin` beside
                        // `tools/image/packages`.
                        within(folder.join("..").join("lib").join("node_modules"))
                            || within(folder.join("node_modules"))
                            || within(
                                folder
                                    .join("..")
                                    .join("tools")
                                    .join("image")
                                    .join("packages"),
                            )
                    })
            }
            _ => false,
        })
    };
    homebrew
        .or_else(winget)
        .or_else(installer)
        .or_else(bun)
        .or_else(npm)
}

fn bun_folder(package: &str) -> Vec<&str> {
    [".bun", "install", "global", "node_modules"]
        .into_iter()
        .chain(package.split('/'))
        .collect()
}

/// The path's folder and file names, in order, read lossily: every name a
/// route matches is ASCII.
fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Whether `names` holds `run` as consecutive names.
fn holds_run<S: AsRef<str>>(names: &[String], run: &[S]) -> bool {
    !run.is_empty()
        && names.windows(run.len()).any(|window| {
            window
                .iter()
                .zip(run)
                .all(|(name, wanted)| same(name, wanted.as_ref()))
        })
}

/// A name compared as the system compares it: Windows ignores case.
fn same(name: &str, wanted: &str) -> bool {
    if cfg!(windows) {
        name.eq_ignore_ascii_case(wanted)
    } else {
        name == wanted
    }
}

fn starts_with(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len())
        .is_some_and(|start| same(start, prefix))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::route;
    use crate::agents::adapter;

    /// The command the route for `agent`'s program at `program` names, in a
    /// home laid out the way one installer leaves it.
    fn command(agent: &str, program: &Path, home: &Path) -> Option<&'static str> {
        route(adapter(agent).unwrap().updates(), program, home).map(|route| route.command)
    }

    fn file(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        path.to_owned()
    }

    fn folder(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    fn link(at: &Path, to: &Path) -> PathBuf {
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(to, at).unwrap();
        at.to_owned()
    }

    /// nvm, fnm and Homebrew's Node all link `<prefix>/bin/<cli>` into
    /// `<prefix>/lib/node_modules/<package>`; Claude Code's npm package may
    /// also copy its binary into `bin`, which the folder beside it still
    /// names.
    #[cfg(unix)]
    #[test]
    fn a_cli_in_an_npm_prefix_is_updated_with_npm() {
        let home = tempfile::tempdir().unwrap();
        let prefix = home.path().join(".nvm/versions/node/v22.11.0");
        let cli = file(&prefix.join("lib/node_modules/@anthropic-ai/claude-code/cli.js"));
        let linked = link(&prefix.join("bin/claude"), &cli);
        assert_eq!(
            command("claude-code", &linked, home.path()),
            Some("npm install -g @anthropic-ai/claude-code@latest")
        );
        let copied = file(&prefix.join("bin/codex"));
        folder(&prefix.join("lib/node_modules/@openai/codex"));
        assert_eq!(
            command("codex", &copied, home.path()),
            Some("npm install -g @openai/codex")
        );
    }

    /// Windows' npm prefix (`%AppData%\npm`) holds the `.cmd` shim beside its
    /// `node_modules`, and Volta keeps its shims in `bin` beside
    /// `tools/image/packages`; neither is a link, so both read the same on
    /// every system.
    #[test]
    fn a_windows_npm_shim_and_a_volta_shim_are_updated_with_npm() {
        let home = tempfile::tempdir().unwrap();
        let npm = home.path().join("AppData/Roaming/npm");
        let shim = file(&npm.join("pi.cmd"));
        folder(&npm.join("node_modules/@earendil-works/pi-coding-agent"));
        assert_eq!(
            command("pi", &shim, home.path()),
            Some("npm install -g --ignore-scripts @earendil-works/pi-coding-agent")
        );
        let volta = home.path().join(".volta");
        let shim = file(&volta.join("bin/grok"));
        folder(&volta.join("tools/image/packages/@xai-official/grok"));
        assert_eq!(
            command("grok", &shim, home.path()),
            Some("npm install -g @xai-official/grok")
        );
    }

    /// A Homebrew link names its keg; an npm package left beside it in the
    /// same prefix is not the copy the link runs.
    #[cfg(unix)]
    #[test]
    fn a_homebrew_link_is_updated_with_homebrew_over_a_package_beside_it() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        folder(&home);
        let brew = root.path().join("opt/homebrew");
        let cask = file(&brew.join("Caskroom/claude-code/2.1.200/claude"));
        let claude = link(&brew.join("bin/claude"), &cask);
        folder(&brew.join("lib/node_modules/@anthropic-ai/claude-code"));
        assert_eq!(
            command("claude-code", &claude, &home),
            Some("brew upgrade claude-code")
        );
        let latest = file(&brew.join("Caskroom/claude-code@latest/2.1.300/claude"));
        std::fs::remove_file(&claude).unwrap();
        link(&claude, &latest);
        assert_eq!(
            command("claude-code", &claude, &home),
            Some("brew upgrade claude-code@latest")
        );
        let keg = file(&brew.join("Cellar/omp/18.7.0/bin/omp"));
        let omp = link(&brew.join("bin/omp"), &keg);
        assert_eq!(
            command("omp", &omp, &home),
            Some("brew upgrade can1357/tap/omp")
        );
        let keg = file(&brew.join("Cellar/opencode/1.18.20/bin/opencode"));
        let opencode = link(&brew.join("bin/opencode"), &keg);
        assert_eq!(
            command("opencode", &opencode, &home),
            Some("opencode upgrade")
        );
    }

    /// The vendors' own installers: Claude Code's launcher links into its
    /// versions folder, Grok's into its downloads, Pi's into its agent
    /// folder, and omp's and agy's are files in `~/.local/bin`.
    #[cfg(unix)]
    #[test]
    fn a_copy_from_the_vendors_installer_is_updated_the_vendors_way() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let version = file(&home.join(".local/share/claude/versions/2.1.200"));
        let claude = link(&home.join(".local/bin/claude"), &version);
        assert_eq!(command("claude-code", &claude, home), Some("claude update"));
        let download = file(&home.join(".grok/downloads/grok-1.0.40-macos-aarch64"));
        let grok = link(&home.join(".grok/bin/grok"), &download);
        assert_eq!(command("grok", &grok, home), Some("grok update"));
        let launcher = file(&home.join(".pi/agent/bin/pi"));
        let pi = link(&home.join(".local/bin/pi"), &launcher);
        assert_eq!(command("pi", &pi, home), Some("pi update"));
        let omp = file(&home.join(".local/bin/omp"));
        assert_eq!(command("omp", &omp, home), Some("omp update"));
        let agy = file(&home.join(".local/bin/agy"));
        assert_eq!(
            command("antigravity", &agy, home),
            Some("curl -fsSL https://antigravity.google/cli/install.sh | bash")
        );
        let release = file(&home.join(".codex/packages/standalone/releases/0.150.0/codex"));
        let codex = link(&home.join(".local/bin/codex"), &release);
        assert_eq!(
            command("codex", &codex, home),
            Some("curl -fsSL https://chatgpt.com/codex/install.sh | sh")
        );
        let opencode = file(&home.join(".opencode/bin/opencode"));
        assert_eq!(
            command("opencode", &opencode, home),
            Some("opencode upgrade")
        );
    }

    /// A Windows installer's file is matched by its place under the profile
    /// folder, and a command documented for one system only is not offered
    /// on another.
    #[test]
    fn a_route_is_offered_only_on_the_systems_it_is_documented_for() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let agy = file(&home.join("AppData/Local/agy/bin/agy.exe"));
        let claude = file(&home.join(".local/bin/claude.exe"));
        let winget = file(
            &home.join("AppData/Local/Microsoft/WinGet/Packages/Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe/claude.exe"),
        );
        let codex = file(&home.join(".codex/packages/standalone/current/codex.exe"));
        if cfg!(windows) {
            assert_eq!(
                command("antigravity", &agy, home),
                Some("irm https://antigravity.google/cli/install.ps1 | iex")
            );
            assert_eq!(
                command("claude-code", &winget, home),
                Some("winget upgrade Anthropic.ClaudeCode")
            );
            // Codex documents no update for its Windows installer.
            assert_eq!(command("codex", &codex, home), None);
        } else {
            assert_eq!(command("antigravity", &agy, home), None);
            assert_eq!(command("claude-code", &winget, home), None);
            assert_eq!(
                command("codex", &codex, home),
                Some("curl -fsSL https://chatgpt.com/codex/install.sh | sh")
            );
        }
        assert_eq!(command("claude-code", &claude, home), Some("claude update"));
    }

    /// bun's global folder is `node_modules` too, but npm does not update
    /// what bun installed.
    #[cfg(unix)]
    #[test]
    fn a_bun_global_is_updated_with_bun_and_never_with_npm() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let cli = file(
            &home.join(".bun/install/global/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js"),
        );
        let omp = link(&home.join(".bun/bin/omp"), &cli);
        assert_eq!(
            command("omp", &omp, home),
            Some("bun install -g @oh-my-pi/pi-coding-agent")
        );
        let cli = file(&home.join(".bun/install/global/node_modules/@openai/codex/bin/codex.js"));
        let codex = link(&home.join(".bun/bin/codex"), &cli);
        assert_eq!(command("codex", &codex, home), None);
    }

    /// A copy no documented way explains (a system package, pnpm, a build
    /// of one's own) gets no command at all.
    #[test]
    fn a_copy_hide_cannot_place_gets_no_command() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let pnpm = file(&home.join("Library/pnpm/codex"));
        folder(&home.join("Library/pnpm/global/5/node_modules/@openai/codex"));
        assert_eq!(command("codex", &pnpm, home), None);
        let elsewhere = file(&home.join("usr/bin/claude"));
        assert_eq!(command("claude-code", &elsewhere, home), None);
        let grok = file(&home.join(".local/bin/grok"));
        assert_eq!(command("grok", &grok, home), None);
    }
}
