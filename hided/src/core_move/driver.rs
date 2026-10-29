//! The driver's steps of a forward move that wait on files or on the other
//! machine (PRD core-host-node-move B4, B5): each runs on a blocking
//! thread, and the supervisor switches the process's role between them
//! (`supervisor`). Every step takes the journal and answers a typed failure.

use std::path::{Path, PathBuf};
use std::time::Duration;

use herdr_core::node_migration::{self, IdTable, KnownProject, OwnerChange, copy};
use hide_node::ssh::session::SshTarget;
use hide_node::ssh::transfer::FileCopy;
use hide_node::ssh::upstream::Upstream;
use serde_json::{Value, json};

use super::answer::{StepAnswer, StepLine};
use super::handover::{Handover, HandoverState};
use super::journal::{Journal, MoveFailure, Peer};

/// The most a step on the other machine may print.
const STEP_OUTPUT_CAP: usize = 64 * 1024;
/// How long one step on the other machine may take; starting its core
/// waits up to 30 s for it to take links.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);

/// The other machine, reached through one owned `ssh` for the move, signed
/// in once and kept for its later steps and copies. The connection ends
/// when this is dropped, on every way a move's turn ends.
pub struct Remote {
    /// Dialed by the first step that reaches the peer: `ssh` reads the
    /// alias then, and a dial that fails is the peer not reached, which a
    /// wait asks again, so an alias that does not resolve at a start never
    /// keeps the daemon from it.
    upstream: Upstream,
    program: String,
    state_dir: Option<String>,
    /// This machine's build: the only build whose answers it reads.
    build: String,
}

impl Remote {
    /// Reads and dials nothing yet: the first step does.
    pub fn new(home: &Path, alias: &str, program: &str, build: &str) -> Self {
        Self {
            upstream: Upstream::new(SshTarget::new(home, alias)),
            program: program.to_owned(),
            state_dir: None,
            build: build.to_owned(),
        }
    }

    pub fn with_state_dir(mut self, state_dir: &str) -> Self {
        self.state_dir = Some(state_dir.to_owned());
        self
    }

    /// Installs this machine's build from `packages` beside the `hided` it
    /// runs there, through the core update's install (the version folder
    /// under the same helper root, at most two builds kept), and answers
    /// the installed `hided`.
    pub fn install_build(&self, packages: &Path) -> Result<String, MoveFailure> {
        let root = hide_kit::build_of(&self.program)
            .map_err(|reason| MoveFailure::Local { reason })?
            .root;
        self.upstream
            .install_build(
                &hide_node::ssh::host::HelperPackages::new(Some(packages.to_path_buf())),
                &root,
            )
            .map_err(|error| MoveFailure::Copy {
                reason: error.to_string(),
            })
    }

