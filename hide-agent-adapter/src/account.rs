//! Agent profiles and the one answer to where an account's folders are
//! (docs/ARCHITECTURE.md, Agent profiles and accounts).
//!
//! A [`Profile`] is a way of running one agent: its account (a folder, or
//! omp's profile name), a program to run instead, extra variables. The
//! built-in default profile has none of them. [`resolve`] turns a profile
//! into an [`Account`] by the definition's `[account]` rule
//! ([`crate::definition::AccountRule`]): the variables a start sets, each
//! feature's place, and the account's identity. Every reader and installer
//! of an agent's folders asks it; no other code names those folders.
//!
//! It is pure: the caller hands in what its node captured
//! ([`NodeEnv`]: the account's home, the variables that node honours, the
//! system), and checking that a folder exists, whom it belongs to, or making
//! it, is the node's at registration.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::definition::{
    ACCOUNT_PART, AccountPath, AccountRule, Base, By, Definition, PathRule, Template,
    reserved_variable, valid_variable,
};

/// The most variables a profile adds to a start (PRD agent-profiles D-29).
pub const MAX_PROFILE_ENV: usize = 64;
/// The longest name of a profile's variable.
pub const MAX_ENV_KEY_BYTES: usize = 256;
/// The longest value of a profile's variable.
pub const MAX_ENV_VALUE_BYTES: usize = 32 * 1024;
/// The longest account a profile names: a folder's spelling, or a name.
pub const MAX_ACCOUNT_BYTES: usize = 4096;
const MAX_NAME_BYTES: usize = 64;

/// A profile's stable id: what a start, an agent's record and a project's
/// choice name it by, kept when the profile is renamed.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProfileId(pub String);

impl ProfileId {
    /// The built-in default profile of every agent.
    pub fn default_profile() -> Self {
        Self("default".to_owned())
    }

    pub fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

/// The account a profile runs in, fixed when it is added.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountRef {
    /// A folder, as an absolute path spelled exactly as it is set into the
    /// agent's variable (`~` expanded when it was added, a final `/` kept:
    /// Claude keys its login by the exact spelling).
    Folder(String),
    /// omp's own profile name.
    Name(String),
}

/// One way of running an agent; it holds no secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    /// The agent's definition id (`claude-code`, `codex`, ...).
    pub kind: String,
    /// Unique among the agent's profiles.
    pub name: String,
    /// `None` runs in the account the agent reads by default.
    pub account: Option<AccountRef>,
    /// A program to run instead of the definition's (a wrapper), which hands
    /// the account's variables on.
    pub program: Option<String>,
    /// Variables added to the start, never secrets.
    pub env: Vec<(String, String)>,
    /// Raised by every change, so a late answer about an older version is
    /// told apart.
    pub version: u64,
}

impl Profile {
    /// The built-in default profile of the agent `kind`: no account, no
    /// program, no variable.
    pub fn builtin_default(kind: &str) -> Self {
        Self {
            id: ProfileId::default_profile(),
            kind: kind.to_owned(),
            name: "default".to_owned(),
            account: None,
            program: None,
            env: Vec::new(),
            version: 0,
        }
    }
}

/// What a start carries: the agent and the profile it runs with.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchTarget {
    pub kind: String,
    pub profile: ProfileId,
}

/// An account's identity: the value set into the agent's account variable,
/// as set, or none for the default. Two profiles with one key are one account.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccountKey {
    Default,
    Folder(String),
    Name(String),
}

/// One feature's place in an account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Place {
    /// The agent has no such place.
    Absent,
    /// The folder every agent reads (`~/.agents/skills`), which no account owns.
    Shared,
    At(PathBuf),
}

/// What [`resolve`] answers: the account's identity, the variables a start
/// sets (the account's, then the profile's own), and each feature's place.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Account {
    pub key: AccountKey,
    pub env: Vec<(String, String)>,
    paths: BTreeMap<AccountPath, Place>,
}

impl Account {
    /// The feature's place, `None` when the agent has none or it is shared.
    pub fn path(&self, feature: AccountPath) -> Option<&Path> {
        match self.place(feature) {
            Place::At(path) => Some(path),
            Place::Absent | Place::Shared => None,
        }
    }

    pub fn place(&self, feature: AccountPath) -> &Place {
        self.paths.get(&feature).unwrap_or(&Place::Absent)
    }
}

/// The system a node runs on, which decides what an absolute path is and
/// whether variable names differ by case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Os {
    Unix,
    Windows,
}

impl Os {
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// What a node captured for [`resolve`]: the account's home and the account
/// variables it honours, as its own process has them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeEnv {
    pub home: Option<PathBuf>,
    variables: BTreeMap<String, OsString>,
    pub os: Os,
}

