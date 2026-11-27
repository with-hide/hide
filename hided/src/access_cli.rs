//! `hide access join|request|status|leave|list`: an external tool's own
//! connection to Hide (PRD hide-external-control B1-B19, B32), and the key
//! a tool's every other `hide` command runs with.
//!
//! A tool keeps one file per connection in its own home
//! (`hide_kit::layout::access_keys`), private to its account: `<name>.request`
//! while the operator has not answered, `<name>.key` once approved or joined.
//! Each holds the key whose hash the core keeps; the key never leaves this
//! file and the bootstrap socket, which hands the core only its hash.
//!
//! `hide access join <code>` keeps its key in `join.pending` before it asks,
//! so a join whose answer was lost finishes when run again with the same
//! code; a home keeps one unfinished join at a time.

#![allow(clippy::disallowed_macros)] // the terminal is this command's output: it prints to the operator's own terminal and holds no lock

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_agent_hooks::hook_request::install::AccessAsk;
use hide_core::access::{normalize_code, reservable_name, spaced_code, valid_key, valid_name};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::Env;

pub const USAGE: &str = "hide access join <code> [--json]\nhide access request --name <name> [--no-wait] [--json]\nhide access status [--json]\nhide access leave [--json]\nhide access list [--json]";

const KEY_SCHEMA_VERSION: u32 = 1;
/// A key file holds a name and a key: anything longer is not one.
const KEY_FILE_LIMIT: u64 = 4096;
const REQUEST_EXTENSION: &str = "request";
/// How long `hide access request` waits for the operator's answer before it
/// ends with the request still waiting (D-56).
const ANSWER_WAIT: Duration = Duration::from_secs(600);
/// How often a waiting request asks the core where it stands.
const ANSWER_POLL: Duration = Duration::from_secs(1);
const KEY_EXTENSION: &str = "key";
/// The one join a home has not finished: its code and the key it sent.
const JOIN_FILE: &str = "join.pending";

