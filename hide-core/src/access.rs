//! External tools' connections to this core (PRD hide-external-control).
//!
//! A program outside every Hide pane on the core's own machine (a chat
//! gateway, a cron job, a script) asks once with `hide access request
//! --name <name>`; the operator approves it as a bot from Settings, in one
//! of two modes, the whole Hide or its own work, and from then on the bot
//! runs the ordinary `hide` commands as that connection.
//!
//! The tool makes its key itself and keeps it in an owner-only file under
//! its own home; this core keeps only the key's SHA-256 (as phone pairing
//! keeps a phone's credential), so the connection list on disk proves
//! nothing to whoever reads it. A request waits in memory alone: a restart
//! drops it, and the tool asks again. Approved connections are this file
//! (`hide_kit::layout::access_store`), which moves with the core.
//!
//! The operator can also make a bot first, in Settings: a name and a mode
//! that wait, as an invite, for a program to join with the one-time code
//! the screen shows (`hide access join <code>`). The invite and its code
//! live in memory alone, like a request: a restart drops them, and the
//! operator makes the bot again.
//!
//! Every connection lives on the core's own machine (D-07): the bootstrap
//! socket that takes a key is the core's local one, so the store names no
//! machine.

use std::collections::VecDeque;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Approved connections at most (D-21).
pub const MAX_CONNECTIONS: usize = 16;
/// Requests waiting for the operator at most, one per name (D-21).
pub const MAX_PENDING: usize = 4;
/// Bots the operator made that no program has joined yet, at most, apart
/// from the requests; each holds one of the sixteen seats while its code is
/// live.
pub const MAX_INVITES: usize = 4;
/// How long a made bot's code joins it, from its making or its new code.
pub const INVITE_TTL_MS: u64 = 10 * 60_000;
/// A code's length in [`CODE_ALPHABET`] characters: sixty random bits, which
/// no program of the account guesses over the private socket in ten
/// minutes, so a wrong code needs no count of its own.
pub const CODE_LEN: usize = 12;
/// Crockford's base 32: digits and capitals without I, L, O and U, so a code
/// reads the same aloud and in any font.
pub const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// How many ended requests and connections are remembered, so a tool's next
/// look says why it ended; older ones read as never known.
const MAX_ENDED: usize = 64;
/// The longest name a tool may choose.
pub const NAME_LIMIT: usize = 64;
/// A connection's last use is moved, and the store saved for it, at most
/// once a minute per tool (B34).
pub const USE_INTERVAL_MS: u64 = 60_000;
/// The store's size at most: sixteen short records.
pub(crate) const READ_CAP: u64 = 256 * 1024;
/// Schema 2 replaced a connection's level (View only or Act) with its mode;
/// schema 1 is read once more on load ([`decode`]).
const SCHEMA_VERSION: u32 = 2;

/// What an approved bot reaches (mission-87): every bot runs what the
/// operator does through `hide`; a mode says over whose work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mode {
    /// The whole Hide: the operator's work and every bot's, on every
    /// connected device, and the settings-wide state.
    All,
    /// Its own work only: the agents it started, the agents started under
    /// them, and the missions they conduct (`Runtime::own_work`); it starts
    /// agents in any project on any connected device, and those are its own.
    Own,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Own => "own",
        }
    }
}

/// A name a tool may ask under: letters, digits, `.`, `_` and `-`, starting
/// with a letter or a digit, so it reads the same in a letter's sender and
/// on the screen.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A name a new bot may take: a [`valid_name`] that is a file name on every
/// system, since its key file is `<name>.key` (no `CON`, `NUL`, `COM1`, nor a
/// trailing dot). Asked only where a name is taken anew, a request or a made
/// bot, so a connection an earlier build saved keeps its name.
pub fn reservable_name(name: &str) -> bool {
    valid_name(name) && !hide_platform::fs::identity::win32_rewrites(name)
}

/// A key as a tool makes it: 32 random bytes in lowercase hex.
pub fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A code as a program wrote it, in its one form: capitals, without the
/// dashes and spaces it is shown with; `None` when it is not a code.
pub fn normalize_code(text: &str) -> Option<String> {
    let code: String = text
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    (code.len() == CODE_LEN && code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)))
        .then_some(code)
}

/// A code as the screen and the sentence show it: groups of four.
pub fn spaced_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|group| String::from_utf8_lossy(group).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

/// Whether two bot names are the same name: a home keeps `<name>.key`, and
/// macOS and Windows read file names without case, so `Hermes` and `hermes`
/// are one name everywhere.
pub fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The SHA-256 of `key`, which is all this core keeps of it.
pub fn key_hash(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

/// A request's id, which its connection keeps once approved: sixteen
/// random lowercase hex digits hided draws when it asks (`Call::Request`),
/// so nothing of the key leaves the store; a new key (a request after a
/// disconnect) is a new connection (D-40).
pub fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// `a == b` in time that depends only on their lengths.
fn same_hash(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |differs, (x, y)| differs | (x ^ y))
            == 0
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    /// The program that asked, as the kernel named the requesting process's
    /// parent; shown, never trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    pub created_ms: u64,
    pub last_used_ms: u64,
    pub key_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pending {
    pub id: String,
    pub name: String,
    pub program: Option<String>,
    pub requested_ms: u64,
    key_sha256: String,
}

/// A bot the operator made in Settings that waits for a program to join it
/// with its code (`hide access join`). Memory only: never in the store and
/// never in the snapshot, which carry its name, mode and time alone.
#[derive(Clone, Eq, PartialEq)]
pub struct Invite {
    /// The connection's id once a program joins.
    pub id: String,
    pub name: String,
    pub mode: Mode,
    code: String,
    pub created_ms: u64,
    /// When its code stops joining; past it the row stays, says so and
    /// holds no seat, until the operator asks a new code or cancels it.
    pub expires_ms: u64,
}

impl Invite {
    pub fn live(&self, now_ms: u64) -> bool {
        now_ms < self.expires_ms
    }

    /// The code, for the window that made the bot and nothing else.
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// Leaves the code out, so no diagnostic or panic message carries it.
impl fmt::Debug for Invite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Invite")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("mode", &self.mode)
            .field("expires_ms", &self.expires_ms)
            .finish_non_exhaustive()
    }
}

/// Why a key no longer reaches a request or a connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ended {
    /// The operator refused the request (B7).
    Refused,
    /// A newer request pushed it out of the four (B9).
    Expired,
    /// The operator disconnected the tool (B13).
    Revoked,
    /// A View only connection of an earlier build: a bot runs everything a
    /// connection at Act ran, so it waits for the operator to approve it
    /// again rather than gain that unasked (mission-87 D4).
    Reapprove,
}