    /// Runs `hided core-move <step>` there and answers its line. A line of
    /// another build is `OtherBuild`, a refusal `Refused` (`Load` for a
    /// file the verify could not load), and a busy answer `Busy`.
    pub fn step(&self, step: &str, args: &[(&str, &str)]) -> Result<StepAnswer, MoveFailure> {
        let mut command = vec![
            self.program.clone(),
            "core-move".to_owned(),
            step.to_owned(),
        ];
        if let Some(state_dir) = &self.state_dir {
            command.extend(["--state-dir".to_owned(), state_dir.clone()]);
        }
        for (flag, value) in args {
            command.extend([format!("--{flag}"), (*value).to_owned()]);
        }
        let output = self
            .upstream
            .run(&command, STEP_OUTPUT_CAP, STEP_TIMEOUT)
            .map_err(|error| {
                if error.never_reached_server() || error.a_move_can_change() {
                    MoveFailure::Unreachable {
                        reason: error.to_string(),
                    }
                } else {
                    MoveFailure::Refused {
                        step: step.to_owned(),
                        reason: error.to_string(),
                    }
                }
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.lines().last().unwrap_or_default();
        read_line(step, line, &self.build).map_err(|unread| {
            unread.unwrap_or_else(|| MoveFailure::Refused {
                step: step.to_owned(),
                reason: format!(
                    "exit {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                        .trim()
                        .chars()
                        .take(512)
                        .collect::<String>()
                ),
            })
        })
    }

    pub fn upload(
        &self,
        files: &[FileCopy],
        sent: &(dyn Fn(u64) + Sync),
    ) -> Result<(), MoveFailure> {
        self.upstream
            .upload(files, sent)
            .map_err(|error| MoveFailure::Copy {
                reason: error.to_string(),
            })
    }

    /// Downloads `files`, each of which must be below `into`.
    pub fn download(
        &self,
        into: &Path,
        files: &[FileCopy],
        received: &(dyn Fn(u64) + Sync),
    ) -> Result<(), MoveFailure> {
        self.upstream
            .download(into, files, received)
            .map_err(|error| MoveFailure::Copy {
                reason: error.to_string(),
            })
    }
}

/// A step's answer from its line. A line that is not one answers `None`,
/// for the caller to name by the step's exit; one written by another build
/// is not read past its build.
fn read_line(step: &str, line: &str, build: &str) -> Result<StepAnswer, Option<MoveFailure>> {
    let read: StepLine = match serde_json::from_str(line) {
        Ok(read) => read,
        Err(_) => {
            // Another build's line may have another shape; its build is
            // where every build writes it.
            let theirs = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|line| line.get("build")?.as_str().map(str::to_owned));
            return Err(theirs
                .filter(|theirs| theirs != build)
                .map(|build| MoveFailure::OtherBuild { build }));
        }
    };
    if read.build != build {
        return Err(Some(MoveFailure::OtherBuild { build: read.build }));
    }
    match read.answer {
        StepAnswer::Refused {
            file: Some(file),
            reason,
        } if step == "verify" => Err(Some(MoveFailure::Load { file, reason })),
        StepAnswer::Refused { reason, .. } => Err(Some(MoveFailure::Refused {
            step: step.to_owned(),
            reason,
        })),
        StepAnswer::Busy { reason } => Err(Some(MoveFailure::Busy {
            step: step.to_owned(),
            reason,
        })),
        answer => Ok(answer),
    }
}

/// An answer `step` does not give: refused, never read as one it does.
pub(super) fn unexpected(step: &str, answer: &StepAnswer) -> MoveFailure {
    MoveFailure::Refused {
        step: step.to_owned(),
        reason: format!("an answer this step does not give: {answer:?}"),
    }
}

/// What the other machine says of itself before anything moves.
pub type Inspected = super::answer::Inspection;

/// Asks the other machine what it says of itself; `ai` names the agents
/// Hide AI asks, which it checks with its own logins.
pub fn inspect(remote: &Remote, ai: &[(String, String)]) -> Result<Inspected, MoveFailure> {
    let asks = serde_json::to_string(ai).map_err(|error| MoveFailure::Local {
        reason: error.to_string(),
    })?;
    let args: &[(&str, &str)] = if ai.is_empty() { &[] } else { &[("ai", &asks)] };
    match remote.step("inspect", args)? {
        StepAnswer::Inspected(inspected) => Ok(*inspected),
        other => Err(unexpected("inspect", &other)),
    }
}

/// The owner change of a forward move from this machine to the device
/// `source` names, and the id table from every project both machines
/// hold. This machine's project roots are resolved here, as its node will
/// report them to the new core. The device registers this machine by
/// `MoveSource::own_label`.
pub fn forward_change(
    source: &herdr_core::MoveSource,
    home: &Path,
    host_name: Option<&str>,
    own_herdr_socket: String,
    target_herdr_socket: String,
) -> Result<(OwnerChange, IdTable), MoveFailure> {
    let registration = serde_json::from_value(json!({
        "id": source.node,
        "label": source.own_label(host_name),
        "inbound": true,
    }))
    .map_err(|error| MoveFailure::Local {
        reason: error.to_string(),
    })?;
    let change = OwnerChange {
        old_owner: source.node.clone(),
        old_owner_as: source.node.clone(),
        new_owner: source.device_node.clone(),
        new_owner_was: source.device.clone(),
        old_owner_registration: registration,
        old_owner_herdr_socket: own_herdr_socket,
        new_owner_herdr_socket: target_herdr_socket,
    };
    let roots = project_roots(&source.projects, &source.node, home);
    let projects = source
        .projects
        .iter()
        .zip(&roots)
        .map(|(project, root)| KnownProject {
            id: &project.id,
            device_id: &project.device_id,
            path: &project.path,
            root,
            checkouts: project
                .checkouts
                .iter()
                .map(|(id, path)| (id.as_str(), path.as_str()))
                .collect(),
        });
    let ids = node_migration::id_table(&change, projects);
    Ok((change, ids))
}

/// The repository root each project's id digests: this machine's own
/// projects resolved on its disk, another machine's by the path it
/// reported as its root.
pub fn project_roots(
    projects: &[herdr_core::MoveProject],
    own_node: &str,
    home: &Path,
) -> Vec<PathBuf> {
    projects
        .iter()
        .map(|project| {
            if project.device_id != own_node {
                return PathBuf::from(&project.path);
            }
            // A registered project is the operator's confirmation of its
            // folder, outside home too (PRD wsl-device D-10).
            let path = Path::new(&project.path);
            match hide_host::register::check(path, Some(path), home) {
                Ok(registrable) => PathBuf::from(registrable.root),
                // A folder that is gone is grouped by its own path, as this
                // machine's node answers for it.
                Err(error) => {
                    herdr_core::diagnostic!(json!({
                        "component": "core_move",
                        "kind": "staging.root_unresolved",
                        "project": project.id,
                        "reason": error.to_string(),
                    }));
                    PathBuf::from(&project.path)
                }
            }
        })
        .collect()
}

/// Copies this machine's brain state and the Hide AI settings at
/// `ai_settings` into its staging folder with the device's label records
/// beside it, and changes the copy's owner.
pub fn stage(
    state_dir: &Path,
    ai_settings: &Path,
    journal: &Journal,
    device_labels: &Value,
) -> Result<copy::Manifest, MoveFailure> {
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    let failed = |refusal: node_migration::Refusal| MoveFailure::Staging {
        file: refusal.file.display().to_string(),
        reason: refusal.reason,
    };
    copy::stage(state_dir, &staging, ai_settings).map_err(failed)?;
    carry_labels(&staging, &journal.change.new_owner_was, device_labels).map_err(|reason| {
        MoveFailure::Staging {
            file: staging.join("labels.json").display().to_string(),
            reason,
        }
    })?;
    let outcome = node_migration::reown(&staging, &journal.change, &journal.ids).map_err(failed)?;
    herdr_core::diagnostic!(json!({
        "component": "core_move",
        "kind": "staging.reowned",
        "intent": journal.intent,
        "files": outcome.files,
        "pruned": outcome.pruned,
    }));
    // The target loads the copy before it is placed; this machine asks the
    // same of it first, while its own core can still be started again.
    copy::check_loadable(&staging).map_err(failed)?;
    copy::digest(&staging).map_err(failed)
}

/// Writes the device's in-memory label records into the copy's
/// `labels.json` as the section `reown` makes the new owner's own.
pub(crate) fn carry_labels(staging: &Path, device: &str, records: &Value) -> Result<(), String> {
    let empty = records.as_object().is_none_or(serde_json::Map::is_empty);
    let path = staging.join("labels.json");
    let mut labels: Value = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if empty {
                return Ok(());
            }
            json!({"version": 1, "targets": {}})
        }
        Err(error) => return Err(error.to_string()),
    };
    if empty {
        return Ok(());
    }
    let object = labels
        .as_object_mut()
        .ok_or("labels.json is not an object")?;
    let moved = object
        .entry("moved")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or("labels.json moved is not a map")?;
    moved.insert(format!("device:{device}"), records.clone());
    let bytes = serde_json::to_vec(&labels).map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Sends the staged copy and its manifest into the peer's
