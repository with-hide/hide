//! An agent definition file as written (schema 1): the fields every file
//! may carry. The build script reads the built-in files through this module
//! too (`build.rs`), so a built-in file the schema refuses fails the build.

use serde::Deserialize;

/// The schema this build reads.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub schema: u32,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// A built-in agent's id this file copies; user files only.
    #[serde(default)]
    pub based_on: Option<String>,
    #[serde(default)]
    pub executables: Option<Vec<String>>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
    /// The agent's own folder for hooks and sessions, under the account's
    /// home: `~/` and the folders below it.
    #[serde(default)]
    pub home: Option<String>,
    #[serde(default)]
    pub hooks: Option<Hooks>,
    #[serde(default)]
    pub start: Option<Start>,
    #[serde(default)]
    pub resume: Option<Resume>,
    /// The screen rules, read by `hide_screen_rules`; kept as a table here
    /// so the build script needs no rule grammar.
    #[serde(default)]
    pub screen: Option<toml::Table>,
}

/// Which of Hide's own code reads the agent's hooks, sessions and thread:
/// a dialect, never data. A new value comes only with the code for it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Hooks {
    Claude,
    Codex,
    Grok,
    Opencode,
    Pi,
    Omp,
    Agy,
    /// No hook, no session reader: the state is read from the screen.
    None,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    /// Words after the program's name on every start; a spawn's native
    /// arguments follow them.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resume {
    /// The words that resume a known session, with one `{session}`.
    #[serde(default)]
    pub session: Option<Vec<String>>,
    /// The words that open the folder's last conversation when no session
    /// is known.
    #[serde(default)]
    pub last: Option<Vec<String>>,
}
