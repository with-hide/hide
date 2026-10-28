//! Pi 1.0.4 and omp 18.7.0 recorded JSONL histories, not active model context,
//! and Grok 1.0.46's per-session directories (`grok`).
//! Native session-manager metadata is the authority; neither folder encoding
//! nor a filename suffix identifies a checkout or session.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Agent, ConversationEvent, DiscoveryBudget, EventKind, LineResult, RootRefusal,
    SESSION_LINE_LIMIT_BYTES, SessionError, SessionIdentity,
};

pub(crate) struct Header {
    pub id: String,
    pub cwd: PathBuf,
    /// When the session itself was made, which a fork's copied records do
    /// not tell: Pi's and omp's header `timestamp`, Grok's summary
    /// `created_at`.
    pub opened_at_unix_ms: Option<u64>,
}

/// Routing constraint of Pi's native --session <id>: its default cwd folder
/// is searched first. Global matches prompt to fork, so they cannot wake an
/// existing conversation without an operator decision.
fn root_suffix(agent: Agent) -> &'static str {
    match agent {
        Agent::Grok => crate::GROK_SESSIONS,
        Agent::Pi => crate::PI_SESSIONS,
        Agent::Omp => crate::OMP_SESSIONS,
        Agent::Cursor => crate::cursor::SESSIONS,
        Agent::Antigravity => crate::antigravity::SESSIONS,
        _ => unreachable!("native-file policy requires a native-file format"),
    }
}

fn encode_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':') {
                '-'
            } else {
                ch
            }
        })
        .collect()
}

fn absolute_directory_name(cwd: &Path) -> String {
    let spelling = cwd.to_string_lossy();
    let spelling = spelling.strip_prefix(['/', '\\']).unwrap_or(&spelling);
    format!("--{}--", encode_path(Path::new(spelling)))
}

fn relative_directory_name(prefix: &str, relative: &Path) -> String {
    let encoded = encode_path(relative);
    if encoded.is_empty() {
        prefix.to_owned()
    } else if prefix.ends_with('-') {
        format!("{prefix}{encoded}")
    } else {
        format!("{prefix}-{encoded}")
    }
}

struct OmpDirectory {
    name: String,
    scope: &'static str,
    shadowed_home: Option<String>,
    canonical_home: PathBuf,
}

fn omp_directory(home: &Path, cwd: &Path) -> Result<OmpDirectory> {
    let canonical_home = hide_platform::fs::identity::canonical(home)
        .map_err(|_| anyhow!("label_session_default_directory_unconfirmed"))?;
    let temp = hide_platform::fs::identity::canonical(&std::env::temp_dir())
        .map_err(|_| anyhow!("label_session_default_directory_unconfirmed"))?;
    // This is upstream's isRelativeWithin, including its refusal of a child
    // whose first name starts with '..'. A broader containment test routes
    // such a checkout to a different native bucket.
    let within = |root: &Path| {
        cwd.strip_prefix(root)
            .ok()
            .filter(|relative| !relative.to_string_lossy().starts_with(".."))
            .map(Path::to_path_buf)
    };
    let home_relative = within(&canonical_home);
    // Native omp checks temp before home, including temp nested under home.
    let (name, scope, shadowed_home) = if let Some(relative) = within(&temp) {
        (
            relative_directory_name("-tmp", &relative),
            "tmp",
            home_relative.map(|relative| relative_directory_name("-", &relative)),
        )
    } else if let Some(relative) = home_relative {
        (relative_directory_name("-", &relative), "home", None)
    } else {
        (absolute_directory_name(cwd), "abs", None)
    };
    Ok(OmpDirectory {
        name,
        scope,
        shadowed_home,
        canonical_home,
    })
}

/// The folder the agent's default resolver reads a checkout's sessions
/// from: the session file's folder for Pi and omp, the group holding each
/// session's folder for Grok.
pub(crate) fn default_directory(home: &Path, agent: Agent, cwd: &Path) -> Result<PathBuf> {
    let name = match agent {
        Agent::Omp => omp_directory(home, cwd)?.name,
        Agent::Grok => crate::grok::group_name(cwd)?,
        Agent::Cursor => {
            crate::cursor::directory(cwd).map_err(|_| anyhow!("session_route_unconfirmed"))?
        }
        _ => absolute_directory_name(cwd),
    };
    Ok(home.join(root_suffix(agent)).join(name))
}

