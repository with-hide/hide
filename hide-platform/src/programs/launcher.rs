//! npm's Windows launcher read as the program it runs, so a start gives that
//! program its arguments itself instead of through `cmd.exe`
//! (with-hide/hide-private#573).
//!
//! For each command a package names in its `package.json` `bin`, npm writes
//! three launchers beside one another from one template: a `.cmd`, a `.ps1`
//! and a `sh` script with no extension (cmd-shim 4 to 8, npm 7 onwards).
//! The `.cmd` is the one read here, because it is the file a search by
//! `PATHEXT` finds ([`super::Program::find`]); the other two are never read.
//! It is read only when it is that template, matched whole, and what it
//! names is checked against the package's own `bin`; any other launcher is
//! refused with the reason, never guessed at. pnpm's and Yarn's launchers
//! (another template, `@zkochan/cmd-shim`) set `NODE_PATH` and sometimes
//! `PATH` for the program, which a start without them would not give it, so
//! they are refused too, as is npm 6's older template.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use super::Start;
use crate::host;

/// The lines npm's launcher begins with.
const HEAD: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n";

/// What follows [`HEAD`] in the launcher of a script run by Node, up to the
/// options its first line gives Node: Node beside the launcher, else the
/// first on the `PATH` with `.JS` taken out of `PATHEXT`, then the line that
/// runs it.
const NODE: &str = "\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\" ";

/// How the launcher's last line ends: the caller's arguments.
const TAIL: &str = " %*\r\n";

/// How the target the launcher names begins: its own folder.
const TARGET: &str = "\"%dp0%\\";

/// The longest launcher read; npm's is under 400 bytes.
const LAUNCHER_LIMIT: u64 = 16 * 1024;

/// The longest `package.json` read.
const PACKAGE_LIMIT: u64 = 1024 * 1024;

/// The program the npm launcher `launcher` runs, with the words it puts
/// before the caller's arguments; `extensions` is `PATHEXT`, which picks the
/// Node the launcher would find on `search`. `Err` says why it is not read.
pub(super) fn resolve(launcher: &Path, search: &OsStr, extensions: &str) -> Result<Start, String> {
    let text = read_capped(launcher, LAUNCHER_LIMIT, "the launcher")?;
    let folder = launcher
        .parent()
        .ok_or_else(|| "the launcher has no folder".to_owned())?;
    let body = text
        .strip_prefix(HEAD)
        .ok_or_else(|| "the launcher is not npm's".to_owned())?;
    let (program, leading, target) = if let Some(line) = body.strip_prefix(NODE) {
        let line = line
            .strip_suffix(TAIL)
            .ok_or_else(|| "the launcher is not npm's".to_owned())?;
        let at = line
            .rfind(&format!(" {TARGET}"))
            .ok_or_else(|| "the launcher names no script".to_owned())?;
        let target = target_of(folder, &line[at + 1..])?;
        let mut leading = node_options(&line[..at])?;
        leading.push(target.clone().into_os_string());
        (node(folder, search, extensions)?, leading, target)
    } else if body.starts_with("@SET ") {
        return Err("the launcher sets variables for its program".to_owned());
    } else {
        // A target with no first line naming a program: npm runs the file
        // itself, which only an executable takes without `cmd.exe`.
        let line = body
            .strip_suffix(TAIL)
            .and_then(|line| line.strip_suffix("  "))
            .ok_or_else(|| "the launcher is not npm's".to_owned())?;
        let target = target_of(folder, line)?;
        if !has_extension(&target, &["exe", "com"]) {
            return Err("the launcher runs a file that is not a program".to_owned());
        }
        (target.clone(), Vec::new(), target)
    };
    if !target.is_file() {
        return Err("the launcher names a file that is not there".to_owned());
    }
    confirm_bin(launcher, &target)?;
    Ok(Start::Direct { program, leading })
}