impl NodeEnv {
    /// A node on this system whose account has `home`, honouring no variable:
    /// the default profile reads every folder at the agent's default place.
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            variables: BTreeMap::new(),
            os: Os::current(),
        }
    }

    /// The node honours its value of `name`; `None` leaves it unset.
    pub fn with_variable(mut self, name: &str, value: Option<OsString>) -> Self {
        if let Some(value) = value {
            self.variables.insert(name.to_owned(), value);
        }
        self
    }

    /// [`Self::with_variable`] for every variable of an agent's `[account]`
    /// `rule`, read by `variable` (`hide_platform::host::variable` for the
    /// process).
    pub fn with_account_variables(
        mut self,
        rule: Option<&AccountRule>,
        variable: impl Fn(&str) -> Option<OsString>,
    ) -> Self {
        for var in rule.iter().flat_map(|rule| &rule.vars) {
            self = self.with_variable(&var.name, variable(&var.name));
        }
        self
    }

    pub fn on(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    fn variable(&self, name: &str) -> Option<&OsStr> {
        self.variables
            .get(name)
            .map(OsString::as_os_str)
            .filter(|value| !value.is_empty())
    }
}

/// Why a profile has no account. Each names a variable only by a name that
/// passed [`valid_variable`], never a value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The profile is another agent's.
    OtherAgent,
    /// A place starts at the home and the account has none.
    NoAccountHome,
    /// The node's own value of the variable is no absolute path, which the
    /// agent would read against a folder Hide does not share.
    NodeVariable {
        variable: String,
    },
    /// The agent's account cannot be moved; the definition says why.
    AccountUnsupported {
        reason: String,
    },
    /// A folder for an agent that names accounts, or a name for one that
    /// keeps them in folders.
    AccountKind {
        by: By,
    },
    AccountEmpty,
    /// The folder is no absolute path on the node's system (`~` included).
    AccountRelative,
    /// The folder is where the default profile reads: use the default.
    AccountIsDefault,
    /// A profile name holds what is not a letter, digit, `-`, `_` or `.`.
    AccountName,
    TooManyVariables {
        limit: usize,
    },
    /// A variable's name is not a name.
    VariableName,
    /// A variable the account, the home or Hide owns.
    VariableReserved {
        key: String,
    },
    VariableRepeated {
        key: String,
    },
    VariableValue {
        key: String,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OtherAgent => f.write_str("the profile belongs to another agent"),
            Self::NoAccountHome => f.write_str("the account has no home folder"),
            Self::NodeVariable { variable } => write!(f, "{variable} is not an absolute path"),
            Self::AccountUnsupported { reason } => {
                write!(f, "this agent's account cannot be moved: {reason}")
            }
            Self::AccountKind { by: By::Name } => {
                f.write_str("this agent names an account by a profile name, not a folder")
            }
            Self::AccountKind { .. } => f.write_str("this agent keeps an account in a folder"),
            Self::AccountEmpty => f.write_str("the account is empty"),
            Self::AccountRelative => f.write_str("the account folder is not an absolute path"),
            Self::AccountIsDefault => f.write_str(
                "the account folder is where the agent reads by default; use the default profile",
            ),
            Self::AccountName => f.write_str(
                "the profile name holds characters other than letters, digits, -, _ and .",
            ),
            Self::TooManyVariables { limit } => {
                write!(f, "a profile adds at most {limit} variables")
            }
            Self::VariableName => {
                f.write_str("a variable's name is not letters, digits and _ of at most 256 bytes")
            }
            Self::VariableReserved { key } => {
                write!(f, "{key} is the account's, the home's or Hide's own")
            }
            Self::VariableRepeated { key } => write!(f, "{key} is set twice"),
            Self::VariableValue { key } => write!(
                f,
                "the value of {key} holds a NUL or is longer than {MAX_ENV_VALUE_BYTES} bytes"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The account `profile` runs `definition` in, on the node `node` describes.
///
/// The default account sets no account variable: each place follows the
/// node's own value of the variable when it has one, and the definition's
/// default otherwise. Another account sets its variables from the account
/// exactly as stored. The profile's own variables come after the account's.
pub fn resolve(
    definition: &Definition,
    profile: &Profile,
    node: &NodeEnv,
) -> Result<Account, Refusal> {
    resolve_rule(&definition.id, definition.account.as_ref(), profile, node)
}

/// [`resolve`] for the agent `kind` whose `[account]` is `rule`: what a
/// built-in agent's adapter asks with the rule alone
/// ([`crate::definition::builtin_account`]), with no screen rules compiled.
pub fn resolve_rule(
    kind: &str,
    rule: Option<&AccountRule>,
    profile: &Profile,
    node: &NodeEnv,
) -> Result<Account, Refusal> {
    if profile.kind != kind {
        return Err(Refusal::OtherAgent);
    }
    let account = profile
        .account
        .as_ref()
        .map(|account| checked_account(rule, account, node.os))
        .transpose()?;
    // Each variable's value for the default profile, where it has one: the
    // default reads its places by them, another account only compares.
    let mut defaults: BTreeMap<&str, Result<PathBuf, Refusal>> = BTreeMap::new();
    for var in rule.iter().flat_map(|rule| &rule.vars) {
        let value = match node.variable(&var.name) {
            Some(value) if is_absolute(value, node.os) => Ok(PathBuf::from(value)),
            Some(_) => Err(Refusal::NodeVariable {
                variable: var.name.clone(),
            }),
            None => match &var.default {
                Some(default) => expand(default, &BTreeMap::new(), None, node),
                None => continue,
            },
        };
        defaults.insert(&var.name, value);
    }
    let (key, mut env, values) = match (&account, rule) {
        (Some(account), Some(rule)) => {
            let mut env = Vec::with_capacity(rule.vars.len());
            let mut values = BTreeMap::new();
            for var in &rule.vars {
                let value = expand(&var.profile, &BTreeMap::new(), Some(account), node)?;
                if let Some(Ok(default)) = defaults.get(var.name.as_str())
                    && same_folder(default, &value, node.os)
                {
                    return Err(Refusal::AccountIsDefault);
                }
                env.push((var.name.clone(), text(&value)));
                values.insert(var.name.as_str(), value);
            }
            (account.key(), env, values)
        }
        _ => {
            let mut values = BTreeMap::new();
            for (name, value) in defaults {
                values.insert(name, value?);
            }
            (AccountKey::Default, Vec::new(), values)
        }
    };
    env.extend(checked_env(rule, &profile.env, node.os)?);
    let mut paths = BTreeMap::new();
    for feature in AccountPath::ALL {
        let place = match rule.map(|rule| rule.path(feature)) {
            None | Some(PathRule::Absent) => Place::Absent,
            Some(PathRule::Shared) => Place::Shared,
            Some(PathRule::Same(template)) => {
                Place::At(expand(template, &values, account.as_ref(), node)?)
            }
            Some(PathRule::Split { default, profile }) => Place::At(match &account {
                None => expand(default, &values, None, node)?,
                Some(account) => expand(profile, &values, Some(account), node)?,
            }),
        };
        paths.insert(feature, place);
    }
    Ok(Account { key, env, paths })
}

/// The folders below the home where `path` is for the default profile of a
/// node that honours no variable: what a fake home lays out for the agent
/// (`journey::JourneyAdapter::home_folder`). `None` when the agent has no
/// such place of its own.
pub fn default_folder(rule: &AccountRule, path: AccountPath) -> Option<Vec<String>> {
    let template = match rule.path(path) {
        PathRule::Same(template)
        | PathRule::Split {
            default: template, ..
        } => template,
        PathRule::Absent | PathRule::Shared => return None,
    };
    let mut parts = match &template.base {
        Base::Home => Vec::new(),
        Base::Var(name) => {
            let default = rule.var(name)?.default.as_ref()?;
            debug_assert_eq!(default.base, Base::Home);
            default.parts.clone()
        }
        Base::Account => return None,
    };
    parts.extend(template.parts.iter().cloned());
    Some(parts)
}

/// A profile's account, checked against the rule; the stored spelling is
/// kept as it is.
enum Checked<'a> {
    Folder(&'a str),
    Name(&'a str),
}

impl Checked<'_> {
    fn key(&self) -> AccountKey {
        match self {
            Self::Folder(folder) => AccountKey::Folder((*folder).to_owned()),
            Self::Name(name) => AccountKey::Name((*name).to_owned()),
        }
    }
}

fn checked_account<'a>(
    rule: Option<&AccountRule>,
    account: &'a AccountRef,
    os: Os,
) -> Result<Checked<'a>, Refusal> {
    let Some(rule) = rule else {
        return Err(Refusal::AccountUnsupported {
            reason: "Hide keeps no folder of this agent".to_owned(),
        });
    };
    match (rule.by, account) {
        (By::None, _) => Err(Refusal::AccountUnsupported {
            reason: rule.reason.clone().unwrap_or_default(),
        }),
        (By::Folder, AccountRef::Folder(folder)) => {
            if folder.is_empty() {
                Err(Refusal::AccountEmpty)
            } else if folder.len() > MAX_ACCOUNT_BYTES
                || folder.contains('\0')
                || !is_absolute(OsStr::new(folder), os)
            {
                Err(Refusal::AccountRelative)
            } else {
                Ok(Checked::Folder(folder))
            }
        }
        (By::Name, AccountRef::Name(name)) => {
            if name.is_empty() {
                Err(Refusal::AccountEmpty)
            } else if name.len() > MAX_NAME_BYTES
                || name.starts_with('.')
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            {
                Err(Refusal::AccountName)
            } else {
                Ok(Checked::Name(name))
            }
        }
        (by, _) => Err(Refusal::AccountKind { by }),
    }
}

