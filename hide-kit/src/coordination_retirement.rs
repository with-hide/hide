//! One-release retirement, never a ledger migration or a compatibility command.
//! Reads of the old ledger are bounded preflight predicates only.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{KitRefusal, KitTarget, Observed, RefusalCause};

use crate::retirement_inspection::{self, MAX_ENTRIES, MAX_LEDGER_BYTES, MAX_STATE_BYTES};

#[derive(Default, Serialize, Deserialize)]
struct Progress {
    homes: Vec<(PathBuf, PathBuf)>,
    step: String,
    failure: Option<String>,
    complete: bool,
}

fn path(target: &KitTarget) -> PathBuf {
    crate::kit_state_dir(&target.home).join("coordination-retirement.json")
}

fn homes(target: &KitTarget) -> Vec<PathBuf> {
    let mut paths = vec![
        target.home.join(".hide/hcoord"),
        target.home.join(".hcoord"),
    ];
    if let Some(home) = &target.legacy_coordination_home
        && !paths.contains(home)
    {
        paths.push(home.clone());
    }
    paths
}

fn read_json(path: &Path, limit: u64) -> Result<Option<Value>, KitRefusal> {
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(KitRefusal::unsafe_path(
                path,
                format!(
                    "{} cannot be inspected: {error}; resolve it before retrying retirement",
                    path.display()
                ),
            ));
        }
    };
    if hide_platform::fs::private::handle_others_can_modify(&file).map_err(|error| {
        KitRefusal::unsafe_path(
            path,
            format!(
                "{} permissions cannot be inspected: {error}",
                path.display()
            ),
        )
    })? {
        return Err(KitRefusal::unsafe_path(
            path,
            format!(
                "{} can be changed by another account; inspect its permissions before retrying retirement",
                path.display()
            ),
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| KitRefusal::record(path, error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(KitRefusal::record(
            path,
            format!(
                "{} exceeds the preflight size bound; inspect it before retrying retirement",
                path.display()
            ),
        ));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        KitRefusal::record(
            path,
            format!(
                "{} cannot be inspected: {error}; no retirement can begin",
                path.display()
            ),
        )
    })
}

fn load(target: &KitTarget) -> Result<Option<Progress>, KitRefusal> {
    crate::record::private_state_dir(&target.home, false)?;
    let receipt = path(target);
    let progress: Option<Progress> = read_json(&receipt, 64 * 1024)?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| {
            KitRefusal::record(
                &receipt,
                format!("retirement progress is unreadable: {error}"),
            )
        })?;
    if let Some(progress) = &progress {
        for (source, destination) in &progress.homes {
            let prefix = format!(
                "{}.retired-",
                source.file_name().unwrap_or_default().to_string_lossy()
            );
            if !homes(target).contains(source)
                || source.parent() != destination.parent()
                || !destination
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            {
                return Err(KitRefusal::record(
                    &receipt,
                    "retirement progress names an unexpected folder; inspect it before retrying",
                ));
            }
        }
    }
    Ok(progress)
}

fn save(target: &KitTarget, progress: &Progress) -> Result<(), String> {
    let bytes = serde_json::to_vec(progress).map_err(|error| error.to_string())?;
    crate::record::private_state_dir(&target.home, true)?;
    crate::write_atomically(&path(target), &bytes, hide_platform::fs::Access::Private)
}

