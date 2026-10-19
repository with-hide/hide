//! The requests a node answers (`hided node serve` on a device), one JSON
//! object per line.
//!
//! A request is `{"id": n, "op": "...", ...}` and its answer is
//! `{"id": n, "ok": ...}` or `{"id": n, "error": {"code", "message"}}`.
//! Answers may arrive out of order; the id pairs them. A long call may send
//! reports first, each `{"progress": n, "report": ...}` ([`Progress`]), and
//! a `cancel` request for it stops it. Every root-bearing
//! request names the root's path and the identity the first `root_open`
//! reported, so the helper refuses a checkout replaced between requests.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RootIdentity;
use crate::error::HostError;

/// Bumped when a request or an answer changes shape. Except for the audited
/// 24 transition, another version is refused; normal payload replacement
/// still installs the helper this build carries.
/// 9: `changes` takes the View displays' `diffs` and answers each (PRD S7
/// A5); a helper on 8 would ignore them and answer none.
/// 10: `kit` installs, judges and removes the device's install kit (PRD
/// device-parity); a helper on 9 would refuse it as an unknown operation.
/// 11: `home_sync` manages Hide's Home folder and its project links.
/// 12: `label_transcript` reads a pane's conversation for its label (PRD
/// labels-in-hided D-03); a helper on 11 would refuse it as unknown.
/// 13: a helper running from the default root `~/.hide/host-helper` takes
/// the old layout's helper root and bridge folder off the device in its
/// `kit` apply, and its report carries `legacy_retirement` (PRD
/// hide-home-layout D-13); a helper on 12 would leave them.
/// 14: the kit carries `codex_per_pane` with its `off` state, and a
/// `reinstall` names the parts the operator turned off (PRD
/// overview-request-view D-21, D-24); a helper on 13 would not know them.
/// 15: `session_activity` answers only a proven session's modification time
/// and size, for the parent-owned inactivity watcher.
/// 16: worktree facts carry lock reasons and measured ignored repositories;
/// `worktree_removal_check` measures the exact accepted deletion before any
/// pane closes. A helper without this preflight must never remove instead.
/// 17: Hello carries native machine identity for lineage, and the kit reports
/// the one-release coordination retirement instead of installing it.
/// 18: `link_files` lists the device's session files changed since a time
/// and `link_read` reads their link facts from a checkpoint (PRD link-graph
/// D-21); a helper on 17 would refuse both as unknown.
/// 19: the kit has seven agents, each with a Herdr integration piece in its
/// report, retires the other thirteen once, and no longer carries
/// `codex_per_pane`; `reinstall` has no `turn_off` and a report names the
/// Codex daemon capability itself (PRD settings-cleanup D-06, D-13, D-14). A
/// helper on 18 would still turn the Codex daemon off and know none of it.
/// 20: a `reinstall` can carry `codex_daemon_off`, the operator's own request
/// to turn Codex's shared server off on that device, and the report that
/// answers it carries `codex_daemon_off` (PRD settings-cleanup B27). A helper
/// on 19 would run the pass and silently ignore the request. An agent row
/// of the report carries `chosen`, the operator's own choice on record (PRD
/// settings-cleanup B9, D-07); a report without it reads as no choice.
/// 21: that `codex_daemon_off` request also stops the device's running
/// shared Codex daemon after the operator confirmed it, and the report says
/// whether a daemon answers (`codex_daemon_running`) and answers a stop that
/// did not take effect as `stop_failed` (PRD codex-daemon-apply D-11). A
/// helper on 20 is refused when it connects, so it never receives a request
/// that would only turn autostart off.
/// 22: the node answers the machine work the core used to do itself (PRD
/// core-host-node D-21): Git, `gh`, the provider CLIs, sessions, ports,
/// disk, attachments, project creation, clone, worktree cleanup and the Git
/// watch, each its own call. A helper on 21 would refuse each as unknown, so
/// it is refused at Hello instead.
/// 23: `process_descendants` lists the processes under a pid, and a running
/// `process_starts` answer names its program, so a worktree deletion waits
/// for every process under a pane's shell and names any still running (issue
/// 707). A helper on 22 would refuse the first as unknown and answer without
/// a name.
/// 24: the device runs this program in its node role (`hided node serve`)
/// instead of `hide-host-helper`, so the payload carries `hided` (PRD
/// core-host-node D-02); a device still running a helper answers Hello with
/// 23 or less, is refused, and the next connection installs the new payload. Its
/// panes ask for credentials and run `hide` commands over this link
/// (`panes_start`, `pane_proof_answer`, `pane_inspect`, the stream calls and
/// [`crate::panes::NodeEvent`]) instead of a separate bridge.
/// 25: Hello carries exact independently implemented reader features. A
/// matching protocol does not grant a reader, and an authenticated 24 link
/// retains only its audited legacy features until normal payload replacement.
/// 26: the device's panes' terminals flow inside this link (PRD
/// core-host-node-terminal D-10, D-18): `terminals_start` starts the node's
/// terminal service and [`crate::terminal::TerminalLine`]s carry controls,
/// keys and output both ways. A node on 25 would read those lines as
/// unreadable requests, so it is refused at Hello and reinstalled; the
/// audited 24 link keeps its legacy features and starts no terminals.
/// 27: `line_input` answers how a pane shell's terminal takes typed input,
/// so a start waits for the shell's line editor before it types a line the
/// terminal could cut. A node on 26 would refuse it as unknown, and a long
/// first prompt could not start on that device, so it is refused at Hello and
/// reinstalled.
/// 28: a node may dial its core (PRD core-host-node-remote-core D-04): the
/// core reaches such a node's Herdr and its desktop's browser gateway
/// through streams inside the link (`link_open`, `link_write`, `link_close`
/// and [`crate::panes::NodeEvent::LinkData`]), asks that gateway for a
/// capability (`browser_gateway`), takes a label generator's lock on the
/// node that owns the Herdr server, and terminal lines name the screen a
/// view or a key came from. A node on 27 would refuse the first as unknown,
/// so it is refused at Hello and reinstalled.
/// An event may gain an optional field within 28 for the same reason:
/// [`crate::panes::NodeEvent::StreamOpen`]'s `caller` is absent when the node
/// proved nothing, and the core then checks the command with `pane_inspect`;
/// a core uploads its own build to a device it dials, so no peer lacks it.
/// 29: `shell_integration` writes the startup files that mark a pane shell's
/// first prompt and answers the command and environment an agent's pane
/// starts its shell with, so a start types only once the shell has finished
/// starting. A node on 28 would refuse it as unknown, and a start on that
/// device could be typed into a startup question, so it is refused at Hello
/// and reinstalled.
/// 30: subscription sources use one credential-free `usage_limit_fetch`
/// call. Old helpers must not be asked for credentials by a newer core.
/// 31: pane proofs may carry one typed hook read and answer it directly.
/// Older peers cannot decode the hook answer and are refused at Hello.
/// 32: link reads carry bounded raw requests and tool-output issue sightings.
/// Older helpers cannot supply a session work record and are upgraded at Hello.
/// 33: `feedback` reads and removes a screen's staged View feedback
/// screenshots and places, removes and sweeps a letter's attachments on the
/// recipient's machine (PRD view-opener-feedback D-16). An older helper would
/// refuse it as unknown, so a Send to its agents could never carry a
/// screenshot; it is refused at Hello and upgraded.
/// 34: `guardrail_policy` writes the project guardrails' policy the agent
/// hooks of the node's account read (PRD agent-guardrails D-11). A node on 33
/// would refuse it as unknown, and a protection saved for a project there
/// would not reach its agents, so it is refused at Hello and reinstalled.
/// 35: `session_work_turn` answers the one turn a backlog item came from and
/// replaces `session_work_context`, whose issue templates fed the removed AI
/// issue draft, and a disk row carries its volume's size. The session search
/// index reads (`session_index_read`, `session_stamps`) and the `search`
/// reader feature left with the Conversation history screen. A helper on 34
/// does not know the new call and is upgraded.
/// 36: a Hide AI request (`AiRequest`) carries the operator's output
/// language, which has no default (PRD ai-output-language D-02). A helper on
/// 35 would refuse every request the core sends it, so it is refused at Hello
/// and upgraded.
/// 37: `copy` and `import` make a copy of an Explorer item and write a drop
/// from the file manager (PRD prelaunch explorer-actions). A helper on 36
/// would refuse them as unknown, so a paste or a drop on a device checkout
/// could never land; it is refused at Hello and upgraded.
/// 38: Antigravity CLI is a session format (`antigravity`), and three calls
/// serve it (PRD antigravity-first-class D-07, D-08): `antigravity_fork_mark`
/// and `antigravity_fork` find the conversation agy's `/fork` made, and
/// `antigravity_trust` records or takes out agy's trust of a worktree Hide
/// made. A helper on 37 could not read an agy session or start agy in a
/// worktree without its trust question, so it is refused at Hello and
/// upgraded.
/// 39: the pull request card query also reads the head commit's checks and
/// the reviews, for the card's timeline (PRD pr-card-and-session-record-
/// redesign D-04). A helper on 38 allows only the query text it knows, so it
/// would refuse every card read of a device project; it is refused at Hello
/// and upgraded.
/// 40: `LetterPull` carries `tool`, a hook's ask at a tool event for the
/// letters due while the agent works (PRD letters-to-busy-agents D-02). A
/// helper on 39 would drop the field and answer a prompt's letters, so it is
/// refused at Hello and upgraded.
/// 41: an agent row names its mark (`mark`: turn, going, waiting, done,
/// unread, quiet) instead of a glyph (`symbol`), its mark counts are four, a
/// row may wait on its pull request's checks (`wait: checks`) and its status
/// codes add `unfinished` (status-marks-simplify). A screen relayed through a
/// node on 40 would draw none of it, so it is refused at Hello and upgraded.
/// 42: `antigravity_trust` becomes `agent_trust`, which names the agent whose
/// folder trust it records or takes out, for Claude Code and Codex as for agy
/// (docs/agent-hooks.md, Folder trust for the worktrees Hide makes). A helper
/// on 41 would refuse the call, so a Claude Code or Codex start in a worktree
/// Hide made on that device would wait at the agent's own trust question; it
/// is refused at Hello and upgraded.
/// 43: `opencode_fork` forks an OpenCode session for a pane to open: by
/// OpenCode 1.x's own `--fork`, or through OpenCode 2.x's server, whose
/// interactive command has no `--fork` (docs/ARCHITECTURE.md, OpenCode). A
/// helper on 42 would refuse the call, so no OpenCode session on that device
/// could be forked; it is refused at Hello and upgraded.
/// 44: a Hide AI request carries the effort it asks of each provider
/// (`provider_efforts`), and the device's Codex backend honours a
/// `PlainJson` request by naming the schema in its instructions instead of
/// sending a strict `outputSchema` (PRD label-judgment-fixes D-01, D-12). A
/// helper on 43 would drop the effort and still send the schema Codex
/// refuses, so every agent label judged on that device's Codex would fail;
/// it is refused at Hello and upgraded.
/// 45: `terminate_group` answers what it did (`GroupEnd`): ended, with each
/// process under a Windows leader that was left running and why, or gone
/// already, where it answered nothing and refused a job that had ended. A
/// helper on 44 answers `null`, which a sleep could not read; it is refused
/// at Hello and upgraded.
/// 46: creation requests declare fork holds and bind their pane on the node.
/// Older helpers cannot protect bare pane keys; Hello refuses them and the
/// existing helper replacement installs a compatible build.
/// 47: hook letter pulls name prompt, tool or turn end.
/// A helper on 46 cannot distinguish the intake point; Hello refuses it
/// until the existing helper replacement installs a compatible build.
/// 48: side-channel pane input waits for the node's creation holder and
/// gets a permission result before writing through Herdr, without attaching.
/// Older terminal helpers cannot grant this permission and are replaced.
/// 49: the owning node's shell recipe declares Integrated or Plain readiness.
/// Older helpers cannot preserve ordinary shell starts and their readiness;
/// Hello refuses them until the existing helper replacement upgrades them.
/// 50: Memory worktree retirement is a regular machine operation.
/// Factory operations are no longer part of the public protocol.
/// An older helper cannot decode retirement and is upgraded at Hello.
pub const PROTOCOL_VERSION: u32 = 50;

