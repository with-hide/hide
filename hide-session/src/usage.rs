//! Content-free usage reads. Checkpoints contain positions and native metadata,
//! never a torn JSON line, prompt, response, or tool result.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Agent, SESSION_INCREMENT_READ_LIMIT_BYTES, SESSION_LINE_LIMIT_BYTES};

mod grok;
mod opencode;
mod records;

pub const CANDIDATE_LIMIT: usize = 10_000;
const DISCOVERY_ENTRY_LIMIT: usize = 100_000;
const DISCOVERY_DEPTH_LIMIT: usize = 16;
const PATH_BYTES_LIMIT: usize = 4096;
pub const PARSER_VERSION: u32 = 3;
/// The model of a call whose records name none (agy's): a ledger key that
/// is never shown as a model's name.
pub const UNNAMED_MODEL: &str = "unnamed";
const STEP_TIME: Duration = Duration::from_millis(10);
const STEP_EVENTS: usize = 8192;
const WITNESS_BYTES: u64 = 256;

/// Disjoint components: output already includes reasoning. One-hour writes
/// are a subset of cache_write, retained only for their different price.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_1h: u64,
    pub reasoning: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub key: String,
    pub at_ms: u64,
    pub model: String,
    pub tokens: Tokens,
    pub recorded_cost_micro_usd: Option<u64>,
    pub session: String,
    pub cwd: Option<String>,
    pub provider: Option<String>,
    /// Native usage is a lower bound. Its recorded cost must not be displayed.
    pub incomplete: bool,
}

/// Errors deliberately carry no filesystem path or source record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Unsupported,
    Unreadable,
    OutsideRoot,
    InvalidCheckpoint,
    Database,
    /// A database that holds none of the schemas Hide reads.
    Schema,
    /// An environment value of the node that the read depends on is unusable.
    Environment,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "usage_read_{self:?}")
    }
}
impl std::error::Error for Refusal {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Absolute node-qualified spelling, normalized by hide-platform.
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Candidates {
    pub files: Vec<Candidate>,
    pub capped: bool,
    pub skipped: u64,
}

/// Bounded state only. The offset rewinds to a torn record's beginning;
/// oversized records carry only the fact that their remainder is discarded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub offset: u64,
    identity: Option<String>,
    discarding: bool,
    parser_version: u32,
    context: records::Context,
    database: opencode::Cursor,
    #[serde(default)]
    witness: Option<Witness>,
    #[serde(default)]
    finished: bool,
}

/// Hashes only, never source bytes. The prefix and last consumed bytes catch
/// replacement during append; a same-size rewrite is also fenced by its stamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Witness {
    size: u64,
    modified_ns: u64,
    digest: String,
}

impl Checkpoint {
    pub fn is_finished(&self) -> bool {
        self.parser_version == PARSER_VERSION && self.finished
    }
}