/// No file creation, permissions change, launchctl, daemon RPC or install work.
/// A refusal names what the operator acts on (`KitRefusal`).
pub fn preflight(target: &KitTarget) -> Result<(), KitRefusal> {
    if load(target)?.is_some_and(|progress| progress.complete) {
        return Ok(());
    }
    for home in homes(target) {
        inspect_legacy_home(target, &home)?;
        let ledger_path = home.join("ledger.json");
        let Some(ledger) = read_json(&ledger_path, MAX_LEDGER_BYTES)? else {
            continue;
        };
        retirement_inspection::legacy_ledger(&ledger)
            .map_err(|detail| KitRefusal::record(&ledger_path, detail))?;
    }
    inspect_below(
        &target.home,
        &crate::kit_state_dir(&target.home).join("hcoord"),
    )?;
    let (state, variable) = crate::layout::state_dir_choice_from_process(&target.home);
    let chosen = |refusal: KitRefusal| refusal.chosen_by(variable);
    if let Ok(anchor) = home_anchor(&target.home, &state) {
        inspect_below(&anchor, &state).map_err(chosen)?;
    } else {
        inspect_state_override(&state).map_err(chosen)?;
    }
    let ledger_path = crate::layout::delivery_ledger(&state);
    if let Some(ledger) = read_json(&ledger_path, MAX_LEDGER_BYTES).map_err(chosen)? {
        retirement_inspection::delivery_ledger(&ledger)
            .map_err(|detail| KitRefusal::record(&ledger_path, detail))?;
    }
    if target.retirement_projects.len() > MAX_ENTRIES {
        return Err(KitRefusal::new(
            RefusalCause::Record,
            None,
            "registered checkouts exceed the preflight entry bound; inspect them before retirement",
        ));
    }
    sasu_preflight(target)?;
    for project in &target.retirement_projects {
        project_preflight(project)?;
    }
    Ok(())
}

/// An external state override has no selected HOME or checkout anchor.
/// Authenticate its namespace before a missing ledger can mean "empty".
fn inspect_state_override(state: &Path) -> Result<bool, KitRefusal> {
    use hide_platform::fs::private::{InspectionDirectory, inspect_directory_entry};

    let mut selected = state.to_path_buf();
    let mut inspected = 0;
    let mut system_alias_target = false;
    'namespace: loop {
        if !selected.is_absolute()
            || (!system_alias_target
                && selected.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                }))
        {
            return Err(KitRefusal::unsafe_path(state, "the Hide state override has no absolute plain inspection path; choose a safe state directory and retry retirement"));
        }
        let root = selected.ancestors().last().ok_or_else(|| {
            KitRefusal::unsafe_path(state, "the Hide state override has no inspection root; choose a safe state directory and retry retirement")
        })?;
        let mut remaining = selected.strip_prefix(root).unwrap().components();
        let mut current = root.to_path_buf();
        loop {
            inspected += 1;
            if inspected > MAX_ENTRIES {
                return Err(KitRefusal::unsafe_path(state, "the Hide state override exceeds the preflight component bound; inspect its path before retrying retirement"));
            }
            #[cfg(all(test, unix))]
            let entry = crate::tests::inspect_system_alias_fixture(&current)
                .unwrap_or_else(|| inspect_directory_entry(&current));
            #[cfg(not(all(test, unix)))]
            let entry = inspect_directory_entry(&current);
            match entry {
                Ok(InspectionDirectory::Real) => {}
                Ok(InspectionDirectory::SystemAlias(target)) => {
                    let parent = current.parent().ok_or_else(|| {
                        KitRefusal::unsafe_path(&current, "a state override alias has no parent; inspect its path before retrying retirement")
                    })?;
                    selected = parent.join(target).join(remaining.as_path());
                    system_alias_target = true;
                    continue 'namespace;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(KitRefusal::unsafe_path(
                        &current,
                        format!(
                            "the Hide state override cannot be inspected at {}: {error}; resolve its ownership, permissions or links before retrying retirement",
                            current.display()
                        ),
                    ));
                }
            }
            match remaining.next() {
                Some(std::path::Component::Normal(name)) => current.push(name),
                // A relative system alias such as /var/run -> ../run is
                // followed in filesystem order. Never normalize away an
                // unchecked directory or alias before its parent component.
                Some(std::path::Component::ParentDir) if system_alias_target && current.pop() => {}
                Some(_) => {
                    return Err(KitRefusal::unsafe_path(state, "the Hide state override leaves its inspection root; choose a safe state directory and retry retirement"));
                }
                None => return Ok(true),
            }
        }
    }
}