/// The file `quoted` (`"%dp0%\<relative path>"`) names from `folder`.
fn target_of(folder: &Path, quoted: &str) -> Result<PathBuf, String> {
    let relative = quoted
        .strip_prefix(TARGET)
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| "the launcher's target is not quoted as npm quotes it".to_owned())?;
    if relative.is_empty()
        || relative
            .chars()
            .any(|c| matches!(c, '"' | '%' | '!' | '^') || c.is_control())
    {
        return Err("the launcher's target holds characters cmd.exe reads".to_owned());
    }
    let mut path = folder.to_path_buf();
    for name in relative.split('\\').filter(|name| !name.is_empty()) {
        path.push(name);
    }
    Ok(normalized(&path))
}

/// The options the script's first line gives Node, each a word with
/// nothing in it that `cmd.exe` reads.
fn node_options(options: &str) -> Result<Vec<OsString>, String> {
    options
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(|word| {
            if word.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=' | '.' | ',' | ':' | '/')
            }) {
                Ok(OsString::from(word))
            } else {
                Err("the launcher gives Node an option cmd.exe reads".to_owned())
            }
        })
        .collect()
}

/// The Node the launcher runs: `node.exe` beside it, else the first `node`
/// on `search` by `extensions` without `.JS`, as the launcher's own
/// `PATHEXT` change leaves them; only an executable is taken.
fn node(folder: &Path, search: &OsStr, extensions: &str) -> Result<PathBuf, String> {
    let beside = folder.join("node.exe");
    if beside.is_file() {
        return Ok(beside);
    }
    let extensions = without_js(extensions);
    let names = host::names_with_extensions("node", &extensions);
    let found = std::env::split_paths(search)
        .filter(|folder| folder.is_absolute())
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| host::is_program(candidate))
        .ok_or_else(|| "the launcher's Node is not on the search".to_owned())?;
    if !has_extension(&found, &["exe", "com"]) {
        return Err("the launcher's Node is not an executable".to_owned());
    }
    Ok(found)
}

/// `extensions` with the launcher's `%PATHEXT:;.JS;=;%` applied: each
/// `;.JS;`, in any case, becomes `;`, left to right as `cmd.exe` replaces.
fn without_js(extensions: &str) -> String {
    // ASCII lowering keeps every byte where it was.
    let lower = extensions.to_ascii_lowercase();
    let mut out = String::with_capacity(extensions.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(";.js;") {
        out.push_str(&extensions[at..at + found]);
        out.push(';');
        at += found + ";.js;".len();
    }
    out.push_str(&extensions[at..]);
    out
}

/// Whether the package `target` belongs to names it in its `package.json`
/// `bin` as the command the launcher is named for: npm's own record of what
/// the launcher runs, so a launcher edited by hand, or left over from
/// another version of its package, is not taken for it.
fn confirm_bin(launcher: &Path, target: &Path) -> Result<(), String> {
    let command = launcher
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "the launcher's name is not text".to_owned())?;
    let spelled = target
        .to_str()
        .ok_or_else(|| "the launcher's target is not text".to_owned())?;
    let package = crate::process::package_of_script(spelled)
        .ok_or_else(|| "the launcher's target is in no package".to_owned())?;
    let folder = package_folder(target, &package)
        .ok_or_else(|| "the launcher's target is in no package".to_owned())?;
    let manifest = read_capped(&folder.join("package.json"), PACKAGE_LIMIT, "package.json")?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|_| "the package's package.json is not JSON".to_owned())?;
    let named = match manifest.get("bin") {
        Some(serde_json::Value::String(path)) => {
            let name = package.rsplit('/').next().unwrap_or(&package);
            name.eq_ignore_ascii_case(command).then_some(path.as_str())
        }
        Some(serde_json::Value::Object(bins)) => bins
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(command))
            .and_then(|(_, path)| path.as_str()),
        _ => None,
    }
    .ok_or_else(|| "the package's bin does not name the launcher's command".to_owned())?;
    let mut declared = folder.clone();
    for name in named.split(['/', '\\']).filter(|name| !name.is_empty()) {
        declared.push(name);
    }
    if same_file_name(&normalized(&declared), target) {
        Ok(())
    } else {
        Err("the package's bin names another file than the launcher".to_owned())
    }
}

