//! An agent definition file as written (schema 1): the fields every file
//! may carry and the checks each field passes. The build script reads the
//! built-in files through [`read`] too (`build.rs`), so a built-in file that
//! fails a check here fails the build, and the operator's files are held to
//! the same checks at run time (`super::read_file`). Only the screen rules
//! are compiled outside, by the reader that links `hide_screen_rules`.

use serde::Deserialize;

/// The schema this build reads.
pub const SCHEMA: u32 = 1;
/// The most words a line typed into a pane's shell holds, its program
/// included, and the most bytes its words hold: Herdr's resume command
/// limits (`validate_resume_argv`). A definition's words are checked
/// against them here and the line a start or resume types against them in
/// the core's own engine (`engine/own/shell_line.rs`, `check_resume`).
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
    pub aliases: Option<Vec<String>>,
    /// The agent's own folder under the account's home, as its parts.
    pub home: Option<Vec<String>>,
    pub hooks: Option<Hooks>,
    pub start_args: Option<Vec<String>>,
    pub resume_session: Option<Vec<String>>,
    pub resume_last: Option<Vec<String>>,
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
    let start_args = file.start.map(|start| start.args);
    if let Some(args) = &start_args {
        check_words(name, "start.args", args, 0)?;
    }
    let resume = file.resume.unwrap_or_default();
    if let Some(words) = &resume.session {
        check_words(name, "resume.session", words, 1)?;
    }
    if let Some(words) = &resume.last {
        check_words(name, "resume.last", words, 0)?;
    }
    Ok(Fields {
        id: file.id,
        name: file.name,
        based_on: file.based_on,
        executables: file.executables,
        aliases: file.aliases,
        home,
        hooks: file.hooks,
        start_args,
        resume_session: resume.session,
        resume_last: resume.last,
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

/// Programs are compared by their file's name, so none is a path or holds a
/// space; the first is what a start types into the pane's shell, so it is a
/// plain command name, as Herdr's resume check holds it
/// (`validate_resume_argv`).
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

/// Words a start or a resume types after the program: Herdr's resume
/// command rules (no control character, no apostrophe, 64 words and 8 KiB
/// with the program), and `{session}` exactly where the field takes it.
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