/// Only the selected anchor may have aliases above it. Every directory below
/// it must be a real, owned directory that another account cannot replace.
/// A path that leaves its root came from a record or a registration, so the
/// refusal is the record's; a folder on the way is refused as unsafe.
fn inspect_below(anchor: &Path, path: &Path) -> Result<bool, KitRefusal> {
    if !anchor.is_absolute() {
        return Err(KitRefusal::record(
            anchor,
            "an inspection root is not absolute; inspect its registration before retrying",
        ));
    }
    let relative = path.strip_prefix(anchor).map_err(|_| {
        KitRefusal::record(
            path,
            "an inspection path has no trusted root; register its checkout before retrying retirement",
        )
    })?;
    let leaves = || {
        KitRefusal::record(
            path,
            "an inspection path leaves its trusted root; inspect it before retrying retirement",
        )
    };
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(leaves());
    }
    let mut current = anchor.to_path_buf();
    for (count, component) in relative.components().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Err(leaves());
        };
        if count >= MAX_ENTRIES {
            return Err(KitRefusal::record(
                path,
                "an inspection path exceeds the component bound; inspect it before retirement",
            ));
        }
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(KitRefusal::unsafe_path(
                        &current,
                        format!(
                            "{} is not an owned real directory; inspect it before retrying retirement",
                            current.display()
                        ),
                    ));
                }
                let unchecked = |error: std::io::Error| {
                    KitRefusal::unsafe_path(&current, error.to_string())
                };
                let owned = hide_platform::fs::private::owned_by_current_user(&current)
                    .map_err(unchecked)?;
                let writable =
                    hide_platform::fs::private::others_can_modify(&current).map_err(unchecked)?;
                if !owned || writable {
                    return Err(KitRefusal::unsafe_path(
                        &current,
                        format!(
                            "{} can be changed by another account; inspect its ownership and permissions before retrying retirement",
                            current.display()
                        ),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(KitRefusal::unsafe_path(
                    &current,
                    format!(
                        "{} cannot be inspected: {error}; resolve it before retrying retirement",
                        current.display()
                    ),
                ));
            }
        }
    }
    Ok(true)
}

fn home_anchor(home: &Path, path: &Path) -> Result<PathBuf, KitRefusal> {
    if home.is_absolute() && path.starts_with(home) {
        return Ok(home.to_path_buf());
    }
    // Resolve only the selected anchor, never a child that may redirect a read.
    let resolved = fs::canonicalize(home).map_err(|error| {
        KitRefusal::unsafe_path(home, format!("HOME authority cannot be inspected: {error}"))
    })?;
    if path.starts_with(&resolved) {
        return Ok(resolved);
    }
    Err(KitRefusal::unsafe_path(
        path,
        "the legacy coordination home has no trusted HOME anchor; inspect its location before retrying retirement",
    ))
}

fn inspect_legacy_home(target: &KitTarget, path: &Path) -> Result<bool, KitRefusal> {
    let anchor = home_anchor(&target.home, path)?;
    let state = crate::layout::state_dir_from_process(&target.home);
    let state_anchor = home_anchor(&target.home, &state).ok();
    // A canonical spelling of HOME still protects the selected state spelling.
    let state = state_anchor
        .as_ref()
        .and_then(|root| state.strip_prefix(root).ok())
        .map(|relative| anchor.join(relative))
        .unwrap_or(state);
    let wire = |spelled: &Path| {
        hide_platform::path::to_wire(spelled)
            .map_err(|error| KitRefusal::unsafe_path(path, error.to_string()))
    };
    retirement_inspection::legacy_location(&wire(&anchor)?, &wire(path)?, &wire(&state)?)
        .map_err(|detail| KitRefusal::unsafe_path(path, detail))?;
    inspect_below(&anchor, path)
}

fn run_state_anchor(target: &KitTarget, path: &Path) -> Result<PathBuf, KitRefusal> {
    let path_wire = hide_platform::path::to_wire(path)
        .map_err(|error| KitRefusal::record(path, error.to_string()))?;
    let authorized = |root: &Path| {
        hide_platform::path::to_wire(root).is_ok_and(|root| {
            retirement_inspection::inspection_root(&path_wire, std::iter::once(root.as_str()))
                .is_ok()
        })
    };
    for root in &target.retirement_projects {
        if authorized(root) {
            return Ok(root.clone());
        }
    }
    if let Ok(anchor) = home_anchor(&target.home, path) {
        return Ok(anchor);
    }
    for root in &target.retirement_projects {
        if let Ok(resolved) = fs::canonicalize(root)
            && authorized(&resolved)
        {
            return Ok(resolved);
        }
    }
    Err(KitRefusal::record(
        path,
        "a sasu run state has no trusted HOME or registered checkout root; register its owning checkout, then retry retirement",
    ))
}

