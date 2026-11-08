//! An agent definition file as written (schema 1): the fields every file
//! may carry and the checks each field passes. The build script reads the
//! built-in files through [`read`] too (`build.rs`), so a built-in file that
//! fails a check here fails the build, and the operator's files are held to
//! the same checks at run time (`super::read_file`). Only the screen rules
//! are compiled outside, by the reader that links `hide_screen_rules`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The schema this build reads.
pub const SCHEMA: u32 = 1;
/// The most words a line typed into a pane's shell holds, its program
/// included, and the most bytes its words hold: the resume command
/// limits of Herdr's `validate_resume_argv`, which the engine's start line
/// keeps. A definition's words are checked against them here and the line a
/// start or resume types against them in the core's own engine (`engine/own/shell_line.rs`, `check_resume`).
pub const LINE_WORDS: usize = 64;
pub const LINE_BYTES: usize = 8 * 1024;
/// The most words a start or a resume adds after the program's name.
const MAX_WORDS: usize = LINE_WORDS - 1;
const SESSION: &str = "{session}";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
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
    pub packages: Option<Vec<String>>,
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
    /// Where the agent keeps an account's folders and which variables move
    /// them ([`AccountRule`]).
    #[serde(default)]
    pub account: Option<AccountFile>,
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
    pub args: Option<Vec<String>>,
    /// The id of the `[screen]` rule, an idle one, that shows the program
    /// ready for its first input: a start is typed into only once the screen
    /// has matched it (`docs/ARCHITECTURE.md`, Agents on the core's own
    /// engine). For an agent that reads keys and draws its input before it
    /// takes a line, as OpenCode 2 does before its model loads.
    #[serde(default)]
    pub ready: Option<String>,
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

/// Which files a definition comes from: what each may say differs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// `agents/*.toml` in this crate.
    Builtin,
    /// The operator's folder.
    User,
}

/// A file parsed and every field it carries checked, on its own, before it
/// is set beside the others; the screen rules are still a table.
#[derive(Debug)]
pub struct Fields {
    pub id: String,
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub executables: Option<Vec<String>>,
    pub packages: Option<Vec<String>>,
    pub aliases: Option<Vec<String>>,
    /// The agent's own folder under the account's home, as its parts.
    pub home: Option<Vec<String>>,
    pub hooks: Option<Hooks>,
    pub start_args: Option<Vec<String>>,
    pub start_ready: Option<String>,
    pub resume_session: Option<Vec<String>>,
    pub resume_last: Option<Vec<String>>,
    pub account: Option<AccountRule>,
    pub screen: Option<toml::Table>,
}

