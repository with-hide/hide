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
/// 51: kit calls distinguish attaching to a saved Herdr session from server
/// replacement consent, and pane proofs carry passive hook observations.
/// Older helpers could replace the selected server or reject its observation.
/// 52: the core fetches a node's repositories in the background
/// (`Call::FetchRemote`), reads a header's incoming and unpushed commits, and
/// names GitHub's default branch to the worktree read; the core also keeps a
/// `Call::GitWatch` open on each connected device, and both run on a lane of
/// their own (`Call::is_background`). An older helper cannot fetch without
/// prompting and is upgraded at Hello.
/// 53: a worktree removal answers why it stopped as a code
/// (`worktrees::RemovalStop`), `Call::WorktreeRemovalCheck` answers a stop
/// as its answer rather than an error, and a diff names why it is short or
/// absent as a code (`git::DiffNotice`). An older core would read the new
/// check's stop as a pass, so the helper is upgraded at Hello.
/// 54: a Hide AI suggestion investigation reads a repository's history and
/// searches its tracked files (`git::GitCommand::Log`, `GitCommand::Grep`)
/// and the end of a CI job's failed log (`gh run view --job <id>
/// --log-failed`, answered cut to its end) on the node that holds the
/// Project (PRD hide-ai-suggestions B7). A helper on 53 would refuse them
/// as unknown, so it is refused at Hello and upgraded.
/// 55: every device link carries what used to need the SSH connection
/// beside it (PRD wsl-device D-14, B9, B10): a stream to a page server on the
/// node's own loopback ([`LinkEnd::Loopback`]) and a terminal attachment
/// staged on the node's machine ([`Call::StageAttachment`],
/// [`Call::RemoveAttachments`]); and a device answers the Hide AI calls, so
/// an agent signed in only there runs Hide AI with its own login (D-03,
/// D-06). A helper on 54 would refuse all three, so it is refused at Hello
/// and upgraded.
/// 56: `Call::Registrable` carries the operator's confirmation of a folder
/// outside the host's home (`confirmed_outside_home`), and the helper answers
/// `needs_confirmation` for such a folder without one (PRD wsl-device D-02,
/// D-10). A helper on 55 would drop the confirmation and refuse every folder
/// outside its home, so it is refused at Hello and upgraded.
/// 57: a node started with `node serve --own-herdr` bridges its own Herdr
/// over its link and names that server's socket in its Hello
/// (`Hello::herdr_socket`), which is how the core reaches a WSL
/// distribution's Herdr (PRD wsl-device D-07). A helper on 56 has no such
/// flag, so it is refused at Hello and upgraded.
/// 58: a node takes `hide <path>` from a plain shell of its own account on
/// its bridge socket and sends it up as `NodeEvent::ProjectOpen`, answered
/// by `ProofAnswer::ProjectOpenAccepted` and `ProofAnswer::ProjectOpen`. A
/// helper on 57 would end on the new answers, so it is refused at Hello and
/// upgraded.
/// 59: a node reads a branch against its base (`Call::BranchFacts`) and merges
/// it into the base on the machine that owns the checkout, never pushing and
/// changing nothing it cannot merge cleanly (`Call::BranchMerge`,
/// [`crate::branch::MergeOutcome`]), reads one file's committed diff against
/// the merge base (`Call::BranchDiff`), and deletes a branch ref that no
/// worktree has checked out (`Call::BranchDiscard`; PRD
/// hide-ai-branch-first-apply). A helper on 58 would refuse all four, so it is
/// refused at Hello and upgraded.
/// 60: a device's test asks its node which Git it runs
/// (`GitCommand::Version`) over the link it has, where it used to report Git
/// without asking (PRD system-ssh-device-link B25). A helper on 59 would
/// refuse the command, so it is refused at Hello and upgraded.
/// 61: a node moves a checkout's branch to its upstream as last fetched,
/// fast-forward only and never fetching (`Call::BranchCatchUp`,
/// [`crate::branch::CatchUpOutcome`]). A helper on 60 would refuse it, so it
/// is refused at Hello and upgraded.
/// 62: a node merges a Hide AI branch into its base and pushes the base to
/// its upstream, putting the local base back when the remote does not take it
/// (`Call::BranchMergePush`, [`crate::branch::MergePushOutcome`]). A helper on
/// 61 would refuse it, so it is refused at Hello and upgraded.
/// 63: a merge answers where git failed while it wrote the merge
/// (`MergeOutcome::Failed`, [`crate::branch::MergeStep`]) instead of an
/// error whose words name paths, a merge and push refused for a base apart
/// from its upstream counts the base's own commits, and a node moves a base
/// to its upstream, keeping the base's own commits on a new branch
/// (`Call::BaseSync`, [`crate::branch::BaseSyncOutcome`]). A helper on 62
/// would refuse it, so it is refused at Hello and upgraded.
/// 64: the open pull requests' facts query also reads each reviewer's latest
/// review, whom each review request named and the base branch's classic
/// approval count, and a node reads the approvals a branch's rulesets ask
/// for (`gh::required_approvals_arguments`; PRD pr-review-status). A helper on
/// 63 would refuse both reads, so it is refused at Hello and upgraded.
/// 65: a device pane's hook reports its agent's turn and subagent counts to
/// the core through its node (`panes::HookCommand::Status`) instead of to
/// the device's Herdr. A helper on 64 would refuse the command, so it is
/// refused at Hello and upgraded.
/// 66: a node reads the thread of the session an agent pane runs, a page of
/// ACP session updates, newest, before a page or forward from one
/// (`Call::SessionThread`; PRD session-docs-and-assistant B25). A helper on
/// 65 would refuse it, so it is refused at Hello and upgraded.
/// 67: a node adds a worktree for the core's own engine
/// (`git::GitCommand::WorktreeAdd`; PRD engine-bundle-3-core-owns D-28). A
/// helper on 66 would refuse it, so it is refused at Hello and upgraded.
/// 68: a node hands input to a running agent through the agent's own way
/// in (`Call::Deliver`). A helper on 67 would refuse it, so it is refused
/// at Hello and upgraded.
/// 69: a node runs its machine's panes in pane holders for the core's own
/// engine, with no Herdr (`Call::HoldersStart`), and answers that engine's
/// starts, ends, asks and input (`Call::Pane`) and follows its panes
/// (`Call::PaneWatch`) on a lane of their own (PRD engine-bundle-3-core-owns
/// D-22). A helper on 68 would refuse all three, so it is refused at Hello
/// and upgraded.
/// 70: a node answers `Call::AgentInstalled` with how the agent's program
/// starts (a [`StartForm`], null when it is not on the `PATH`), not a bool,
/// so a prompt `cmd.exe` would read again rides no start. A helper on 69
/// would answer a bool, so it is refused at Hello and upgraded.
/// 71: a node answers where a holder pane's screen is in its scrollback,
/// moves it, and reads the rows it shows (`holders::PaneCall::Scroll`,
/// `holders::PaneCall::ReadVisible`), so the core's own engine routes a pane
/// search and shows its matches. A helper on 70 would refuse both asks, so it is
/// refused at Hello and upgraded.
/// 72: pane mutations require connection ownership and a durable reserved
/// existence record. An older node cannot fence late requests on reconnect.
/// 73: removes the running-agent queue call. Running input is typed through
/// the engine so a letter reaches an ongoing turn; older helpers upgrade.
/// 74: adds `HerdrSave` and `HerdrStop`, so the core moves a device's Herdr
/// session into its own engine (PRD engine-bundle-4-herdr-removal D-10); an
/// older helper would refuse both, so it upgrades.
/// 75: a letter's text is a list of parts (`holders::PaneCall::Letter`), so
/// the core types Herdr's focus report before a Copilot prompt and its
/// paste boundary after a Windows Codex one, inside the submission. An
/// older helper would refuse the letter, so it upgrades.
/// 76: drops what started or fed a node's Herdr services (`PanesStart`,
/// `TerminalsStart`, the kit call's Herdr socket and saved-session flag) and
/// the pane proofs' passive hook observations, since every node runs its
/// panes in holders (PRD engine-bundle-4-herdr-removal D-10), and the Herdr
/// link stream, the Hello's Herdr socket and the label lock's Herdr socket
/// (the lock now names the engine folder, `scope`); a helper on 75 still
/// takes them, so it upgrades.
/// 77: a Windows node answers `Call::AgentInstalled` with the file a start
/// runs and the words before its arguments (`StartForm::Direct`), npm's
/// launcher read as the program it runs, so a first input rides the start
/// there too; a helper on 76 would answer `program` or `batch` for it, so
/// it upgrades.
/// 78: a pointer event reaches a pane's holder as what it means
/// (`TerminalControl::Mouse`), which the holder encodes for the program's
/// mouse mode, in place of the core's own bytes (`Write`); a helper on 77
/// would refuse it, so it upgrades.
/// 79: a device's terminals send what a pane's program wrote to the
/// clipboard (`TerminalUp::Clipboard`), once, for the core's window in
/// front; a core on 78 cannot read the line and would drop every copy, so
/// the helper upgrades.
/// 80: `home_list` lists one folder under the node's home for Add a
/// project's folder list and `new_folder_target` says where a clone or a new
/// project would land under that home, so both run on any device (PRD
/// device-add-project D-01, D-07); a helper on 79 would refuse both, so it
/// upgrades.
/// 82: `importable_projects` answers the projects worth adding on the node,
/// from its agents' sessions and Paseo's and Herdr's lists, for Add a
/// project on any device (PRD device-add-project D-09); a helper on 81
/// would refuse it, so it upgrades (81 is taken by an open change).
/// 83 (81 was held for this change while 82 landed first): a node renews a
/// pane's holder from this build's version folder
/// (`holders::PaneCall::Renew`, with-hide/hide-private#646), which the core
/// asks before it wakes an agent in the pane, and takes only the sleep's own
/// input (`holders::PaneCall::SleeperInput`) into a pane whose agent is
/// asleep; a helper on an earlier protocol would refuse both calls, so it
/// upgrades.
/// 84: `owed_confirmations` hands the core the letter confirmations a pane's
/// hooks could not send in time (`hide_agent_hooks::owed::take`), which the
/// core asks once the pane rests (`docs/delivery.md`, A confirmation that
/// missed its hook); a helper on 83 would refuse it, so it upgrades.
/// 85: the home folder is a project (PRD home-folder-project). `Registrable`
/// asks about it instead of refusing it (`HostError::home`), `Index` carries
/// the other projects of the node (`others`) so the home project's walk
/// skips them, and Home never links it; a helper on 84 would refuse the
/// home folder as a project, so it upgrades.
/// 86: a node keeps its machine's agent profiles (`ProfileList`,
/// `ProfileApply`, `ProfileInstall`, PRD agent-profiles D-04) and its kit
/// installs into each profile's account; a helper on 85 would refuse the
/// calls, so it upgrades, and a node the core still admits on an older
/// protocol answers them unrun, which the core reads as no profiles there.
/// 87: `home_retire` replaces `home_sync` (PRD retire-hide-home): the core no
/// longer keeps a Home folder, and asks each helper once to remove what
/// Hide's marker lists in `~/hide`; a helper on 86 would refuse the call, so
/// it upgrades first and keeps its `~/hide` until then.
/// 88: a node answers how a start of one of its profiles runs there
/// (`ProfileLaunch`, PRD agent-profiles D-26): the profile's variables and
/// program, checked on that machine, and the `hide` beside the node that
/// starts it (`hide agent exec`); a helper on 87 would refuse the call, so
/// it upgrades, and a node the core still admits on an older protocol
/// answers it unrun, which the core reads as a machine that starts only each
/// agent's built-in default profile. The same step lets the core read one
/// process of a pane fresh (`holders::PaneCall::Process`) and carries the
/// process that sent a start's report (`panes::NodeEvent::PaneProof`'s
/// `executor`), by which the core decides what came of that start.
/// 89: a node answers which account it registered holds a session a resume
/// names by its file or id alone (`SessionAccount`, PRD agent-profiles
/// B19), so a session line resumes with the profile of its own account; a
/// helper on 88 keeps profiles but would refuse the call, so it upgrades,
/// and a node the core still admits on an older protocol keeps no profile,
/// which the core reads as the default account.
/// 90: a profile's start line names only its attempt, in a shorter shape,
/// and the agent's own words, and its executor asks for the profile's
/// variables and program just before it runs
/// (`panes::AgentExecEvent::Profile`), answered at the version its start
/// was checked with. An executor of 88 or 89 would read the new line as a
/// start with no variables, and refuses its attempt's shape instead; a
/// helper on 88 or 89 is not admitted, so it upgrades before the core types
/// a profile's start there.
/// 91: a hook's session-start report carries the account variables its
/// agent runs with (`panes::HookStatusReport::account`, PRD agent-profiles
/// D-03, D-35), which a node passes on to the core inside the hook request
/// the report is; a core on 90 refuses a report with a field it does not
/// know, so a helper upgrades with its core. The same step lists each
/// profile's sign-in and account variables (`profiles::ListedProfile`) and
/// lets a usage read name a profile (`Call::UsageLimitFetch`), both optional
/// fields an older side leaves out.
/// 92: each listed profile carries the account its node's `resolve` keyed
/// it by (`profiles::ListedProfile::account_key`, PRD agent-profiles D-12),
/// which the core compares accounts by instead of keying one itself; a
/// helper on 91 lists none, so it is not admitted and upgrades with its
/// core.
/// 93: holder renewal carries
/// the connection's numbered creation request,
/// just like reserve/start/cancel. An earlier node issues a separate number
/// during replacement and could retire the next tab's request, so it must
/// upgrade before any renewal.
/// 94: a Hide AI backend names the profile it runs as
/// (`ai::BackendSpec::profile`, PRD agent-profiles-phase2 D-09), so each
/// item's model list is read in its own account and with its wrapper; a
/// helper on 93 would ignore the field and answer the agent's default
/// account in its place, so it is not admitted, and the helper this build
/// carries is installed on the next connect.
/// 95: `HerdrStop` is gone. Hide no longer stops an earlier Hide's Herdr on a
/// device; the import only reads its save (`HerdrSave`). A helper on 94
/// would still offer the call, so it is refused at Hello and upgraded.
/// 96: a hook's report of a turn the provider refused carries why
/// (`panes::HookStatusReport::refused`, Claude Code's `StopFailure`), which a
/// core on 95 refuses as a field it does not know, so a helper upgrades with
/// its core.
pub const PROTOCOL_VERSION: u32 = 96;