/// `move-incoming/<intent>`, then has the peer compare it; what differs is
/// sent once more. A copy the peer already holds from an earlier try is
/// not sent again.
pub fn send(
    remote: &Remote,
    state_dir: &Path,
    journal: &Journal,
    manifest: &copy::Manifest,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), MoveFailure> {
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    let incoming = format!(
        "{}/move-incoming/{}",
        journal.peer.state_dir.trim_end_matches('/'),
        journal.intent
    );
    let manifest_file = staging.with_extension("manifest.json");
    let bytes = serde_json::to_vec(manifest).map_err(|error| MoveFailure::Local {
        reason: error.to_string(),
    })?;
    hide_platform::fs::atomic::write_file(
        &manifest_file,
        &bytes,
        hide_platform::fs::Access::Private,
    )
    .map_err(|error| MoveFailure::Local {
        reason: error.to_string(),
    })?;
    let manifest_upload = FileCopy {
        local: manifest_file.clone(),
        remote: format!("{incoming}.manifest.json"),
    };
    remote.upload(std::slice::from_ref(&manifest_upload), &|_| {})?;
    let _ = std::fs::remove_file(&manifest_file);
    let total = manifest.total_bytes();
    let sent = std::sync::atomic::AtomicU64::new(0);
    let wanted = differing(remote, journal, manifest)?;
    let held = manifest.files.len().saturating_sub(wanted.len());
    let uploaded = send_until_same(
        wanted,
        |attempt, wanted| {
            let files = copies(wanted, &staging, &incoming)?;
            if attempt == 0 {
                // Files the peer already holds count as sent.
                let held: u64 = manifest
                    .files
                    .iter()
                    .filter(|(path, _)| !wanted.contains(path))
                    .map(|(_, file)| file.size)
                    .sum();
                sent.store(held, std::sync::atomic::Ordering::Relaxed);
                progress(held, total);
            }
            remote.upload(&files, &|bytes| {
                let now = sent.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed) + bytes;
                progress(now.min(total), total);
            })
        },
        || differing(remote, journal, manifest),
    )?;
    herdr_core::diagnostic!(json!({
        "component": "core_move",
        "kind": "copy.sent",
        "intent": journal.intent,
        "held": held,
        "uploaded": uploaded,
    }));
    Ok(())
}