/// Parse one file and check every field it carries: the one reading of a
/// definition, for built-in files and the operator's alike. A refusal is one
/// line that starts with the file's `name`.
pub fn read(name: &str, text: &str, origin: Origin) -> Result<Fields, String> {
    let file: File = toml::from_str(text).map_err(|error| {
        let error = error.to_string();
        let line = error.lines().next().unwrap_or_default().to_owned();
        format!("{name}: not a definition file: {line}")
    })?;
    if file.schema != SCHEMA {
        return Err(format!(
            "{name}: schema {} is not one this Hide reads (it reads {SCHEMA})",
            file.schema
        ));
    }
    if !valid_id(&file.id) {
        return Err(format!(
            "{name}: id {:?} is not 1 to 32 lowercase letters, digits and dashes",
            file.id
        ));
    }
    if origin == Origin::Builtin && file.based_on.is_some() {
        return Err(format!("{name}: based_on is for the operator's files"));
    }
    if let Some(display) = &file.name
        && (display.trim().is_empty()
            || display.chars().count() > 64
            || display.chars().any(char::is_control))
    {
        return Err(format!("{name}: name must be 1 to 64 printable characters"));
    }
    if let Some(executables) = &file.executables {
        check_executables(name, executables)?;
    }
    if let Some(packages) = &file.packages {
        if packages.len() > 16 {
            return Err(format!("{name}: packages holds more than 16 names"));
        }
        if let Some(package) = packages.iter().find(|package| !valid_package(package)) {
            return Err(format!(
                "{name}: package {package:?} is not a full npm package name"
            ));
        }
    }
    if let Some(aliases) = &file.aliases
        && let Some(alias) = aliases.iter().find(|alias| !valid_alias(alias))
    {
        return Err(format!(
            "{name}: alias {alias:?} is not 1 to 32 lowercase letters, digits, dashes and underscores"
        ));
    }
    let home = file
        .home
        .as_deref()
        .map(|home| home_parts(name, home))
        .transpose()?;
    let start = file.start.unwrap_or_default();
    if let Some(args) = &start.args {
        check_words(name, "start.args", args, 0)?;
    }
    let resume = file.resume.unwrap_or_default();
    if let Some(words) = &resume.session {
        check_words(name, "resume.session", words, 1)?;
    }
    if let Some(words) = &resume.last {
        check_words(name, "resume.last", words, 0)?;
    }
    let account = file
        .account
        .map(|account| account_rule(name, account))
        .transpose()?;
    Ok(Fields {
        id: file.id,
        name: file.name,
        based_on: file.based_on,
        executables: file.executables,
        packages: file.packages,
        aliases: file.aliases,
        home,
        hooks: file.hooks,
        start_args: start.args,
        start_ready: start.ready,
        resume_session: resume.session,
        resume_last: resume.last,
        account,
        screen: file.screen,
    })
}

/// A field a definition of its own must carry: a built-in file, or an
/// operator's file for a new agent.
pub fn required<T>(name: &str, field: &str, value: Option<T>) -> Result<T, String> {
    value.ok_or_else(|| format!("{name}: {field} is missing"))
}

fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
}

fn valid_alias(alias: &str) -> bool {
    (1..=32).contains(&alias.len())
        && alias.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// Full registry names, optionally scoped, never paths or version ranges.
fn valid_package(package: &str) -> bool {
    fn part(value: &str) -> bool {
        !value.is_empty()
            && !value.starts_with(['.', '_'])
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._~".contains(&byte)
            })
    }
    if package.len() > 214 {
        return false;
    }
    match package.strip_prefix('@') {
        Some(scoped) => scoped
            .split_once('/')
            .is_some_and(|(scope, name)| part(scope) && part(name)),
        None => part(package),
    }
}

/// Programs are compared by their file's name, so none is a path or holds a
/// space; the first is what a start types into the pane's shell, so it is a
/// plain command name, as the resume check
/// (Herdr's `validate_resume_argv`) holds it.
fn check_executables(name: &str, executables: &[String]) -> Result<(), String> {
    let Some(first) = executables.first() else {
        return Err(format!("{name}: executables is empty"));
    };
    if let Some(bad) = executables.iter().find(|executable| {
        executable.is_empty()
            || executable.len() > 64
            || executable.contains(['/', '\\'])
            || executable
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
    }) {
        return Err(format!("{name}: program {bad:?} is not a file name"));
    }
    if !is_plain_command(first) {
        return Err(format!(
            "{name}: the first program {first:?} is started by name and must be a plain command name"
        ));
    }
    Ok(())
}

/// Words a start or a resume types after the program: the resume
/// command rules of Herdr's `validate_resume_argv` (no control character,
/// no apostrophe, 64 words and 8 KiB with the program), and `{session}` exactly where the field takes it.
fn check_words(name: &str, field: &str, words: &[String], sessions: usize) -> Result<(), String> {
    if words.len() > MAX_WORDS || words.iter().map(String::len).sum::<usize>() > LINE_BYTES {
        return Err(format!(
            "{name}: {field} holds more than {MAX_WORDS} words or {LINE_BYTES} bytes"
        ));
    }
    if !words.iter().all(|word| is_typable_word(word)) {
        return Err(format!(
            "{name}: {field} holds an apostrophe or a control character"
        ));
    }
    let found = words
        .iter()
        .map(|word| word.matches(SESSION).count())
        .sum::<usize>();
    if found != sessions {
        return Err(if sessions == 0 {
            format!("{name}: {field} cannot name {SESSION}")
        } else {
            format!("{name}: {field} names {SESSION} {found} times, not once")
        });
    }
    Ok(())
}

