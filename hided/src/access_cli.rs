//! `hide access request|status|leave|list`: an external tool's own
//! connection to Hide (PRD hide-external-control B1-B19, B32), and the key
//! a tool's every other `hide` command runs with.
//!
//! A tool keeps one file per connection in its own home
//! (`hide_kit::layout::access_keys`), private to its account: `<name>.request`
//! while the operator has not answered, `<name>.key` once approved. Each
//! holds the key whose hash the core keeps; the key never leaves this file
//! and the bootstrap socket, which hands the core only its hash.

use std::io::Read;
use std::path::{Path, PathBuf};

use hide_agent_hooks::hook_request::install::AccessAsk;
use hide_core::access::{valid_key, valid_name};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::Env;

pub const USAGE: &str = "hide access request --name <name> [--json]\nhide access status [--json]\nhide access leave [--json]\nhide access list [--json]";

const KEY_SCHEMA_VERSION: u32 = 1;
/// A key file holds a name and a key: anything longer is not one.
const KEY_FILE_LIMIT: u64 = 4096;
const REQUEST_EXTENSION: &str = "request";
const KEY_EXTENSION: &str = "key";

#[derive(Debug, Eq, PartialEq)]
pub struct AccessCli {
    pub verb: Verb,
    pub json: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Verb {
    Request { name: String },
    Status,
    Leave,
    List,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<AccessCli, String> {
    let verb = args.next().map(String::as_str);
    let mut json = false;
    let mut name = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" if !json => json = true,
            "--name" if verb == Some("request") && name.is_none() => match args.next() {
                Some(value) if valid_name(value) => name = Some(value.clone()),
                _ => return Err(USAGE.to_owned()),
            },
            _ => return Err(USAGE.to_owned()),
        }
    }
    let verb = match (verb, name) {
        (Some("request"), Some(name)) => Verb::Request { name },
        (Some("status"), None) => Verb::Status,
        (Some("leave"), None) => Verb::Leave,
        (Some("list"), None) => Verb::List,
        _ => return Err(USAGE.to_owned()),
    };
    Ok(AccessCli { verb, json })
}

/// Where a tool connection stands with the core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kept {
    /// `<name>.request`: put up, not answered yet.
    Requested,
    /// `<name>.key`: approved.
    Connected,
}

impl Kept {
    fn extension(self) -> &'static str {
        match self {
            Self::Requested => REQUEST_EXTENSION,
            Self::Connected => KEY_EXTENSION,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    schema_version: u32,
    name: String,
    key: String,
}

/// One connection's file in the tool's home.
struct Key {
    name: String,
    key: String,
    kept: Kept,
}

/// A refusal: its code and what the tool does about it.
struct Refusal {
    code: String,
    message: String,
}

impl Refusal {
    fn new(code: &str) -> Self {
        let message = crate::access::next_action(code)
            .unwrap_or_else(|| crate::cli::request_next_action(code))
            .to_owned();
        Self {
            code: code.to_owned(),
            message,
        }
    }
}

pub fn run(env: &Env, cli: &AccessCli) -> Result<(), String> {
    let answer = match &cli.verb {
        Verb::Request { name } => request(env, name),
        Verb::Status => status(env),
        Verb::Leave => leave(env),
        Verb::List => list(env),
    };
    match answer {
        Ok(value) => {
            if cli.json {
                println!("{}", json!({"ok": true, "value": value}));
            } else {
                print!("{}", draw(&cli.verb, &value));
            }
            Ok(())
        }
        Err(refusal) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": refusal.code, "message": refusal.message}})
            );
            Err(refusal.code)
        }
    }
}