/// OMP startup migrates these fixed aliases before resolving a selector.
/// Refuse the pending native work; Hide neither migrates nor edits histories.
fn confirm_omp_migrations(home: &Path, cwd: &Path, directory: &Path) -> Result<()> {
    if !cwd.is_absolute() || cwd.components().any(|part| part == Component::ParentDir) {
        return Err(anyhow!("session_route_unconfirmed"));
    }
    let canonical_cwd = hide_platform::fs::identity::canonical(cwd)
        .map_err(|_| anyhow!("session_route_unconfirmed"))?;
    let policy = omp_directory(home, &canonical_cwd)?;
    let root = home.join(crate::OMP_SESSIONS);
    for spelling in [home, policy.canonical_home.as_path()] {
        let legacy = absolute_directory_name(spelling);
        let prefix = format!("{}-", legacy.strip_suffix("--").unwrap());
        if policy.name == legacy
            || (policy.name.starts_with(&prefix) && policy.name.ends_with("--"))
        {
            // Root-wide home migration can also move the selected absolute
            // bucket itself when an unrelated path shares the encoded prefix.
            return Err(anyhow!("session_route_requires_native_migration"));
        }
    }
    let mut aliases = vec![
        absolute_directory_name(cwd),
        absolute_directory_name(&canonical_cwd),
    ];
    if let Some(shadowed) = &policy.shadowed_home {
        aliases.push(shadowed.clone());
    }
    // Reverse the root-wide first stage through every later migration input.
    // It can materialize an absent legacy absolute alias before that alias
    // is merged into the selected bucket, as well as populate it directly.
    let targets: Vec<_> = std::iter::once(&policy.name)
        .chain(aliases.iter())
        .cloned()
        .collect();
    for target in targets {
        if let Some(remainder) = target.strip_prefix('-') {
            for spelling in [home, policy.canonical_home.as_path()] {
                let encoded = absolute_directory_name(spelling);
                let home_name = encoded.strip_suffix("--").unwrap();
                aliases.push(if remainder.is_empty() {
                    encoded
                } else {
                    format!("{home_name}-{remainder}--")
                });
            }
        }
    }
    // Reconstruct the native 17.2.5-17.2.8 migration key, action-only.
    let mut readable = String::new();
    let mut invalid_run = false;
    for ch in canonical_cwd
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
    {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            readable.push(ch);
            invalid_run = false;
        } else {
            if !invalid_run {
                readable.push('-');
            }
            invalid_run = true;
        }
    }
    let readable = readable.trim_matches('-');
    let readable = &readable[readable.len().saturating_sub(80)..];
    let readable = if readable.is_empty() {
        "project"
    } else {
        readable
    };
    let normalized = canonical_cwd.to_string_lossy().replace('\\', "/");
    aliases.push(format!(
        "{}-{readable}-{:x}",
        policy.scope,
        Sha256::digest(normalized.as_bytes())
    ));
    // At most twelve fixed metadata probes, independent of session count.
    for name in aliases {
        let candidate = root.join(name);
        if candidate.file_name() == directory.file_name() {
            continue;
        }
        match fs::symlink_metadata(candidate) {
            Ok(_) => return Err(anyhow!("session_route_requires_native_migration")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(anyhow!("session_route_unconfirmed")),
        }
    }
    Ok(())
}

/// The home itself may have a platform alias. No component below it may be
/// a link, including .pi and the sessions root. Inspect native entry identity
/// rather than relying on canonicalization to hide internal links.
pub(crate) fn inside_root(
    home: &Path,
    agent: Agent,
    path: &Path,
) -> std::result::Result<PathBuf, RootRefusal> {
    let canonical_home =
        hide_platform::fs::identity::canonical(home).map_err(|_| RootRefusal::Unreadable)?;
    let relative = path
        .strip_prefix(home)
        .or_else(|_| path.strip_prefix(&canonical_home))
        .map_err(|_| RootRefusal::Outside)?;
    if !relative.starts_with(root_suffix(agent)) || relative == Path::new(root_suffix(agent)) {
        return Err(RootRefusal::Outside);
    }
    if agent == Agent::Cursor {
        crate::cursor::checked_path(home, path).map_err(|_| RootRefusal::Outside)
    } else if agent == Agent::Antigravity {
        crate::antigravity::checked_path(home, path)
    } else {
        checked_path(home, agent, path)
    }
}

pub(crate) fn root(home: &Path, agent: Agent) -> std::result::Result<PathBuf, RootRefusal> {
    checked_path(home, agent, &home.join(root_suffix(agent)))
}