/// Sends what `wanted` names and asks the peer again what differs, at most
/// twice: every file sent, once nothing differs, or the files that still
/// do. `upload` is told which pass it is on.
fn send_until_same(
    mut wanted: Vec<String>,
    mut upload: impl FnMut(usize, &[String]) -> Result<(), MoveFailure>,
    mut differing: impl FnMut() -> Result<Vec<String>, MoveFailure>,
) -> Result<Vec<String>, MoveFailure> {
    let mut uploaded = Vec::new();
    for attempt in 0..2 {
        if wanted.is_empty() {
            return Ok(uploaded);
        }
        upload(attempt, &wanted)?;
        uploaded.extend(wanted);
        wanted = differing()?;
    }
    if wanted.is_empty() {
        Ok(uploaded)
    } else {
        Err(MoveFailure::Digest { files: wanted })
    }
}

/// Each of `paths` below the copy at `local` here and at `remote` there.
/// A path a copy does not carry is refused, never joined: it would name a
/// file outside the copy on one machine or the other.
pub(super) fn copies(
    paths: &[String],
    local: &Path,
    remote: &str,
) -> Result<Vec<FileCopy>, MoveFailure> {
    paths
        .iter()
        .map(|path| {
            let relative = copy::carried(path).map_err(|reason| MoveFailure::Local { reason })?;
            let native = relative.to_native().map_err(|error| MoveFailure::Local {
                reason: format!("{path:?}: {error}"),
            })?;
            Ok(FileCopy {
                local: local.join(native),
                remote: hide_platform::path::wire_join(remote, &relative),
            })
        })
        .collect()
}

/// The files the peer's copy lacks or holds differently; none means the
/// peer loaded the copy with its build.
fn differing(
    remote: &Remote,
    journal: &Journal,
    manifest: &copy::Manifest,
) -> Result<Vec<String>, MoveFailure> {
    let answer = remote.step(
        "verify",
        &[
            ("intent", &journal.intent),
            ("target", &journal.change.new_owner),
        ],
    )?;
    differs_of(answer, manifest)
}

