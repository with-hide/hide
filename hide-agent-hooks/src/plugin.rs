//! The script files Hide owns in an agent's own extension folder: OpenCode's
//! plugin (`crate::opencode`), and the one extension Pi and omp share
//! (`crate::pi_extension`).
//!
//! These agents have no command hook. Their extension point is a module the
//! agent loads from a folder at startup and runs inside its own process, so
//! Hide's instrumentation for them is that module: it asks this crate's
//! helper (`hide-agent-hooks <agent> <operation>`, `src/plugin/helper.rs`)
//! and does nothing outside a pane Hide manages or once the helper is gone.
//!
//! The file is Hide's alone, so ownership is the file's first line:
//! `// <marker>@<version> sha256=<hash of the rest>`. A file without it is
//! someone else's and is never touched; a file whose rest no longer hashes to
//! it was edited by the operator, and stays until Reinstall. Hide never
//! creates the agent's own folder: the extension folder is made only inside
//! one the agent (or the operator) already made.
//!
//! Each Hide installation on an account writes its own files: the desktop
//! app's names are the ones earlier builds wrote, and another installation's
//! (a device helper's) carry its mark (`route::mark`). Both load in the same
//! agent, and the one whose installation did not start the pane gets nothing
//! from its helper (docs/agent-hooks.md, More than one Hide on an account).
//!
//! An agent's plugin can be more than one file (`Plugin`): OpenCode's is its
//! 1.x file and its 2.x folder's two halves. They are judged, written and
//! taken out together, so a pass never leaves one version's half beside
//! another's.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::route::HookTarget;
use hide_agent_adapter::PluginDialect;
use sha2::{Digest, Sha256};

/// The one placeholder every template carries: the helper's path, written as
/// a JavaScript string.
const HELPER_PLACEHOLDER: &str = "__HIDE_HELPER__";

/// The line of a template that names the helper.
const HELPER_LINE_START: &str = "const HELPER = ";

/// One agent's script file: where it goes, the marker that proves it is
/// Hide's, and the template it is made from.
#[derive(Debug)]
pub struct PluginFile {
    /// The agent's name in a sentence.
    pub agent: &'static str,
    /// What the agent calls the file: a plugin or an extension.
    pub noun: &'static str,
    /// The marker's name on the file's first line.
    pub source_name: &'static str,
    /// Raised when the file's text changes.
    pub version: u32,
    /// The agent whose own folder holds the file: its default account's
    /// (`AccountPath::State`), which Hide never creates, and the folder in
    /// it the agent loads scripts from (`AccountPath::Plugins`), which Hide
    /// may create.
    pub config: hide_agent_adapter::AgentId,
    /// Hide's own folder inside that one, for a plugin the agent loads as a
    /// folder: made with its first file and taken out with its last.
    pub own_folder: Option<&'static str>,
    /// The file's name inside its folder.
    pub file_name: &'static str,
    /// The script, with `__HIDE_HELPER__` where the helper's path goes.
    pub template: &'static str,
    /// Further placeholders and the JavaScript each one becomes.
    pub constants: &'static [(&'static str, &'static str)],
}

/// An agent's plugin as the kit keeps it: every file Hide writes for it, the
/// first the one Settings names.
#[derive(Debug)]
pub struct Plugin {
    pub files: &'static [PluginFile],
    /// The agent whose folder trust Hide records for the worktrees it starts
    /// it in (`folder_trust`), taken out with the plugin: switching the agent
    /// off and removing Hide from the machine both remove the plugin.
    pub trust: Option<crate::folder_trust::Agent>,
}

/// The plugin `dialect` names.
pub fn of(dialect: PluginDialect) -> &'static Plugin {
    match dialect {
        PluginDialect::OpenCode => &crate::opencode::PLUGIN,
        PluginDialect::Pi => &crate::pi_extension::PI_PLUGIN,
        PluginDialect::Omp => &crate::pi_extension::OMP_PLUGIN,
    }
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What is at the file's path, before the kit decides anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginObserved {
    /// The agent has not made its own folder, so there is nowhere to put the
    /// file yet.
    ConfigAbsent,
    Missing,
    /// This build's file for this helper.
    Current,
    /// Hide's file, unedited, but not this build's; the sentence says what is
    /// there. A pass replaces it.
    Stale(String),
    /// Hide's marker over a body that no longer hashes to it: the operator
    /// edited it. A pass leaves it and only Reinstall puts Hide's back.
    Edited,
    /// A file there that Hide did not write, by its name inside the agent's
    /// folder; it is left as it is.
    Foreign(String),
    Unreadable(String),
}