impl Ended {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Refused => "access_refused",
            Self::Expired => "access_request_expired",
            Self::Revoked => "access_revoked",
            Self::Reapprove => "access_reapprove",
        }
    }
}

/// Where a key stands, as `hide access request` and `status` answer it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Standing {
    Pending {
        id: String,
        name: String,
    },
    Connected(Connection),
    Ended(Ended),
    /// This core does not know the key: never asked, or asked before this
    /// core last started.
    Unknown,
}

/// A request's refusal before it reaches the operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestRefusal {
    /// Sixteen tools are connected (B10).
    Limit,
    /// A connected tool, or a bot the operator made, already has this name
    /// (D-20).
    NameTaken,
}

impl RequestRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Limit => "access_limit",
            Self::NameTaken => "access_name_taken",
        }
    }
}

/// What one request did, for the diagnostic record and the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Requested {
    pub standing: Standing,
    /// A new request was put up, or a waiting one of the same name merged.
    pub put_up: bool,
    /// The request this one pushed out of the four, by id.
    pub expired: Option<String>,
}

/// The context a tool's command carries when the folder it ran in is no
/// registered checkout: its connection on the core's machine, with no
/// checkout, as Desk's carries Desk (`crate::desk::desk_context`).
pub fn access_context(node: &str, connection: &str) -> crate::workspace_control::Context {
    let identity = format!(
        "{}{}:{connection}",
        crate::delivery::HIDE_PREFIX,
        crate::delivery::HIDE_ACCESS_ROLE
    );
    crate::workspace_control::Context {
        device_id: node.into(),
        workspace_id: identity.clone(),
        checkout_id: identity,
        checkout_path: String::new(),
    }
}

/// What hided asks the core for a tool. Each names the key by its hash
/// alone; hided hashes the key the tool sent and forgets it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Call {
    /// `hide access request`: put up, or answer where the key stands.
    Request {
        name: String,
        key_sha256: String,
        program: Option<String>,
        /// The id a request this call puts up takes ([`valid_id`]).
        id: String,
    },
    /// `hide access status`: where the key stands, putting nothing up.
    Status { key_sha256: String },
    /// `hide access join <code>`: the bot the operator made with this code
    /// becomes the connection of this key, or a key already joined answers
    /// where it stands.
    Join {
        code: String,
        key_sha256: String,
        program: Option<String>,
    },
    /// The operator makes a bot in Settings (a window's call only): hided
    /// draws its id and code.
    Invite {
        id: String,
        code: String,
        name: String,
        mode: Mode,
    },
    /// A new code for a made bot, with ten minutes again.
    Renew { id: String, code: String },
    /// A made bot's code again, while it is live, for the sentence the
    /// window copies.
    Code { id: String },
    /// `hide access leave`: the tool ends its own connection (B14).
    Leave { key_sha256: String },
    /// A tool's command: the connection its key proves, for a credential.
    Prove { key_sha256: String },
    /// A command run on that credential: the connection's mode now (B12,
    /// B13), which also moves its last use (B34).
    Check { id: String },
    /// `hide access list`: the section as the screen reads it (B17).
    List,
}

/// The connection list as its file keeps it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// Never written: the key hashes of schema 1's View only connections,
    /// which [`Access::restored`] ends as [`Ended::Reapprove`] (D4). The file
    /// keeps them until its next save, so a restart before that tells the
    /// tool the same.
    #[serde(skip)]
    pub reapprove: Vec<String>,
}

/// A schema 1 connection: a level where schema 2 has a mode.
#[derive(Deserialize)]
struct ConnectionV1 {
    id: String,
    name: String,
    level: LevelV1,
    #[serde(default)]
    program: Option<String>,
    created_ms: u64,
    last_used_ms: u64,
    key_sha256: String,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LevelV1 {
    View,
    Act,
}

#[derive(Deserialize)]
struct StoredV1 {
    #[serde(default)]
    connections: Vec<ConnectionV1>,
}

pub(crate) fn encode(stored: &Stored) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(stored).map_err(|error| error.to_string())
}

/// Reads the file. Schema 1 moves on load (D4): a connection at Act becomes
/// the whole Hide, which is what Act reached; one at View only is left out
/// and its key waits for the operator's approval again, so no tool reaches
/// more than the operator approved.
pub(crate) fn decode(bytes: &[u8]) -> Result<Stored, String> {
    #[derive(Deserialize)]
    struct Version {
        #[serde(default)]
        schema_version: u32,
    }
    let version: Version = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut stored = match version.schema_version {
        0 | 1 => {
            let old: StoredV1 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            let (act, view): (Vec<_>, Vec<_>) = old
                .connections
                .into_iter()
                .partition(|connection| connection.level == LevelV1::Act);
            Stored {
                schema_version: SCHEMA_VERSION,
                connections: act
                    .into_iter()
                    .map(|connection| Connection {
                        id: connection.id,
                        name: connection.name,
                        mode: Mode::All,
                        program: connection.program,
                        created_ms: connection.created_ms,
                        last_used_ms: connection.last_used_ms,
                        key_sha256: connection.key_sha256,
                    })
                    .collect(),
                reapprove: view
                    .into_iter()
                    .map(|connection| connection.key_sha256)
                    .filter(|hash| hash.len() == 64)
                    .take(MAX_CONNECTIONS)
                    .collect(),
            }
        }
        SCHEMA_VERSION => serde_json::from_slice(bytes).map_err(|error| error.to_string())?,
        newer => return Err(format!("schema {newer} is newer than this build reads")),
    };
    stored
        .connections
        .retain(|connection| valid_name(&connection.name) && connection.key_sha256.len() == 64);
    stored.connections.truncate(MAX_CONNECTIONS);
    Ok(stored)
}

/// The connections, the waiting requests and the recently ended keys. Pure:
/// the runtime saves, records and publishes what each step changed.
#[derive(Debug, Default)]
pub struct Access {
    connections: Vec<Connection>,
    pending: Vec<Pending>,
    invites: Vec<Invite>,
    ended: VecDeque<(String, Ended)>,
}

impl Access {
    pub fn restored(stored: Stored) -> Self {
        let mut access = Self {
            connections: stored.connections,
            ..Self::default()
        };
        for key_sha256 in stored.reapprove {
            access.end(key_sha256, Ended::Reapprove);
        }
        access
    }