fn sasu_preflight(target: &KitTarget) -> Result<(), KitRefusal> {
    let directory = target.home.join(".sasu/supervisor");
    if !inspect_below(&target.home, &directory)? {
        return Ok(());
    }
    let mut latest = directory.join("index.json");
    let mut revision = None;
    match fs::read_dir(&directory) {
        Ok(entries) => {
            for (count, entry) in entries.enumerate() {
                if count >= MAX_ENTRIES {
                    return Err(KitRefusal::record(&directory, "sasu supervisor folder exceeds the preflight entry bound; inspect it before retirement"));
                }
                let entry =
                    entry.map_err(|error| KitRefusal::unsafe_path(&directory, error.to_string()))?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let Some(suffix) = name.strip_prefix("index.json.revision-") else {
                    continue;
                };
                if suffix.len() == 12
                    && suffix.bytes().all(|byte| byte.is_ascii_digit())
                    && revision
                        .as_ref()
                        .is_none_or(|known: &String| suffix > known.as_str())
                {
                    revision = Some(suffix.to_owned());
                    latest = entry.path();
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(KitRefusal::unsafe_path(
                &directory,
                format!("sasu registry cannot be inspected: {error}; retirement has not begun"),
            ));
        }
    }
    #[cfg(all(test, unix))]
    crate::tests::before_supervisor_index_read(&latest);
    let Some(index) = read_json(&latest, MAX_STATE_BYTES)? else {
        return if revision.is_some() {
            Err(KitRefusal::record(&latest, "the selected sasu registry revision disappeared; retry retirement against the current registry; nothing was changed"))
        } else {
            Ok(())
        };
    };
    for state_path in retirement_inspection::supervisor_states(&index)
        .map_err(|detail| KitRefusal::record(&latest, detail))?
    {
        let path = Path::new(state_path);
        let anchor = run_state_anchor(target, path)?;
        let parent = path.parent().ok_or_else(|| {
            KitRefusal::record(
                &latest,
                "a sasu run state has no parent directory; inspect its registry before retrying",
            )
        })?;
        inspect_below(&anchor, parent)?;
        let state = read_json(path, MAX_STATE_BYTES)?.ok_or_else(|| {
            KitRefusal::record(
                &latest,
                "a sasu run state is missing; inspect its registry before retirement",
            )
        })?;
        retirement_inspection::indexed_run(&state)
            .map_err(|detail| KitRefusal::record(path, detail))?;
    }
    Ok(())
}

fn project_preflight(project: &Path) -> Result<(), KitRefusal> {
    let directory = project.join("agents/runs");
    if !inspect_below(project, &directory)? {
        return Ok(());
    }
    let unreadable = |error: std::io::Error| KitRefusal::unsafe_path(&directory, error.to_string());
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(KitRefusal::unsafe_path(
                &directory,
                format!("registered checkout runs cannot be inspected: {error}"),
            ));
        }
    };
    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return Err(KitRefusal::record(
                &directory,
                "registered checkout runs exceed the preflight entry bound",
            ));
        }
        let entry = entry.map_err(unreadable)?;
        let kind = entry.file_type().map_err(unreadable)?;
        if kind.is_symlink() {
            return Err(KitRefusal::unsafe_path(
                &entry.path(),
                "a registered run folder is a symlink; inspect it before retirement",
            ));
        }
        if !kind.is_dir() {
            continue;
        }
        inspect_below(project, &entry.path())?;
        let state_path = entry.path().join("state.json");
        if let Some(state) = read_json(&state_path, MAX_STATE_BYTES)? {
            retirement_inspection::checkout_run(&state)
                .map_err(|detail| KitRefusal::record(&state_path, detail))?;
        }
    }
    Ok(())
}

pub(crate) fn observe(target: &KitTarget) -> Observed {
    match load(target) {
        Err(refusal) => Observed::Blocked(refusal.into()),
        Ok(Some(progress)) if progress.complete => Observed::Current,
        Ok(Some(progress)) if progress.failure.is_some() => {
            Observed::Blocked(progress.failure.unwrap())
        }
        Ok(Some(progress)) => Observed::Stale(format!("retirement resumes at {}", progress.step)),
        Ok(None) => match preflight(target) {
            Err(refusal) => Observed::Blocked(refusal.into()),
            Ok(()) => {
                Observed::Stale("legacy coordination retirement has not run on this machine".into())
            }
        },
    }
}