fn home_parts(name: &str, home: &str) -> Result<Vec<String>, String> {
    let parts: Option<Vec<String>> = home
        .strip_prefix("~/")
        .map(|rest| rest.split('/').map(str::to_owned).collect());
    match parts {
        Some(parts)
            if parts.iter().all(|part| {
                !part.is_empty() && part != "." && part != ".." && !part.contains('\\')
            }) =>
        {
            Ok(parts)
        }
        _ => Err(format!(
            "{name}: home {home:?} is not a folder under the home folder, written ~/<folder>"
        )),
    }
}

/// Whether `program` can be typed as a command by its name in every shell a
/// pane runs: letters, digits, `_`, `-` and `.`, not starting with `-`
/// (shells disagree on how to run a quoted path).
pub fn is_plain_command(program: &str) -> bool {
    !program.is_empty()
        && !program.starts_with('-')
        && program
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Whether `word` can be typed into a pane's shell inside the quotes every
/// shell's start line puts it in: no control character and no apostrophe.
pub fn is_typable_word(word: &str) -> bool {
    !word.contains('\'') && !word.chars().any(char::is_control)
}

/// The `[account]` of a definition file alone: its `[account]` tables, cut
/// from the file by [`account_section`], read through the same checks as
/// [`read`] does. For an asker that cannot wait for the whole file.
pub fn read_account(name: &str, text: &str) -> Result<Option<AccountRule>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Only {
        #[serde(default)]
        account: Option<AccountFile>,
    }
    let only: Only = toml::from_str(&account_section(text)).map_err(|error| {
        let error = error.to_string();
        let line = error.lines().next().unwrap_or_default().to_owned();
        format!("{name}: not a definition file: {line}")
    })?;
    only.account
        .map(|account| account_rule(name, account))
        .transpose()
}

/// The lines of a definition file's `[account]` and `[account.*]` tables.
pub fn account_section(text: &str) -> String {
    let mut inside = false;
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            inside = trimmed.starts_with("[account]") || trimmed.starts_with("[account.");
        }
        if inside {
            section.push_str(line);
            section.push('\n');
        }
    }
    section
}

/// How an agent tells one account from another: a folder its variables
/// name, a name of its own (omp's `OMP_PROFILE`), or not at all.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum By {
    Folder,
    Name,
    None,
}

/// What an account's folders are for, the one closed list every caller
/// asks [`AccountRule`] by; a definition names each one, `false` when the
/// agent has no such place.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccountPath {
    /// The file Hide's hooks go into.
    Hooks,
    /// The file the agent keeps its approval of hooks in.
    HookTrust,
    /// The folder Hide's plugin or extension file goes into.
    Plugins,
    /// The folder Hide's skill goes into; `"shared"` for the folder every
    /// agent reads (`~/.agents/skills`), which no profile owns.
    Skills,
    /// The file the agent keeps its trust of folders in.
    FolderTrust,
    /// Where the agent keeps its sessions: a folder, or OpenCode's database.
    Sessions,
    /// The file that holds the account's sign-in.
    Login,
    /// The agent's own folder, whose presence says the agent is set up.
    State,
    /// What a usage read opens beside the sessions.
    Usage,
}

impl AccountPath {
    pub const ALL: [Self; 9] = [
        Self::Hooks,
        Self::HookTrust,
        Self::Plugins,
        Self::Skills,
        Self::FolderTrust,
        Self::Sessions,
        Self::Login,
        Self::State,
        Self::Usage,
    ];