/// What agy's index held in a checkout before Hide typed `/fork`
/// ([`Call::AntigravityForkMark`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntigravityForkMark {
    /// The root conversations already in the checkout, newest first.
    pub known: Vec<String>,
    /// How many steps the source conversation had.
    pub steps: i64,
}

/// How an OpenCode session is forked ([`Call::OpenCodeFork`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum OpenCodeFork {
    /// OpenCode 1.x: the pane forks it itself, `opencode -s <source> --fork`.
    Flag,
    /// OpenCode 2.x: its server made the session `id`, which the pane opens
    /// with `opencode -s <id>`.
    Forked { id: String },
}

/// How long a core waits for a node's OpenCode fork ([`Call::OpenCodeFork`]).
pub const OPENCODE_FORK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a node gives OpenCode 2.x's server to make the fork, the
/// `opencode` CLI's own start included. It is short of
/// [`OPENCODE_FORK_TIMEOUT`], so a fork that runs out is the node's answer
/// while its core still waits, rather than a call the core gave up on.
pub const OPENCODE_FORK_TIME: std::time::Duration = std::time::Duration::from_secs(20);

const _: () = assert!(OPENCODE_FORK_TIME.as_secs() + 10 <= OPENCODE_FORK_TIMEOUT.as_secs());