pub(crate) fn location(target: &KitTarget) -> PathBuf {
    path(target)
}

pub(crate) fn install(target: &KitTarget) -> Result<crate::Retirement, String> {
    preflight(target)?;
    let mut progress = load(target)?.unwrap_or_else(|| Progress {
        homes: homes(target)
            .into_iter()
            .filter(|home| home.exists())
            .map(|home| {
                let name = format!(
                    "{}.retired-{}",
                    home.file_name().unwrap_or_default().to_string_lossy(),
                    date()
                );
                let destination = home.with_file_name(name);
                (home, destination)
            })
            .collect(),
        ..Progress::default()
    });
    if progress.complete {
        return Ok(crate::Retirement::default());
    }
    let mut retirement = crate::Retirement::default();
    for step in [
        "stop daemon",
        "remove login agent",
        "remove owned links",
        "remove kit copy",
        "preserve ledger",
    ] {
        if target.stop.load(Ordering::Relaxed) {
            return Err("Hide is quitting; retirement resumes on the next pass".into());
        }
        progress.step = step.into();
        progress.failure = None;
        save(target, &progress)?;
        let outcome = match step {
            "stop daemon" => homes(target).iter().try_for_each(|home| stop_daemon(home)),
            "remove login agent" => homes(target)
                .iter()
                .try_for_each(|home| remove_agent(target, home)),
            "remove owned links" => remove_links(target).map(|report| {
                retirement = report;
            }),
            "remove kit copy" => remove_copy(target),
            _ => preserve(target, &progress),
        };
        if let Err(reason) = outcome {
            progress.failure = Some(format!(
                "{} failed: {reason}; resolve it and retry retirement",
                progress.step
            ));
            save(target, &progress)?;
            return Err(progress.failure.unwrap());
        }
    }
    progress.complete = true;
    progress.step = "complete".into();
    save(target, &progress).map(|()| retirement)
}