/// The folder of `package` (`name` or `@scope/name`) that holds `target`:
/// the path up to its last `node_modules` and the package's own names.
fn package_folder(target: &Path, package: &str) -> Option<PathBuf> {
    let parts: Vec<Component<'_>> = target.components().collect();
    let at = parts
        .iter()
        .rposition(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))?;
    let mut folder: PathBuf = parts[..=at].iter().collect();
    for name in package.split('/') {
        folder.push(name);
    }
    Some(folder)
}

/// `path` with each `.` dropped and each `..` taking the name before it.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

/// Whether `a` and `b` spell one path, names compared as Windows compares
/// them, without case.
fn same_file_name(a: &Path, b: &Path) -> bool {
    let (a, b): (Vec<_>, Vec<_>) = (a.components().collect(), b.components().collect());
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(a, b)| a.as_os_str().eq_ignore_ascii_case(b.as_os_str()))
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|wanted| extension.eq_ignore_ascii_case(wanted))
        })
}

fn read_capped(path: &Path, limit: u64, what: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("{what} could not be opened ({:?})", error.kind()))?;
    let mut text = String::new();
    file.take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{what} could not be read ({:?})", error.kind()))?;
    if text.len() as u64 > limit {
        return Err(format!("{what} is longer than {limit} bytes"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Launchers as the package managers wrote them, into `node_modules/.bin`
    // of an install laid out as `install` lays it out: npm 11's cmd-shim
    // 8.0.0 (`npm-*`), npm 6's cmd-shim 3.0.3 (`npm6-*`) and pnpm's
    // @zkochan/cmd-shim 7.0.0 (`pnpm-*`).

    /// `@openai/codex`'s `bin/codex.js`, first line `#!/usr/bin/env node`.
    const SCRIPT: &str = include_str!("../../tests/fixtures/launchers/npm-script.cmd");
    /// `@earendil-works/pi-coding-agent`'s `dist/bundle/options.js`, first
    /// line `#!/usr/bin/env node --no-warnings=ExperimentalWarning
    /// --max-old-space-size=4096`.
    const NODE_OPTIONS: &str = include_str!("../../tests/fixtures/launchers/npm-node-options.cmd");
    /// `@anthropic-ai/claude-code`'s `bin/claude.exe`.
    const EXECUTABLE: &str = include_str!("../../tests/fixtures/launchers/npm-executable.cmd");
    /// `@vendor/env-tool`'s `bin/env.js`, first line
    /// `#!/usr/bin/env -S FORCE_COLOR=1 node`.
    const ENV: &str = include_str!("../../tests/fixtures/launchers/npm-env.cmd");
    /// `SCRIPT`'s script, by npm 6.
    const NPM6: &str = include_str!("../../tests/fixtures/launchers/npm6-script.cmd");
    /// `SCRIPT`'s script, by pnpm with its `NODE_PATH`.
    const PNPM: &str = include_str!("../../tests/fixtures/launchers/pnpm-script.cmd");

    const CODEX: &str = "..\\@openai\\codex\\bin\\codex.js";

    /// `text` with `from` replaced by `to`, `from` required to be there so a
    /// variant never silently stays the launcher it was made from.
    fn with(text: &str, from: &str, to: &str) -> String {
        assert!(text.contains(from), "{from:?} is not in the launcher");
        text.replace(from, to)
    }

    struct Install {
        _root: tempfile::TempDir,
        bin: PathBuf,
        package: PathBuf,
        nodes: PathBuf,
    }

    /// A local npm install of `package` whose `package.json` `bin` is `bin`,
    /// with its launchers folder, and a folder for a Node on the search.
    fn install(package: &str, bin: serde_json::Value) -> Install {
        let root = tempfile::tempdir().unwrap();
        let modules = root.path().join("node_modules");
        let folder = package
            .split('/')
            .fold(modules.clone(), |folder, name| folder.join(name));
        std::fs::create_dir_all(folder.join("bin")).unwrap();
        std::fs::write(
            folder.join("package.json"),
            serde_json::json!({"name": package, "bin": bin}).to_string(),
        )
        .unwrap();
        let bin = modules.join(".bin");
        std::fs::create_dir_all(&bin).unwrap();
        let nodes = root.path().join("nodejs");
        std::fs::create_dir_all(&nodes).unwrap();
        Install {
            bin,
            package: folder,
            nodes,
            _root: root,
        }
    }

    fn file(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    const EXTENSIONS: &str = ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WSF";

    /// A script's launcher runs the first Node on the search, an
    /// executable, with the script as its first word; `node.js` in an
    /// earlier folder is passed over as the launcher's `PATHEXT` passes it.
    #[test]
    fn a_script_launcher_runs_the_node_on_the_search_with_its_script() {
        let install = install(
            "@openai/codex",
            serde_json::json!({"codex": "bin/codex.js"}),
        );
        file(
            &install.package.join("bin/codex.js"),
            "#!/usr/bin/env node\n",
        );
        let launcher = install.bin.join("codex.cmd");
        file(&launcher, SCRIPT);
        let decoy = install.bin.parent().unwrap().join("decoy");
        std::fs::create_dir_all(&decoy).unwrap();
        file(&decoy.join("node.js"), "");
        file(&install.nodes.join("node.exe"), "");
        let search = std::env::join_paths([&decoy, &install.nodes]).unwrap();
        assert_eq!(
            resolve(&launcher, &search, EXTENSIONS),
            Ok(Start::Direct {
                program: install.nodes.join("node.exe"),
                leading: vec![
                    install
                        .package
                        .join("bin")
                        .join("codex.js")
                        .into_os_string()
                ],
            })
        );
    }

    /// Node beside the launcher wins over the search, and the options the
    /// script's first line gives Node come before the script, as given.
    #[test]
    fn node_beside_the_launcher_runs_with_the_scripts_options() {
        let install = install(
            "@earendil-works/pi-coding-agent",
            serde_json::json!({"pi": "dist/bundle/options.js"}),
        );
        std::fs::create_dir_all(install.package.join("dist/bundle")).unwrap();
        file(&install.package.join("dist/bundle/options.js"), "");
        let launcher = install.bin.join("pi.cmd");
        file(&launcher, NODE_OPTIONS);
        file(&install.bin.join("node.exe"), "");
        file(&install.nodes.join("node.exe"), "");
        assert_eq!(
            resolve(&launcher, install.nodes.as_os_str(), EXTENSIONS),
            Ok(Start::Direct {
                program: install.bin.join("node.exe"),
                leading: vec![
                    "--no-warnings=ExperimentalWarning".into(),
                    "--max-old-space-size=4096".into(),
                    install
                        .package
                        .join("dist")
                        .join("bundle")
                        .join("options.js")
                        .into_os_string(),
                ],
            })
        );
    }

    /// A package whose command is an executable has a launcher that runs
    /// it, and the start runs that executable with no Node.
    #[test]
    fn an_executable_target_runs_itself() {
        let install = install(
            "@anthropic-ai/claude-code",
            serde_json::json!({"claude": "bin/claude.exe"}),
        );
        file(&install.package.join("bin/claude.exe"), "");
        let launcher = install.bin.join("claude.cmd");
        file(&launcher, EXECUTABLE);
        assert_eq!(
            resolve(&launcher, OsStr::new(""), EXTENSIONS),
            Ok(Start::Direct {
                program: install.package.join("bin").join("claude.exe"),
                leading: Vec::new(),
            })
        );
    }

    /// Whatever is not npm's template, or names what its package does not,
    /// is refused with the reason and never guessed at.
    #[test]
    fn a_launcher_npm_did_not_write_as_its_package_says_is_refused() {
        let install = install(
            "@openai/codex",
            serde_json::json!({"codex": "bin/codex.js"}),
        );
        file(&install.package.join("bin/codex.js"), "");
        file(&install.package.join("bin/other.js"), "");
        file(&install.package.join("bin/tool.js"), "");
        file(&install.nodes.join("node.exe"), "");
        let search = install.nodes.as_os_str();
        let refused = |name: &str, text: &str| {
            let launcher = install.bin.join(name);
            file(&launcher, text);
            resolve(&launcher, search, EXTENSIONS).unwrap_err()
        };
        let not_npms = "the launcher is not npm's";
        assert_eq!(refused("codex.cmd", PNPM), not_npms);
        assert_eq!(refused("codex.cmd", NPM6), not_npms);
        // A wrapper that sets variables and calls another program, as the
        // agent journey's own used to.
        assert_eq!(
            refused(
                "codex.cmd",
                "@echo off\r\nset \"HOME=C:\\h\"\r\n\"C:\\npm\\codex.cmd\" %*\r\n"
            ),
            not_npms
        );
        assert_eq!(
            refused("codex.cmd", &SCRIPT.replace("\r\n", "\n")),
            not_npms
        );
        assert_eq!(
            refused("codex.cmd", ENV),
            "the launcher sets variables for its program"
        );
        assert_eq!(
            refused(
                "codex.cmd",
                &with(SCRIPT, "\"%_prog%\"  ", "\"%_prog%\" --title=%USERNAME% ")
            ),
            "the launcher gives Node an option cmd.exe reads"
        );
        assert_eq!(
            refused("codex.cmd", &with(SCRIPT, "codex.js\"", "%X%.js\"")),
            "the launcher's target holds characters cmd.exe reads"
        );
        assert_eq!(
            refused("codex.cmd", &with(SCRIPT, "codex.js\"", "gone.js\"")),
            "the launcher names a file that is not there"
        );
        assert_eq!(
            refused("codex.cmd", &with(SCRIPT, "codex.js\"", "other.js\"")),
            "the package's bin names another file than the launcher"
        );
        assert_eq!(
            refused("tool.cmd", &with(SCRIPT, "codex.js\"", "tool.js\"")),
            "the package's bin does not name the launcher's command"
        );
        assert_eq!(
            refused(
                "codex.cmd",
                &with(
                    EXECUTABLE,
                    "..\\@anthropic-ai\\claude-code\\bin\\claude.exe",
                    CODEX
                )
            ),
            "the launcher runs a file that is not a program"
        );
    }

    /// A Node the launcher would find that is not an executable is refused,
    /// as is a search with none.
    #[test]
    fn a_launcher_whose_node_is_no_executable_is_refused() {
        let install = install(
            "@openai/codex",
            serde_json::json!({"codex": "bin/codex.js"}),
        );
        file(&install.package.join("bin/codex.js"), "");
        let launcher = install.bin.join("codex.cmd");
        file(&launcher, SCRIPT);
        assert_eq!(
            resolve(&launcher, install.nodes.as_os_str(), EXTENSIONS),
            Err("the launcher's Node is not on the search".to_owned())
        );
        file(&install.nodes.join("node.cmd"), "");
        assert_eq!(
            resolve(&launcher, install.nodes.as_os_str(), EXTENSIONS),
            Err("the launcher's Node is not an executable".to_owned())
        );
    }

    /// The launcher's `%PATHEXT:;.JS;=;%` takes `.JS` out only between two
    /// separators, in any case.
    #[test]
    fn only_a_js_between_separators_leaves_pathext() {
        assert_eq!(without_js(".COM;.EXE;.js;.CMD"), ".COM;.EXE;.CMD");
        assert_eq!(without_js(".JS;.EXE;.JS"), ".JS;.EXE;.JS");
        assert_eq!(without_js(".A;.JS;.JS;.B"), ".A;.JS;.B");
        assert_eq!(without_js(".EXE"), ".EXE");
    }
}