/// What confirms readiness of the owning node's shell recipe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellReadiness {
    /// The recipe reports Hide's first-prompt token.
    #[default]
    Integrated,
    /// No prompt token is coming; use the existing foreground-shell policy.
    Plain,
}

/// The owning node's exact shell command, environment and readiness policy
/// ([`Call::ShellIntegration`]), independent of the core machine's shell.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellLaunch {
    pub command: Vec<String>,
    pub readiness: ShellReadiness,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// What a link stream the core opens on a node that dialed it reaches on
/// the node's machine ([`Call::LinkOpen`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkEnd {
    /// The node's own Herdr socket (D-18, D-19).
    Herdr,
    /// The browser relay of the node's daemon, which carries a caller's CDP
    /// to the gateway of the desktop window whose registration the daemon
    /// holds: a caller on another machine driving a page only that window
    /// shows (B15). The stream's first bytes ask for a relay ticket the
    /// node handed out over this link.
    BrowserRelay,
}

impl LinkEnd {
    /// Streams to this end one link carries at once; the next open is
    /// refused.
    pub const fn cap(self) -> usize {
        match self {
            Self::Herdr => crate::panes::MAX_HERDR_STREAMS,
            Self::BrowserRelay => crate::panes::MAX_BROWSER_STREAMS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: Call,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RootRef {
    pub path: String,
    pub identity: RootIdentity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Call {
    Hello,
    RootOpen {
        root: String,
    },
    List {
        root: RootRef,
        path: String,
    },
    /// A stamp per folder (`hide_host::list::stamps`), at most
    /// `MAX_STAMPED_FOLDERS`, for a device Explorer's watch.
    Stamps {
        root: RootRef,
        folders: Vec<String>,
    },
    /// At most `hide_host::bytes::MAX_RANGE` bytes of a regular file from
    /// `offset` (`hide_host::bytes::read`).
    Bytes {
        root: RootRef,
        path: String,
        offset: u64,
        length: u64,
    },
    /// Every file under the root the ignore files admit, capped
    /// (`hide_host::index::walk`), for the ⌘P palette.
    Index {
        root: RootRef,
    },
    OpenDocument {
        root: RootRef,
        path: String,
    },
    /// The content revision of a file now, for settling a save whose answer
    /// was lost with the connection.
    Revision {
        root: RootRef,
        path: String,
    },
    Save {
        root: RootRef,
        path: String,
        contents: String,
        expected_revision: String,
    },
    /// A new empty file, or a folder when `directory`, named `name` in the
    /// folder `parent`. Never replaces an existing item.
    Create {
        root: RootRef,
        parent: String,
        name: String,
        directory: bool,
    },
    /// `path` takes the name `name` in its own folder.
    Rename {
        root: RootRef,
        path: String,
        name: String,
    },
    /// `path` moves into the folder `destination`, keeping its name.
    Move {
        root: RootRef,
        path: String,
        destination: String,
    },
    /// `path` is copied into the folder `destination`, whole and recursively.
    /// The copy takes the item's name, or `name copy.ext`, `name copy 2.ext`
    /// when that is taken, and appears all at once or not at all
    /// (`hide_host::copy::copy_into`); the answer is the name it took.
    Copy {
        root: RootRef,
        path: String,
        destination: String,
    },
    /// The files and folders of a drop from the operator's file manager,
    /// written into the folder `destination` under the names the copy rule
    /// gives (`hide_host::copy::import`). The bytes are the operator's and
    /// came from the core's machine, so a device writes what it was sent.
    Import {
        root: RootRef,
        destination: String,
        entries: Vec<crate::mutate::ImportEntry>,
    },
    /// `path` goes to this machine's Trash; `inode` is the item the operator
    /// confirmed, and another item found at the path is refused.
    Trash {
        root: RootRef,
        path: String,
        inode: Option<u64>,
    },
    /// The checkout's Git changes under `scope`, with the diff of `selected`
    /// from the working or the committed group, and of each of `diffs`
    /// (`hide_host::git`).
    Changes {
        root: RootRef,
        scope: String,
        selected: Option<String>,
        committed: bool,
        base: Option<String>,
        #[serde(default)]
        diffs: Vec<crate::git::DiffTarget>,
    },
    /// The project facts of a folder on this machine (`hide_project::facts`):
    /// a Herdr pane's directory or a registered project's path, which the
    /// daemon's catalog groups into projects with this device's id. Only
    /// facts are answered, never contents.
    Project {
        path: String,
    },
    /// The worktrees of the repository that holds `path`
    /// (`hide_host::worktrees::read`), measured against `bases` or the
    /// operator's `base_override`. `null` for a folder that is not a
    /// repository. With `reuse_unchanged`, a worktree whose HEAD, index and
    /// base refs did not move since the node last measured it keeps that
    /// measurement; without it every worktree is measured again.
    Worktrees {
        path: String,
        bases: std::collections::BTreeMap<String, String>,
        base_override: Option<String>,
        #[serde(default)]
        reuse_unchanged: bool,
    },
    /// Whether `branch` may be created in the repository at `path`
    /// (`hide_host::worktrees::check_new_branch`); nothing is created.
    BranchCheck {
        path: String,
        branch: String,
    },
    /// The real path of an existing directory, or `null`.
    Directory {
        path: String,
    },
    /// The project a folder would be registered as, judged against the
    /// host's own home folder (`hide_host::register::check`).
    Registrable {
        path: String,
    },
    /// Makes this host's Hide Home folder (`~/hide`) hold one link per
    /// project in `projects` and nothing Hide did not make
    /// (`hide_host::home::sync`).
    HomeSync {
        projects: Vec<String>,
    },
    /// Retires a worktree owned by a completed or discarded background task.
    WorktreeRetire {
        root: String,
        checkout: String,
        branch: String,
        discard: bool,
    },
    /// Rechecks and removes one operator-confirmed linked worktree, forced
    /// only as far as the operator accepted
    /// (`hide_host::worktrees::remove_confirmed`).
    WorktreeRemove {
        removal: crate::worktrees::ConfirmedRemoval,
    },
    /// Non-mutating authoritative check before any pane is closed.
    WorktreeRemovalCheck {
        removal: crate::worktrees::ConfirmedRemoval,
    },
    /// The device's install kit (`hide_host::kit`): the helper acts on the
    /// helper root it runs from, never on a folder the request names.
    /// `cli_dir` is where the consent allows the `hide` link, and
    /// `herdr_socket` the registration's Herdr socket; both may start with
    /// `~/`.
    Kit {
        action: KitAction,
        cli_dir: String,
        herdr_socket: Option<String>,
        /// Known registered checkout roots on this helper's device only.
        #[serde(default)]
        retirement_projects: Vec<String>,
    },
    /// Whether the node's account has Hide's agent hooks in place, read from
    /// that account's own configuration (`hide_agent_hooks::Diagnosis`).
    HookDiagnosis,
    /// Replaces the project guardrails' policy file under the node's home
    /// with `policy`, the projects of this machine with a protection on
    /// (`hide_agent_hooks::guardrails::write_policy`); answers `null` once
    /// the file is in place. A `sequence` below the last one this node
    /// applied leaves the newer file alone and is answered
    /// `{"superseded_by": <that sequence>}`: a write that outlived its
    /// caller's wait, or a caller whose sequence fell behind (another core,
    /// a clock set back), which writes again above it.
    GuardrailPolicy {
        policy: hide_agent_hooks::guardrails::Policy,
        sequence: u64,
    },
    /// Every worktree Git registers for the repository at `root`, the main
    /// one first (`worktrees::Registered`).
    WorktreesRegistered {
        root: String,
    },
    /// The first ignored folder of the worktree that holds a repository of
    /// its own, relative to it, or `null`.
    IgnoredRepository {
        worktree: String,
    },
    /// Makes a project at `path` (`project::ProjectCreated`): its folder
    /// first when `new_folder`, at the literal path the shell checked, then
    /// a repository in it when `initialize_git` and it has none.
    ProjectCreate {
        path: String,
        new_folder: bool,
        initialize_git: bool,
    },
    /// What each path is on this node, as the core's catalog reads it
    /// (`catalog::PathFacts`), at most `catalog::PATH_FACTS_LIMIT` paths.
    PathFacts {
        paths: Vec<String>,
    },
    /// Each path with its links and aliases resolved (`cleanup::PathState`).
    RealPaths {
        paths: Vec<String>,
    },
    /// The repository that holds `path`, read from its `.git` files
    /// (`cleanup::RepositoryDirs`), or `null`.
    Repository {
        path: String,
    },
    /// Judges each chosen folder of the checkout at `root` again from its
    /// files, looking for a nested repository within `walk`
    /// (`cleanup::FolderJudgments`).
    JudgeFolders {
        root: String,
        folders: Vec<crate::cleanup::FolderToJudge>,
        walk: crate::cleanup::WalkAllowance,
    },
    /// Moves `folder` into the trash under `common` with one rename;
    /// answers the trash entry.
    SetAsideFolder {
        common: String,
        folder: String,
    },
    /// Removes the clean worktree at `checkout` without force
    /// (`cleanup::CleanRemoval`).
    WorktreeRemoveClean {
        root: String,
        checkout: String,
        common: String,
    },
    /// Deletes what waits in the trash under `common`, waiting up to
    /// `wait_ms` for `ours`; answers how many of them remain.
    DrainTrash {
        common: String,
        ours: Vec<String>,
        wait_ms: u64,
    },
    /// Clones `source` into `parent/<name>` (`clone::CloneAnswer`),
    /// reporting while Git runs (`clone::CloneReport`); a report answered
    /// with false cancels the clone.
    RepositoryClone {
        source: crate::clone::CloneSource,
        parent: String,
    },
    /// One fixed git command in the repository at `root`; answers its
    /// output, trimmed.
    Git {
        root: String,
        command: crate::git::GitCommand,
    },
    /// Whether a background AI backend can answer now
    /// (`ai::Logged<ai::Availability>`).
    AiAvailability {
        backend: crate::ai::BackendSpec,
    },
    /// The models a backend's provider offers the logged-in account
    /// (`ai::Logged<ai::ModelCatalog>`).
    AiModels {
        backend: crate::ai::BackendSpec,
    },
    /// One request to a backend (`ai::Logged<Result<ai::AiResponse,
    /// ai::AiError>>`), reporting at least once a second; a report answered
    /// with false cancels it.
    AiExecute {
        backend: crate::ai::BackendSpec,
        request: crate::ai::AiRequest,
    },
    /// The backend's process measurement from its last request
    /// (`ai::ProcessMeasurement`).
    AiMeasurement {
        backend: crate::ai::BackendSpec,
    },
    /// Ends the backend's resident process so its next request starts a
    /// fresh one.
    AiRestart {
        backend: crate::ai::BackendSpec,
    },
    /// Drops the backend the core no longer has, ending its process.
    AiRelease {
        instance: u64,
    },
    /// A declared subscription source, fetched on the core's own node.
    /// `root` is the Claude working directory or Codex home for that source.
    /// Devices refuse it; credentials never cross the link (`usage::LimitFetch`).
    UsageLimitFetch {
        source: crate::usage::LimitSource,
        root: String,
    },
    /// One bounded public price-table read (`usage::PriceFetch`).
    /// No caller-selected URL or payload; devices refuse it.
    UsagePriceFetch,
    /// Reads files the operator picked to attach to a terminal
    /// (`attachments::ReadFile`, in order), reporting before each file; a
    /// report answered with false ends the read as cancelled.
    ReadAttachments {
        paths: Vec<String>,
    },
    /// Reads the files of a drop from the operator's file manager that hided
    /// staged on the core's machine (`attachments::ReadFile`, in order), with
    /// the caps of one drop (`mutate::IMPORT_MAX_ITEMS`). Devices refuse it:
    /// the staging is the core machine's.
    ReadDropped {
        paths: Vec<String>,
    },
    /// Removes the clipboard image a paste left at `path`
    /// (`attachments::is_clipboard_path`, refused otherwise); a file already
    /// gone is not an error.
    RemoveClipboard {
        path: String,
    },
    /// Ends the job `leader` leads, a pane's foreground job, for agent sleep
    /// (`hide_platform::process::terminate_group`): `SIGTERM` to its process
    /// group on Unix, and on Windows, where Herdr names the job by its first
    /// process, that process and everything under it. A group of 1 or less
    /// is refused unsent, since kill(-0) and kill(-1) reach far more. The
    /// answer is a [`crate::process::GroupEnd`]: ended, with what was left
    /// running, or gone already.
    TerminateGroup {
        leader: u32,
    },
    /// Whether an agent CLI named `name` is on the node's `PATH`, answered
    /// as a bool before a pane is asked to start it.
    AgentInstalled {
        name: String,
    },
    /// What agy's index holds in `cwd` before Hide types `/fork` into a
    /// pane that resumed `source`, an [`AntigravityForkMark`]
    /// (`hide_session::antigravity::fork_mark`).
    AntigravityForkMark {
        source: String,
        cwd: String,
    },
    /// The conversation agy's `/fork` made from `source` in `cwd` after
    /// `mark`, as an id, or `null` while agy has not written it
    /// (`hide_session::antigravity::forked`).
    AntigravityFork {
        source: String,
        cwd: String,
        mark: AntigravityForkMark,
    },
    /// Forks the OpenCode root session `source` in `cwd` for a pane to open,
    /// an [`OpenCodeFork`]: decided by the schema of the database holding it
    /// (`hide_session::opencode::fork_route`), and for OpenCode 2.x made by its
    /// server before the answer, within the node's own bound.
    #[serde(rename = "opencode_fork")]
    OpenCodeFork {
        source: String,
        cwd: String,
    },
    /// Records (`trusted`) or takes out an agent's trust of a worktree Hide
    /// made, in the node account's own settings of that agent
    /// (`hide_agent_hooks::folder_trust`). `agent` is the start's kind, as
    /// the adapter names it.
    AgentTrust {
        agent: String,
        path: String,
        trusted: bool,
    },
    /// Answers the account's exact normal shell recipe and readiness policy.
    /// Tries Hide's first-prompt integration, otherwise preserves the same
    /// account shell with Plain readiness. Missing authoritative launch facts
    /// are an error, never a substitute shell (`hide_host::shell_integration`).
    ShellIntegration,
    /// The start time and program of each pid, in order
    /// (`process::ProcessStart`).
    ProcessStarts {
        pids: Vec<u32>,
    },
    /// Every process under `pid`, parents before children, as a list of
    /// pids; one that has ended, or has no children, has none.
    ProcessDescendants {
        pid: u32,
    },
    /// How the terminal `pid` controls takes typed input now
    /// (`process::LineInput`), read before a pane's shell is given a line its
    /// terminal could cut.
    LineInput {
        pid: u32,
    },
    /// Measures each of `paths` (`disk::DiskUsage`), reporting each one as
    /// it finishes and answering them all. The entries of `shared_git` are
    /// a repository's shared Git directory, measured as one size.
    DiskUsage {
        paths: Vec<String>,
        #[serde(default)]
        shared_git: Vec<String>,
    },
    /// One `gh` command with the operator's login, in `cwd` when named
    /// (`gh::allowed` names the command lines; any other is refused unrun).
    /// `repository` (`[HOST/]OWNER/NAME`, `gh::valid_repository`) names the
    /// repository a command that would read it from `cwd` works on instead,
    /// for a checkout on another machine. Answers `gh::GhAnswer`.
    Gh {
        cwd: Option<String>,
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository: Option<String>,
    },
    /// The machine's TCP listeners and where each was started
    /// (`ports::ListeningPorts`).
    ListeningPorts,
    /// Bytes free to an unprivileged writer on the volume holding `path`;
    /// `None` when the volume cannot say.
    VolumeFree {
        path: String,
    },
    /// One bounded read of a pane's conversation for its label, from the
    /// checkpoint the caller kept (`hide_session::label_transcript::read`).
    /// The helper keeps nothing between reads; it answers events and the
    /// next checkpoint, never a path.
    LabelTranscript {
        request: hide_session::label_transcript::LabelTranscriptRequest,
    },
    /// The one original turn a backlog item came from, for its panel's
    /// origin and the Start prompt. No other turn is returned.
    SessionWorkTurn {
        request: hide_session::label_transcript::WorkTurnRequest,
    },
    /// Metadata-only activity using the same native ownership proof as labels.
    SessionActivity {
        request: hide_session::session_activity::SessionActivityRequest,
    },
    /// The device's session files changed since a time and, for a later
    /// page, up to one, newest first and capped
    /// (`hide_session::links::candidates`).
    LinkFiles {
        since_unix_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until_unix_ms: Option<u64>,
    },
    /// Link facts read from each file's checkpoint, a few files and one read
    /// budget each (`hide_session::links::read`). The answer carries branch
    /// spans, pull request addresses and the request before each, never a
    /// conversation; a path outside the agent roots is refused.
    LinkRead {
        requests: Vec<hide_session::links::ReadRequest>,
    },
    /// Content-free usage candidates from this node's native agent roots.
    /// Requires the independently advertised `Usage` reader feature.
    UsageFiles {
        agent: hide_session::Agent,
        since_unix_ms: u64,
    },
    /// One bounded usage step; the core owns the durable checkpoint and
    /// ledger. This never returns conversation records.
    UsageRead {
        agent: hide_session::Agent,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<hide_session::usage::Checkpoint>,
    },
    /// The agent sessions the node holds for `project`, newest first, at most
    /// `hide_session::SESSION_DISCOVERY_LIMIT` files visited
    /// (`hide_session::ProjectSession`).
    ProjectSessions {
        project: hide_project::ProjectIdentity,
    },
    /// Watches the Git facts of each repository whose common directory is in
    /// `common_dirs`, at most `worktrees::GIT_WATCH_LIMIT`, reporting
    /// `worktrees::GitWatchReport` at least once a second until the caller
    /// answers a report with false; the call then answers nothing.
    GitWatch {
        common_dirs: Vec<String>,
    },
    /// The session file's size and modification time (`sessions::SessionStat`).
    SessionStat {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<hide_session::SessionReadScope>,
    },
    /// The whole session file at `path`, at most
    /// `hide_session::SESSION_READ_LIMIT_BYTES`, as text.
    SessionText {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<hide_session::SessionReadScope>,
    },
    /// A bounded provider-neutral conversation page, including native graphs.
    SessionConversation {
        agent: hide_session::Agent,
        path: String,
        scope: hide_session::SessionReadScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<hide_session::ConversationCheckpoint>,
    },
    /// Starts this node's pane service for the Herdr at `herdr_socket`: the
    /// bootstrap socket a device pane's `hide` asks on. Answered with
    /// [`crate::panes::PanesStarted`]; asking again for the same Herdr
    /// answers the same socket.
    PanesStart {
        herdr_socket: String,
    },
    /// Starts this node's terminal service for the Herdr at `herdr_socket`:
    /// from then on the link's terminal lines reach it. Asking again while
    /// it runs changes nothing.
    TerminalsStart {
        herdr_socket: String,
    },
    /// The core's answer to the pane proof `request` the node sent up.
    PaneProofAnswer {
        request: u64,
        answer: crate::panes::ProofAnswer,
    },
    /// `pane_id`'s identity now, read from this node's Herdr and kernel, so
    /// the core can tell a credential's pane from a replaced one.
    PaneInspect {
        pane_id: String,
    },
    /// Base64 bytes for the `hide` command on `stream`; answered once they
    /// are written to it.
    StreamWrite {
        stream: u64,
        data: String,
    },
    /// Ends `stream` from the core's side.
    StreamClose {
        stream: u64,
    },
    /// Opens `stream` to `end` on this node's machine for the core: the
    /// only way a core reaches what listens only there on a node that
    /// dialed it (PRD core-host-node-remote-core D-18, D-19, B15). What the
    /// node reads arrives as [`crate::panes::NodeEvent::LinkData`]. A node
    /// with nothing at that end, a device its core dialed included,
    /// refuses it.
    LinkOpen {
        stream: u64,
        end: LinkEnd,
    },
    /// Base64 bytes for `stream`'s end; answered once they are written.
    LinkWrite {
        stream: u64,
        data: String,
    },
    /// Ends `stream` from the core's side.
    LinkClose {
        stream: u64,
    },
    /// Asks the browser gateway of the desktop window on this node's
    /// machine for a capability in `scope` (the Workspace, area and display
    /// the core decided for its caller). Answered with the gateway's
    /// `cdp_http_url` and `browser_ws_url`, both on this machine's loopback;
    /// with `relay`, with a one-shot `relay_url` on this node's daemon
    /// instead, whose relay carries the CDP to the gateway: a caller on
    /// this machine dials it, and the core reaches it for a caller on
    /// another machine through a [`LinkEnd::BrowserRelay`] stream. A node
    /// with no gateway registered refuses it, its reason in the message.
    BrowserGateway {
        scope: serde_json::Value,
        relay: bool,
    },
    /// Takes the label generator lock of the Herdr server at `herdr_socket`
    /// on this node's machine, or of the server this link's pane service
    /// serves when it names none, for the core's worker `generator`, and
    /// holds it while the link lives. Answered with [`LabelLock`]; asked
    /// again by the same worker while held, it is held, and by another
    /// worker of the same core, it is not.
    LabelLock {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        herdr_socket: Option<String>,
        generator: u64,
    },
    /// Gives back the lock [`Call::LabelLock`] took for `generator` on the
    /// same server; a lock another worker holds stays held.
    LabelUnlock {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        herdr_socket: Option<String>,
        generator: u64,
    },
    /// Stops the reporting call `request`: its next report is answered with
    /// false, and it answers as stopped. Answered at once, before any
    /// waiting request, and a request that is no longer running is left
    /// as it is.
    Cancel {
        request: u64,
    },
    /// View feedback's files on this machine: a screen's staged screenshots
    /// and a letter's placed attachments ([`crate::feedback::FeedbackCall`]).
    Feedback {
        call: crate::feedback::FeedbackCall,
    },
}

impl Call {
    /// Whether this request reports until it is told to stop rather than
    /// finish (the Git watch). A link that drains stops these; any other
    /// call, a clone included, settles to its own result.
    pub fn runs_until_stopped(&self) -> bool {
        matches!(self, Self::GitWatch { .. })
    }

    /// Whether this request is link control rather than machine work: a
    /// greeting, a pane's proof answer or stream, or a Herdr stream. Each is
    /// short on the node, so both ends run them in a lane of their own, and
    /// four slow machine calls (a port scan, a disk walk) never hold a pane's
    /// reply or a Herdr write behind them. One may still wait: a link write
    /// to an end that stops reading holds its worker until the write
    /// deadline (`hide_host::link_bridge`, 5 s) and then ends that stream,
    /// so four such streams can hold the lane for one such window.
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Hello
                | Self::PaneProofAnswer { .. }
                | Self::StreamWrite { .. }
                | Self::StreamClose { .. }
                | Self::LinkOpen { .. }
                | Self::LinkWrite { .. }
                | Self::LinkClose { .. }
        )
    }

    /// Whether a device's node answers this request. A device answers the
    /// work on its own files, repositories, sessions, processes, kit and
    /// panes; what acts with the operator's own logins (GitHub, the AI
    /// providers and their usage), the files the operator picked on this
    /// machine, are the core's own node's, and a device refuses them unrun.
    /// The match names every request, so
    /// a new one is decided here before it compiles.
    pub fn answered_by_device(&self) -> bool {
        match self {
            Self::Hello
            | Self::RootOpen { .. }
            | Self::List { .. }
            | Self::Stamps { .. }
            | Self::Bytes { .. }
            | Self::Index { .. }
            | Self::OpenDocument { .. }
            | Self::Revision { .. }
            | Self::Save { .. }
            | Self::Create { .. }
            | Self::Rename { .. }
            | Self::Move { .. }
            | Self::Copy { .. }
            | Self::Import { .. }
            | Self::Trash { .. }
            | Self::Changes { .. }
            | Self::Project { .. }
            | Self::Worktrees { .. }
            | Self::BranchCheck { .. }
            | Self::Directory { .. }
            | Self::Registrable { .. }
            | Self::HomeSync { .. }
            | Self::WorktreeRetire { .. }
            | Self::WorktreeRemove { .. }
            | Self::WorktreeRemovalCheck { .. }
            | Self::Kit { .. }
            | Self::HookDiagnosis
            | Self::GuardrailPolicy { .. }
            | Self::WorktreesRegistered { .. }
            | Self::IgnoredRepository { .. }
            | Self::ProjectCreate { .. }
            | Self::PathFacts { .. }
            | Self::RealPaths { .. }
            | Self::Repository { .. }
            | Self::JudgeFolders { .. }
            | Self::SetAsideFolder { .. }
            | Self::WorktreeRemoveClean { .. }
            | Self::DrainTrash { .. }
            | Self::RepositoryClone { .. }
            | Self::Git { .. }
            | Self::TerminateGroup { .. }
            | Self::AgentInstalled { .. }
            | Self::AntigravityForkMark { .. }
            | Self::AntigravityFork { .. }
            | Self::OpenCodeFork { .. }
            | Self::AgentTrust { .. }
            | Self::ShellIntegration
            | Self::ProcessStarts { .. }
            | Self::ProcessDescendants { .. }
            | Self::LineInput { .. }
            | Self::DiskUsage { .. }
            | Self::ListeningPorts
            | Self::VolumeFree { .. }
            | Self::LabelTranscript { .. }
            | Self::SessionWorkTurn { .. }
            | Self::SessionActivity { .. }
            | Self::LinkFiles { .. }
            | Self::LinkRead { .. }
            | Self::UsageFiles { .. }
            | Self::UsageRead { .. }
            | Self::ProjectSessions { .. }
            | Self::GitWatch { .. }
            | Self::SessionStat { .. }
            | Self::SessionText { .. }
            | Self::SessionConversation { .. }
            | Self::PanesStart { .. }
            | Self::TerminalsStart { .. }
            | Self::PaneProofAnswer { .. }
            | Self::PaneInspect { .. }
            | Self::StreamWrite { .. }
            | Self::StreamClose { .. }
            | Self::LinkOpen { .. }
            | Self::LinkWrite { .. }
            | Self::LinkClose { .. }
            | Self::BrowserGateway { .. }
            | Self::LabelLock { .. }
            | Self::LabelUnlock { .. }
            | Self::Feedback { .. }
            | Self::Cancel { .. } => true,
            Self::AiAvailability { .. }
            | Self::AiModels { .. }
            | Self::AiExecute { .. }
            | Self::AiMeasurement { .. }
            | Self::AiRestart { .. }
            | Self::AiRelease { .. }
            | Self::UsageLimitFetch { .. }
            | Self::UsagePriceFetch
            | Self::Gh { .. }
            | Self::ReadAttachments { .. }
            | Self::ReadDropped { .. }
            | Self::RemoveClipboard { .. } => false,
        }
    }
}

/// What a `kit` request does. `apply` and `reinstall` answer a
/// `hide_kit::KitReport`, `status` answers one without changing anything, and
/// `remove` answers a [`KitRemoved`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KitAction {
    /// The connection pass: install what was never installed and replace
    /// what is outdated.
    Apply,
    /// The operator's choice on the machine's row: Reinstall of these
    /// parts, or an agent switched on (`agents_on`, which is also Reinstall
    /// of an agent that is on) or off (`agents_off`).
    Reinstall {
        components: Vec<hide_kit::ComponentId>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agents_on: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agents_off: Vec<String>,
        /// Turn Codex's shared server off on this device (the operator's own
        /// request, never part of a pass).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        codex_daemon_off: bool,
    },
    Status,
    /// The device is being removed from Hide: Hide's parts come off, then
    /// the helper root.
    Remove,
}

/// What removing the kit from a device did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KitRemoved {
    pub kit: hide_kit::RemoveReport,
    /// The helper root and every build under it.
    pub helper_root: hide_kit::RemoveOutcome,
}