/// An absent endpoint is already stopped, before IPC validates its name.
/// An existing entry must be an owned local endpoint, never a followed link.
fn daemon_socket_present(path: &Path) -> Result<bool, String> {
    let endpoint = match hide_platform::ipc::is_endpoint(path) {
        Ok(endpoint) => endpoint,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("daemon socket cannot be inspected: {error}")),
    };
    if !endpoint {
        return Err(
            "daemon socket is not an owned local endpoint; inspect it before retrying".into(),
        );
    }
    let trusted = hide_platform::fs::private::owned_by_current_user(path)
        .and_then(|owned| Ok(owned && !hide_platform::fs::private::others_can_modify(path)?))
        .map_err(|error| format!("daemon socket ownership cannot be inspected: {error}"))?;
    if !trusted {
        return Err("daemon socket can be changed by another account; inspect its ownership before retrying".into());
    }
    Ok(true)
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn stop_daemon(home: &Path) -> Result<(), String> {
    let path = home.join("api.sock");
    if !daemon_socket_present(&path)? {
        return Ok(());
    }
    let mut socket = match hide_platform::ipc::LocalStream::connect(&path) {
        Ok(socket) => socket,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(format!("daemon socket cannot be reached: {error}")),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    socket
        .write_all(b"{\"version\":1,\"operation\":\"daemon.stop\",\"args\":{}}\n")
        .map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("daemon stop response deadline reached".into());
        }
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|error| error.to_string())?;
        let count = socket.read(&mut chunk).map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("daemon closed before acknowledging stop".into());
        }
        response.extend_from_slice(&chunk[..count]);
        if response.len() > 64 * 1024 {
            return Err("daemon stop response exceeded the size bound".into());
        }
        if let Some(newline) = response.iter().position(|byte| *byte == b'\n') {
            response.truncate(newline);
            break;
        }
    }
    let value: Value = serde_json::from_slice(&response).map_err(|error| error.to_string())?;
    if value["ok"] != true || value["value"]["stopped"] != true {
        return Err("daemon refused the stop request".into());
    }
    let end = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !daemon_socket_present(&path)? {
            return Ok(());
        }
        match hide_platform::ipc::LocalStream::connect(&path) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(format!("daemon exit cannot be confirmed: {error}")),
            Ok(_) if std::time::Instant::now() >= end => {
                return Err("daemon still answers after stop; retirement did not proceed".into());
            }
            Ok(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn label(home: &Path) -> Result<String, String> {
    let account = hide_platform::user_agents::account_home()
        .map_err(|error| format!("account home cannot be resolved: {error}"))?;
    if home == account.join(".hide/hcoord") {
        return Ok("com.hcoord.daemon".into());
    }
    let digest = Sha256::digest(home.to_string_lossy().as_bytes());
    Ok(format!(
        "com.hcoord.daemon.{}",
        &format!("{digest:x}")[..12]
    ))
}

fn remove_agent(target: &KitTarget, home: &Path) -> Result<(), String> {
    let label = label(home)?;
    target
        .user_agents
        .remove(&label, &target.home, &target.stop)
        .map_err(|error| error.to_string())
}

fn remove_links(target: &KitTarget) -> Result<crate::Retirement, String> {
    let mut folders = vec![target.cli_dir.clone(), target.home.join(".local/bin")];
    if let Some(path) = std::env::var_os("PATH") {
        folders.extend(std::env::split_paths(&path));
    }
    folders.sort();
    folders.dedup();
    for folder in folders {
        let link = folder.join("hcoord");
        let Ok(destination) = fs::read_link(&link) else {
            continue;
        };
        let destination = if destination.is_absolute() {
            destination
        } else {
            folder.join(destination)
        };
        if homes(target)
            .iter()
            .any(|home| destination == home.join("bin/hcoord"))
        {
            hide_platform::fs::link::remove_link(&link)
                .map_err(|error| format!("owned command link could not be removed: {error}"))?;
        }
    }
    let plugin = crate::labels::retire_hcoord_plugin(target);
    if !plugin.failures.is_empty() {
        return Err(plugin.failures.join("; "));
    }
    Ok(plugin)
}

/// Herdr v0.9.1 persists its global registry here and its CLI can uninstall
/// without a server. Read strictly first: Herdr's offline list intentionally
/// treats a corrupt registry as empty, which cannot prove retirement.
pub(crate) fn retire_offline_plugin(target: &KitTarget) -> Result<Option<&'static str>, String> {
    use hide_herdr_client::plugin::{InstalledPlugin, PluginSourceKind};

    let config = crate::labels::herdr_config_dir(target)?;
    if let Ok(anchor) = home_anchor(&target.home, &config) {
        if !inspect_below(&anchor, &config)? {
            return Ok(None);
        }
    } else if !inspect_state_override(&config)? {
        return Ok(None);
    }
    let registry = config.join("plugins.json");
    let entries = || -> Result<Vec<InstalledPlugin>, String> {
        let Some(value) = read_json(&registry, MAX_STATE_BYTES)? else {
            return Ok(Vec::new());
        };
        let entries: Vec<InstalledPlugin> = serde_json::from_value(value)
            .map_err(|error| format!("Herdr's offline plugin registry is unreadable: {error}"))?;
        if entries.len() > MAX_ENTRIES {
            return Err("Herdr's offline plugin registry exceeds the entry bound".into());
        }
        Ok(entries)
    };
    let Some(plugin) = entries()?
        .into_iter()
        .find(|plugin| plugin.plugin_id == crate::HCOORD_PLUGIN_ID)
    else {
        return Ok(None);
    };
    // The CLI owns the registry lock, unregistering and managed-file removal.
    // It leaves locally linked source and per-plugin configuration intact.
    crate::labels::uninstall_managed(target, crate::HCOORD_PLUGIN_ID)?;
    if entries()?
        .iter()
        .any(|plugin| plugin.plugin_id == crate::HCOORD_PLUGIN_ID)
    {
        return Err("Herdr's offline plugin registration remains; retry retirement".into());
    }
    Ok(Some(match plugin.source.kind {
        PluginSourceKind::Github => "GitHub",
        PluginSourceKind::Local => "linked folder",
    }))
}