/// The profile's own variables, refused when one is not a name, is the
/// account's, a forbidden one, the home's or Hide's, repeats, or holds a NUL
/// or too much; Windows reads names regardless of case.
fn checked_env(
    rule: Option<&AccountRule>,
    env: &[(String, String)],
    os: Os,
) -> Result<Vec<(String, String)>, Refusal> {
    if env.len() > MAX_PROFILE_ENV {
        return Err(Refusal::TooManyVariables {
            limit: MAX_PROFILE_ENV,
        });
    }
    let same = |a: &str, b: &str| match os {
        Os::Windows => a.eq_ignore_ascii_case(b),
        Os::Unix => a == b,
    };
    let owned: Vec<&str> = rule
        .iter()
        .flat_map(|rule| {
            rule.vars
                .iter()
                .map(|var| var.name.as_str())
                .chain(rule.forbid.iter().map(String::as_str))
        })
        .collect();
    let mut seen: Vec<&str> = Vec::with_capacity(env.len());
    for (key, value) in env {
        if !valid_variable(key) || key.len() > MAX_ENV_KEY_BYTES {
            return Err(Refusal::VariableName);
        }
        let reserved = match os {
            Os::Windows => reserved_variable(key),
            Os::Unix => key == "HOME" || key == "USERPROFILE" || key.starts_with("HIDE_"),
        };
        if reserved || owned.iter().any(|owned| same(owned, key)) {
            return Err(Refusal::VariableReserved { key: key.clone() });
        }
        if seen.iter().any(|seen| same(seen, key)) {
            return Err(Refusal::VariableRepeated { key: key.clone() });
        }
        if value.len() > MAX_ENV_VALUE_BYTES || value.contains('\0') {
            return Err(Refusal::VariableValue { key: key.clone() });
        }
        seen.push(key);
    }
    Ok(env.to_vec())
}