    pub fn stored(&self) -> Stored {
        Stored {
            schema_version: SCHEMA_VERSION,
            connections: self.connections.clone(),
            reapprove: Vec::new(),
        }
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    pub fn invites(&self) -> &[Invite] {
        &self.invites
    }

    /// The seats taken of the sixteen: the connections and the made bots
    /// whose code is live.
    fn seats(&self, now_ms: u64) -> usize {
        self.connections.len()
            + self
                .invites
                .iter()
                .filter(|invite| invite.live(now_ms))
                .count()
    }

    /// Whether a connection or a made bot holds `name`.
    fn bot_named(&self, name: &str) -> bool {
        self.connections
            .iter()
            .any(|connection| same_name(&connection.name, name))
            || self
                .invites
                .iter()
                .any(|invite| same_name(&invite.name, name))
    }

    pub fn connection(&self, id: &str) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    /// Where the key whose hash is `key_sha256` stands. Every record is
    /// compared, so the time taken does not say which one matched.
    pub fn standing(&self, key_sha256: &str) -> Standing {
        let mut found = Standing::Unknown;
        for connection in &self.connections {
            if same_hash(&connection.key_sha256, key_sha256) {
                found = Standing::Connected(connection.clone());
            }
        }
        for pending in &self.pending {
            if same_hash(&pending.key_sha256, key_sha256) {
                found = Standing::Pending {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                };
            }
        }
        if found == Standing::Unknown
            && let Some((_, ended)) = self
                .ended
                .iter()
                .rev()
                .find(|(hash, _)| same_hash(hash, key_sha256))
        {
            found = Standing::Ended(*ended);
        }
        found
    }

    /// `hide access request` for `name` with the key whose hash is
    /// `key_sha256`. A key already known answers where it stands, so the
    /// same command asked again converges (engineering 11); a new key is
    /// put up for the operator, merged with a waiting request of the same
    /// name, and past four the oldest waiting request is dropped (B9).
    pub fn request(
        &mut self,
        name: &str,
        key_sha256: &str,
        program: Option<String>,
        now_ms: u64,
        id: String,
    ) -> Result<Requested, RequestRefusal> {
        let standing = self.standing(key_sha256);
        if standing != Standing::Unknown {
            return Ok(Requested {
                standing,
                put_up: false,
                expired: None,
            });
        }
        if self.bot_named(name) {
            return Err(RequestRefusal::NameTaken);
        }
        if self.seats(now_ms) >= MAX_CONNECTIONS {
            return Err(RequestRefusal::Limit);
        }
        let request = Pending {
            id: id.clone(),
            name: name.to_owned(),
            program,
            requested_ms: now_ms,
            key_sha256: key_sha256.to_owned(),
        };
        let mut expired = None;
        if let Some(same) = self
            .pending
            .iter()
            .position(|pending| same_name(&pending.name, name))
        {
            // One waiting request per name (B9): the newer key takes its
            // place, and the older key's next look reads it as expired.
            let earlier = self.pending.remove(same);
            self.end(earlier.key_sha256, Ended::Expired);
        } else if self.pending.len() >= MAX_PENDING {
            let oldest = self.pending.remove(0);
            expired = Some(oldest.id.clone());
            self.end(oldest.key_sha256, Ended::Expired);
        }
        self.pending.push(request);
        Ok(Requested {
            standing: Standing::Pending {
                id,
                name: name.to_owned(),
            },
            put_up: true,
            expired,
        })
    }

    /// The operator approves the waiting request `id` in `mode` (B4).
    pub fn approve(
        &mut self,
        id: &str,
        mode: Mode,
        now_ms: u64,
    ) -> Result<&Connection, &'static str> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id == id)
            .ok_or("access_request_gone")?;
        if self.seats(now_ms) >= MAX_CONNECTIONS {
            return Err("access_limit");
        }
        if self.bot_named(&self.pending[index].name) {
            return Err("access_name_taken");
        }
        let pending = self.pending.remove(index);
        self.connections.push(Connection {
            id: pending.id,
            name: pending.name,
            mode,
            program: pending.program,
            created_ms: now_ms,
            last_used_ms: now_ms,
            key_sha256: pending.key_sha256,
        });
        Ok(self.connections.last().expect("just pushed"))
    }