/// The files a peer's verify answer says it lacks or holds differently.
/// Only files `manifest` names can be sent: an answer that names another
/// is refused, never read as a file of this machine's to send.
fn differs_of(answer: StepAnswer, manifest: &copy::Manifest) -> Result<Vec<String>, MoveFailure> {
    let (differs, extra) = match answer {
        StepAnswer::Loadable => return Ok(Vec::new()),
        StepAnswer::Differs { differs, extra } => (differs, extra),
        other => return Err(unexpected("verify", &other)),
    };
    let unsent: Vec<&String> = differs
        .iter()
        .filter(|path| !manifest.files.contains_key(*path))
        .collect();
    if !unsent.is_empty() {
        return Err(MoveFailure::Refused {
            step: "verify".to_owned(),
            reason: format!("the answer names files the copy does not hold: {unsent:?}"),
        });
    }
    // The peer names these paths, so its answer is read as untrusted: a
    // path named twice would be sent twice.
    let mut named = std::collections::BTreeSet::new();
    let twice: Vec<&String> = differs
        .iter()
        .filter(|path| !named.insert(path.as_str()))
        .collect();
    if !twice.is_empty() {
        return Err(MoveFailure::Refused {
            step: "verify".to_owned(),
            reason: format!("the answer names files more than once: {twice:?}"),
        });
    }
    if differs.is_empty() {
        // Only files the manifest does not name differ: they are not sent,
        // and the peer would place them.
        return Err(MoveFailure::Refused {
            step: "verify".to_owned(),
            reason: format!("the copy holds files the move did not send: {extra:?}"),
        });
    }
    Ok(differs)
}

pub fn place(remote: &Remote, journal: &Journal) -> Result<(), MoveFailure> {
    match remote.step(
        "place",
        &[
            ("intent", &journal.intent),
            ("source", &journal.change.old_owner),
            ("target", &journal.change.new_owner),
        ],
    )? {
        StepAnswer::Placed { .. } => Ok(()),
        other => Err(unexpected("place", &other)),
    }
}

/// Starts the peer's pending core; answers how long its lease has left as
/// the peer counts it, a span no clock difference changes.
pub fn start_target(remote: &Remote, journal: &Journal) -> Result<Duration, MoveFailure> {
    match remote.step("start", &[("intent", &journal.intent)]) {
        Ok(StepAnswer::Started { lease_left_ms, .. }) => Ok(Duration::from_millis(lease_left_ms)),
        Ok(other) => Err(unexpected("start", &other)),
        Err(MoveFailure::Refused { reason, .. }) => Err(MoveFailure::NotStarted { reason }),
        Err(other) => Err(other),
    }
}

/// What the peer's handover says of the move once the link that carries
/// its intent may have been sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetSays {
    Active,
    /// Pending, no record, or another move's: the move did not commit
    /// there.
    NotCommitted,
}

pub fn target_status(remote: &Remote, journal: &Journal) -> Result<TargetSays, MoveFailure> {
    status_of(remote.step("status", &[("intent", &journal.intent)])?)
}

/// What a status answer says of the move; one it does not give is refused,
/// never read as not committed, which would start a core here while one
/// may run there.
fn status_of(answer: StepAnswer) -> Result<TargetSays, MoveFailure> {
    match answer {
        StepAnswer::Status {
            handover:
                Some(Handover {
                    state: HandoverState::Active,
                    ..
                }),
        } => Ok(TargetSays::Active),
        StepAnswer::Status {
            handover:
                None
                | Some(Handover {
                    state: HandoverState::Pending { .. },
                    ..
                }),
        }
        | StepAnswer::OtherMove { .. } => Ok(TargetSays::NotCommitted),
        other => Err(unexpected("status", &other)),
    }
}

/// What the peer's abort left there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Aborted {
    /// The move committed there after all.
    Active,
    /// No core of this move runs there and its copy is back in
    /// `move-incoming`, or another move holds the machine.
    Undone,
    /// No core of this move runs or starts there, but its copy is still in
    /// the state folder.
    CopyLeft { reason: String },
}

/// Stops the peer's pending core and takes its copy back into
/// `move-incoming`.
pub fn abort_target(remote: &Remote, journal: &Journal) -> Result<Aborted, MoveFailure> {
    aborted_of(remote.step("abort", &[("intent", &journal.intent)])?)
}

fn aborted_of(answer: StepAnswer) -> Result<Aborted, MoveFailure> {
    match answer {
        StepAnswer::Active => Ok(Aborted::Active),
        StepAnswer::Aborted | StepAnswer::OtherMove { .. } => Ok(Aborted::Undone),
        StepAnswer::StoppedNotReturned { reason } => Ok(Aborted::CopyLeft { reason }),
        other => Err(unexpected("abort", &other)),
    }
}