fn checked_path(
    home: &Path,
    agent: Agent,
    path: &Path,
) -> std::result::Result<PathBuf, RootRefusal> {
    checked_path_under(home, root_suffix(agent), path)
}

pub(crate) fn checked_path_under(
    home: &Path,
    suffix: &str,
    path: &Path,
) -> std::result::Result<PathBuf, RootRefusal> {
    let canonical_home =
        hide_platform::fs::identity::canonical(home).map_err(|_| RootRefusal::Unreadable)?;
    let relative = path
        .strip_prefix(home)
        .or_else(|_| path.strip_prefix(&canonical_home))
        .map_err(|_| RootRefusal::Outside)?;
    if !relative.starts_with(suffix) {
        return Err(RootRefusal::Outside);
    }
    let mut checked = canonical_home;
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(RootRefusal::Outside);
        };
        checked.push(part);
        let native = hide_platform::fs::identity::file_id_nofollow(&checked).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RootRefusal::Missing
            } else {
                RootRefusal::Unreadable
            }
        })?;
        let followed =
            hide_platform::fs::identity::file_id(&checked).map_err(|_| RootRefusal::Unreadable)?;
        if native != followed {
            return Err(RootRefusal::Outside);
        }
    }
    hide_platform::fs::identity::canonical(&checked).map_err(|_| RootRefusal::Unreadable)
}

/// The folder of a session file that [`default_directory`] names.
pub(crate) fn directory_of(agent: Agent, path: &Path) -> Option<&Path> {
    if agent == Agent::Cursor {
        path.parent().and_then(Path::parent)
    } else if agent == Agent::Grok {
        crate::grok::group_of(path)
    } else if agent == Agent::Antigravity {
        crate::antigravity::directory_of(path)
    } else {
        path.parent()
    }
}

pub(crate) fn header(agent: Agent, path: &Path) -> Result<Header> {
    let mut remaining = u64::MAX;
    header_budgeted(agent, path, &mut remaining)
}

fn header_budgeted(agent: Agent, path: &Path, remaining: &mut u64) -> Result<Header> {
    if agent == Agent::Antigravity {
        // The index row, not the transcript, names the conversation's checkout.
        return crate::antigravity::header(path);
    }
    if agent == Agent::Grok {
        let mut read = 0;
        let summary = crate::grok::summary(path, &mut read);
        if read > *remaining {
            return Err(anyhow!("session_discovery_read_capacity"));
        }
        *remaining -= read;
        let summary = summary?;
        return Ok(Header {
            id: summary.id,
            cwd: summary.cwd,
            opened_at_unix_ms: summary.created_at_unix_ms,
        });
    }
    let file =
        crate::open_session_file(path).map_err(|_| anyhow!("label_session_file_unavailable"))?;
    let mut reader = BufReader::new(file);
    header_from_reader(agent, &mut reader, remaining)
}

fn header_line(reader: &mut impl BufRead, remaining: &mut u64) -> Result<Value> {
    let limit = ((SESSION_LINE_LIMIT_BYTES + 1) as u64).min(remaining.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .take(limit)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| anyhow!("label_session_metadata_read_failed"))?;
    if bytes.len() as u64 > *remaining {
        return Err(anyhow!("session_discovery_read_capacity"));
    }
    *remaining -= bytes.len() as u64;
    if bytes.len() > SESSION_LINE_LIMIT_BYTES {
        return Err(anyhow!("label_session_metadata_line_capacity"));
    }
    if bytes.last() != Some(&b'\n') {
        return Err(anyhow!("label_session_metadata_unconfirmed"));
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("label_session_metadata_unconfirmed"))
}

pub(crate) fn header_from_reader(
    agent: Agent,
    reader: &mut impl BufRead,
    remaining: &mut u64,
) -> Result<Header> {
    let first = header_line(reader, remaining)?;
    let value = if agent == Agent::Omp && first["type"] == "title" {
        title_snapshot(&first).ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?;
        header_line(reader, remaining)?
    } else {
        first
    };
    if value["type"] != "session" || value["version"] != 3 {
        return Err(anyhow!("label_session_metadata_unconfirmed"));
    }
    let id = value["id"]
        .as_str()
        .filter(|id| crate::label_owner::valid_native_id(id))
        .ok_or_else(|| anyhow!("label_session_id_invalid"))?;
    let cwd = value["cwd"]
        .as_str()
        .filter(|cwd| !cwd.chars().any(char::is_control))
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute())
        .ok_or_else(|| anyhow!("label_session_cwd_unconfirmed"))?;
    Ok(Header {
        id: id.to_owned(),
        cwd,
        opened_at_unix_ms: crate::timestamp_ms(value.get("timestamp")).ok(),
    })
}