    /// The operator makes a bot named `name` in `mode`, which a program
    /// joins with `code` for ten minutes. Its name may be no connection's,
    /// request's or other made bot's; it takes a seat while its code is
    /// live; and past four, the oldest made bot whose code ended goes, or,
    /// all four live, none is made.
    pub fn invite(
        &mut self,
        id: String,
        code: String,
        name: &str,
        mode: Mode,
        now_ms: u64,
    ) -> Result<&Invite, &'static str> {
        if !reservable_name(name)
            || !valid_id(&id)
            || normalize_code(&code).as_deref() != Some(&code)
        {
            return Err("invalid_request");
        }
        if self.bot_named(name)
            || self
                .pending
                .iter()
                .any(|pending| same_name(&pending.name, name))
        {
            return Err("access_name_taken");
        }
        if self.seats(now_ms) >= MAX_CONNECTIONS {
            return Err("access_limit");
        }
        if self.invites.len() >= MAX_INVITES {
            let ended = self
                .invites
                .iter()
                .position(|invite| !invite.live(now_ms))
                .ok_or("access_invite_limit")?;
            self.invites.remove(ended);
        }
        self.invites.push(Invite {
            id,
            name: name.to_owned(),
            mode,
            code,
            created_ms: now_ms,
            expires_ms: now_ms + INVITE_TTL_MS,
        });
        Ok(self.invites.last().expect("just pushed"))
    }

    /// A new code for made bot `id`, live for ten minutes from now; one
    /// whose code had ended takes a seat again, if one is free.
    pub fn renew(&mut self, id: &str, code: String, now_ms: u64) -> Result<&Invite, &'static str> {
        if normalize_code(&code).as_deref() != Some(&code) {
            return Err("invalid_request");
        }
        let index = self
            .invites
            .iter()
            .position(|invite| invite.id == id)
            .ok_or("access_invite_gone")?;
        if !self.invites[index].live(now_ms) && self.seats(now_ms) >= MAX_CONNECTIONS {
            return Err("access_limit");
        }
        let invite = &mut self.invites[index];
        invite.code = code;
        invite.expires_ms = now_ms + INVITE_TTL_MS;
        Ok(invite)
    }

    /// Made bot `id` while its code is live, for the sentence its window
    /// copies again.
    pub fn invite_code(&self, id: &str, now_ms: u64) -> Result<&Invite, &'static str> {
        let invite = self
            .invites
            .iter()
            .find(|invite| invite.id == id)
            .ok_or("access_invite_gone")?;
        if invite.live(now_ms) {
            Ok(invite)
        } else {
            Err("access_invite_expired")
        }
    }

    /// The operator takes back made bot `id`; its code joins nothing more.
    pub fn cancel(&mut self, id: &str) -> Result<Invite, &'static str> {
        let index = self
            .invites
            .iter()
            .position(|invite| invite.id == id)
            .ok_or("access_invite_gone")?;
        Ok(self.invites.remove(index))
    }

    /// `hide access join <code>` with the key whose hash is `key_sha256`. A
    /// key already known answers where it stands, so a join whose answer was
    /// lost converges when run again (engineering 11); otherwise the made
    /// bot whose live code this is becomes the key's connection, under the
    /// name and mode the operator gave it, and the code is spent. Every made
    /// bot's code is compared, so the time taken says nothing of which.
    pub fn join(
        &mut self,
        code: &str,
        key_sha256: &str,
        program: Option<String>,
        now_ms: u64,
    ) -> Result<Requested, &'static str> {
        let standing = self.standing(key_sha256);
        if standing != Standing::Unknown {
            return Ok(Requested {
                standing,
                put_up: false,
                expired: None,
            });
        }
        let mut found = None;
        for (index, invite) in self.invites.iter().enumerate() {
            if same_hash(&invite.code, code) & invite.live(now_ms) {
                found = Some(index);
            }
        }
        let index = found.ok_or("access_code_invalid")?;
        // A live code holds its seat, so the list has room for it.
        let invite = self.invites.remove(index);
        self.connections.push(Connection {
            id: invite.id,
            name: invite.name,
            mode: invite.mode,
            program,
            created_ms: now_ms,
            last_used_ms: now_ms,
            key_sha256: key_sha256.to_owned(),
        });
        let connection = self.connections.last().expect("just pushed").clone();
        Ok(Requested {
            standing: Standing::Connected(connection),
            put_up: true,
            expired: None,
        })
    }

    /// The operator refuses the waiting request `id` (B7).
    pub fn refuse(&mut self, id: &str) -> Result<Pending, &'static str> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id == id)
            .ok_or("access_request_gone")?;
        let pending = self.pending.remove(index);
        self.end(pending.key_sha256.clone(), Ended::Refused);
        Ok(pending)
    }

    /// The operator sets connection `id`'s mode; the bot's next command
    /// runs in it (B12). Whether it changed.
    pub fn set_mode(&mut self, id: &str, mode: Mode) -> Result<bool, &'static str> {
        let connection = self
            .connections
            .iter_mut()
            .find(|connection| connection.id == id)
            .ok_or("access_connection_gone")?;
        let changed = connection.mode != mode;
        connection.mode = mode;
        Ok(changed)
    }

    /// Ends connection `id`: the operator's disconnect, or the tool's own
    /// `hide access leave`, which is the same (D-32).
    pub fn revoke(&mut self, id: &str) -> Result<Connection, &'static str> {
        let index = self
            .connections
            .iter()
            .position(|connection| connection.id == id)
            .ok_or("access_connection_gone")?;
        let connection = self.connections.remove(index);
        self.end(connection.key_sha256.clone(), Ended::Revoked);
        Ok(connection)
    }

    /// Notes that connection `id` ran a command at `now_ms`. Its last use
    /// moves only once a minute (B34); whether it moved, so the store is
    /// saved and the screen told no more often than that.
    pub fn used(&mut self, id: &str, now_ms: u64) -> bool {
        match self
            .connections
            .iter_mut()
            .find(|connection| connection.id == id)
        {
            Some(connection)
                if now_ms.saturating_sub(connection.last_used_ms) >= USE_INTERVAL_MS =>
            {
                connection.last_used_ms = now_ms;
                true
            }
            _ => false,
        }
    }

    fn end(&mut self, key_sha256: String, ended: Ended) {
        if self.ended.len() >= MAX_ENDED {
            self.ended.pop_front();
        }
        self.ended.push_back((key_sha256, ended));
    }
}

/// What a bot's mode must allow for a command (B20-B23, mission-87 D3). The
/// one table: hided judges a bot's command by it at the command boundary,
/// and the core again when the command runs (`runtime::access`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Need {
    /// Reaches agents, panes or missions, or only the bot's own letters and
    /// watches: every bot runs it, and the core holds an own-mode bot's to
    /// its own work where the command names or lists what it reaches
    /// (`runtime::access`, `Runtime::own_work`).
    Work,
    /// Reaches what is no one bot's work: the operator's screen (Views,
    /// files, the project's records), devices and the settings-wide state.
    /// The whole Hide only.
    All,
    /// The operator's alone, whatever the mode: adding or removing a device
    /// (operator decision 2026-11-21), as approving a bot is.
    Operator,
    /// Means an agent in a Hide pane: no bot runs it; the bot starts an
    /// agent and asks it instead (B22).
    Agent,
    /// A spawn under a parent: a bot is never one (B23).
    Parent,
}

/// An own-mode bot reached past its own work.
pub const NOT_OWN_WORK: &str = "access_not_own_work";
/// What only the operator does, such as adding or removing a device.
pub const OPERATOR_ONLY: &str = "access_operator_only";
/// What a bot refused [`NOT_OWN_WORK`] is told, by the core's refusals and
/// hided's alike.
pub const NOT_OWN_WORK_NEXT_ACTION: &str = "This bot acts on its own work only: the agents it started, the agents under them and the missions they conduct; start an agent with hide agent spawn and ask it, or ask the operator to give the bot the whole Hide in Settings";
/// A command only an agent in a Hide pane can mean (B22).
pub const NEEDS_AGENT: &str = "access_needs_agent";
/// A bot's spawn named a parent (B23).
pub const SPAWN_PARENT: &str = "access_spawn_parent";

/// Whether a bot in `mode` runs a command that needs `need`, or the reason
/// it is refused. What a `Need::Work` command of an own-mode bot names is
/// judged where it is read, against its own work.
pub fn admit(mode: Mode, need: Need) -> Result<(), &'static str> {
    match (mode, need) {
        (_, Need::Work) | (Mode::All, Need::All) => Ok(()),
        (Mode::Own, Need::All) => Err(NOT_OWN_WORK),
        (_, Need::Operator) => Err(OPERATOR_ONLY),
        (_, Need::Agent) => Err(NEEDS_AGENT),
        (_, Need::Parent) => Err(SPAWN_PARENT),
    }
}