impl PluginObserved {
    /// How much a file's state decides its plugin's: the plugin reads as its
    /// worst file, so one Hide may not touch holds every file back, an edit
    /// waits for Reinstall, and an old or missing file is put right by a pass.
    fn weight(&self) -> u8 {
        match self {
            Self::Current => 0,
            Self::Missing => 1,
            Self::Stale(_) => 2,
            Self::Edited => 3,
            Self::Foreign(_) => 4,
            Self::Unreadable(_) => 5,
            Self::ConfigAbsent => 6,
        }
    }
}

impl Plugin {
    /// The file Settings names: the one the agent's own listing shows.
    pub fn entry(&self) -> &'static PluginFile {
        &self.files[0]
    }

    pub fn agent(&self) -> &'static str {
        self.entry().agent
    }

    pub fn noun(&self) -> &'static str {
        self.entry().noun
    }

    pub fn config_directory(&self, home: &Path) -> PathBuf {
        self.entry().config_directory(home)
    }

    pub fn path(&self, home: &Path, install: Option<&str>) -> PathBuf {
        self.entry().path(home, install)
    }

    /// Judges the plugin by its worst file.
    pub fn observe<'a>(&self, home: &Path, helper: impl Into<HookTarget<'a>>) -> PluginObserved {
        let helper = helper.into();
        self.files
            .iter()
            .map(|file| file.observe(home, helper))
            .max_by_key(PluginObserved::weight)
            .expect("a plugin has a file")
    }

    /// Writes this build's files once none of them refuses: a file Hide did
    /// not write anywhere in the plugin leaves every file as it was. True
    /// when a file changed.
    pub fn install<'a>(
        &self,
        home: &Path,
        helper: impl Into<HookTarget<'a>>,
    ) -> Result<bool, String> {
        let helper = helper.into();
        for file in self.files {
            file.refusal(home, &file.observe(home, helper), helper.install())?;
        }
        let mut changed = false;
        for file in self.files {
            changed |= file.install(home, helper)?;
        }
        // An earlier build of this installation wrote the app's names; its
        // files run this helper, and would answer beside the new ones.
        if helper
            .install()
            .is_some_and(|install| crate::route::mark(install).is_some())
            && self.files.iter().all(|file| file.earlier_is(home, &helper))
        {
            changed |= self.remove_files(home, None)?;
        }
        Ok(changed)
    }

    /// Takes Hide's unedited files out, and Hide's own folder once nothing is
    /// left in it. An edit makes the plugin the operator's, so every file of
    /// it stays and no half is left without the other; a file Hide did not
    /// write stays too, and so does the folder that holds it. True when a
    /// file went.
    pub fn remove(&self, home: &Path, install: Option<&str>) -> Result<bool, String> {
        // Hide's trust entries go whether or not the operator edited the
        // file: they are Hide's, and nothing else takes them out.
        let forgotten = match self.trust {
            Some(agent) => {
                crate::folder_trust::forget_all(agent, home, crate::folder_trust::Tools::none())
                    .map_err(|failure| failure.message())?
            }
            None => 0,
        };
        Ok(self.remove_files(home, install)? || forgotten > 0)
    }

    /// Takes the installation `install`'s unedited files out, unless one of
    /// them is edited, and its own folder once empty. True when a file went.
    fn remove_files(&self, home: &Path, install: Option<&str>) -> Result<bool, String> {
        for file in self.files {
            if file.owner(home, install)? == Owner::Edited {
                return Ok(false);
            }
        }
        let mut removed = false;
        for file in self.files {
            removed |= file.remove(home, install)?;
        }
        for file in self.files {
            if let Some(own) = file.own_folder {
                remove_if_empty(&file.scripts_directory(home).join(named(own, install)))?;
            }
        }
        Ok(removed)
    }
}

/// Whose the file at a plugin file's path is, read without judging its
/// version or helper.
#[derive(Debug, Eq, PartialEq)]
enum Owner {
    Absent,
    /// Hide's, as Hide wrote it.
    Hide,
    /// Hide's marker over a body that no longer hashes to it.
    Edited,
    Other,
}