fn witness(file: &mut fs::File, offset: u64) -> Result<(Witness, u64), Refusal> {
    let metadata = file.metadata().map_err(|_| Refusal::Unreadable)?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .ok_or(Refusal::Unreadable)?;
    let count = offset.min(WITNESS_BYTES);
    let mut digest = Sha256::new();
    for start in [0, offset.saturating_sub(count)] {
        file.seek(SeekFrom::Start(start))
            .map_err(|_| Refusal::Unreadable)?;
        let mut bytes = vec![0; count as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| Refusal::Unreadable)?;
        digest.update(bytes);
    }
    Ok((
        Witness {
            size: metadata.len(),
            modified_ns,
            digest: format!("{:x}", digest.finalize()),
        },
        count * 2,
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub events: Vec<Event>,
    pub checkpoint: Checkpoint,
    pub read_bytes: u64,
    pub has_more: bool,
    pub rescanned: bool,
    pub skipped: BTreeMap<String, u64>,
}

fn roots(home: &Path, agent: Agent) -> Result<Vec<PathBuf>, Refusal> {
    match agent {
        Agent::Cursor => Err(Refusal::Unsupported),
        Agent::OpenCode => Ok(vec![
            crate::opencode::database_path(home).map_err(|_| Refusal::Environment)?,
        ]),
        Agent::Codex => Ok(vec![
            home.join(crate::CODEX_SESSIONS),
            home.join(".codex/archived_sessions"),
        ]),
        _ => Ok(vec![
            crate::session_root(home, agent).ok_or(Refusal::Unsupported)?,
        ]),
    }
}

/// Lists native candidates without reading conversation bodies or following
/// directory links. The discovery work and the retained candidates are capped
/// independently, including folders containing no session files.
pub fn list(home: &Path, agent: Agent, since_ms: u64) -> Result<Candidates, Refusal> {
    let mut out = Candidates::default();
    let mut pending: Vec<_> = roots(home, agent)?.into_iter().map(|p| (p, 0)).collect();
    let mut visited = 0;
    while let Some((path, depth)) = pending.pop() {
        visited += 1;
        if visited > DISCOVERY_ENTRY_LIMIT
            || path.as_os_str().as_encoded_bytes().len() > PATH_BYTES_LIMIT
        {
            out.capped = true;
            break;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        if metadata.is_dir() {
            if depth == DISCOVERY_DEPTH_LIMIT {
                out.capped = true;
                continue;
            }
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(_) => {
                    out.skipped += 1;
                    continue;
                }
            };
            for entry in entries {
                if pending.len() + visited >= DISCOVERY_ENTRY_LIMIT {
                    out.capped = true;
                    break;
                }
                match entry {
                    // agy keeps tool output and artifacts beside each
                    // transcript; only the transcript is a candidate.
                    Ok(entry) if agent == Agent::Antigravity && depth == 0 => pending.push((
                        crate::antigravity::transcript(home, &entry.file_name().to_string_lossy()),
                        depth + 1,
                    )),
                    Ok(entry) => pending.push((entry.path(), depth + 1)),
                    Err(_) => out.skipped += 1,
                }
            }
        } else if metadata.is_file()
            && (agent == Agent::OpenCode
                || (agent == Agent::Grok && path.file_name().is_some_and(|n| n == "updates.jsonl"))
                || (agent == Agent::Antigravity && crate::antigravity::id_of(&path).is_some())
                || (!matches!(agent, Agent::Grok | Agent::Antigravity)
                    && path.extension().is_some_and(|n| n == "jsonl")))
        {
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            let Some(modified_ms) = modified_ms else {
                out.skipped += 1;
                continue;
            };
            if modified_ms < since_ms {
                continue;
            }
            if out.files.len() == CANDIDATE_LIMIT {
                out.capped = true;
                break;
            }
            let Ok(path) = hide_platform::path::to_wire(&path) else {
                out.skipped += 1;
                continue;
            };
            out.files.push(Candidate {
                path,
                size: metadata.len(),
                modified_ms,
            });
        }
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn checked_path(home: &Path, agent: Agent, path: &Path) -> Result<PathBuf, Refusal> {
    let canonical =
        hide_platform::fs::identity::canonical(path).map_err(|_| Refusal::Unreadable)?;
    let allowed = roots(home, agent)?.into_iter().any(|root| {
        hide_platform::fs::identity::canonical(&root).is_ok_and(|root| {
            if agent == Agent::OpenCode {
                canonical == root
            } else {
                canonical != root && canonical.starts_with(root)
            }
        })
    });
    if !allowed {
        return Err(Refusal::OutsideRoot);
    }
    Ok(canonical)
}

fn checked_file(home: &Path, agent: Agent, path: &Path) -> Result<fs::File, Refusal> {
    let canonical = checked_path(home, agent, path)?;
    let file = crate::open_session_file_nofollow(path).map_err(|_| Refusal::Unreadable)?;
    // Check the opened descriptor too, so a parent link swapped after the
    // canonical check cannot redirect this read outside the allowed root.
    if hide_platform::fs::identity::file_id_of(&file).map_err(|_| Refusal::Unreadable)?
        != hide_platform::fs::identity::file_id(&canonical).map_err(|_| Refusal::Unreadable)?
    {
        return Err(Refusal::OutsideRoot);
    }
    Ok(file)
}

/// Reads at most one MiB. Partial lines are re-read on the next step, and
/// skipped oversized lines advance without ever persisting their bytes.
pub fn read(
    home: &Path,
    agent: Agent,
    path: &Path,
    saved: Option<Checkpoint>,
) -> Result<Step, Refusal> {
    let started = Instant::now();
    if agent == Agent::OpenCode {
        // The shared OpenCode guard owns every open/close of its database.
        // A preliminary File here would release a peer's POSIX SQLite lock.
        checked_path(home, agent, path)?;
        return opencode::read(home, saved);
    }
    if agent == Agent::Cursor {
        return Err(Refusal::Unsupported);
    }
    let mut file = checked_file(home, agent, path)?;
    let identity =
        hide_platform::fs::identity::file_id_of(&file).map_err(|_| Refusal::Unreadable)?;
    let identity = format!("{}:{}", identity.volume(), identity.index());
    let size = file.metadata().map_err(|_| Refusal::Unreadable)?.len();
    let summary = if agent == Agent::Grok {
        Some(grok::summary(home, path)?)
    } else {
        None
    };
    let mut checkpoint = saved.unwrap_or_default();
    if serde_json::to_vec(&checkpoint)
        .map_err(|_| Refusal::InvalidCheckpoint)?
        .len()
        > 16 * 1024
    {
        return Err(Refusal::InvalidCheckpoint);
    }
    let mut rescanned = checkpoint.identity.is_some()
        && (checkpoint.identity.as_ref() != Some(&identity)
            || size < checkpoint.offset
            || checkpoint.parser_version != PARSER_VERSION);
    let mut witness_bytes = 0;
    if !rescanned && let Some(saved) = &checkpoint.witness {
        let (current, read) = witness(&mut file, checkpoint.offset)?;
        witness_bytes += read;
        rescanned = saved.digest != current.digest
            || (saved.size == current.size && saved.modified_ns != current.modified_ns);
    }
    if rescanned {
        checkpoint = Checkpoint::default();
    }
    checkpoint.identity = Some(identity);
    checkpoint.parser_version = PARSER_VERSION;
    if agent == Agent::Antigravity {
        let id = crate::antigravity::id_of(path).ok_or(Refusal::OutsideRoot)?;
        // The checkout attributes the calls; a conversation the index does
        // not place yet is still counted, under no project until it does. An
        // index that cannot be read now (agy holding it) refuses the read,
        // which the next pass repeats, rather than losing the project.
        let cwd = crate::antigravity::checkout(home, id)
            .map_err(|crate::antigravity::IndexBusy| Refusal::Database)?
            .and_then(|cwd| cwd.to_str().map(str::to_owned));
        records::antigravity_context(&mut checkpoint.context, id, cwd);
    }
    if summary.as_ref().is_some_and(|s| s.is_child()) {
        checkpoint.offset = size;
        checkpoint.finished = true;
        return Ok(Step {
            events: Vec::new(),
            checkpoint,
            read_bytes: summary.as_ref().unwrap().read_bytes,
            has_more: false,
            rescanned,
            skipped: BTreeMap::new(),
        });
    }
    file.seek(SeekFrom::Start(checkpoint.offset))
        .map_err(|_| Refusal::Unreadable)?;
    let mut bytes = Vec::new();
    let metadata_bytes = summary.as_ref().map_or(0, |s| s.read_bytes);
    // Reserve both boundary checks inside the same one-MiB read budget.
    let payload_budget = SESSION_INCREMENT_READ_LIMIT_BYTES - metadata_bytes - 4 * WITNESS_BYTES;
    (&mut file)
        .take(payload_budget)
        .read_to_end(&mut bytes)
        .map_err(|_| Refusal::Unreadable)?;
    let mut step = Step {
        events: Vec::new(),
        checkpoint,
        read_bytes: bytes.len() as u64 + metadata_bytes + witness_bytes,
        has_more: false,
        rescanned,
        skipped: BTreeMap::new(),
    };
    let mut yielded = false;
    for (index, fragment) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        // Grok's one native prompt may carry at most 128 model events.
        if index > 0 && (started.elapsed() >= STEP_TIME || step.events.len() > STEP_EVENTS - 128) {
            yielded = true;
            break;
        }
        let complete = fragment.last() == Some(&b'\n');
        if step.checkpoint.discarding || fragment.len() > SESSION_LINE_LIMIT_BYTES {
            if !step.checkpoint.discarding {
                *step.skipped.entry("record_capacity".into()).or_default() += 1;
            }
            step.checkpoint.offset += fragment.len() as u64;
            step.checkpoint.discarding = !complete;
        } else if complete {
            let parsed = if let Some(summary) = &summary {
                grok::parse(fragment, summary)
            } else {
                records::parse(agent, fragment, &mut step.checkpoint.context)
                    .map(|event| event.into_iter().collect())
            };
            match parsed {
                Ok(events) => step.events.extend(events),
                Err(reason) => *step.skipped.entry(reason.into()).or_default() += 1,
            }
            step.checkpoint.offset += fragment.len() as u64;
        } else {
            break;
        }
    }
    let (current, read) = witness(&mut file, step.checkpoint.offset)?;
    step.read_bytes += read;
    step.checkpoint.witness = Some(current);
    step.has_more =
        step.checkpoint.offset < size && (yielded || bytes.len() as u64 == payload_budget);
    step.checkpoint.finished = !step.has_more;
    Ok(step)
}

fn key(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}