/// `hide access request --name <name>`: puts the tool up for the operator,
/// or, asked again, answers where that request stands (B1, B5, B7, B9).
fn request(env: &Env, name: &str) -> Result<Value, Refusal> {
    refuse_in_pane(env)?;
    let folder = hide_kit::layout::access_keys(&env.home);
    let kept = match kept(&folder, name) {
        // A damaged file proves nothing, and the core cannot be told which
        // connection it held: it goes, and a new key asks anew (B32). An
        // old connection of the name still stands until the operator
        // disconnects it, which the core's `access_name_taken` says.
        Err(refusal) if refusal.code == "access_key_unreadable" => {
            remove_key(&folder, name, Kept::Connected);
            remove_key(&folder, name, Kept::Requested);
            None
        }
        kept => kept?,
    };
    match kept {
        Some(key) => settle(env, &folder, key),
        None => {
            let key = fresh_key()?;
            write_key(&folder, name, &key, Kept::Requested)?;
            let answer = ask(
                env,
                AccessAsk::Request {
                    name: name.to_owned(),
                    key: key.clone(),
                },
            );
            match answer {
                Ok(value) if value["state"] == "pending" => Ok(waiting(value, name)),
                // A new key is never connected or unknown to the core it
                // was just put up with.
                Ok(_) => {
                    remove_key(&folder, name, Kept::Requested);
                    Err(Refusal::new("access_unavailable"))
                }
                Err(refusal) => {
                    remove_key(&folder, name, Kept::Requested);
                    Err(refusal)
                }
            }
        }
    }
}

/// Reads where a kept connection stands and keeps the file in step: an
/// approved request becomes the key, an ended one is removed so the next
/// `request` asks anew.
fn settle(env: &Env, folder: &Path, key: Key) -> Result<Value, Refusal> {
    let answer = ask(
        env,
        AccessAsk::Status {
            key: key.key.clone(),
        },
    );
    let gone = |code: &str| {
        remove_key(folder, &key.name, key.kept);
        Err(Refusal::new(code))
    };
    match answer {
        Ok(value) => match (value["state"].as_str(), key.kept) {
            (Some("pending"), Kept::Requested) => Ok(waiting(value, &key.name)),
            (Some("connected"), Kept::Requested) => {
                // Asked once more as the request it was, so the core
                // records the key's issue (B33); a known key is answered
                // where it stands and puts nothing up.
                let issued = ask(
                    env,
                    AccessAsk::Request {
                        name: key.name.clone(),
                        key: key.key.clone(),
                    },
                )?;
                write_key(folder, &key.name, &key.key, Kept::Connected)?;
                remove_key(folder, &key.name, Kept::Requested);
                Ok(issued)
            }
            (Some("connected"), Kept::Connected) => Ok(value),
            // Pending requests live in the core's memory only (D-26): a
            // request the core no longer knows was dropped, and a key it
            // no longer knows was let go of.
            (_, Kept::Requested) => gone("access_request_expired"),
            (_, Kept::Connected) => gone("access_revoked"),
        },
        Err(refusal) if ended(&refusal.code) => {
            remove_key(folder, &key.name, key.kept);
            Err(refusal)
        }
        Err(refusal) => Err(refusal),
    }
}

/// `hide access status`: where this tool stands, asking for nothing (B6).
fn status(env: &Env) -> Result<Value, Refusal> {
    refuse_in_pane(env)?;
    let folder = hide_kit::layout::access_keys(&env.home);
    let Some(key) = chosen(env, &folder, false)? else {
        return Ok(json!({"state": "not_connected"}));
    };
    let answer = ask(
        env,
        AccessAsk::Status {
            key: key.key.clone(),
        },
    )?;
    match (answer["state"].as_str(), key.kept) {
        (Some("pending" | "connected"), _) => Ok(waiting_or(answer, &key.name)),
        (_, Kept::Requested) => Err(Refusal::new("access_request_expired")),
        (_, Kept::Connected) => Err(Refusal::new("access_revoked")),
    }
}

/// `hide access leave`: the tool ends its own connection, as the operator's
/// disconnect would, and its key file goes (B14, D-32).
fn leave(env: &Env) -> Result<Value, Refusal> {
    refuse_in_pane(env)?;
    let folder = hide_kit::layout::access_keys(&env.home);
    let key = chosen(env, &folder, false)?.ok_or_else(|| Refusal::new("access_not_connected"))?;
    let answer = ask(
        env,
        AccessAsk::Leave {
            key: key.key.clone(),
        },
    );
    match answer {
        Ok(_) => {}
        // The core has let go of it already: the file is all that is left.
        Err(refusal) if ended(&refusal.code) => {}
        Err(refusal) => return Err(refusal),
    }
    remove_key(&folder, &key.name, key.kept);
    Ok(json!({"state": "left", "name": key.name}))
}