/// How a start of an agent's CLI carries its arguments on the machine that
/// runs it ([`Call::AgentInstalled`]), as `hide_platform::programs::Start`
/// reads it there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartForm {
    /// The program itself takes them, each whole, started by its name, which
    /// the pane's shell finds as the node did (macOS, Linux).
    Program,
    /// The start names `program`, a file on the node, and puts `leading`
    /// before its arguments, which it takes each whole: on Windows, where
    /// the shell would find another file by the name, and npm's launcher is
    /// read as the program it runs (`hide_platform::programs::START_NAMES_FILE`).
    Direct {
        program: String,
        leading: Vec<String>,
    },
    /// Windows runs the program through `cmd.exe`: a batch file that is not
    /// npm's launcher (`hide_platform::programs::Start::Batch`), so nothing
    /// that `cmd.exe` would read again rides its start.
    Batch,
}

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

/// What a link stream the core opens on a node reaches on the node's
/// machine ([`Call::LinkOpen`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkEnd {
    /// The browser relay of the node's daemon, which carries a caller's CDP
    /// to the gateway of the desktop window whose registration the daemon
    /// holds: a caller on another machine driving a page only that window
    /// shows (B15). The stream's first bytes ask for a relay ticket the
    /// node handed out over this link.
    BrowserRelay,
    /// A server on the node machine's own loopback, which a Browser View of
    /// the core's machine shows (PRD wsl-device B9): every device's link
    /// carries it, however the link opened. The node refuses any address
    /// that is not its loopback.
    Loopback { address: std::net::SocketAddr },
}