    /// The key a definition writes under `[account.paths]`.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Hooks => "hooks",
            Self::HookTrust => "hook_trust",
            Self::Plugins => "plugins",
            Self::Skills => "skills",
            Self::FolderTrust => "folder_trust",
            Self::Sessions => "sessions",
            Self::Login => "login",
            Self::State => "state",
            Self::Usage => "usage",
        }
    }
}

/// Where a template starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Base {
    /// The account's home folder, `~`.
    Home,
    /// The profile's account folder, `{account}`.
    Account,
    /// The value of one of the definition's account variables, `{NAME}`.
    Var(String),
}

/// A place written in a definition: where it starts and the folder names
/// below it, each a plain name or `{account}` (an account named by name,
/// set into a folder).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    pub base: Base,
    pub parts: Vec<String>,
}

/// The `{account}` part of a [`Template`].
pub const ACCOUNT_PART: &str = "{account}";

/// One variable that moves an account's folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountVar {
    pub name: String,
    /// Where the agent puts the folder when the variable is unset: what the
    /// built-in default profile reads, never set into its environment.
    /// `None` for a variable whose absence is the agent's default.
    pub default: Option<Template>,
    /// The value a profile sets, from its account.
    pub profile: Template,
    /// Set to [`Self::default`] below a home a node makes for itself
    /// (`hide_host::serve::agent_home_variables`), rather than left to the
    /// account's environment: Codex on Windows reads the account's profile
    /// folder from Windows, not from the home it is given.
    pub isolated_home: bool,
}

/// One feature's place.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathRule {
    /// The agent has no such place (`false`).
    Absent,
    /// The folder every agent reads, which no profile owns (`"shared"`).
    Shared,
    /// One template for every profile.
    Same(Template),
    /// A template for the built-in default profile and one for the others.
    Split {
        default: Template,
        profile: Template,
    },
}

/// A definition's `[account]`: how its accounts are told apart, the
/// variables that move them, each feature's place and the variables that
/// would move a place behind the account's back. `hide_agent_adapter::resolve`
/// reads it; nothing else names an agent's folders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountRule {
    pub by: By,
    /// Why an agent of `by = "none"` has no profile of its own account.
    pub reason: Option<String>,
    pub vars: Vec<AccountVar>,
    /// Every feature of [`AccountPath::ALL`], in that order.
    pub paths: Vec<(AccountPath, PathRule)>,
    /// Variables a profile's own environment may not set.
    pub forbid: Vec<String>,
}

impl AccountRule {
    pub fn path(&self, feature: AccountPath) -> &PathRule {
        &self
            .paths
            .iter()
            .find(|(each, _)| *each == feature)
            .expect("an account rule names every feature")
            .1
    }

