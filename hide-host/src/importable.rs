//! `Call::ImportableProjects`: the projects worth adding on this node
//! (PRD device-add-project D-09 to D-14, B20 to B28).
//!
//! Three kinds of source name folders: every agent whose sessions Hide reads
//! (`hide_session::SessionCatalog::worked_folders`, the walk the session
//! archive reads, so an agent that gains a session reader joins with no
//! change here, D-10), Paseo's project registry and Herdr's saved session.
//! Each folder is judged by the rule a registration is judged by
//! (`register::judge`, unconfirmed), which names the project root it would
//! add and leaves out home, a folder outside home, the home dotfiles
//! repository and a folder that is gone; folders of one root are one
//! project carrying every source that named them (D-11).
//!
//! Every file is only read, opened without waiting and only when it is a
//! regular file (`hide_platform::fs::open_regular`), and each read is capped
//! (D-12, B26). A source that cannot be read is answered as dropped with
//! why, for the caller's log, and never takes another source with it (B27).
//! The search ends by its deadline: a source not reached by then is dropped
//! as late, so no call outlives the caller waiting for it.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hide_platform::fs::identity;
use serde::Deserialize;
use serde_json::Value;

pub use hide_node_link::importable::{
    DroppedSource, EXCLUDE_CAP, IMPORTABLE_CAP, IMPORTABLE_SEARCH_BUDGET, ImportApp, ImportSource,
    ImportableProject, ImportableProjects, JUDGED_FOLDER_CAP, PASEO_READ_CAP, RECENT_WINDOW_MS,
};

/// Why a source the search had not reached by its deadline is dropped.
const LATE: &str = "the search ran out of time";

/// A folder one source named, and when that source last saw it in use.
struct Named {
    folder: PathBuf,
    source: ImportSource,
    active_at_unix_ms: u64,
}

/// The projects worth adding for the account whose home is `home`, read at
/// `now_unix_ms`, with `variables` the account's environment (Paseo's and
/// Herdr's folders follow it), leaving out each root `exclude` names, by
/// `deadline`.
pub fn find(
    home: &Path,
    variables: hide_platform::host::Variables,
    now_unix_ms: u64,
    exclude: &[String],
    deadline: Instant,
) -> ImportableProjects {
    let since = now_unix_ms.saturating_sub(RECENT_WINDOW_MS);
    let mut named = Vec::new();
    let mut dropped = Vec::new();
    agents(home, since, deadline, &mut named, &mut dropped);
    for app in [ImportApp::Paseo, ImportApp::Herdr] {
        let read = if Instant::now() >= deadline {
            Err(LATE.to_owned())
        } else {
            match app {
                ImportApp::Paseo => paseo(home, variables),
                ImportApp::Herdr => herdr(variables),
            }
        };
        match read {
            Ok(rows) => named.extend(rows),
            Err(reason) => dropped.push(DroppedSource {
                source: app_name(app).to_owned(),
                reason,
            }),
        }
    }
    ImportableProjects {
        projects: merge(home, named, since, exclude, deadline),
        dropped,
    }
}

fn app_name(app: ImportApp) -> &'static str {
    match app {
        ImportApp::Paseo => "paseo",
        ImportApp::Herdr => "herdr",
    }
}

fn agents(
    home: &Path,
    since: u64,
    deadline: Instant,
    named: &mut Vec<Named>,
    dropped: &mut Vec<DroppedSource>,
) {
    let read = hide_session::SessionCatalog::new(home, "").worked_folders(since, deadline);
    named.extend(read.folders.into_iter().map(|folder| Named {
        folder: folder.cwd,
        source: ImportSource::Agent(folder.agent),
        active_at_unix_ms: folder.active_at_unix_ms,
    }));
    dropped.extend(read.refusals.into_iter().map(|refusal| DroppedSource {
        source: agent_name(refusal.agent),
        reason: refusal.reason,
    }));
}

fn agent_name(agent: hide_session::Agent) -> String {
    serde_json::to_value(agent)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{agent:?}"))
}

/// Reads at most `cap` bytes of the regular file at `path`, a link to it
/// followed; `None` when nothing is there. A pipe or a device at the path
/// is refused at once rather than waited on.
fn read_capped(path: &Path, cap: u64) -> Result<Option<String>, String> {
    let file =
        match std::fs::canonicalize(path).and_then(|real| hide_platform::fs::open_regular(&real)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("it could not be opened: {error}")),
        };
    let mut text = String::new();
    file.take(cap + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("it could not be read: {error}"))?;
    if text.len() as u64 > cap {
        return Err(format!("it is larger than {cap} bytes"));
    }
    Ok(Some(text))
}

/// A path an app wrote, native and absolute, or nothing.
fn absolute(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text);
    (path.is_absolute() && !text.chars().any(char::is_control)).then_some(path)
}