/// `hide access list`: the tools connected and the requests waiting, read
/// as the connected tool this command runs as (B17).
fn list(env: &Env) -> Result<Value, Refusal> {
    let answer = crate::workspace_cli::Credential::acquire(env)
        .and_then(|mut credential| crate::workspace_cli::request_access_list(&mut credential))
        .map_err(|code| Refusal::new(&code))?;
    if answer["ok"] == true {
        return Ok(answer["result"].clone());
    }
    let code = answer["reason"].as_str().unwrap_or("access_unavailable");
    Err(Refusal {
        code: code.to_owned(),
        message: answer["next_action"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| Refusal::new(code).message),
    })
}

/// The connected tool a command outside Hide's panes runs as: none for a
/// pane's command, which runs as its pane however many keys its home keeps
/// (B19), and none when this home keeps no key.
pub(crate) fn tool_key(env: &Env) -> Result<Option<String>, String> {
    if env.pane_id.is_some() {
        return Ok(None);
    }
    let folder = hide_kit::layout::access_keys(&env.home);
    chosen(env, &folder, true)
        .map(|key| key.map(|key| key.key))
        .map_err(|refusal| refusal.code)
}

/// The state folder of the daemon a tool's command asks: `HIDE_STATE_DIR`
/// or `XDG_STATE_HOME` when set, else the account's own default, so a tool
/// that runs with a home of its own still finds the operator's Hide (D-03).
pub(crate) fn state_dir(env: &Env) -> PathBuf {
    if env.state_dir == hide_kit::layout::default_state_dir(&env.home)
        && let Ok(account) = hide_platform::user_agents::account_home()
    {
        return hide_kit::layout::default_state_dir(&account);
    }
    env.state_dir.clone()
}

/// The core's answer for a connection it has ended (`hide_core::access::Ended`).
fn ended(code: &str) -> bool {
    matches!(
        code,
        "access_refused" | "access_request_expired" | "access_revoked"
    )
}

fn refuse_in_pane(env: &Env) -> Result<(), Refusal> {
    if env.pane_id.is_some() || std::env::var_os(crate::env::HIDE_CAP_REF).is_some() {
        return Err(Refusal::new("access_from_pane"));
    }
    Ok(())
}

fn ask(env: &Env, ask: AccessAsk) -> Result<Value, Refusal> {
    crate::workspace_cli::access_bootstrap(&state_dir(env), ask).map_err(|(code, message)| {
        Refusal {
            message: message.unwrap_or_else(|| Refusal::new(&code).message),
            code,
        }
    })
}

fn waiting(mut value: Value, name: &str) -> Value {
    value["next_action"] = json!(format!(
        "Wait until the operator approves the request in Settings > External tools, then run hide access request --name {name} again"
    ));
    value
}

fn waiting_or(value: Value, name: &str) -> Value {
    if value["state"] == "pending" {
        waiting(value, name)
    } else {
        value
    }
}

/// The connection `HIDE_ACCESS` names, or the only one this home keeps.
/// `keys_only` counts approved keys alone: a command other than `hide
/// access` runs only as a connected tool.
fn chosen(env: &Env, folder: &Path, keys_only: bool) -> Result<Option<Key>, Refusal> {
    if let Some(name) = &env.access {
        return match kept(folder, name)? {
            Some(key) if !keys_only || key.kept == Kept::Connected => Ok(Some(key)),
            _ => Err(Refusal::new("access_not_connected")),
        };
    }
    let mut names = names(folder, keys_only)?;
    match names.len() {
        0 => Ok(None),
        1 => kept(folder, &names.remove(0)),
        _ => Err(Refusal::new("access_ambiguous")),
    }
}