/// `name` as the installation `install` writes it: as it is for the app, and
/// with the installation's mark before its extension for another.
fn named(name: &str, install: Option<&str>) -> String {
    match install.and_then(crate::route::mark) {
        None => name.to_owned(),
        Some(mark) => match name.rsplit_once('.') {
            Some((stem, extension)) => format!("{stem}-{mark}.{extension}"),
            None => format!("{name}-{mark}"),
        },
    }
}

/// Removes `folder` when it holds nothing.
fn remove_if_empty(folder: &Path) -> Result<(), String> {
    match std::fs::read_dir(folder) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Ok(());
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("{} could not be read: {error}", folder.display())),
    }
    std::fs::remove_dir(folder)
        .map_err(|error| format!("{} could not be removed: {error}", folder.display()))
}

impl PluginFile {
    /// The agent's own folder, the one Herdr's integration for it used too.
    pub fn config_directory(&self, home: &Path) -> PathBuf {
        self.config
            .adapter()
            .place_in(home, hide_agent_adapter::AccountPath::State)
    }

    /// The folder in it the agent loads scripts from.
    pub fn scripts_directory(&self, home: &Path) -> PathBuf {
        self.config
            .adapter()
            .place_in(home, hide_agent_adapter::AccountPath::Plugins)
    }

    /// Where the installation `install` keeps this file: Hide's own folder
    /// carries its mark when the plugin has one, and the file does otherwise.
    pub fn path(&self, home: &Path, install: Option<&str>) -> PathBuf {
        self.scripts_directory(home).join(self.name(install))
    }

    /// The file's name inside the agent's folder, as a sentence names it.
    pub fn name(&self, install: Option<&str>) -> String {
        match self.own_folder {
            Some(own) => format!("{}/{}", named(own, install), self.file_name),
            None => named(self.file_name, install),
        }
    }

    /// Whether the file at the app's name is an earlier build's of the
    /// installation whose helper is `helper`: Hide's, unedited, and running
    /// that helper when it names one (OpenCode 2's terminal half takes it
    /// from the server half). A missing one is too, so a plugin of several
    /// files is judged by those that are there.
    fn earlier_is(&self, home: &Path, helper: &HookTarget<'_>) -> bool {
        let Ok(text) = std::fs::read_to_string(self.path(home, None)) else {
            return true;
        };
        let (first, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
        self.marker(first)
            .is_some_and(|(_, hash)| digest(body).eq_ignore_ascii_case(hash))
            && (!self.template.contains(HELPER_PLACEHOLDER)
                || helper_of(body).is_some_and(|found| Path::new(&found) == &**helper))
    }

    fn body<'a>(&self, helper: impl Into<HookTarget<'a>>) -> String {
        let helper = helper.into();
        // A JSON string is a JavaScript string literal, whatever the path holds.
        let literal = serde_json::to_string(&helper.display().to_string())
            .expect("a string always serializes");
        // The constants go in first, so a placeholder's spelling inside the
        // helper's path is left as the path's. The global instruction guard's
        // prefilter is the guard's own (`crate::global_guard`), the same in
        // every script that asks the helper about a tool call.
        let guard = crate::global_guard::prefilter_constants();
        let constants = self
            .constants
            .iter()
            .map(|(placeholder, value)| (*placeholder, *value))
            .chain(
                guard
                    .iter()
                    .map(|(placeholder, value)| (*placeholder, value.as_str())),
            );
        constants
            .fold(self.template.to_owned(), |body, (placeholder, value)| {
                body.replacen(placeholder, value, 1)
            })
            .replacen(
                "__HIDE_ROUTE__",
                &serde_json::to_string(&helper.arguments().split_whitespace().collect::<Vec<_>>())
                    .expect("strings serialize"),
                1,
            )
            .replacen(HELPER_PLACEHOLDER, &literal, 1)
    }