/// Prove the exact ID's native CLI route before an effect. Pi picks the first
/// matching header, skips malformed prefixes and falls back to prefix/global
/// matches. Refuse uncertainty instead of depending on directory enumeration.
/// This directory scan is action-only, never a snapshot or per-file catalog read.
pub(crate) fn confirm_route(
    home: &Path,
    agent: Agent,
    path: &Path,
    id: &str,
    cwd: &Path,
) -> Result<()> {
    if agent == Agent::Cursor {
        return crate::cursor::confirm_route(home, path, id, cwd)
            .map_err(|error| anyhow!(error.to_string()));
    }
    if agent == Agent::Antigravity {
        return crate::antigravity::confirm_route(home, path, id, cwd);
    }
    let expected =
        inside_root(home, agent, path).map_err(|_| anyhow!("session_route_unconfirmed"))?;
    if agent == Agent::Grok {
        return confirm_grok_route(home, &expected, id, cwd);
    }
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("session_route_unconfirmed"))?;
    if agent == Agent::Omp {
        confirm_omp_migrations(home, cwd, directory)?;
    }
    let mut budget = DiscoveryBudget::default();
    let mut remaining = crate::SESSION_INCREMENT_READ_LIMIT_BYTES;
    let paths = crate::read_directory(directory, &mut budget).map_err(|error| match error {
        SessionError::Capacity { .. } => anyhow!("session_route_capacity"),
        _ => anyhow!("session_route_unconfirmed"),
    })?;
    let mut matches = 0;
    for candidate in paths {
        if agent == Agent::Omp
            && let Some(primary) = candidate
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".bak"))
                .and_then(|name| name.rsplit_once('.').map(|(primary, _)| primary))
                .filter(|name| name.ends_with(".jsonl"))
        {
            match fs::metadata(directory.join(primary)) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(anyhow!("session_route_requires_native_recovery"));
                }
                Err(_) => return Err(anyhow!("session_route_unconfirmed")),
            }
        }
        if !candidate
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".jsonl"))
        {
            continue;
        }
        let checked = inside_root(home, agent, &candidate)
            .map_err(|_| anyhow!("session_route_unconfirmed"))?;
        let file = crate::open_session_file(&candidate)
            .map_err(|_| anyhow!("session_route_unconfirmed"))?;
        if !file.metadata().is_ok_and(|metadata| metadata.is_file())
            || hide_platform::fs::identity::link_count(&file).ok() != Some(1)
        {
            return Err(anyhow!("session_route_unconfirmed"));
        }
        // A strict first header is a safe subset of native discovery. Any
        // prefixed, malformed, incomplete or unreadable sibling is uncertain.
        let header = header_budgeted(agent, &candidate, &mut remaining).map_err(|error| {
            if error.to_string() == "session_discovery_read_capacity" {
                anyhow!("session_route_capacity")
            } else {
                anyhow!("session_route_unconfirmed")
            }
        })?;
        let native_match = if agent == Agent::Omp {
            let needle = id.to_ascii_lowercase();
            let stem = candidate
                .file_stem()
                .and_then(|name| name.to_str())
                .filter(|name| name.is_ascii())
                .ok_or_else(|| anyhow!("session_route_unconfirmed"))?
                .to_ascii_lowercase();
            // Native uses ECMAScript Unicode lowercasing. Only ASCII native
            // filenames have the same guaranteed selector semantics here.
            header.id.to_ascii_lowercase().starts_with(&needle)
                || stem.starts_with(&needle)
                || stem
                    .rsplit_once('_')
                    .is_some_and(|(_, suffix)| suffix.starts_with(&needle))
        } else {
            header.id == id
        };
        if native_match {
            matches += 1;
            if matches != 1 || checked != expected {
                return Err(anyhow!("session_route_ambiguous"));
            }
        }
    }
    if matches != 1 {
        return Err(anyhow!("session_route_missing"));
    }
    Ok(())
}