fn remove_copy(target: &KitTarget) -> Result<(), String> {
    let path = crate::record::private_state_dir(&target.home, false)?.join("hcoord");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("the kit copy is a link; inspect its ownership before retrying".into())
        }
        Ok(_) => {
            fs::remove_dir_all(path).map_err(|error| format!("kit copy cannot be removed: {error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn preserve(target: &KitTarget, progress: &Progress) -> Result<(), String> {
    for (home, destination) in &progress.homes {
        if inspect_legacy_home(target, home)? {
            if destination.exists() {
                return Err("the preserved ledger name already exists; keep both folders and resolve the collision before retrying".into());
            }
            fs::rename(home, destination)
                .map_err(|error| format!("ledger folder cannot be renamed: {error}"))?;
        } else if !inspect_below(&home_anchor(&target.home, destination)?, destination)? {
            return Err("both the old ledger folder and its preserved destination are missing; inspect before retrying".into());
        }
    }
    Ok(())
}

fn date() -> String {
    // Gregorian civil date from UTC days. No shell command or runtime dependency.
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400;
    let z = days as i64 + 719468;
    let era = z / 146097;
    let day = z - era * 146097;
    let year_of_era = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_of_year = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_of_year + 2) / 5 + 1;
    let month = month_of_year + if month_of_year < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(all(test, unix))]
mod daemon_tests {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn an_existing_socket_with_a_long_name_keeps_its_transport_failure() {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("original.sock");
        let listener = UnixListener::bind(&original).unwrap();
        listener.set_nonblocking(true).unwrap();
        let home = fixture.path().join(format!("home-{}", "l".repeat(160)));
        fs::create_dir(&home).unwrap();
        let path = home.join("api.sock");
        fs::rename(original, &path).unwrap();
        let before = fs::symlink_metadata(&path).unwrap();
        assert!(before.file_type().is_socket());
        let failure = stop_daemon(&home).unwrap_err();
        assert!(
            failure.contains("daemon socket cannot be reached"),
            "{failure}"
        );
        assert!(failure.contains("sun_path"), "{failure}");
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), before.ino());
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn untrusted_socket_entries_are_refused_without_connecting_or_changing_them() {
        for entry in ["file", "folder", "link", "writable socket"] {
            let fixture = tempfile::tempdir().unwrap();
            let home = fixture.path().join("home");
            fs::create_dir(&home).unwrap();
            let path = home.join("api.sock");
            let other_path = fixture.path().join("other.sock");
            let other = UnixListener::bind(&other_path).unwrap();
            other.set_nonblocking(true).unwrap();
            let listener = match entry {
                "file" => {
                    fs::write(&path, b"preserved bytes").unwrap();
                    None
                }
                "folder" => {
                    fs::create_dir(&path).unwrap();
                    None
                }
                "link" => {
                    std::os::unix::fs::symlink(&other_path, &path).unwrap();
                    None
                }
                _ => {
                    let listener = UnixListener::bind(&path).unwrap();
                    listener.set_nonblocking(true).unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
                    Some(listener)
                }
            };
            let before = fs::symlink_metadata(&path).unwrap();
            let failure = stop_daemon(&home).unwrap_err();
            assert!(failure.contains("daemon socket"), "{entry}: {failure}");
            let after = fs::symlink_metadata(&path).unwrap();
            assert_eq!(after.ino(), before.ino());
            assert_eq!(after.permissions().mode(), before.permissions().mode());
            assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
            if entry == "file" {
                assert_eq!(fs::read(&path).unwrap(), b"preserved bytes");
            } else if entry == "link" {
                assert_eq!(fs::read_link(&path).unwrap(), other_path);
            }
            assert!(
                matches!(other.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
            );
            if let Some(listener) = listener {
                assert!(
                    matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
                );
            }
        }
    }

    #[test]
    fn an_endpoint_that_cannot_be_inspected_is_not_absent() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("not-a-folder");
        fs::write(&home, b"preserved bytes").unwrap();
        let failure = stop_daemon(&home).unwrap_err();
        assert!(
            failure.contains("daemon socket cannot be inspected"),
            "{failure}"
        );
        assert_eq!(fs::read(home).unwrap(), b"preserved bytes");
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    }
}