    pub fn var(&self, name: &str) -> Option<&AccountVar> {
        self.vars.iter().find(|var| var.name == name)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountFile {
    by: By,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, VarFile>,
    paths: PathsFile,
    #[serde(default)]
    forbid: Option<ForbidFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VarFile {
    #[serde(default)]
    default: Option<String>,
    profile: String,
    #[serde(default)]
    isolated_home: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForbidFile {
    vars: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathsFile {
    hooks: PathFile,
    hook_trust: PathFile,
    plugins: PathFile,
    skills: PathFile,
    folder_trust: PathFile,
    sessions: PathFile,
    login: PathFile,
    state: PathFile,
    usage: PathFile,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PathFile {
    Off(bool),
    Text(String),
    Split { default: String, profile: String },
}

/// The most variables an account rule names, and the longest reason.
const MAX_ACCOUNT_VARS: usize = 8;
const MAX_REASON_CHARS: usize = 200;

fn account_rule(name: &str, file: AccountFile) -> Result<AccountRule, String> {
    let fail = |what: String| format!("{name}: account{what}");
    match (file.by, &file.reason) {
        (By::None, None) => return Err(fail(": by = \"none\" says why in reason".into())),
        (By::None, Some(reason))
            if reason.trim().is_empty()
                || reason.chars().count() > MAX_REASON_CHARS
                || reason.chars().any(char::is_control) =>
        {
            return Err(fail(format!(
                ".reason must be 1 to {MAX_REASON_CHARS} printable characters"
            )));
        }
        (By::Folder | By::Name, Some(_)) => {
            return Err(fail(".reason is for by = \"none\" only".into()));
        }
        _ => {}
    }
    if file.vars.len() > MAX_ACCOUNT_VARS {
        return Err(fail(format!(".vars names more than {MAX_ACCOUNT_VARS}")));
    }
    if file.by == By::None && !file.vars.is_empty() {
        return Err(fail(
            ".vars is empty for by = \"none\": no variable moves the account".into(),
        ));
    }
    if file.by != By::None && file.vars.is_empty() {
        return Err(fail(
            ".vars names the variable that moves the account".into(),
        ));
    }
    let declared: Vec<(String, bool)> = file
        .vars
        .iter()
        .map(|(var, spec)| (var.clone(), spec.default.is_some()))
        .collect();
    let mut vars = Vec::with_capacity(file.vars.len());
    for (var, spec) in file.vars {
        if !valid_variable(&var) || reserved_variable(&var) {
            return Err(fail(format!(
                ".vars: {var:?} is not an agent's own variable name"
            )));
        }
        let at = format!(".vars.{var}");
        let default = spec
            .default
            .as_deref()
            .map(|text| template(text, &[], Side::Default, file.by))
            .transpose()
            .map_err(|why| fail(format!("{at}.default {why}")))?;
        if default
            .as_ref()
            .is_some_and(|default| default.base != Base::Home)
        {
            return Err(fail(format!("{at}.default starts at ~")));
        }
        if spec.isolated_home && default.is_none() {
            return Err(fail(format!(
                "{at}.isolated_home needs a default to set below the home"
            )));
        }
        // A folder account's variable is its folder or a folder in it; a
        // named account's is its name.
        let profile = match file.by {
            By::Name if spec.profile == ACCOUNT_PART => Template {
                base: Base::Account,
                parts: Vec::new(),
            },
            By::Name => return Err(fail(format!("{at}.profile is {ACCOUNT_PART}, the name"))),
            _ => template(&spec.profile, &[], Side::Profile, file.by)
                .map_err(|why| fail(format!("{at}.profile {why}")))?,
        };
        if profile.base != Base::Account {
            return Err(fail(format!("{at}.profile starts at {ACCOUNT_PART}")));
        }
        vars.push(AccountVar {
            name: var,
            default,
            profile,
            isolated_home: spec.isolated_home,
        });
    }
    let forbid = file.forbid.map(|forbid| forbid.vars).unwrap_or_default();
    if let Some(bad) = forbid
        .iter()
        .find(|var| !valid_variable(var) || declared.iter().any(|(declared, _)| declared == *var))
    {
        return Err(fail(format!(
            ".forbid: {bad:?} is not a variable name apart from .vars"
        )));
    }
    let PathsFile {
        hooks,
        hook_trust,
        plugins,
        skills,
        folder_trust,
        sessions,
        login,
        state,
        usage,
    } = file.paths;
    let written = [
        hooks,
        hook_trust,
        plugins,
        skills,
        folder_trust,
        sessions,
        login,
        state,
        usage,
    ];
    let mut paths = Vec::with_capacity(written.len());
    for (feature, written) in AccountPath::ALL.into_iter().zip(written) {
        let at = format!(".paths.{}", feature.key());
        let rule = match written {
            PathFile::Off(false) => PathRule::Absent,
            PathFile::Off(true) => {
                return Err(fail(format!("{at} is a place, or false for none")));
            }
            PathFile::Text(text) if text == "shared" => {
                if feature != AccountPath::Skills {
                    return Err(fail(format!("{at} cannot be shared")));
                }
                PathRule::Shared
            }
            PathFile::Text(text) => PathRule::Same(
                template(&text, &declared, Side::Default, file.by)
                    .map_err(|why| fail(format!("{at} {why}")))?,
            ),
            PathFile::Split { default, profile } => {
                if file.by == By::None {
                    return Err(fail(format!(
                        "{at} has no profile side: by = \"none\" has no profile"
                    )));
                }
                PathRule::Split {
                    default: template(&default, &declared, Side::Default, file.by)
                        .map_err(|why| fail(format!("{at}.default {why}")))?,
                    profile: template(&profile, &declared, Side::Profile, file.by)
                        .map_err(|why| fail(format!("{at}.profile {why}")))?,
                }
            }
        };
        paths.push((feature, rule));
    }
    Ok(AccountRule {
        by: file.by,
        reason: file.reason,
        vars,
        paths,
        forbid,
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Side {
    /// What the built-in default profile reads: no account.
    Default,
    /// What another profile reads: its account.
    Profile,
}

/// Read one place: `~`, `{account}` or `{NAME}` (a declared variable), then
/// `/` and folder names. `declared` is each variable with whether it has a
/// default; the variables' own templates name none.
fn template(
    text: &str,
    declared: &[(String, bool)],
    side: Side,
    by: By,
) -> Result<Template, String> {
    let mut words = text.split('/');
    let head = words.next().unwrap_or_default();
    let base = if head == "~" {
        Base::Home
    } else if head == ACCOUNT_PART {
        if side == Side::Default || by != By::Folder {
            return Err(format!(
                "{text:?} starts at {ACCOUNT_PART}, which only a folder account's profile has"
            ));
        }
        Base::Account
    } else if let Some(var) = head
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    {
        match declared.iter().find(|(name, _)| name == var) {
            None => return Err(format!("{text:?} names {head}, which .vars does not")),
            Some((_, false)) if side == Side::Default => {
                return Err(format!(
                    "{text:?} names {head}, which has no default for the default profile"
                ));
            }
            Some(_) => Base::Var(var.to_owned()),
        }
    } else {
        return Err(format!(
            "{text:?} does not start at ~, {ACCOUNT_PART} or a variable of .vars"
        ));
    };
    let parts: Vec<String> = words.map(str::to_owned).collect();
    for part in &parts {
        if part == ACCOUNT_PART {
            if side == Side::Default || by != By::Name {
                return Err(format!(
                    "{text:?} sets {ACCOUNT_PART} into a folder, which only a named account's profile does"
                ));
            }
            continue;
        }
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains(['\\', '{', '}', ':'])
            || part.chars().any(char::is_control)
        {
            return Err(format!("{text:?} holds a folder name that is not plain"));
        }
    }
    Ok(Template { base, parts })
}

/// An environment variable's name as a definition or a profile writes it:
/// a letter or `_`, then letters, digits and `_`, at most 256 bytes.
pub fn valid_variable(name: &str) -> bool {
    (1..=256).contains(&name.len())
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A variable no definition or profile moves: the home itself and Hide's own.
pub fn reserved_variable(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper == "HOME" || upper == "USERPROFILE" || upper.starts_with("HIDE_")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `build.rs` reads every built-in file through `read`, so a refusal
    /// here is the build failing on that file.
    #[test]
    fn a_built_in_file_whose_home_leaves_the_home_folder_is_refused_by_the_read_the_build_runs() {
        let text = "schema = 1\nid = \"x\"\nname = \"X\"\nexecutables = [\"x\"]\nhome = \"~/../x\"\nhooks = \"none\"\n";
        let error = read("x.toml", text, Origin::Builtin).unwrap_err();
        assert_eq!(
            error,
            "x.toml: home \"~/../x\" is not a folder under the home folder, written ~/<folder>"
        );
        let fine = text.replace("~/../x", "~/.x");
        assert_eq!(
            read("x.toml", &fine, Origin::Builtin).unwrap().home,
            Some(vec![".x".to_owned()])
        );
    }
}