/// A report a running call sends before its answer, as its own line
/// (`{"progress": id, "report": ...}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub progress: u64,
    pub report: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Value),
    Error(HostError),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub version: String,
    pub os: String,
    pub arch: String,
    /// The home directory of the account the helper runs as, the boundary
    /// the device's own registration listing applies (PRD S5.5 B24).
    pub home: Option<String>,
    /// Read once when this connection starts; a failure leaves lineage
    /// unresolved without making the file helper unavailable.
    pub machine_identity: MachineIdentity,
    /// Absent on protocol24. Missing or invalid facts on the current
    /// protocol grant no reader; files, Git and unrelated node operations
    /// remain available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_features: Option<crate::sessions::ReaderFeatures>,
}

impl Hello {
    pub fn readers(&self) -> crate::sessions::ReaderFeatures {
        match self.protocol {
            24 => crate::sessions::ReaderFeatures::protocol24(),
            PROTOCOL_VERSION => self.reader_features.clone().unwrap_or_else(|| {
                crate::sessions::ReaderFeatures::unavailable("reader_advertisement_missing")
            }),
            _ => crate::sessions::ReaderFeatures::unavailable("reader_protocol_unsupported"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MachineIdentity {
    Available { id: String },
    Unavailable { reason: String },
}

impl MachineIdentity {
    pub fn into_result(self) -> Result<String, String> {
        match self {
            Self::Available { id }
                if !id.is_empty()
                    && id.len() <= 256
                    && id == id.trim()
                    && !id.bytes().any(|byte| byte.is_ascii_control()) =>
            {
                Ok(id)
            }
            Self::Available { .. } => {
                Err("The device helper reported an invalid machine identity".to_owned())
            }
            Self::Unavailable { reason } => Err(reason),
        }
    }
}

/// What [`Call::LabelLock`] answers: whether this core now generates labels
/// for the server, and the process that holds its lock when the system lets
/// it be read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LabelLock {
    pub held: bool,
    #[serde(default)]
    pub holder: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RootOpened {
    pub identity: RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionNow {
    pub revision: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An older caller's `reinstall` has no `codex_daemon_off` and means
    /// "leave the shared server alone"; the request is on the wire only when
    /// it is made.
    #[test]
    fn a_reinstall_asks_to_turn_the_codex_server_off_only_when_it_says_so() {
        let plain = serde_json::json!({"kind": "reinstall", "components": ["cli"]});
        let KitAction::Reinstall {
            codex_daemon_off, ..
        } = serde_json::from_value(plain).unwrap()
        else {
            panic!("a reinstall");
        };
        assert!(!codex_daemon_off);
        let asked = KitAction::Reinstall {
            components: Vec::new(),
            agents_on: Vec::new(),
            agents_off: Vec::new(),
            codex_daemon_off: true,
        };
        assert_eq!(
            serde_json::to_value(&asked).unwrap(),
            serde_json::json!({"kind": "reinstall", "components": [], "codex_daemon_off": true})
        );
        let unasked = KitAction::Reinstall {
            components: Vec::new(),
            agents_on: Vec::new(),
            agents_off: Vec::new(),
            codex_daemon_off: false,
        };
        assert_eq!(
            serde_json::to_value(&unasked).unwrap(),
            serde_json::json!({"kind": "reinstall", "components": []})
        );
    }

    #[test]
    fn machine_identity_reports_unavailability_and_refuses_invalid_ids() {
        let decode = |value| serde_json::from_value::<MachineIdentity>(value).unwrap();
        assert_eq!(
            decode(serde_json::json!({"state":"available", "id":"machine-device"})).into_result(),
            Ok("machine-device".to_owned())
        );
        assert_eq!(
            decode(
                serde_json::json!({"state":"unavailable", "reason":"native identity unavailable"})
            )
            .into_result(),
            Err("native identity unavailable".to_owned())
        );
        let longest_id = "x".repeat(256);
        assert_eq!(
            MachineIdentity::Available {
                id: longest_id.clone()
            }
            .into_result(),
            Ok(longest_id)
        );
        for id in [
            String::new(),
            "device\n".to_owned(),
            "device\0suffix".to_owned(),
            " device".to_owned(),
            "x".repeat(257),
        ] {
            assert!(MachineIdentity::Available { id }.into_result().is_err());
        }
        assert!(
            serde_json::from_value::<MachineIdentity>(serde_json::json!({"state":"available"}))
                .is_err()
        );
    }
}