/// The names this home keeps a file for, approved keys alone when asked.
fn names(folder: &Path, keys_only: bool) -> Result<Vec<String>, Refusal> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(Refusal::new("access_key_unreadable")),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            let counted =
                extension == KEY_EXTENSION || (!keys_only && extension == REQUEST_EXTENSION);
            let name = path.file_stem()?.to_str()?;
            (counted && valid_name(name)).then(|| name.to_owned())
        })
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The file kept for `name`: its key once approved, else its request.
fn kept(folder: &Path, name: &str) -> Result<Option<Key>, Refusal> {
    for kept in [Kept::Connected, Kept::Requested] {
        let path = key_path(folder, name, kept);
        match read_key(&path) {
            Ok(Some(key)) if key.name == name => {
                return Ok(Some(Key {
                    name: key.name,
                    key: key.key,
                    kept,
                }));
            }
            Ok(None) => {}
            Ok(Some(_)) | Err(()) => return Err(Refusal::new("access_key_unreadable")),
        }
    }
    Ok(None)
}

fn key_path(folder: &Path, name: &str, kept: Kept) -> PathBuf {
    folder.join(format!("{name}.{}", kept.extension()))
}

/// Reads a key file: `None` when there is none, an error when it is not
/// the account's own private file or not a key file.
fn read_key(path: &Path) -> Result<Option<KeyFile>, ()> {
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !hide_platform::fs::private::is_private(path).unwrap_or(false) {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(KEY_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > KEY_FILE_LIMIT {
        return Err(());
    }
    let key: KeyFile = serde_json::from_slice(&bytes).map_err(|_| ())?;
    (key.schema_version == KEY_SCHEMA_VERSION && valid_name(&key.name) && valid_key(&key.key))
        .then_some(Some(key))
        .ok_or(())
}

/// Writes `name`'s key whole, private to this account on every system (B35).
fn write_key(folder: &Path, name: &str, key: &str, kept: Kept) -> Result<(), Refusal> {
    let unwritable = |_| Refusal::new("access_key_unreadable");
    hide_platform::fs::private::create_dir_all(folder).map_err(unwritable)?;
    let bytes = serde_json::to_vec(&KeyFile {
        schema_version: KEY_SCHEMA_VERSION,
        name: name.to_owned(),
        key: key.to_owned(),
    })
    .map_err(|_| Refusal::new("access_key_unreadable"))?;
    hide_platform::fs::atomic::write_file(
        &key_path(folder, name, kept),
        &bytes,
        hide_platform::fs::Access::Private,
    )
    .map(|_| ())
    .map_err(unwritable)
}

fn remove_key(folder: &Path, name: &str, kept: Kept) {
    let _ = std::fs::remove_file(key_path(folder, name, kept));
}

fn fresh_key() -> Result<String, Refusal> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| Refusal::new("access_unavailable"))?;
    Ok(hex::encode(bytes))
}

fn level_words(value: &Value) -> &'static str {
    match value["level"].as_str() {
        Some("act") => "Act",
        _ => "View only",
    }
}

/// The answer as lines for a person.
fn draw(verb: &Verb, value: &Value) -> String {
    let state = value["state"].as_str().unwrap_or_default();
    let name = value["name"].as_str().unwrap_or_default();
    match (verb, state) {
        (Verb::List, _) => draw_list(value),
        (_, "pending") => format!(
            "Waiting for the operator to approve {name} in Settings > External tools.\n{}\n",
            value["next_action"].as_str().unwrap_or_default()
        ),
        (_, "connected") => format!("Connected as {name} · {}\n", level_words(value)),
        (_, "left") => format!(
            "Left. {name}'s key is removed, and the agents it started are the operator's now.\n"
        ),
        _ => "Not connected. Run hide access request --name <name> to connect.\n".to_owned(),
    }
}

fn draw_list(value: &Value) -> String {
    let rows = |key: &str| value[key].as_array().cloned().unwrap_or_default();
    let (connections, pending) = (rows("connections"), rows("pending"));
    let mut text = format!(
        "{} of {} tools connected on {}\n",
        connections.len(),
        value["limit"],
        value["machine"].as_str().unwrap_or_default()
    );
    for connection in &connections {
        text.push_str(&format!(
            "  {}  {}  {} agents  {} missions  last used {}\n",
            connection["name"].as_str().unwrap_or_default(),
            level_words(connection),
            connection["agent_count"],
            connection["mission_count"],
            ago(connection["last_used_ms"].as_u64().unwrap_or_default()),
        ));
    }
    if !pending.is_empty() {
        text.push_str("Waiting for the operator\n");
        for request in &pending {
            text.push_str(&format!(
                "  {}  {}\n",
                request["name"].as_str().unwrap_or_default(),
                request["program"].as_str().unwrap_or("unknown program"),
            ));
        }
    }
    text
}