#[derive(Debug, Eq, PartialEq)]
pub struct AccessCli {
    pub verb: Verb,
    pub json: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Verb {
    /// The code as it was written; [`join`] reads it.
    Join {
        code: String,
    },
    /// `wait`: stays until the operator answers, up to [`ANSWER_WAIT`];
    /// `--no-wait` answers at once.
    Request {
        name: String,
        wait: bool,
    },
    Status,
    Leave,
    List,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<AccessCli, String> {
    let verb = args.next().map(String::as_str);
    let mut json = false;
    let mut name = None;
    let mut code = None;
    let mut wait = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" if !json => json = true,
            text if verb == Some("join") && code.is_none() && !text.starts_with('-') => {
                code = Some(text.to_owned());
            }
            "--no-wait" if verb == Some("request") && wait => wait = false,
            "--name" if verb == Some("request") && name.is_none() => match args.next() {
                Some(value) if reservable_name(value) => name = Some(value.clone()),
                _ => return Err(USAGE.to_owned()),
            },
            _ => return Err(USAGE.to_owned()),
        }
    }
    let verb = match (verb, name, code) {
        (Some("join"), None, Some(code)) => Verb::Join { code },
        (Some("request"), Some(name), None) => Verb::Request { name, wait },
        (Some("status"), None, None) => Verb::Status,
        (Some("leave"), None, None) => Verb::Leave,
        (Some("list"), None, None) => Verb::List,
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

/// `join.pending`: the code an unfinished join used and the key it sent.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JoinFile {
    schema_version: u32,
    code: String,
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
    fn with(code: &str, message: String) -> Self {
        Self {
            code: code.to_owned(),
            message,
        }
    }

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
        Verb::Join { code } => join(env, code),
        Verb::Request { name, wait } => request(env, name, *wait),
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

/// `hide access join <code>`: takes the bot the operator made in Settings
/// with this code. The key is kept in `join.pending` before it is sent, so
/// an answer lost on the way, or a Hide not running, loses nothing: the same
/// command run again sends the same key, which the core then answers as the
/// bot it already joined. A spent or wrong code takes the file away.
fn join(env: &Env, code: &str) -> Result<Value, Refusal> {
    refuse_in_pane(env)?;
    let code = normalize_code(code).ok_or_else(|| Refusal::new("access_code_invalid"))?;
    let folder = hide_kit::layout::access_keys(&env.home);
    let key = pending_join(&folder, &code)?;
    let answer = ask(
        env,
        AccessAsk::Join {
            code,
            key: key.clone(),
        },
    );
    let value = match answer {
        Ok(value) => value,
        // The core answered for this key and code: nothing will finish it.
        Err(refusal)
            if ended(&refusal.code)
                || matches!(
                    refusal.code.as_str(),
                    "access_code_invalid" | "access_from_pane" | "invalid_request"
                ) =>
        {
            remove_join(&folder, &key);
            return Err(refusal);
        }
        // Hide did not answer: the file keeps the key for the next try.
        Err(refusal) => return Err(refusal),
    };
    let name = value["name"].as_str().filter(|name| valid_name(name));
    let (Some("connected"), Some(name)) = (value["state"].as_str(), name) else {
        // A key this command made is no request; the core cannot have
        // put it up, so the answer is not one to keep.
        remove_join(&folder, &key);
        return Err(Refusal::new("access_unavailable"));
    };
    keep_joined_key(&folder, name, &key)?;
    remove_join(&folder, &key);
    Ok(value)
}

/// The key the join with `code` sends: the one `join.pending` keeps for it,
/// or a new one kept there first. Two joins of one code at once in one home
/// send the same key, since the file is put in place only where none is.
fn pending_join(folder: &Path, code: &str) -> Result<String, Refusal> {
    let path = folder.join(JOIN_FILE);
    for _ in 0..2 {
        match read_join(&path) {
            Ok(Some(kept)) if kept.code == code => return Ok(kept.key),
            Ok(Some(kept)) => {
                return Err(Refusal::with(
                    "access_join_pending",
                    format!(
                        "This home has a join that did not finish; run hide access join {} again to finish it, then run this one",
                        spaced_code(&kept.code)
                    ),
                ));
            }
            // A damaged file proves nothing: it goes, as a damaged key does.
            Err(()) => {
                let _ = std::fs::remove_file(&path);
            }
            Ok(None) => {}
        }
        let key = fresh_key()?;
        let bytes = serde_json::to_vec(&JoinFile {
            schema_version: KEY_SCHEMA_VERSION,
            code: code.to_owned(),
            key: key.clone(),
        })
        .map_err(|_| Refusal::new("access_key_unreadable"))?;
        if put_new(folder, &path, &bytes)? {
            return Ok(key);
        }
        // Another join of this home put its file first: read that one.
    }
    Err(Refusal::new("access_key_unreadable"))
}

/// The joined key as `<name>.key`, never over another: a file that already
/// holds this key is this join done before, and one that holds another key,
/// or a request of the name, is another connection's, perhaps of another
/// Hide, so both stay and `join.pending` keeps this one for the next try.
fn keep_joined_key(folder: &Path, name: &str, key: &str) -> Result<(), Refusal> {
    let taken = || Refusal::new("access_key_exists");
    for _ in 0..2 {
        match read_key(&key_path(folder, name, Kept::Connected)) {
            Ok(Some(kept)) if kept.key == key => return Ok(()),
            Ok(Some(_)) | Err(()) => return Err(taken()),
            Ok(None) => {}
        }
        if !matches!(read_key(&key_path(folder, name, Kept::Requested)), Ok(None)) {
            return Err(taken());
        }
        let bytes = serde_json::to_vec(&KeyFile {
            schema_version: KEY_SCHEMA_VERSION,
            name: name.to_owned(),
            key: key.to_owned(),
        })
        .map_err(|_| Refusal::new("access_key_unreadable"))?;
        if put_new(folder, &key_path(folder, name, Kept::Connected), &bytes)? {
            return Ok(());
        }
    }
    Err(taken())
}

/// Puts `bytes` at `path` whole and private, only where no file is: false
/// when one appeared first.
fn put_new(folder: &Path, path: &Path, bytes: &[u8]) -> Result<bool, Refusal> {
    let unwritable = |_| Refusal::new("access_key_unreadable");
    hide_platform::fs::private::create_dir_all(folder).map_err(unwritable)?;
    let mut nonce = [0u8; 8];
    getrandom::getrandom(&mut nonce).map_err(|_| Refusal::new("access_unavailable"))?;
    let staged = folder.join(format!(".{}.tmp", hex::encode(nonce)));
    hide_platform::fs::atomic::write_file(&staged, bytes, hide_platform::fs::Access::Private)
        .map_err(unwritable)?;
    let placed = hide_platform::fs::atomic::rename_no_replace_path(&staged, path);
    match placed {
        Ok(()) => Ok(true),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Ok(false)
            } else {
                Err(Refusal::new("access_key_unreadable"))
            }
        }
    }
}