/// What a delivery command needs: listing, showing, spawning and ending
/// agents and the bot's letters and watches reach work, a spawn
/// under a parent never runs, and what names the caller's own pane
/// (`here`, `--beside`, a plan node, registering, asking a person, a hook's
/// reads) needs an agent. A spawn with a task is Hide's own, which the
/// spawn's rule refuses any other caller (`coordination`).
pub fn delivery_need(command: &crate::delivery::Command) -> Need {
    use crate::coordination::Command as Agents;
    use crate::delivery::Command as Delivery;
    match command {
        Delivery::Agents { command } => match command {
            Agents::Show { id } if id == crate::coordination::HERE => Need::Agent,
            Agents::Spawn {
                parent: Some(_), ..
            } => Need::Parent,
            Agents::Spawn {
                beside, plan_node, ..
            } if *beside || plan_node.is_some() => Need::Agent,
            Agents::List { .. }
            | Agents::Show { .. }
            | Agents::Spawn { .. }
            | Agents::End { .. } => Need::Work,
            Agents::RegisterSelf { .. }
            | Agents::Register { .. }
            | Agents::AskHuman { .. }
            | Agents::AskHumanClear => Need::Agent,
        },
        // A hook's own reads: a runtime in a pane asks them.
        Delivery::Pull { .. } | Delivery::Confirm { .. } => Need::Agent,
        Delivery::Send { .. }
        | Delivery::Reply { .. }
        | Delivery::Ack { .. }
        | Delivery::AckBundle { .. }
        | Delivery::Cancel { .. }
        | Delivery::Show { .. }
        | Delivery::Inbox { .. }
        | Delivery::WatchStart { .. }
        | Delivery::WatchStop { .. }
        | Delivery::WatchAssign { .. }
        | Delivery::WatchList => Need::Work,
    }
}

/// What a pane command needs: reading a pane and typing into one reach the
/// work it runs, and opening a pane beside the caller or naming its
/// purpose needs the caller's own pane.
pub fn pane_need(command: &crate::pane_command::PaneCommand) -> Need {
    use crate::pane_command::PaneCommand;
    match command {
        PaneCommand::Read { .. }
        | PaneCommand::ReadScreen { .. }
        | PaneCommand::AgentRead { .. }
        | PaneCommand::Send { .. } => Need::Work,
        PaneCommand::Open { .. } | PaneCommand::Purpose { .. } => Need::Agent,
    }
}

/// What a mission command needs: reading missions reaches their work; a bot leads none, a Conductor it starts does (D-12).
pub fn mission_need(command: &crate::missions::MissionCommand) -> Need {
    use crate::missions::MissionCommand;
    match command {
        MissionCommand::List | MissionCommand::Show { .. } => Need::Work,
        _ => Need::Agent,
    }
}

/// What a device command needs: devices are no bot's work, and adding or
/// removing one is the operator's alone.
pub fn device_need(command: &crate::device_command::DeviceCommand) -> Need {
    use crate::device_command::DeviceCommand;
    match command {
        DeviceCommand::List | DeviceCommand::Retry { .. } => Need::All,
        DeviceCommand::Add { .. } | DeviceCommand::Remove { .. } => Need::Operator,
    }
}

/// How another machine's core stands toward this one (PRD
/// device-connect-iroh D-72), read from the pairing facts hided tells the
/// core (`crate::peer_machines`). Its agents' commands enter here as a
/// bot's of the whole Hide do ([`Mode::All`]), and the same table judges
/// them: a machine is opened whole or not at all, so none stands in own
/// mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreStanding {
    /// It holds an active grant to open this machine: what it asks runs as
    /// a bot of the whole Hide would run it.
    Opens,
    /// Only this machine opens it. It asked for nothing here, so it writes
    /// nothing first: it answers a letter it got and sends the result
    /// report of what was started there, as a tool hears only what it
    /// asked for.
    Answers,
}

/// A machine with no active pairing in either direction asked, or this
/// machine's agent named one it may not open.
pub const MACHINE_NOT_PERMITTED: &str = "machine_not_permitted";

/// What an agent refused with [`MACHINE_NOT_PERMITTED`] does next.
pub const MACHINE_NOT_PERMITTED_NEXT: &str = "Another machine's core does this only for a machine it opened itself to: an agent on the machine that connected (hide device list) lists, shows, reads and starts agents there and writes to them; an agent there answers with hide request reply and reports to the agent that started it, and writes nothing first. hide pane send types into no pane of another machine's core, and an agent on a connected device cannot use --machine";

/// Where `machine` stands: it opens this machine while its grant here is
/// active, and otherwise it only answers while this machine's grant there
/// is. A paused, unconfirmed or removed pairing stands nowhere.
pub fn core_standing(machine: &crate::peer_machines::PeerMachine) -> Option<CoreStanding> {
    use crate::peer_machines::{PeerOpenedBy, PeerOpens};
    if machine.opened_by == Some(PeerOpenedBy::Active) {
        Some(CoreStanding::Opens)
    } else if machine.opens == Some(PeerOpens::Active) {
        Some(CoreStanding::Answers)
    } else {
        None
    }
}

/// Where this machine stands on `machine`, the mirror of
/// [`core_standing`]: it opens that machine while its own grant there is
/// active, and otherwise only answers it while that machine's grant here
/// is. An agent's command for another core is judged by it before it
/// leaves, so a letter the other core would refuse is refused at once.
pub fn core_standing_there(machine: &crate::peer_machines::PeerMachine) -> Option<CoreStanding> {
    use crate::peer_machines::{PeerOpenedBy, PeerOpens};
    if machine.opens == Some(PeerOpens::Active) {
        Some(CoreStanding::Opens)
    } else if machine.opened_by == Some(PeerOpenedBy::Active) {
        Some(CoreStanding::Answers)
    } else {
        None
    }
}

/// What a delivery command another core's agent asks is, to a machine that
/// only answers ([`CoreStanding::Answers`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreAsk {
    /// A reply to a letter.
    Reply,
    /// A `report` letter to an agent that asked that machine for a start
    /// (`ledger::CoreStart`).
    ResultReport,
    /// Anything else, a first letter among them.
    First,
}

/// Whether a core standing as `standing` runs `command` here, or the reason
/// it is refused. A core that opens this machine is a bot of the whole
/// Hide ([`admit`], [`delivery_need`]); one that only answers runs a reply and
/// the result report of a start this core asked it for, and is refused the
/// rest as a machine that may not write first.
pub fn core_admit(
    standing: CoreStanding,
    command: &crate::delivery::Command,
    ask: CoreAsk,
) -> Result<(), &'static str> {
    admit(Mode::All, delivery_need(command))?;
    match (standing, ask) {
        (CoreStanding::Opens, _)
        | (CoreStanding::Answers, CoreAsk::Reply | CoreAsk::ResultReport) => Ok(()),
        (CoreStanding::Answers, CoreAsk::First) => Err(MACHINE_NOT_PERMITTED),
    }
}