/// How long ago `ms` (Unix milliseconds) was, in a person's words.
fn ago(ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    match now.saturating_sub(ms) / 60_000 {
        0 => "just now".to_owned(),
        minutes @ 1..60 => format!("{minutes} min ago"),
        minutes @ 60..1440 => format!("{} h ago", minutes / 60),
        minutes => format!("{} d ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn parse_reads_each_verb_and_refuses_what_is_not_one() {
        let request = args(&["request", "--name", "nightly-bot", "--json"]);
        assert_eq!(
            parse(request.iter()),
            Ok(AccessCli {
                verb: Verb::Request {
                    name: "nightly-bot".to_owned()
                },
                json: true
            })
        );
        for verb in ["status", "leave", "list"] {
            assert!(parse(args(&[verb]).iter()).is_ok(), "{verb}");
        }
        for wrong in [
            &["request"][..],
            &["request", "--name", "-x"],
            &["request", "--name", "a b"],
            &["status", "--name", "x"],
            &["approve", "x"],
            &["list", "--json", "--json"],
        ] {
            assert!(parse(args(wrong).iter()).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn a_key_file_is_private_and_found_by_its_name() {
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        let key = "a".repeat(64);
        assert!(write_key(&folder, "bot", &key, Kept::Requested).is_ok());
        assert!(
            hide_platform::fs::private::is_private(&key_path(&folder, "bot", Kept::Requested))
                .unwrap()
        );
        let found = kept(&folder, "bot").ok().flatten().unwrap();
        assert_eq!(
            (found.key.as_str(), found.kept),
            (key.as_str(), Kept::Requested)
        );
        assert!(write_key(&folder, "bot", &key, Kept::Connected).is_ok());
        assert_eq!(
            kept(&folder, "bot").ok().flatten().unwrap().kept,
            Kept::Connected
        );
        assert!(kept(&folder, "other").ok().unwrap().is_none());
    }

    #[test]
    fn a_damaged_key_file_is_refused_as_unreadable() {
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        assert!(write_key(&folder, "bot", &"b".repeat(64), Kept::Connected).is_ok());
        hide_platform::fs::atomic::write_file(
            &key_path(&folder, "bot", Kept::Connected),
            b"{not json",
            hide_platform::fs::Access::Private,
        )
        .unwrap();
        assert_eq!(
            kept(&folder, "bot").err().map(|refusal| refusal.code),
            Some("access_key_unreadable".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        assert!(write_key(&folder, "bot", &"c".repeat(64), Kept::Connected).is_ok());
        let path = key_path(&folder, "bot", Kept::Connected);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            kept(&folder, "bot").err().map(|refusal| refusal.code),
            Some("access_key_unreadable".to_owned())
        );
    }

    #[test]
    fn several_keys_need_hide_access_and_requests_do_not_count_for_commands() {
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        let mut env = crate::env::load_from(|key| match key {
            "HOME" => Some(home.path().to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(tool_key(&env), Ok(None));
        assert!(write_key(&folder, "one", &"1".repeat(64), Kept::Connected).is_ok());
        assert!(write_key(&folder, "two", &"2".repeat(64), Kept::Requested).is_ok());
        assert_eq!(tool_key(&env), Ok(Some("1".repeat(64))));
        assert!(write_key(&folder, "two", &"2".repeat(64), Kept::Connected).is_ok());
        assert_eq!(tool_key(&env), Err("access_ambiguous".to_owned()));
        env.access = Some("two".to_owned());
        assert_eq!(tool_key(&env), Ok(Some("2".repeat(64))));
        env.access = Some("three".to_owned());
        assert_eq!(tool_key(&env), Err("access_not_connected".to_owned()));
        // A pane runs as its pane whatever keys its home keeps (B19).
        env.pane_id = Some("w1:p1".to_owned());
        assert_eq!(tool_key(&env), Ok(None));
    }
}