/// `grok --resume <id>` takes the session folder named `<id>` in the
/// launch cwd's own group before searching any other group, so that folder
/// holding this session's summary is the one it resumes.
fn confirm_grok_route(home: &Path, path: &Path, id: &str, cwd: &Path) -> Result<()> {
    if !cwd.is_absolute() || cwd.components().any(|part| part == Component::ParentDir) {
        return Err(anyhow!("session_route_unconfirmed"));
    }
    let cwd = hide_platform::fs::identity::canonical(cwd)
        .map_err(|_| anyhow!("session_route_unconfirmed"))?;
    let group = default_directory(home, Agent::Grok, &cwd)?;
    let group =
        inside_root(home, Agent::Grok, &group).map_err(|_| anyhow!("session_route_unconfirmed"))?;
    if directory_of(Agent::Grok, path) != Some(group.as_path()) {
        return Err(anyhow!("session_route_unconfirmed"));
    }
    let summary = crate::grok::summary(path, &mut 0).map_err(|error| {
        if error.to_string() == "label_session_not_root" {
            error
        } else {
            anyhow!("session_route_unconfirmed")
        }
    })?;
    if summary.id != id {
        return Err(anyhow!("session_route_missing"));
    }
    Ok(())
}

pub(crate) fn locate(
    home: &Path,
    agent: Agent,
    identity: Option<&SessionIdentity>,
    cwd: Option<&str>,
    budget: &mut DiscoveryBudget,
) -> crate::Result<PathBuf> {
    if agent == Agent::Cursor {
        return crate::cursor::locate(home, identity, cwd, budget);
    }
    if agent == Agent::Antigravity {
        return crate::antigravity::locate(home, identity, cwd, budget);
    }
    let cwd = cwd.ok_or(SessionError::CwdUnavailable)?;
    if let Some(SessionIdentity::Path(path)) = identity {
        crate::confirm_session_file(home, agent, path, None, Some(cwd))
            .map_err(|error| SessionError::Checkpoint(error.to_string()))?;
        return Ok(path.clone());
    }
    let reported_id = match identity {
        Some(SessionIdentity::Id(id)) if crate::label_owner::valid_native_id(id) => {
            Some(id.as_str())
        }
        Some(_) => return Err(SessionError::SessionFileMissing),
        None => None,
    };
    if self::root(home, agent).is_err() {
        return Err(SessionError::SessionFileMissing);
    }
    let mut candidates = Vec::new();
    let expected_cwd = hide_platform::fs::identity::canonical(Path::new(cwd))
        .map_err(|_| SessionError::CwdUnavailable)?;
    let directory = default_directory(home, agent, &expected_cwd)
        .map_err(|error| SessionError::Checkpoint(error.to_string()))?;
    if inside_root(home, agent, &directory).is_err() {
        return Err(SessionError::SessionFileMissing);
    }
    let mut remaining = crate::SESSION_INCREMENT_READ_LIMIT_BYTES;
    let paths = match (agent, reported_id) {
        (Agent::Grok, Some(id)) => vec![directory.join(id).join(crate::grok::UPDATES)],
        (Agent::Grok, None) => crate::read_directory(&directory, budget)?
            .into_iter()
            .map(|session| session.join(crate::grok::UPDATES))
            .collect(),
        _ => crate::jsonl_files(&directory, budget)?,
    };
    for path in paths {
        if inside_root(home, agent, &path).is_err() {
            continue;
        }
        let header = match header_budgeted(agent, &path, &mut remaining) {
            Ok(header) => header,
            Err(error) if error.to_string() == "session_discovery_read_capacity" => {
                return Err(SessionError::Capacity {
                    resource: "discovery_read_bytes",
                    limit: crate::SESSION_INCREMENT_READ_LIMIT_BYTES,
                });
            }
            Err(_) => continue,
        };
        if reported_id.is_some_and(|id| header.id != id)
            || !hide_platform::fs::identity::canonical(&header.cwd)
                .is_ok_and(|native| native == expected_cwd)
        {
            continue;
        }
        if let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) {
            candidates.push((modified, path));
        }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let path = candidates
        .into_iter()
        .next()
        .map(|(_, path)| path)
        .ok_or(SessionError::SessionFileMissing)?;
    crate::confirm_session_file(home, agent, &path, reported_id, Some(cwd))
        .map_err(|error| SessionError::Checkpoint(error.to_string()))?;
    Ok(path)
}