/// Reads `join.pending`: `None` when there is none, an error when it is not
/// the account's own private file or not a join file.
fn read_join(path: &Path) -> Result<Option<JoinFile>, ()> {
    let Some(bytes) = read_private(path)? else {
        return Ok(None);
    };
    let kept: JoinFile = serde_json::from_slice(&bytes).map_err(|_| ())?;
    (kept.schema_version == KEY_SCHEMA_VERSION
        && normalize_code(&kept.code).as_deref() == Some(kept.code.as_str())
        && valid_key(&kept.key))
    .then_some(Some(kept))
    .ok_or(())
}

/// Takes `join.pending` away once its join is done or can never be, and
/// only while it still holds `key`.
fn remove_join(folder: &Path, key: &str) {
    let path = folder.join(JOIN_FILE);
    if matches!(read_join(&path), Ok(Some(kept)) if kept.key == key) {
        let _ = std::fs::remove_file(path);
    }
}

/// `hide access request --name <name>`: puts the tool up for the operator,
/// or, asked again, answers where that request stands (B1, B5, B7, B9), and
/// unless `wait` is false stays until the operator answers (D-56).
fn request(env: &Env, name: &str, wait: bool) -> Result<Value, Refusal> {
    let answer = put_up(env, name)?;
    if !wait || answer["state"] != "pending" {
        return Ok(answer);
    }
    wait_for_answer(env, name, answer)
}

/// Asks the core where `name`'s request stands until the operator answers
/// or [`ANSWER_WAIT`] passes. The request stays when the wait ends, and an
/// interrupted wait loses nothing: the next command picks up the key once
/// the operator approves ([`tool_key`]).
#[allow(clippy::disallowed_methods)] // polls a state the operator's window changes in the core, bounded by ANSWER_WAIT
fn wait_for_answer(env: &Env, name: &str, mut answer: Value) -> Result<Value, Refusal> {
    let folder = hide_kit::layout::access_keys(&env.home);
    eprintln!(
        "Waiting up to {} minutes for the operator to approve {name} in Settings > Bots.",
        ANSWER_WAIT.as_secs() / 60
    );
    let deadline = Instant::now() + ANSWER_WAIT;
    while Instant::now() < deadline {
        std::thread::sleep(ANSWER_POLL);
        // Only this tool's own `leave` or a damaged file takes the request
        // away meanwhile; either is answered as a fresh request would be.
        let Some(key) = kept(&folder, name)? else {
            return Err(Refusal::new("access_request_expired"));
        };
        answer = settle(env, &folder, key)?;
        if answer["state"] != "pending" {
            return Ok(answer);
        }
    }
    Ok(answer)
}

/// Puts the tool up, or answers where its kept request or key stands.
fn put_up(env: &Env, name: &str) -> Result<Value, Refusal> {
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
    match chosen(env, &folder, true) {
        Ok(Some(key)) => Ok(Some(key.key)),
        Ok(None) => Ok(approved_request(env, &folder)),
        Err(refusal) if refusal.code == "access_not_connected" => {
            approved_request(env, &folder).map_or(Err(refusal.code), |key| Ok(Some(key)))
        }
        Err(refusal) => Err(refusal.code),
    }
}

/// The key of the request this home keeps once the operator has approved
/// it, kept as the key from now on, so a tool whose `hide access request`
/// ended before the answer needs no second one (D-56). A request still
/// waiting, or one the core ended, leaves the command as it was with no
/// key: a waiting request does not count for commands (B19), and an ended
/// one is removed so the next `hide access request` asks anew.
fn approved_request(env: &Env, folder: &Path) -> Option<String> {
    let request = chosen(env, folder, false)
        .ok()
        .flatten()
        .filter(|key| key.kept == Kept::Requested)?;
    let key = request.key.clone();
    let answer = settle(env, folder, request).ok()?;
    (answer["state"] == "connected").then_some(key)
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
        "access_refused" | "access_request_expired" | "access_revoked" | "access_reapprove"
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
        "Once the operator approves it in Settings > Bots, the next hide command runs as {name}; hide access request --name {name} waits for the answer"
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
    let Some(bytes) = read_private(path)? else {
        return Ok(None);
    };
    let key: KeyFile = serde_json::from_slice(&bytes).map_err(|_| ())?;
    (key.schema_version == KEY_SCHEMA_VERSION && valid_name(&key.name) && valid_key(&key.key))
        .then_some(Some(key))
        .ok_or(())
}

/// A key folder file's bytes: `None` when there is none, an error when it is
/// not the account's own private file or longer than a key file is.
fn read_private(path: &Path) -> Result<Option<Vec<u8>>, ()> {
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
    Ok(Some(bytes))
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

/// The mode a bot runs in, in words.
fn mode_words(value: &Value) -> &'static str {
    match value["mode"]["kind"].as_str() {
        Some("own") => "own space",
        _ => "Full",
    }
}