pub fn finish_target(remote: &Remote, journal: &Journal) -> Result<(), MoveFailure> {
    match remote.step("finish", &[("intent", &journal.intent)])? {
        StepAnswer::Finished => Ok(()),
        other => Err(unexpected("finish", &other)),
    }
}

/// What the peer of a move this machine gave up on says once reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Abandoned {
    /// No core of the move runs there, and what one ran on is set aside.
    Ended,
    /// Another move's record holds the machine, so this move holds nothing
    /// there.
    OtherMove { intent: String },
}

/// Has the peer end what the move `intent`, given up on here, left there.
pub fn abandon_target(remote: &Remote, intent: &str) -> Result<Abandoned, MoveFailure> {
    match remote.step("abandon", &[("intent", intent)])? {
        StepAnswer::Abandoned => Ok(Abandoned::Ended),
        StepAnswer::OtherMove { intent } => Ok(Abandoned::OtherMove { intent }),
        other => Err(unexpected("abandon", &other)),
    }
}

/// The peer this machine moves its core to, from the core's facts and the
/// peer's own answer.
pub fn peer(source: &herdr_core::MoveSource, inspected: &Inspected) -> Peer {
    Peer {
        device: source.device.clone(),
        alias: source.ssh_alias.clone(),
        node: inspected.node.clone(),
        program: source.helper_path.clone(),
        state_dir: inspected.state_dir.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(paths: &[&str]) -> copy::Manifest {
        let mut manifest = copy::Manifest::default();
        for path in paths {
            manifest.files.insert(
                (*path).to_owned(),
                copy::FileDigest {
                    size: 1,
                    sha256: "00".to_owned(),
                },
            );
        }
        manifest
    }

    /// The peer names the files it wants sent; only files the copy holds
    /// are read from this machine, and an answer that names another, or one
    /// twice, is refused with it named.
    #[test]
    fn a_verify_answer_naming_a_file_the_copy_does_not_hold_is_refused() {
        let manifest = manifest(&["labels.json", "factory-files/a.prd"]);
        let differs = |paths: &[&str]| StepAnswer::Differs {
            differs: paths.iter().map(|path| (*path).to_owned()).collect(),
            extra: Vec::new(),
        };
        assert_eq!(
            differs_of(differs(&["labels.json"]), &manifest).unwrap(),
            vec!["labels.json".to_owned()]
        );
        for path in [
            "/var/elsewhere/.ssh/id_ed25519",
            "../labels.json",
            "mobile.json",
        ] {
            match differs_of(differs(&["labels.json", path]), &manifest) {
                Err(MoveFailure::Refused { step, reason }) => {
                    assert_eq!(step, "verify");
                    assert!(reason.contains(path), "{reason}");
                }
                other => panic!("{path}: {other:?}"),
            }
        }
        match differs_of(differs(&["labels.json", "labels.json"]), &manifest) {
            Err(MoveFailure::Refused { step, reason }) => {
                assert_eq!(step, "verify");
                assert!(reason.contains("more than once"), "{reason}");
            }
            other => panic!("a path named twice: {other:?}"),
        }
    }

    fn line(build: &str, answer: StepAnswer) -> String {
        serde_json::to_string(&StepLine {
            build: build.to_owned(),
            answer,
        })
        .unwrap()
    }

    /// Both ends run one build, so a line of another build is not read
    /// past its build, whatever its shape; a refusal and a busy answer are
    /// failures of the step that gave them.
    #[test]
    fn a_step_line_is_read_only_from_this_build() {
        assert_eq!(
            read_line("status", &line("b1", StepAnswer::Aborted), "b1"),
            Ok(StepAnswer::Aborted)
        );
        assert_eq!(
            read_line("status", &line("b0", StepAnswer::Aborted), "b1"),
            Err(Some(MoveFailure::OtherBuild {
                build: "b0".to_owned()
            }))
        );
        assert_eq!(
            read_line("status", r#"{"build":"b0","state":"active"}"#, "b1"),
            Err(Some(MoveFailure::OtherBuild {
                build: "b0".to_owned()
            }))
        );
        assert_eq!(read_line("status", "usage: hided", "b1"), Err(None));
        let refused = |file: Option<&str>| StepAnswer::Refused {
            file: file.map(str::to_owned),
            reason: "no".to_owned(),
        };
        assert_eq!(
            read_line("verify", &line("b1", refused(Some("labels.json"))), "b1"),
            Err(Some(MoveFailure::Load {
                file: "labels.json".to_owned(),
                reason: "no".to_owned()
            }))
        );
        assert_eq!(
            read_line("place", &line("b1", refused(Some("labels.json"))), "b1"),
            Err(Some(MoveFailure::Refused {
                step: "place".to_owned(),
                reason: "no".to_owned()
            }))
        );
        let busy = read_line(
            "abort",
            &line(
                "b1",
                StepAnswer::Busy {
                    reason: "held".to_owned(),
                },
            ),
            "b1",
        );
        assert!(
            matches!(&busy, Err(Some(failure)) if failure.transient()),
            "{busy:?}"
        );
    }

    /// Only a handover that says active commits the move, and only a
    /// pending one, none, or another move's says it did not: any other
    /// answer is refused rather than read as either.
    #[test]
    fn a_status_or_abort_answer_the_step_does_not_give_is_refused() {
        let handover = |state| Some(Handover::new("move-1", "a", "b", state));
        assert_eq!(
            status_of(StepAnswer::Status {
                handover: handover(HandoverState::Active)
            }),
            Ok(TargetSays::Active)
        );
        for answer in [
            StepAnswer::Status {
                handover: handover(HandoverState::pending_from_now()),
            },
            StepAnswer::Status { handover: None },
            StepAnswer::OtherMove {
                intent: "move-2".to_owned(),
            },
        ] {
            assert_eq!(status_of(answer), Ok(TargetSays::NotCommitted));
        }
        for answer in [
            StepAnswer::Aborted,
            StepAnswer::Status {
                handover: handover(HandoverState::StoppedFor),
            },
            StepAnswer::Finished,
        ] {
            assert!(
                matches!(status_of(answer.clone()), Err(MoveFailure::Refused { .. })),
                "{answer:?}"
            );
        }
        assert_eq!(aborted_of(StepAnswer::Active), Ok(Aborted::Active));
        assert!(matches!(
            aborted_of(StepAnswer::Loadable),
            Err(MoveFailure::Refused { .. })
        ));
    }

    #[test]
    fn a_copied_file_is_named_below_the_copy_on_both_machines() {
        let staging = Path::new("/state/move-staging/i1");
        let files = copies(
            &["factory-files/a.prd".to_owned()],
            staging,
            "/core/move-incoming/i1/",
        )
        .unwrap();
        assert_eq!(files[0].local, staging.join("factory-files").join("a.prd"));
        assert_eq!(
            files[0].remote,
            "/core/move-incoming/i1/factory-files/a.prd"
        );
        for path in ["/var/elsewhere/.zshrc", "factory-files/../../.zshrc"] {
            assert!(
                copies(&[path.to_owned()], staging, "/core/move-incoming/i1").is_err(),
                "{path}"
            );
        }
    }

    /// A copy the peer finds whole only after the second pass ends sent,
    /// naming both passes' files, so its record is written on that exit too;
    /// one still differing after it is refused with what differs.
    #[test]
    fn a_copy_whole_only_after_the_second_pass_ends_sent_with_both_passes_files() {
        let mut answers = vec![Vec::new(), vec!["links.sqlite3".to_owned()]];
        let mut passes = Vec::new();
        let uploaded = send_until_same(
            vec!["labels.json".to_owned(), "links.sqlite3".to_owned()],
            |attempt, files| {
                passes.push((attempt, files.to_vec()));
                Ok(())
            },
            || Ok(answers.pop().expect("asked at most twice")),
        )
        .unwrap();
        assert_eq!(uploaded, ["labels.json", "links.sqlite3", "links.sqlite3"]);
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[1], (1, vec!["links.sqlite3".to_owned()]));

        let refused = send_until_same(
            vec!["links.sqlite3".to_owned()],
            |_, _| Ok(()),
            || Ok(vec!["links.sqlite3".to_owned()]),
        )
        .unwrap_err();
        assert!(
            matches!(refused, MoveFailure::Digest { ref files } if files == &["links.sqlite3"]),
            "{refused:?}"
        );
    }
}