impl LinkEnd {
    /// Streams to this end one link carries at once; the next open is
    /// refused.
    pub const fn cap(self) -> usize {
        match self {
            Self::BrowserRelay => crate::panes::MAX_BROWSER_STREAMS,
            Self::Loopback { .. } => crate::panes::MAX_PAGE_STREAMS,
        }
    }

    /// Whether `other` counts against the same cap: every page stream does,
    /// whichever port it reaches.
    pub fn shares_cap(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
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
    /// Read-only status and current commit for the front checkout's header.
    CheckoutGit {
        path: String,
    },
    /// Fetches `origin` of the repository that holds `path` without asking
    /// for anything (`hide_host::worktrees::fetch_remote`), within
    /// `worktrees::FETCH_TIME`; answers `worktrees::FetchOutcome`.
    FetchRemote {
        path: String,
    },
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
    ///
    /// When the root is the node's home folder the walk leaves out its
    /// top-level hidden names and app-data folder and every folder in
    /// `others`, the other projects registered on the node.
    Index {
        root: RootRef,
        #[serde(default)]
        others: Vec<String>,
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
    /// a pane's directory or a registered project's path, which the
    /// daemon's catalog groups into projects with this device's id. Only
    /// facts are answered, never contents.
    Project {
        path: String,
    },
    /// The worktrees of the repository that holds `path`
    /// (`hide_host::worktrees::read`), measured against `bases` or the
    /// operator's `base_override`, with `remote_default` the default branch
    /// GitHub names, preferred over `origin/HEAD`. `null` for a folder that is not a
    /// repository. With `reuse_unchanged`, a worktree whose HEAD, index and
    /// base refs did not move since the node last measured it keeps that
    /// measurement; without it every worktree is measured again.
    Worktrees {
        path: String,
        bases: std::collections::BTreeMap<String, String>,
        base_override: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote_default: Option<String>,
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
    /// host's own home folder (`hide_host::register::check`); a folder
    /// written outside it is judged only when `confirmed_outside_home` names
    /// it, the operator's confirmation of that one path.
    Registrable {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmed_outside_home: Option<String>,
    },
    /// One folder under this host's home as Add a project lists it
    /// (`browse::HomeListing`): `path` is `~`, `~/...` or an absolute path
    /// in either spelling, and has to lead to home or a folder under it
    /// without leaving it through a link. Hidden names are left out, and a
    /// repository carries its branch and a folder its repository count.
    HomeList {
        path: String,
    },
    /// The projects worth adding on this host (`importable::ImportableProjects`):
    /// the folders its agents worked in and the projects Paseo and Herdr keep,
    /// each as the root a registration would add under home, newest within
    /// thirty days first, at most twenty, leaving out every root `exclude`
    /// names (the core's registrations here, at most
    /// `importable::EXCLUDE_CAP`). It only reads.
    ImportableProjects {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exclude: Vec<String>,
    },
    /// Where a clone (`kind` `clone`) or a new project (`project`) named
    /// `name` would land in `parent`, home or a folder under it, judged
    /// against this host's home (`browse::NewFolderTarget`); nothing is
    /// made.
    NewFolderTarget {
        parent: String,
        name: String,
        kind: crate::browse::NewFolderKind,
    },
    /// Retires this host's old Hide Home folder (`~/hide`) once: removes the
    /// entries Hide's marker lists as its own and, unless something else is
    /// there or `keep_empty`, the folder (`hide_host::home::retire`).
    /// Repeating it changes nothing more.
    HomeRetire {
        keep_empty: bool,
        /// The second call for a folder that stays: the first leaves the
        /// marker until the caller has saved the answer, this one removes it.
        #[serde(default)]
        settle: bool,
    },
    /// A branch measured against `base` in the repository at `root`
    /// (`hide_host::branch_merge::facts`): how far ahead, whether it is in
    /// the base, and the committed diff from their merge base. `tip` is the
    /// commit the branch last pointed at, measured instead when the branch
    /// is gone. Reads only.
    BranchFacts {
        root: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<String>,
        base: String,
    },
    /// Merges `branch` into `base` in the repository at `root`
    /// (`hide_host::branch_merge::merge`), locally: no push, no fetch. The
    /// answer is a [`crate::branch::MergeOutcome`] and only `Merged` changes
    /// anything; a retry after a merge answers `already_merged`.
    BranchMerge {
        root: String,
        branch: String,
        /// The commit the operator read; a branch that moved since is not
        /// merged (`MergeOutcome::BranchMoved`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<String>,
        base: String,
        message: String,
    },
    /// Merges `branch` into `base` as [`Call::BranchMerge`] does and pushes
    /// the base to its upstream (`hide_host::branch_merge::merge_push`): the
    /// upstream is fetched first and the local base caught up to it, fast
    /// forward only, and a push the remote does not take puts the local base
    /// back. The answer is a [`crate::branch::MergePushOutcome`].
    BranchMergePush {
        root: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<String>,
        base: String,
        message: String,
    },
    /// Moves `base` to its upstream as just fetched, after the commits only
    /// the local base has are put on a new branch named `branch` (or
    /// `branch-2` and on while that is taken), and only in a worktree with
    /// nothing uncommitted (`hide_host::branch_merge::sync_base`). The answer
    /// is a [`crate::branch::BaseSyncOutcome`]; nothing is deleted.
    BaseSync {
        root: String,
        base: String,
        branch: String,
    },
    /// The committed diff of one repository-relative `path` from the merge
    /// base of `base` and `branch` to the branch's tip
    /// (`hide_host::branch_merge::diff`), answered as a
    /// [`crate::git::Diff`] bounded like a Changes diff. Reads only.
    BranchDiff {
        root: String,
        branch: String,
        base: String,
        path: String,
    },
    /// Deletes `refs/heads/<branch>` in the repository at `root`
    /// (`hide_host::branch_merge::discard`) and nothing else: no worktree, no
    /// remote. With `tip`, only while the branch still points at it. The
    /// answer is a [`crate::branch::DiscardOutcome`].
    BranchDiscard {
        root: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<String>,
    },
    /// Moves the branch checked out in the worktree at `root` to its
    /// upstream as last fetched (`hide_host::branch_merge::catch_up`):
    /// fast-forward only, no fetch, no push, and only while `HEAD` is still
    /// `head`, the commit the operator was shown. The answer is a
    /// [`crate::branch::CatchUpOutcome`] and only `Updated` changes anything.
    BranchCatchUp {
        root: String,
        head: String,
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
    /// Non-mutating authoritative check before any pane is closed; answers
    /// `null` when the removal may go on, or the
    /// [`crate::worktrees::Stopped`] that stops it.
    WorktreeRemovalCheck {
        removal: crate::worktrees::ConfirmedRemoval,
    },
    /// The device's install kit (`hide_host::kit`): the helper acts on the
    /// helper root it runs from, never on a folder the request names.
    /// `cli_dir` is where the consent allows the `hide` link; it may start
    /// with `~/`.
    Kit {
        action: KitAction,
        cli_dir: String,
        /// Known registered checkout roots on this helper's device only.
        #[serde(default)]
        retirement_projects: Vec<String>,
    },
    /// The machine's agent profiles (`crate::profiles::ProfileList`), each
    /// with its pieces as the kit judges them; `values` adds each variable's
    /// value, for the screen that edits a profile.
    ProfileList {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        values: bool,
    },
    /// Changes the machine's profiles and installs or takes out the pieces
    /// of the accounts it touches (`crate::profiles::ProfileApplied`).
    /// `expected_version` is the list's version the caller changes; a list
    /// that has moved since is refused as a conflict.
    ProfileApply {
        /// Named `change` because a call's own tag is `op`.
        change: crate::profiles::ProfileOp,
        expected_version: u64,
    },
    /// Reinstall of one profile's pieces, even those the operator took out;
    /// answers the list. Named `profile` because a request's own `id` sits
    /// beside a call's fields on the wire.
    ProfileInstall {
        profile: hide_agent_adapter::ProfileId,
    },
    /// How a start of the agent `kind` with the profile `profile` runs on
    /// this machine (`crate::profiles::ProfileLaunch`): ready, gone, or
    /// refused when the profile is being removed or another agent's, when its
    /// account or variables no longer resolve here, or when its program is
    /// not found. The built-in default profile is never asked: it starts as
    /// the agent always has.
    ProfileLaunch {
        kind: String,
        profile: hide_agent_adapter::ProfileId,
    },
    /// Whether the node's account has Hide's agent hooks in place, read from
    /// that account's own configuration (`hide_agent_hooks::Diagnosis`).
    HookDiagnosis,
    /// Hands over and forgets the letter confirmations the hooks of the
    /// pane `pane` (the engine's id on this machine) could not send in time
    /// (`hide_agent_hooks::owed::take`): `null`, or the session the letters
    /// reached and their ids (`hide_agent_hooks::owed::Owed`).
    OwedConfirmations {
        pane: String,
    },
    /// The node account's Herdr save and whether a server runs on its default
    /// socket (`crate::herdr_save::Found`), only read.
    HerdrSave,
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
    /// `profile` reads the account of one of the node's profiles instead of
    /// the agent's default one (PRD agent-profiles B23), as the node resolves
    /// it from its own profile file, so the core never names an account's
    /// folder. Devices refuse it; credentials never cross the link
    /// (`usage::LimitFetch`).
    UsageLimitFetch {
        source: crate::usage::LimitSource,
        root: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<hide_agent_adapter::ProfileId>,
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
    /// Stages the `index`th of the `count` files of terminal attachment
    /// `request` on this node's machine, for a pane there, and answers its
    /// path (PRD wsl-device B10). One file per call keeps a request line
    /// under the link's line cap. A file already staged at that place with
    /// the same bytes is kept, so a retry writes nothing twice; other bytes
    /// there are refused. A request whose files did not all arrive when the
    /// link ends is removed with it.
    StageAttachment {
        request: String,
        index: usize,
        count: usize,
        file: crate::feedback::File,
    },
    /// Removes every file attachment `request` staged; one already gone is
    /// not an error.
    RemoveAttachments {
        request: String,
    },
    /// Ends the job `leader` leads, a pane's foreground job, for agent sleep
    /// (`hide_platform::process::terminate_group`): `SIGTERM` to its process
    /// group on Unix, and on Windows, where the job is named by its first
    /// process, that process and everything under it. A group of 1 or less
    /// is refused unsent, since kill(-0) and kill(-1) reach far more. The
    /// answer is a [`crate::process::GroupEnd`]: ended, with what was left
    /// running, or gone already.
    TerminateGroup {
        leader: u32,
    },
    /// How a start of the agent CLI named `name` on the node carries its
    /// arguments, a [`StartForm`], or null when it is not on the node's
    /// `PATH`: asked before a pane is asked to start it, and before a prompt
    /// rides that start (`hide-core` `start_prompt`).
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
    /// `profile` is the profile the fork's agent starts with, as its
    /// machine answered it ([`Call::ProfileLaunch`]): OpenCode runs with its
    /// variables and program, as the agent will (PRD agent-profiles D-26).
    #[serde(rename = "opencode_fork")]
    OpenCodeFork {
        source: String,
        cwd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<crate::profiles::ProfileStart>,
    },
    /// Records (`trusted`) or takes out an agent's trust of a worktree Hide
    /// made, in the agent's settings of the account the start runs in
    /// (`hide_agent_hooks::folder_trust`): the account of `profile` when the
    /// start names one, the node account's default one otherwise. `agent` is
    /// the start's kind, as the adapter names it.
    AgentTrust {
        agent: String,
        path: String,
        trusted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<hide_agent_adapter::ProfileId>,
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
        /// Boxed: the request is the largest a node takes, and the call
        /// enum is as large as its largest variant.
        request: Box<hide_session::label_transcript::LabelTranscriptRequest>,
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
    /// The account the node registered that holds the session a resume
    /// names by its file or id alone (`hide_session::session_account::read`).
    SessionAccount {
        request: hide_session::session_account::SessionAccountRequest,
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
    /// One page of the thread of the session an agent pane runs, named as
    /// the engine reports the pane's session (`hide_session::thread::read_pane_thread`):
    /// ACP session updates and the page's cursors, never a path.
    SessionThread {
        request: hide_session::thread::ThreadRequest,
    },
    /// A bounded provider-neutral conversation page, including native graphs.
    SessionConversation {
        agent: hide_session::Agent,
        path: String,
        scope: hide_session::SessionReadScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<hide_session::ConversationCheckpoint>,
    },
    /// The core's answer to the pane proof `request` the node sent up.
    PaneProofAnswer {
        request: u64,
        answer: crate::panes::ProofAnswer,
    },
    /// `pane_id`'s identity now, read from this node's pane holder and kernel, so
    /// the core can tell a credential's pane from a replaced one.
    PaneInspect {
        pane_id: String,
    },
    /// Starts this node's pane holders and the terminal service that shows
    /// their screens, over the node's own state folder, so the core's own
    /// engine runs the machine's panes (PRD
    /// engine-bundle-3-core-owns D-22): from then on the link's terminal
    /// lines reach that service, [`Call::Pane`] its holders, and a pane's
    /// `hide` is proved by the pane's holder record on the node's pane
    /// service, whose bootstrap socket it answers with
    /// ([`crate::panes::PanesStarted`]). Asking again while they run changes
    /// nothing. Only a device's link carries it:
    /// the core's own node is given its holders when it is made.
    HoldersStart,
    /// One request of the core's engine to this node's pane holders (start,
    /// end, ask, input), answered with a [`crate::holders::PaneAnswer`].
    /// The core's own node and a device's answer it the same way, so the
    /// engine of any machine reaches its panes by the machine's node alone.
    Pane {
        call: crate::holders::PaneCall,
    },
    /// Follows this node's panes, reporting a
    /// [`crate::holders::PaneWatchReport`] for each pane's facts and each
    /// pane whose program exited, and at least every
    /// [`crate::holders::PANE_WATCH_QUIET`], until the caller answers a
    /// report with false; the call then answers nothing. A newer watch
    /// takes the place of an older one.
    PaneWatch,
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
    /// Takes the label generator lock of the engine whose folder is
    /// `scope` on this node's machine, for the core's worker `generator`,
    /// and holds it while the link lives. Answered with [`LabelLock`]; asked
    /// again by the same worker while held, it is held, and by another
    /// worker of the same core, it is not. Only the core's own node answers
    /// it; a device's link refuses it, and its core labels that device's
    /// agents alone.
    LabelLock {
        scope: String,
        generator: u64,
    },
    /// Gives back the lock [`Call::LabelLock`] took for `generator` on the
    /// same engine; a lock another worker holds stays held.
    LabelUnlock {
        scope: String,
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
        matches!(self, Self::GitWatch { .. } | Self::PaneWatch)
    }

    /// Whether this request is the core's engine asking the node's pane
    /// holders: a start waits up to its holder's ten seconds and a restore
    /// asks eight at once, so both ends run them on a lane of their own,
    /// [`crate::holders::PANE_LANE`] wide, and they never hold a file read
    /// or a Git call behind them, nor wait behind one.
    pub fn is_pane(&self) -> bool {
        matches!(
            self,
            Self::Pane { .. } | Self::PaneWatch | Self::HoldersStart
        )
    }

    /// Whether this request is link control rather than machine work: a
    /// greeting, a pane's proof answer or stream, or a link stream. Each is
    /// short on the node, so both ends run them in a lane of their own, and
    /// four slow machine calls (a port scan, a disk walk) never hold a pane's
    /// reply or a stream write behind them. One may still wait: a link write
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

    /// Background Git work a core keeps out on a link for long: a device's
    /// Git watch and a background fetch. It runs on a lane of its own at
    /// both ends, so it never takes a worker an ordinary call needs.
    pub fn is_background(&self) -> bool {
        matches!(self, Self::GitWatch { .. } | Self::FetchRemote { .. })
    }

    /// Whether a device's node answers this request. A device answers the
    /// work on its own files, repositories, sessions, processes, kit, panes
    /// and the Hide AI agents signed in there; GitHub and the usage reads,
    /// which act with the core machine's logins, and the files the operator
    /// picked on the core's machine are the core's own node's, and a device
    /// refuses them unrun.
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
            | Self::CheckoutGit { .. }
            | Self::FetchRemote { .. }
            | Self::BranchCheck { .. }
            | Self::Directory { .. }
            | Self::Registrable { .. }
            | Self::HomeRetire { .. }
            | Self::HomeList { .. }
            | Self::ImportableProjects { .. }
            | Self::NewFolderTarget { .. }
            | Self::BranchFacts { .. }
            | Self::BranchMerge { .. }
            | Self::BranchMergePush { .. }
            | Self::BaseSync { .. }
            | Self::BranchDiscard { .. }
            | Self::BranchCatchUp { .. }
            | Self::BranchDiff { .. }
            | Self::WorktreeRetire { .. }
            | Self::WorktreeRemove { .. }
            | Self::WorktreeRemovalCheck { .. }
            | Self::Kit { .. }
            | Self::ProfileList { .. }
            | Self::ProfileApply { .. }
            | Self::ProfileInstall { .. }
            | Self::ProfileLaunch { .. }
            | Self::HookDiagnosis
            | Self::OwedConfirmations { .. }
            | Self::HerdrSave
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
            | Self::SessionAccount { .. }
            | Self::LinkFiles { .. }
            | Self::LinkRead { .. }
            | Self::UsageFiles { .. }
            | Self::UsageRead { .. }
            | Self::ProjectSessions { .. }
            | Self::GitWatch { .. }
            | Self::SessionStat { .. }
            | Self::SessionText { .. }
            | Self::SessionConversation { .. }
            | Self::SessionThread { .. }
            | Self::PaneProofAnswer { .. }
            | Self::PaneInspect { .. }
            | Self::HoldersStart
            | Self::Pane { .. }
            | Self::PaneWatch
            | Self::StreamWrite { .. }
            | Self::StreamClose { .. }
            | Self::LinkOpen { .. }
            | Self::LinkWrite { .. }
            | Self::LinkClose { .. }
            | Self::BrowserGateway { .. }
            | Self::Feedback { .. }
            | Self::StageAttachment { .. }
            | Self::RemoveAttachments { .. }
            // An agent signed in on the device runs Hide AI there with its
            // own login; no credential crosses the link (PRD wsl-device
            // D-03, D-06).
            | Self::AiAvailability { .. }
            | Self::AiModels { .. }
            | Self::AiExecute { .. }
            | Self::AiMeasurement { .. }
            | Self::AiRestart { .. }
            | Self::AiRelease { .. }
            | Self::Cancel { .. } => true,
            Self::UsageLimitFetch { .. }
            | Self::UsagePriceFetch
            | Self::Gh { .. }
            | Self::ReadAttachments { .. }
            | Self::ReadDropped { .. }
            | Self::RemoveClipboard { .. }
            // A label lock is the core's own node's, keyed by its engine
            // folder; a device's labels run unshared.
            | Self::LabelLock { .. }
            | Self::LabelUnlock { .. } => false,
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
    /// The same folder as the node's filesystem names it (every link
    /// resolved, on-disk case), so a registered folder, which a node
    /// answers resolved the same way, is the home folder when the two
    /// are equal; the core measures no size for it (PRD home-folder-project
    /// D-06). Absent from a node older than protocol 85, or when the home
    /// cannot be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_real: Option<String>,
    /// Read once when this connection starts; a failure leaves lineage
    /// unresolved without making the file helper unavailable.
    pub machine_identity: MachineIdentity,
    /// Absent on protocol24. Missing or invalid facts on the current
    /// protocol grant no reader; files, Git and unrelated node operations
    /// remain available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_features: Option<crate::sessions::ReaderFeatures>,
    /// Where the node shows the drives of the Windows PC it runs on, when it
    /// is a WSL distribution (`hide_platform::host::windows_drives`), so Add
    /// a project can spell a pasted `C:\` path as that node's own path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_drives: Option<String>,
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

    /// A usage read of the default account is written as an older core
    /// wrote it, and an older core's read is one of the default account.
    #[test]
    fn a_usage_read_names_a_profile_only_when_it_reads_one() {
        let default = Call::UsageLimitFetch {
            source: crate::usage::LimitSource::ClaudeUsage,
            root: "/hide".into(),
            profile: None,
        };
        let written = serde_json::to_value(&default).unwrap();
        assert!(!written.to_string().contains("profile"), "{written}");
        assert_eq!(serde_json::from_value::<Call>(written).unwrap(), default);
        let profile = Call::UsageLimitFetch {
            source: crate::usage::LimitSource::CodexUsage,
            root: "/hide".into(),
            profile: Some(hide_agent_adapter::ProfileId("p-a".into())),
        };
        let written = serde_json::to_value(&profile).unwrap();
        assert_eq!(serde_json::from_value::<Call>(written).unwrap(), profile);
    }

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

    /// The branch calls are on the wire as `op` objects; a facts request
    /// without a remembered tip omits it, and a helper answers a device.
    #[test]
    fn the_branch_calls_round_trip_and_a_device_answers_them() {
        let facts = Call::BranchFacts {
            root: "/repo".into(),
            branch: "feat".into(),
            tip: None,
            base: "main".into(),
        };
        assert_eq!(
            serde_json::to_value(&facts).unwrap(),
            serde_json::json!({"op": "branch_facts", "root": "/repo", "branch": "feat", "base": "main"})
        );
        let merge = Call::BranchMerge {
            root: "/repo".into(),
            branch: "feat".into(),
            tip: Some("b".repeat(40)),
            base: "main".into(),
            message: "Merge feat".into(),
        };
        let discard = Call::BranchDiscard {
            root: "/repo".into(),
            branch: "feat".into(),
            tip: Some("a".repeat(40)),
        };
        let diff = Call::BranchDiff {
            root: "/repo".into(),
            branch: "feat".into(),
            base: "main".into(),
            path: "src/a.rs".into(),
        };
        let catch_up = Call::BranchCatchUp {
            root: "/repo".into(),
            head: "c".repeat(40),
        };
        assert_eq!(
            serde_json::to_value(&catch_up).unwrap(),
            serde_json::json!({"op": "branch_catch_up", "root": "/repo", "head": "c".repeat(40)})
        );
        let merge_push = Call::BranchMergePush {
            root: "/repo".into(),
            branch: "feat".into(),
            tip: Some("b".repeat(40)),
            base: "main".into(),
            message: "Merge feat".into(),
        };
        assert_eq!(
            serde_json::to_value(&merge_push).unwrap()["op"],
            "branch_merge_push"
        );
        let sync = Call::BaseSync {
            root: "/repo".into(),
            base: "main".into(),
            branch: "local-main-1030".into(),
        };
        assert_eq!(
            serde_json::to_value(&sync).unwrap(),
            serde_json::json!({"op": "base_sync", "root": "/repo", "base": "main", "branch": "local-main-1030"})
        );
        for call in [facts, merge, merge_push, sync, discard, diff, catch_up] {
            let text = serde_json::to_string(&call).unwrap();
            assert_eq!(serde_json::from_str::<Call>(&text).unwrap(), call);
            assert!(call.answered_by_device());
            assert!(!call.runs_until_stopped() && !call.is_control() && !call.is_background());
        }
    }

    /// The profile calls cross the link as whole requests: a call's fields
    /// sit beside the request's own `id` and `op`, so none may be named
    /// either, and each one reads back as it was sent.
    #[test]
    fn the_profile_calls_round_trip_as_requests_and_a_device_answers_them() {
        use crate::profiles::ProfileOp;
        use hide_agent_adapter::{AccountRef, ProfileId};
        let calls = [
            Call::ProfileList { values: true },
            Call::ProfileList { values: false },
            Call::ProfileApply {
                change: ProfileOp::Add {
                    kind: "claude-code".into(),
                    name: "work".into(),
                    account: Some(AccountRef::Folder("~/work/".into())),
                    program: Some("claude-gw".into()),
                    env: vec![("ANTHROPIC_BASE_URL".into(), "https://gw.example".into())],
                    default: true,
                },
                expected_version: 3,
            },
            Call::ProfileApply {
                change: ProfileOp::Edit {
                    id: ProfileId("p-1".into()),
                    name: "office".into(),
                    program: None,
                    env: Vec::new(),
                },
                expected_version: 4,
            },
            Call::ProfileApply {
                change: ProfileOp::Default {
                    kind: "omp".into(),
                    id: ProfileId::default_profile(),
                },
                expected_version: 5,
            },
            Call::ProfileApply {
                change: ProfileOp::Remove {
                    id: ProfileId("p-1".into()),
                },
                expected_version: 6,
            },
            Call::ProfileInstall {
                profile: ProfileId("p-wire-probe".into()),
            },
            Call::ProfileLaunch {
                kind: "claude-code".into(),
                profile: ProfileId("p-1".into()),
            },
        ];
        for call in calls {
            let request = Request { id: 991, call };
            let text = serde_json::to_string(&request).unwrap();
            assert_eq!(
                serde_json::from_str::<Request>(&text).unwrap(),
                request,
                "{text}"
            );
            assert!(request.call.answered_by_device());
        }
        assert_eq!(
            serde_json::to_value(Request {
                id: 991,
                call: Call::ProfileInstall {
                    profile: ProfileId("p-wire-probe".into()),
                },
            })
            .unwrap(),
            serde_json::json!({"id": 991, "op": "profile_install", "profile": "p-wire-probe"})
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