/// The answer as lines for a person.
fn draw(verb: &Verb, value: &Value) -> String {
    let state = value["state"].as_str().unwrap_or_default();
    let name = value["name"].as_str().unwrap_or_default();
    match (verb, state) {
        (Verb::List, _) => draw_list(value),
        (Verb::Join { .. }, "connected") => format!(
            "Connected to Hide as the bot {name} ({}).\nEvery hide command from this home now runs as {name}; a home that keeps several bots' keys picks one with HIDE_ACCESS=<name>.\nWhat the bot may do: hide guide, section Bots.\n",
            mode_words(value)
        ),
        (_, "pending") => format!(
            "Waiting for the operator to approve {name} in Settings > Bots.\n{}\n",
            value["next_action"].as_str().unwrap_or_default()
        ),
        (_, "connected") => format!("Connected as {name} · {}\n", mode_words(value)),
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
            mode_words(connection),
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
                    name: "nightly-bot".to_owned(),
                    wait: true
                },
                json: true
            })
        );
        let no_wait = args(&["request", "--no-wait", "--name", "nightly-bot"]);
        assert_eq!(
            parse(no_wait.iter()),
            Ok(AccessCli {
                verb: Verb::Request {
                    name: "nightly-bot".to_owned(),
                    wait: false
                },
                json: false
            })
        );
        assert_eq!(
            parse(args(&["join", "K7Q4-MZ2P-X3RD", "--json"]).iter()),
            Ok(AccessCli {
                verb: Verb::Join {
                    code: "K7Q4-MZ2P-X3RD".to_owned()
                },
                json: true
            })
        );
        for verb in ["status", "leave", "list"] {
            assert!(parse(args(&[verb]).iter()).is_ok(), "{verb}");
        }
        for wrong in [
            &["join"][..],
            &["join", "A", "B"],
            &["join", "A", "--name", "x"],
            &["request", "--name", "con"],
            &["request"],
            &["request", "--name", "-x"],
            &["request", "--name", "a b"],
            &["status", "--name", "x"],
            &["approve", "x"],
            &["list", "--json", "--json"],
            &["status", "--no-wait"],
            &["request", "--name", "x", "--no-wait", "--no-wait"],
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
    fn a_join_keeps_its_key_first_and_a_home_finishes_one_join_at_a_time() {
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        let first = pending_join(&folder, "K7Q4MZ2PX3RD").ok().unwrap();
        assert!(hide_platform::fs::private::is_private(&folder.join(JOIN_FILE)).unwrap());
        // Run again, the same code sends the same key.
        assert_eq!(
            pending_join(&folder, "K7Q4MZ2PX3RD").ok(),
            Some(first.clone())
        );
        let other = pending_join(&folder, "0123456789AB").err().unwrap();
        assert_eq!(other.code, "access_join_pending");
        assert!(
            other.message.contains("hide access join K7Q4-MZ2P-X3RD"),
            "{}",
            other.message
        );
        // An unfinished join is no key a command runs with.
        assert_eq!(names(&folder, false).ok().unwrap(), Vec::<String>::new());
        remove_join(&folder, &"0".repeat(64));
        assert!(folder.join(JOIN_FILE).exists(), "another key's join stays");
        remove_join(&folder, &first);
        assert!(!folder.join(JOIN_FILE).exists());
    }

    #[test]
    fn a_joined_key_never_replaces_another_key_or_request_of_its_name() {
        let home = tempfile::tempdir().unwrap();
        let folder = hide_kit::layout::access_keys(home.path());
        let key = "a".repeat(64);
        assert!(keep_joined_key(&folder, "hermes", &key).is_ok());
        assert_eq!(kept(&folder, "hermes").ok().flatten().unwrap().key, key);
        assert!(
            keep_joined_key(&folder, "hermes", &key).is_ok(),
            "the same join done before"
        );
        assert_eq!(
            keep_joined_key(&folder, "hermes", &"b".repeat(64))
                .err()
                .map(|refusal| refusal.code),
            Some("access_key_exists".to_owned())
        );
        assert_eq!(kept(&folder, "hermes").ok().flatten().unwrap().key, key);
        assert!(write_key(&folder, "asking", &"c".repeat(64), Kept::Requested).is_ok());
        assert_eq!(
            keep_joined_key(&folder, "asking", &"d".repeat(64))
                .err()
                .map(|refusal| refusal.code),
            Some("access_key_exists".to_owned())
        );
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
            crate::env::HOME => Some(home.path().to_string_lossy().into_owned()),
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