/// A template's place: its start (`~`, the account, a variable's value)
/// with its folder names joined below, `{account}` set to the account's name.
fn expand(
    template: &Template,
    values: &BTreeMap<&str, PathBuf>,
    account: Option<&Checked<'_>>,
    node: &NodeEnv,
) -> Result<PathBuf, Refusal> {
    let mut path = match &template.base {
        Base::Home => node.home.clone().ok_or(Refusal::NoAccountHome)?,
        Base::Account => match account {
            Some(Checked::Folder(folder) | Checked::Name(folder)) => PathBuf::from(folder),
            None => unreachable!("the definition reader keeps {ACCOUNT_PART} off the default"),
        },
        Base::Var(name) => values
            .get(name.as_str())
            .cloned()
            .expect("the definition reader keeps an unset variable off this side"),
    };
    for part in &template.parts {
        if part == ACCOUNT_PART {
            match account {
                Some(Checked::Name(name)) => path.push(name),
                _ => unreachable!("the definition reader keeps {ACCOUNT_PART} to a named profile"),
            }
        } else {
            path.push(part);
        }
    }
    Ok(path)
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether `value` is absolute on `os`: a path from `/` on Unix; on Windows
/// a drive with its root (`C:\`, `C:/`) or a share (`\\server`). Windows'
/// drive-relative (`C:x`) and root-relative (`\x`) forms depend on a current
/// drive or folder Hide does not share with the agent.
fn is_absolute(value: &OsStr, os: Os) -> bool {
    let value = value.to_string_lossy();
    match os {
        Os::Unix => value.starts_with('/'),
        Os::Windows => {
            let bytes = value.as_bytes();
            value.starts_with(r"\\")
                || (bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'\\' | b'/'))
        }
    }
}

/// Whether two spellings name one folder, apart from a final separator, and
/// on Windows apart from case and the separator used.
fn same_folder(a: &Path, b: &Path, os: Os) -> bool {
    let spelling = |path: &Path| {
        let text = path.to_string_lossy();
        let text = match os {
            Os::Windows => text.replace('/', "\\").to_lowercase(),
            Os::Unix => text.into_owned(),
        };
        text.trim_end_matches(['/', '\\']).to_owned()
    };
    spelling(a) == spelling(b)
}