/// Whether a core standing as `standing` runs the read or the start
/// `asked` here. Only a core that opens this machine asks one, and only
/// these: listing and showing agents, reading a pane, and a start, each as
/// the bot table judges it for the whole Hide, so a start under a parent,
/// beside a pane or for a plan step is refused as a bot's is. Typing into a
/// pane and everything else such a bot may do is not another machine's.
pub fn core_call_admit(
    standing: CoreStanding,
    asked: &crate::core_calls::Asked,
) -> Result<(), &'static str> {
    use crate::coordination::Command as Agents;
    use crate::core_calls::Asked;
    use crate::delivery::Command as Delivery;
    use crate::pane_command::PaneCommand;
    if standing != CoreStanding::Opens {
        return Err(MACHINE_NOT_PERMITTED);
    }
    match asked {
        Asked::Delivery(command)
            if matches!(
                **command,
                Delivery::Agents {
                    command: Agents::List { .. } | Agents::Show { .. } | Agents::Spawn { .. },
                }
            ) =>
        {
            admit(Mode::All, delivery_need(command))
        }
        Asked::Pane(
            command @ (PaneCommand::Read { .. }
            | PaneCommand::ReadScreen { .. }
            | PaneCommand::AgentRead { .. }),
        ) => admit(Mode::All, pane_need(command)),
        _ => Err(MACHINE_NOT_PERMITTED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> String {
        key_hash(&format!("{n:064x}"))
    }

    fn id(n: u8) -> String {
        format!("{n:016x}")
    }

    #[test]
    fn a_request_asked_again_is_the_same_request_and_after_approval_the_same_connection() {
        let mut access = Access::default();
        let first = access.request("hermes", &key(1), None, 1, id(1)).unwrap();
        assert!(first.put_up);
        // Asked again, it keeps its id, whatever id this ask brought.
        let again = access.request("hermes", &key(1), None, 2, id(9)).unwrap();
        assert!(!again.put_up);
        assert_eq!(again.standing, first.standing);
        let Standing::Pending { id: pending, .. } = first.standing else {
            panic!("pending")
        };
        assert_eq!(pending, id(1));
        access.approve(&pending, Mode::All, 3).unwrap();
        let connected = access.request("hermes", &key(1), None, 4, id(1)).unwrap();
        assert!(matches!(
            connected.standing,
            Standing::Connected(Connection {
                mode: Mode::All,
                ..
            })
        ));
        assert_eq!(
            access
                .request("hermes", &key(1), None, 5, id(1))
                .unwrap()
                .standing,
            connected.standing
        );
    }

    #[test]
    fn requests_of_one_name_merge_and_a_fifth_name_drops_the_oldest() {
        let mut access = Access::default();
        access.request("a", &key(1), None, 1, id(1)).unwrap();
        access.request("a", &key(2), None, 2, id(2)).unwrap();
        assert_eq!(access.pending().len(), 1);
        assert_eq!(access.standing(&key(1)), Standing::Ended(Ended::Expired));
        for (n, name) in [(3, "b"), (4, "c"), (5, "d")] {
            access
                .request(name, &key(n), None, u64::from(n), id(n))
                .unwrap();
        }
        let fifth = access.request("e", &key(6), None, 6, id(6)).unwrap();
        assert_eq!(fifth.expired.as_deref(), Some(id(2).as_str()));
        assert_eq!(access.pending().len(), MAX_PENDING);
        assert_eq!(access.standing(&key(2)), Standing::Ended(Ended::Expired));
    }

    #[test]
    fn sixteen_connections_refuse_a_new_request_before_it_waits() {
        let mut access = Access::default();
        for n in 0..MAX_CONNECTIONS as u8 {
            let Standing::Pending { id, .. } = access
                .request(&format!("t{n}"), &key(n), None, 0, id(n))
                .unwrap()
                .standing
            else {
                panic!("pending")
            };
            access.approve(&id, Mode::All, 0).unwrap();
        }
        assert_eq!(
            access
                .request("one-more", &key(200), None, 0, id(200))
                .unwrap_err(),
            RequestRefusal::Limit
        );
        assert!(access.pending().is_empty());
    }

    #[test]
    fn refused_and_revoked_keys_say_why_and_a_new_key_asks_again() {
        let mut access = Access::default();
        access.request("t", &key(1), None, 0, id(1)).unwrap();
        access.refuse(&id(1)).unwrap();
        assert_eq!(access.standing(&key(1)), Standing::Ended(Ended::Refused));
        assert!(access.request("t", &key(2), None, 0, id(2)).unwrap().put_up);
        access.approve(&id(2), Mode::All, 0).unwrap();
        access.revoke(&id(2)).unwrap();
        assert_eq!(access.standing(&key(2)), Standing::Ended(Ended::Revoked));
        assert!(access.connections().is_empty());
    }

    #[test]
    fn a_connected_name_is_not_requested_twice() {
        let mut access = Access::default();
        access.request("t", &key(1), None, 0, id(1)).unwrap();
        access.approve(&id(1), Mode::All, 0).unwrap();
        assert_eq!(
            access.request("t", &key(2), None, 0, id(2)).unwrap_err(),
            RequestRefusal::NameTaken
        );
    }

    const CODE: &str = "K7Q4MZ2PX3RD";
    const OTHER_CODE: &str = "0123456789AB";

    fn connect(access: &mut Access, n: u8) {
        access
            .request(&format!("t{n}"), &key(n), None, 0, id(n))
            .unwrap();
        access.approve(&id(n), Mode::All, 0).unwrap();
    }

    #[test]
    fn a_made_bot_joins_once_with_its_code_and_a_lost_answer_converges() {
        let mut access = Access::default();
        access
            .invite(id(1), CODE.into(), "hermes", Mode::Own, 0)
            .unwrap();
        let joined = access.join(CODE, &key(1), None, 1).unwrap();
        assert!(joined.put_up);
        let Standing::Connected(connection) = &joined.standing else {
            panic!("connected")
        };
        assert_eq!(
            (
                connection.id.as_str(),
                connection.name.as_str(),
                connection.mode
            ),
            (id(1).as_str(), "hermes", Mode::Own)
        );
        assert!(access.invites().is_empty());
        // The same key run again, its first answer lost, is the same bot.
        let again = access.join(CODE, &key(1), None, 2).unwrap();
        assert!(!again.put_up);
        assert_eq!(again.standing, joined.standing);
        // The code is spent: another key gets nothing.
        assert_eq!(
            access.join(CODE, &key(2), None, 2).unwrap_err(),
            "access_code_invalid"
        );
        assert_eq!(access.connections().len(), 1);
    }

    #[test]
    fn an_ended_code_joins_nothing_and_a_new_code_replaces_it() {
        let mut access = Access::default();
        access
            .invite(id(1), CODE.into(), "hermes", Mode::Own, 0)
            .unwrap();
        assert_eq!(
            access.join(CODE, &key(1), None, INVITE_TTL_MS).unwrap_err(),
            "access_code_invalid"
        );
        assert_eq!(
            access.invite_code(&id(1), INVITE_TTL_MS).unwrap_err(),
            "access_invite_expired"
        );
        access
            .renew(&id(1), OTHER_CODE.into(), INVITE_TTL_MS)
            .unwrap();
        assert_eq!(
            access
                .join(CODE, &key(1), None, INVITE_TTL_MS + 1)
                .unwrap_err(),
            "access_code_invalid"
        );
        assert!(
            access
                .join(OTHER_CODE, &key(1), None, INVITE_TTL_MS + 1)
                .unwrap()
                .put_up
        );
    }

    #[test]
    fn a_made_bot_holds_a_seat_while_its_code_is_live_and_its_name_while_it_waits() {
        let mut access = Access::default();
        for n in 0..(MAX_CONNECTIONS - 1) as u8 {
            connect(&mut access, n);
        }
        access
            .invite(id(100), CODE.into(), "hermes", Mode::Own, 0)
            .unwrap();
        assert_eq!(
            access
                .request("late", &key(101), None, 1, id(101))
                .unwrap_err(),
            RequestRefusal::Limit
        );
        assert_eq!(
            access
                .invite(id(102), OTHER_CODE.into(), "late", Mode::Own, 1)
                .unwrap_err(),
            "access_limit"
        );
        // Its code ended: the seat is free, the name still its own, in any case.
        let ended = INVITE_TTL_MS;
        assert!(
            access
                .request("late", &key(101), None, ended, id(101))
                .unwrap()
                .put_up
        );
        assert_eq!(
            access
                .request("HERMES", &key(103), None, ended, id(103))
                .unwrap_err(),
            RequestRefusal::NameTaken
        );
        access.approve(&id(101), Mode::All, ended).unwrap();
        // Every seat is taken now, so a new code waits for one.
        assert_eq!(
            access
                .renew(&id(100), OTHER_CODE.into(), ended)
                .unwrap_err(),
            "access_limit"
        );
        assert_eq!(access.cancel(&id(100)).unwrap().name, "hermes");
    }

    #[test]
    fn a_made_bot_takes_no_name_a_connection_a_request_or_another_made_bot_has() {
        let mut access = Access::default();
        connect(&mut access, 1);
        access.request("asking", &key(2), None, 0, id(2)).unwrap();
        access
            .invite(id(3), CODE.into(), "made", Mode::Own, 0)
            .unwrap();
        for name in ["T1", "Asking", "MADE"] {
            assert_eq!(
                access
                    .invite(id(4), OTHER_CODE.into(), name, Mode::Own, 0)
                    .unwrap_err(),
                "access_name_taken",
                "{name}"
            );
        }
    }

    #[test]
    fn past_four_made_bots_the_oldest_whose_code_ended_goes() {
        let mut access = Access::default();
        for n in 0..MAX_INVITES as u8 {
            access
                .invite(
                    id(n),
                    OTHER_CODE.into(),
                    &format!("b{n}"),
                    Mode::Own,
                    u64::from(n),
                )
                .unwrap();
        }
        assert_eq!(
            access
                .invite(id(9), CODE.into(), "fifth", Mode::Own, 5)
                .unwrap_err(),
            "access_invite_limit"
        );
        // The first two have ended; the oldest of them goes.
        access
            .invite(id(9), CODE.into(), "fifth", Mode::Own, INVITE_TTL_MS + 1)
            .unwrap();
        let names: Vec<&str> = access
            .invites()
            .iter()
            .map(|invite| invite.name.as_str())
            .collect();
        assert_eq!(names, ["b1", "b2", "b3", "fifth"]);
    }

    #[test]
    fn a_new_name_is_a_file_name_on_every_system() {
        for name in ["hermes", "night.ly", "a_b-c"] {
            assert!(reservable_name(name), "{name}");
        }
        for name in ["con", "NUL", "com1", "lpt9.key", "hermes.", "", "-x"] {
            assert!(!reservable_name(name), "{name}");
        }
        let mut access = Access::default();
        assert_eq!(
            access
                .invite(id(1), CODE.into(), "aux", Mode::Own, 0)
                .unwrap_err(),
            "invalid_request"
        );
    }

    #[test]
    fn a_code_reads_in_one_form_and_never_reaches_a_debug_line() {
        assert_eq!(normalize_code("k7q4-mz2p-x3rd").as_deref(), Some(CODE));
        assert_eq!(normalize_code("K7Q4 MZ2P X3RD").as_deref(), Some(CODE));
        assert_eq!(spaced_code(CODE), "K7Q4-MZ2P-X3RD");
        for wrong in [
            "K7Q4-MZ2P",
            "K7Q4-MZ2P-X3RDX",
            "I7Q4-MZ2P-X3RD",
            "K7Q4-MZ2P-X3R!",
        ] {
            assert_eq!(normalize_code(wrong), None, "{wrong}");
        }
        let mut access = Access::default();
        access
            .invite(id(1), CODE.into(), "hermes", Mode::Own, 0)
            .unwrap();
        assert_eq!(access.invites()[0].code(), CODE);
        assert!(!format!("{access:?}").contains(CODE));
    }

    #[test]
    fn last_use_moves_once_a_minute() {
        let mut access = Access::default();
        access.request("t", &key(1), None, 0, id(1)).unwrap();
        access.approve(&id(1), Mode::All, 0).unwrap();
        assert!(!access.used(&id(1), USE_INTERVAL_MS - 1));
        assert!(access.used(&id(1), USE_INTERVAL_MS));
        assert!(!access.used(&id(1), USE_INTERVAL_MS + 1));
    }

    #[test]
    fn the_store_keeps_only_the_keys_hash_and_reads_back_what_it_wrote() {
        let mut access = Access::default();
        let raw = format!("{:064x}", 7);
        access
            .request(
                "t",
                &key_hash(&raw),
                Some("/usr/bin/python3".into()),
                0,
                id(7),
            )
            .unwrap();
        access.approve(&id(7), Mode::All, 0).unwrap();
        let bytes = encode(&access.stored()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(&raw));
        assert_eq!(decode(&bytes).unwrap(), access.stored());
    }

    #[test]
    fn every_bot_runs_a_command_on_work_and_only_the_whole_hide_runs_the_rest() {
        assert_eq!(admit(Mode::All, Need::Work), Ok(()));
        assert_eq!(admit(Mode::All, Need::All), Ok(()));
        assert_eq!(admit(Mode::Own, Need::Work), Ok(()));
        assert_eq!(admit(Mode::Own, Need::All), Err(NOT_OWN_WORK));
        for mode in [Mode::All, Mode::Own] {
            assert_eq!(admit(mode, Need::Operator), Err(OPERATOR_ONLY));
            assert_eq!(admit(mode, Need::Agent), Err(NEEDS_AGENT));
            assert_eq!(admit(mode, Need::Parent), Err(SPAWN_PARENT));
        }
    }

    #[test]
    fn a_schema_one_store_moves_act_to_the_whole_hide_and_ends_view_only_for_approval_again() {
        let (act, view) = (key(1), key(2));
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "connections": [
                {"id": id(1), "name": "acts", "level": "act", "created_ms": 1,
                 "last_used_ms": 2, "key_sha256": act},
                {"id": id(2), "name": "looks", "level": "view", "program": "cron",
                 "created_ms": 1, "last_used_ms": 2, "key_sha256": view},
            ],
        }))
        .unwrap();
        let access = Access::restored(decode(&bytes).unwrap());
        let Standing::Connected(moved) = access.standing(&act) else {
            panic!("the Act connection stands")
        };
        assert_eq!((moved.name.as_str(), &moved.mode), ("acts", &Mode::All));
        assert_eq!(access.standing(&view), Standing::Ended(Ended::Reapprove));
        assert_eq!(access.connections().len(), 1);
        // Saved, it is schema 2 and names neither level nor the ended key.
        let saved = encode(&access.stored()).unwrap();
        let text = String::from_utf8_lossy(&saved);
        assert!(text.contains("\"schema_version\": 2"), "{text}");
        assert!(!text.contains("level") && !text.contains(&view), "{text}");
        assert_eq!(decode(&saved).unwrap(), access.stored());
    }

    #[test]
    fn a_mode_reads_and_writes_as_its_kind() {
        for (mode, kind) in [(Mode::All, "all"), (Mode::Own, "own")] {
            let wire = serde_json::json!({"kind": kind});
            assert_eq!(serde_json::to_value(mode).unwrap(), wire);
            assert_eq!(serde_json::from_value::<Mode>(wire).unwrap(), mode);
        }
        assert!(serde_json::from_value::<Mode>(serde_json::json!({"kind": "workspace"})).is_err());
    }

    #[test]
    fn another_core_runs_what_a_tool_at_act_runs_and_one_that_only_answers_writes_nothing_first() {
        use crate::coordination::Command as Agents;
        use crate::delivery::Command as Delivery;
        use crate::peer_machines::{PeerMachine, PeerOpenedBy, PeerOpens};
        let machine = |opens, opened_by| PeerMachine {
            key: "a".repeat(64),
            device_id: "mini".into(),
            name: "Mac mini".into(),
            opens,
            opened_by,
            connected: true,
            path: None,
            reason: None,
            update: None,
        };
        // Standing follows the active pairings only.
        for (opens, opened_by, standing) in [
            (None, Some(PeerOpenedBy::Active), Some(CoreStanding::Opens)),
            (
                Some(PeerOpens::Active),
                Some(PeerOpenedBy::Active),
                Some(CoreStanding::Opens),
            ),
            (Some(PeerOpens::Active), None, Some(CoreStanding::Answers)),
            (
                Some(PeerOpens::Active),
                Some(PeerOpenedBy::Paused),
                Some(CoreStanding::Answers),
            ),
            (None, Some(PeerOpenedBy::Paused), None),
            (Some(PeerOpens::Paused), None, None),
            (Some(PeerOpens::PausedThere), None, None),
            (Some(PeerOpens::Confirming), None, None),
            (Some(PeerOpens::Removing), None, None),
            (Some(PeerOpens::Unpaired), None, None),
        ] {
            assert_eq!(
                core_standing(&machine(opens, opened_by)),
                standing,
                "{opens:?} {opened_by:?}"
            );
        }
        let send = Delivery::Send {
            target: "agent-1".into(),
            intent: "i".into(),
            body: "b".into(),
            kind: "request".into(),
            now: false,
            force: false,
        };
        let reply = Delivery::Reply {
            id: "letter-1".into(),
            intent: "i".into(),
            body: "b".into(),
            now: false,
        };
        let list = Delivery::Agents {
            command: Agents::List {
                live: false,
                machine: None,
            },
        };
        let pull = Delivery::Inbox { bundle: false };
        // A core that opens this machine: the bot table for the whole Hide.
        assert_eq!(
            core_admit(CoreStanding::Opens, &send, CoreAsk::First),
            Ok(())
        );
        assert_eq!(
            core_admit(CoreStanding::Opens, &reply, CoreAsk::Reply),
            Ok(())
        );
        assert_eq!(
            core_admit(CoreStanding::Opens, &list, CoreAsk::First),
            Ok(())
        );
        assert_eq!(
            core_admit(
                CoreStanding::Opens,
                &Delivery::Agents {
                    command: Agents::Show { id: "here".into() }
                },
                CoreAsk::First
            ),
            Err(NEEDS_AGENT)
        );
        // One that only answers: a reply and a result report, nothing first.
        assert_eq!(
            core_admit(CoreStanding::Answers, &reply, CoreAsk::Reply),
            Ok(())
        );
        assert_eq!(
            core_admit(CoreStanding::Answers, &send, CoreAsk::ResultReport),
            Ok(())
        );
        for first in [&send, &list, &pull] {
            assert_eq!(
                core_admit(CoreStanding::Answers, first, CoreAsk::First),
                Err(MACHINE_NOT_PERMITTED)
            );
        }
    }

    #[test]
    fn names_and_keys_are_checked_by_shape() {
        assert!(valid_name("hermes-ctong_1.2"));
        assert!(!valid_name("-x"));
        assert!(!valid_name("a b"));
        assert!(!valid_name(&"a".repeat(NAME_LIMIT + 1)));
        assert!(valid_key(&format!("{:064x}", 1)));
        assert!(!valid_key(&format!("{:064X}", 255)));
        assert!(valid_id("0123456789abcdef"));
        assert!(!valid_id("0123456789ABCDEF"));
        assert!(!valid_id("0123"));
    }
}