/// Only the physical first slot/header may be applied by the parser/cursor.
/// title_change is audit history and never overrides this current snapshot.
pub(crate) fn title_snapshot(item: &Value) -> Option<(String, String)> {
    let source = if item["type"] == "title" {
        if item["v"] != 1 || !item["updatedAt"].is_string() || !item["pad"].is_string() {
            return None;
        }
        item.get("source")
    } else if item["type"] == "session" && item["version"] == 3 {
        item.get("titleSource")
    } else {
        return None;
    };
    if source.is_some_and(|source| source != "auto" && source != "user") {
        return None;
    }
    let title = match item.get("title") {
        Some(Value::String(title)) => title.clone(),
        None if item["type"] == "session" => String::new(),
        _ => return None,
    };
    Some(if source.is_some_and(|source| source == "user") {
        (String::new(), title)
    } else {
        (title, String::new())
    })
}

pub(crate) fn parse_line(agent: Agent, item: &Value) -> LineResult {
    if agent == Agent::Omp
        && let Some((title, custom_title)) = title_snapshot(item)
    {
        return LineResult::TitleSnapshot {
            title,
            custom_title,
        };
    }
    match item["type"].as_str() {
        Some("session_info") if agent == Agent::Pi => {
            return item["name"].as_str().map_or(LineResult::Ignore, |name| {
                // /name and extensions share this slot without a source flag.
                LineResult::Title(name.to_owned())
            });
        }
        Some("compaction") => {
            return match crate::timestamp_ms(item.get("timestamp")) {
                Ok(at) => LineResult::Event(
                    ConversationEvent::new("system", EventKind::Injected, at, "")
                        .with_context_reset(true),
                ),
                Err(reason) => LineResult::Skip(reason),
            };
        }
        Some("message") => (),
        // Hide's own extension writes its hidden message (guidance, letters,
        // Memory) as a custom message the host stores beside the prompt; it is
        // what the agent was given, never what the operator typed, and the
        // Memory receipt inside it is read from here (PRD pi-omp-extension
        // D-03, D-09).
        Some("custom_message") if item["customType"] == "hide" => {
            let at = match crate::timestamp_ms(item.get("timestamp")) {
                Ok(at) => at,
                Err(reason) => return LineResult::Skip(reason),
            };
            let text = crate::session_text(item.get("content")).unwrap_or_default();
            return if text.is_empty() {
                LineResult::Ignore
            } else {
                LineResult::Event(
                    ConversationEvent::new("user", EventKind::Injected, at, text)
                        .with_provider_injected(true),
                )
            };
        }
        // Compaction, branch summaries, other extensions' custom messages and
        // context edits are not human turns. Pi's export retains the raw
        // history.
        _ => return LineResult::Ignore,
    }
    let message = &item["message"];
    let role = message["role"].as_str().unwrap_or("");
    let at = match crate::timestamp_ms(item.get("timestamp").or_else(|| message.get("timestamp"))) {
        Ok(at) => at,
        Err(reason) => return LineResult::Skip(reason),
    };
    if role == "toolResult" || role == "bashExecution" {
        let text = if role == "bashExecution" {
            message["output"].as_str().map(str::to_owned)
        } else {
            crate::session_text(message.get("content"))
        };
        return text.map_or(LineResult::Ignore, |text| {
            LineResult::Sightings(crate::sightings_in(&[&text], at))
        });
    }
    let kind = match role {
        "user" => EventKind::Human,
        "assistant" if message["stopReason"] == "aborted" => EventKind::Interrupted,
        "assistant" => EventKind::Assistant,
        "custom" | "system" | "compactionSummary" | "branchSummary" => EventKind::Injected,
        _ => return LineResult::Ignore,
    };
    let images = message["content"].as_array().map_or(0, |blocks| {
        blocks
            .iter()
            .filter(|block| block["type"] == "image")
            .count() as u32
    });
    let text = crate::session_text(message.get("content")).unwrap_or_default();
    if text.is_empty() && images == 0 && kind != EventKind::Interrupted {
        return LineResult::Ignore;
    }
    let kind = if kind == EventKind::Human && crate::has_injected_prefix(&text) {
        EventKind::Injected
    } else {
        kind
    };
    LineResult::Event(
        ConversationEvent::new(
            if role == "assistant" {
                "assistant"
            } else {
                "user"
            },
            kind,
            at,
            text,
        )
        // Only Hide's own custom message above is provider-injected: a
        // message entry, even one that starts like a reminder, is shown as
        // injected but never asserts a Memory receipt.
        .with_images(images),
    )
}