/// Paseo's project registry, `projects/projects.json` in Paseo's home
/// (`$PASEO_HOME`, else `~/.paseo`, as Paseo's `resolvePaseoHome` reads
/// it): an array of records of which only `rootPath`, `updatedAt` and
/// `archivedAt` are read; an archived project is not one.
fn paseo(home: &Path, variables: hide_platform::host::Variables) -> Result<Vec<Named>, String> {
    let place = match variables("PASEO_HOME").map(PathBuf::from) {
        None => home.join(".paseo"),
        Some(written) => match written.strip_prefix("~") {
            Ok(rest) => home.join(rest),
            Err(_) if written.is_absolute() => written,
            Err(_) => return Err("PASEO_HOME is not an absolute path".to_owned()),
        },
    };
    let file = place.join("projects").join("projects.json");
    let Some(text) = read_capped(&file, PASEO_READ_CAP)? else {
        return Ok(Vec::new());
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Record {
        root_path: String,
        #[serde(default)]
        updated_at: Option<String>,
        #[serde(default)]
        archived_at: Option<String>,
    }
    let records: Vec<Record> = serde_json::from_str(&text)
        .map_err(|error| format!("it is not a Paseo project registry: {error}"))?;
    Ok(records
        .into_iter()
        .filter(|record| record.archived_at.is_none())
        .filter_map(|record| {
            Some(Named {
                folder: absolute(&record.root_path)?,
                source: ImportSource::App(ImportApp::Paseo),
                active_at_unix_ms: record
                    .updated_at
                    .as_deref()
                    .and_then(hide_session::parse_rfc3339)
                    .map_or(0, |seconds| seconds.saturating_mul(1000)),
            })
        })
        .collect())
}

/// Herdr's saved session, `session.json` in Herdr's config folder
/// (`hide_platform::host::herdr_config_dir_from`): each workspace's
/// repository root when Herdr placed it in one, else the folder it is
/// named by, else (the form before tabs) its first pane's folder. Herdr
/// keeps no time per workspace, so each is as recent as the save.
fn herdr(variables: hide_platform::host::Variables) -> Result<Vec<Named>, String> {
    let Ok(config) = hide_platform::host::herdr_config_dir_from(variables) else {
        return Ok(Vec::new());
    };
    let fetched = crate::herdr_save::fetch(&config.join("session.json"));
    let (text, modified_ns) = match fetched {
        crate::herdr_save::Fetched::Missing => return Ok(Vec::new()),
        crate::herdr_save::Fetched::Unreadable { reason } => return Err(reason),
        crate::herdr_save::Fetched::Text {
            text, modified_ns, ..
        } => (text, modified_ns),
    };
    #[derive(Deserialize)]
    struct Save {
        #[serde(default)]
        version: u32,
        #[serde(default)]
        workspaces: Vec<Value>,
    }
    let save: Save = serde_json::from_str(&text)
        .map_err(|error| format!("it is not a Herdr session: {error}"))?;
    if save.version > crate::herdr_save::NEWEST_FORMAT {
        return Err(format!(
            "it is format {}, newer than the format {} this build reads",
            save.version,
            crate::herdr_save::NEWEST_FORMAT
        ));
    }
    let active_at_unix_ms = modified_ns / 1_000_000;
    Ok(save
        .workspaces
        .iter()
        .filter_map(|workspace| {
            let text = |value: Option<&Value>| value.and_then(Value::as_str).and_then(absolute);
            let folder = text(workspace.pointer("/worktree_space/repo_root"))
                .or_else(|| text(workspace.get("identity_cwd")))
                .or_else(|| {
                    let panes = workspace.get("panes")?.as_object()?;
                    let mut keys = panes.keys().collect::<Vec<_>>();
                    keys.sort_by_key(|key| key.parse::<u64>().unwrap_or(u64::MAX));
                    keys.into_iter()
                        .find_map(|key| text(panes.get(key)?.get("cwd")))
                })?;
            Some(Named {
                folder,
                source: ImportSource::App(ImportApp::Herdr),
                active_at_unix_ms,
            })
        })
        .collect())
}

/// The folders `named` names as the projects a registration would add,
/// each root once with every source that named it and its newest activity,
/// those active since `since` and not in `exclude`, newest first, at most
/// [`IMPORTABLE_CAP`]. Only the newest [`JUDGED_FOLDER_CAP`] folders are
/// judged, and none after `deadline`.
fn merge(
    home: &Path,
    named: Vec<Named>,
    since: u64,
    exclude: &[String],
    deadline: Instant,
) -> Vec<ImportableProject> {
    let mut folders: HashMap<PathBuf, Vec<(ImportSource, u64)>> = HashMap::new();
    for row in named {
        folders
            .entry(row.folder)
            .or_default()
            .push((row.source, row.active_at_unix_ms));
    }
    let mut folders = folders.into_iter().collect::<Vec<_>>();
    let newest = |sources: &[(ImportSource, u64)]| sources.iter().map(|(_, at)| *at).max();
    folders.sort_by(|left, right| {
        newest(&right.1)
            .cmp(&newest(&left.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    folders.truncate(JUDGED_FOLDER_CAP);
    let excluded = exclude
        .iter()
        .take(EXCLUDE_CAP)
        .filter_map(|path| hide_platform::path::from_wire(path).ok())
        .filter_map(|path| identity::canonical(&path).ok())
        .collect::<BTreeSet<_>>();
    let home_real = identity::canonical(home).unwrap_or_else(|_| home.to_path_buf());
    let mut projects: Vec<(PathBuf, Vec<ImportSource>, u64)> = Vec::new();
    for (folder, sources) in folders {
        if Instant::now() >= deadline {
            break;
        }
        let Ok(judged) = crate::register::judge(&folder, None, home) else {
            continue;
        };
        if excluded.contains(&judged.root) {
            continue;
        }
        let at = newest(&sources).unwrap_or(0);
        let index = match projects.iter().position(|(root, ..)| *root == judged.root) {
            Some(index) => index,
            None => {
                projects.push((judged.root, Vec::new(), 0));
                projects.len() - 1
            }
        };
        let project = &mut projects[index];
        project.2 = project.2.max(at);
        for (source, _) in sources {
            if !project.1.contains(&source) {
                project.1.push(source);
            }
        }
    }
    projects.retain(|(_, _, at)| *at >= since);
    projects.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    projects.truncate(IMPORTABLE_CAP);
    projects
        .into_iter()
        .map(|(root, sources, active_at_unix_ms)| ImportableProject {
            path: crate::root::wire(&root),
            label: crate::browse::home_label(&root, &home_real),
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::root::wire(&root)),
            sources,
            active_at_unix_ms,
        })
        .collect()
}