    /// The exact bytes Hide writes for `helper`.
    pub fn text<'a>(&self, helper: impl Into<HookTarget<'a>>) -> String {
        let helper = helper.into();
        let body = self.body(helper);
        format!(
            "// {}@{} sha256={}\n{body}",
            self.source_name,
            self.version,
            digest(&body)
        )
    }

    /// The version and hash a first line states, when it is this file's marker.
    fn marker<'a>(&self, line: &'a str) -> Option<(u32, &'a str)> {
        let rest = line.strip_prefix("// ")?.strip_prefix(self.source_name)?;
        let (version, hash) = rest.strip_prefix('@')?.split_once(" sha256=")?;
        let valid = hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
        valid.then_some((version.parse().ok()?, hash))
    }

    /// Judges the file against the one `helper` would be named in.
    fn observe<'a>(&self, home: &Path, helper: impl Into<HookTarget<'a>>) -> PluginObserved {
        let helper = helper.into();
        if !self.config_directory(home).is_dir() {
            return PluginObserved::ConfigAbsent;
        }
        let path = self.path(home, helper.install());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return PluginObserved::Missing,
            Err(error) => {
                return PluginObserved::Unreadable(format!(
                    "{} could not be read: {error}",
                    path.display()
                ));
            }
        };
        let (first, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
        let Some((version, hash)) = self.marker(first) else {
            return PluginObserved::Foreign(self.name(helper.install()));
        };
        if !digest(body).eq_ignore_ascii_case(hash) {
            return PluginObserved::Edited;
        }
        if text == self.text(helper) {
            return PluginObserved::Current;
        }
        let what = self.noun;
        PluginObserved::Stale(match helper_of(body) {
            Some(found) if !Path::new(&found).exists() => {
                format!("the {what} points at {found}, which is gone")
            }
            _ if version != self.version => format!(
                "version {version} of the {what} is there; this build writes version {}",
                self.version
            ),
            Some(found) if Path::new(&found) != &*helper => format!(
                "the {what} runs {found}, not this build's {}",
                helper.display()
            ),
            _ => format!("the {what} is not this build's"),
        })
    }

    /// Why the file may not be written over what is there, if it may not.
    fn refusal(
        &self,
        home: &Path,
        observed: &PluginObserved,
        install: Option<&str>,
    ) -> Result<(), String> {
        match observed {
            PluginObserved::ConfigAbsent => Err(format!(
                "{} does not exist; the {} is put in once {} has made it",
                self.config_directory(home).display(),
                self.noun,
                self.agent
            )),
            PluginObserved::Foreign(_) => Err(format!(
                "{} is a file Hide did not write; Hide left it alone",
                self.path(home, install).display()
            )),
            PluginObserved::Unreadable(reason) => Err(reason.clone()),
            PluginObserved::Current
            | PluginObserved::Missing
            | PluginObserved::Stale(_)
            | PluginObserved::Edited => Ok(()),
        }
    }

    /// Writes this build's file. A file Hide did not write is never replaced,
    /// and the agent's own folder is never created. Writing the same bytes
    /// twice writes nothing; true when the file changed.
    fn install<'a>(&self, home: &Path, helper: impl Into<HookTarget<'a>>) -> Result<bool, String> {
        let helper = helper.into();
        let path = self.path(home, helper.install());
        let observed = self.observe(home, helper);
        self.refusal(home, &observed, helper.install())?;
        if observed == PluginObserved::Current {
            return Ok(false);
        }
        let folder = self.scripts_directory(home);
        for folder in std::iter::once(folder.clone()).chain(
            self.own_folder
                .map(|own| folder.join(named(own, helper.install()))),
        ) {
            match std::fs::create_dir(&folder) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!("{} could not be made: {error}", folder.display()));
                }
            }
        }
        hide_platform::fs::atomic::write_file(
            &path,
            self.text(helper).as_bytes(),
            hide_platform::fs::Access::KeepOrPrivate,
        )
        .map(|_| true)
        .map_err(|error| format!("{} could not be written: {error}", path.display()))
    }

    fn owner(&self, home: &Path, install: Option<&str>) -> Result<Owner, String> {
        let path = self.path(home, install);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Owner::Absent),
            Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
        };
        let (first, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
        Ok(match self.marker(first) {
            Some((_, hash)) if digest(body).eq_ignore_ascii_case(hash) => Owner::Hide,
            Some(_) => Owner::Edited,
            None => Owner::Other,
        })
    }

    /// Takes Hide's unedited file out; an edited one carries the operator's
    /// changes and stays, as does a file Hide did not write. True when a file
    /// went.
    fn remove(&self, home: &Path, install: Option<&str>) -> Result<bool, String> {
        if self.owner(home, install)? != Owner::Hide {
            return Ok(false);
        }
        let path = self.path(home, install);
        std::fs::remove_file(&path)
            .map(|()| true)
            .map_err(|error| format!("{} could not be removed: {error}", path.display()))
    }
}

/// The helper path a file's body names.
fn helper_of(body: &str) -> Option<String> {
    let literal = body
        .lines()
        .find_map(|line| line.strip_prefix(HELPER_LINE_START))?
        .strip_suffix(';')?;
    serde_json::from_str(literal).ok()
}

#[cfg(test)]
mod tests;
