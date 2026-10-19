# Agent Hooks

How Hide learns what an agent has spawned inside its own process, and what it writes to the operator's machine to learn it.

Herdr reports panes.
An agent that spawns a subagent in-process creates no pane, so nothing about it reaches Herdr's wire, and a pane running six subagents is indistinguishable from one working alone.
This is the only part of the tree Hide cannot read from the session snapshot, and it is why anything is written to the operator's configuration at all.

## What is written, and where

`hide-agent-hooks/` is the only code path in the product that writes a file the operator owns outside the app's own state.

| Runtime | File |
| --- | --- |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/hooks.json` |
| OpenCode | `~/.config/opencode/plugins/hide.js` and `~/.config/opencode/plugins/hide-opencode/` (see [OpenCode: Hide's plugin](#opencode-hides-plugin)) |
| Pi | `~/.pi/agent/extensions/hide.ts` (see [Pi and omp: Hide's extension](#pi-and-omp-hides-extension)) |
| omp | `~/.omp/agent/extensions/hide.ts` (same section) |
| Grok | `~/.grok/hooks/hide.json`, Hide's own file ([Grok's own hooks](#groks-own-hooks)) |

On Windows `~` is the account's profile folder (`%USERPROFILE%`), where Claude Code, Codex, Grok keeps these files and OpenCode, Pi and omp load their plugins and extensions from.
Beside the hooks, Hide records Claude Code's, Codex's and agy's own folder trust for a worktree it made, in each agent's own file ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)).

One entry is appended per registered event, and nothing else in the file is touched.
The common writer retains JSON source spans through `serde_json::value::RawValue`, splicing only owned changes so unrelated members, groups and handlers retain their literal whitespace, ordering and escapes.
Removing a Hide handler from a mixed group keeps every unowned sibling.
Duplicate decoded member names, malformed JSON, nesting beyond serde's limit, or input/output beyond 16 MiB are refused without replacement.
The write is atomic - a temporary file beside the target, created private to the account (0600 on macOS and Linux, an access list naming only the account on Windows), that replaces the target in one step (`hide_platform::fs::atomic::write_file`).
The kit holds its account lock around the pass, and the writer refuses an observed source edit or resolved-link change before replacement.
That comparison is not an operating-system compare-and-swap: an arbitrary editor can still race after it.
The file keeps its mode, a new one is created 0600, and a file that is a symlink stays one: the write lands at the file the link resolves to, so a settings file kept in a dotfiles repository is edited there.
Entries belonging to other tools are counted before and after, and a regression test asserts they survive.

Both runtimes register `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `SubagentStart`, `SubagentStop`, `Stop`, and `PostToolUse`.
Claude Code also registers `PostToolUseFailure`; Codex reports nonzero outcomes through Post and gets no invented failure event.
The Post matcher selects `Bash|PowerShell` and produces no replacement output or permission decision.
`SessionEnd` is not registered by either: a session's record is read only while Herdr reports that session for the pane, so a session that ends needs no event to be forgotten.
`PreToolUse` selects `Bash|AskUserQuestion|ExitPlanMode|Edit|Write|MultiEdit|NotebookEdit|Read` in Claude Code and `Bash|request_user_input|request_user_input_async|apply_patch|Edit|Write` in Codex (`install::hook_matcher`, the one function the writer and Codex's trust check both read): the shell, the native questions, and the file tools the [project guardrails](#project-guardrails) judge; a search never starts the helper.
Codex matches a tool by its whole name: with a hook on `request_user_input` alone, the async question tool of 0.162.1 never reached the helper (measured 2026-10-15 in an isolated `CODEX_HOME`), so each question tool is named.
An entry of Hide's whose matcher differs from the one written today reads Outdated at its own version, so the next kit pass rewrites it and records its Codex trust.
The version-ten marker makes earlier entries Outdated until the normal kit pass at launch, connection or Reinstall replaces Hide's entries and records their exact Codex trust.
Already-running sessions pick up the changed hooks when restarted.
The installed-command version (`HOOK_ENTRY_VERSION`, now 10) is separate from the pane-report protocol (`HOOK_VERSION`, still 8).
A pane already reporting protocol 8 keeps its connection and child counts both before and after that kit pass; updating a command does not invalidate a readable report.

Every entry carries `--runtime claude-code|codex` and `--source hide-subagents@<version>` inside its command (in Claude Code's Windows entry, inside its `args`).
The runtime argument selects that runtime's stdout envelope; the current marker also replaces commands not guarded against a missing helper.
The source name proves the entry is Hide's, and the version after the `@` separates a current hook from an outdated one.
The kit also compares the generated command, including its helper and state route; an unedited command for another state folder is stale, while an operator edit stays until Reinstall.
The helper does not pass the marker on: it is an install marker, not the metadata source (see below).

Every active agent's hook or extension carries the installing daemon's state folder as `--state-dir-b64`, a URL-safe base64 argument that preserves spaces, quotes and non-ASCII names across POSIX shells, PowerShell and JavaScript argument arrays.
The helper gives this route to each sibling `hide` call when `HIDE_STATE_DIR` is absent; an explicit pane route and the CLI's capability reference retain their existing priority and authentication checks.
The installer rejects relative, non-UTF-8, NUL-containing or oversized state paths before writing an entry.
Without a nonempty explicit `HIDE_STATE_DIR`, a malformed, relative or duplicate captured route fails the call rather than falling back to another daemon.
This covers an app opened from the Dock without `XDG_STATE_HOME` while an agent's shell exports it, and agents whose hooks do not inherit Hide's environment.
The current Grok and agy entries, Pi and omp extensions, and both OpenCode server plugins use the same rule; the kit updates their owned entries through its normal pass and records the exact route-bearing Codex command as trusted.
The install versions are Claude Code and Codex 9, Grok and agy 8, Pi and omp 8, OpenCode V1 7 and OpenCode's server plugin 2; the TUI plugin stays at 1 because it starts no helper.
Every kit pass compares the captured folder with the running daemon's folder, so a state move or a daemon started with another environment rewrites an unedited generated command without another version bump.
The account kit records which daemon state folder owns those commands; a different daemon sharing that HOME cannot change them while the recorded owner's lifetime lock is held.
File identity treats aliases or case changes of the same state folder as the same owner.
After that daemon ends, the next pass may take ownership and refresh the entries.
Development and QA candidates use a private HOME as well as a private state folder, because changing only the state folder still shares the account's hook files.
Before that pass, an unavailable old folder leaves mail pending and records the existing throttled `intake.failed` diagnostic with code `unavailable` in the helper's private delivery diagnostics; an invalid captured argument records `record` instead.

The shared SessionStart guidance is English.
It distinguishes Herdr's direct terminal input and control from Hide letters' stored delivery, replies and retry management.
`herdr agent prompt` may yield a passive observation but creates no Hide letter, watch or parent relationship.
Supervised delegation uses `hide agent spawn --parent here` to record responsibility and start the automatic watch.

## What the hook returns and reports back

On `SessionStart`, the helper writes one runtime JSON envelope whose `hookSpecificOutput.additionalContext` combines the worktree-purpose instruction with the Project Memory index when Memory is enabled: at most 20 rule, decision and lesson cards, one line each (`m3 Tests run with nextest`), under a line that says to read a card in full with `hide memory show <card>` and to find others with `hide memory search <words>`, and nothing when the Project has no such card (PRD project-memory-v2 D-03, B10); a card waiting to be checked against the repository is left out of the index (PRD memory-card-quality D-06).
A card whose promotion merged into the repository keeps its index line but points at its file (`m3 Tests run with nextest -> docs/TESTING.md`) or its pull request, and is never given in full (B10, B12).
The instruction explains supervised delegation (`hide agent spawn --parent here …`, with an automatic watch) and independent operator handoff (`hide agent spawn …`, without parent or automatic watch); both preserve focus and record origin.
It also says that `--machine <device id>`, used from the machine that runs Hide, starts the agent on a connected device (an agent on a device is refused with `machine_not_permitted`), where the id is the `device_id` `hide workspace info` shows and the `machine` `hide agent list` shows for agents there, and that `--repo` and `--path` are then that device's paths.
It also tells the agent to run `hide factory add` to put work into a Factory instead of adding a GitHub label (see [factory.md](factory.md)).
The same envelope adds Workspace commands only after `hide workspace info --hook` confirms a renderer-connected Workspace for the caller and report its actual capabilities.
The probe does not need a pane id: the daemon binds a caller inside a Herdr pane to that pane, and any other local caller, such as a tool shell or hook inside Codex's shared app-server daemon or a plain terminal, to the registered checkout holding its cwd (`docs/ARCHITECTURE.md`, the Workspace CLI).
The guidance names that checkout path, so a session can read which Workspace its commands reach.
The helper uses the `hide` CLI beside it in the kit folder, or the CLI on `PATH` when none is there; on a device that is the helper root's current build, whose `hide` reaches this Mac's daemon through the device's return route (PRD device-parity B15).
The hook starts nothing and installs nothing; when the CLI does not answer within its two-second bound the session starts without Workspace guidance and the next one tries again (B26).
A session on any linked device receives its Project Memory index and prompt context from the core's one store through the same pane-scoped return route as letters.
The device's node resolves the attested checkout to its own Project identity; the store is never copied to the device.
An unavailable link or a helper without a Memory reader refuses the read immediately with a diagnostic reason; hooks continue without context within their existing budget.
A connected device pane can still receive pending letters through its sibling `hide` CLI and the return route within the same two-second caller budget; [delivery.md](delivery.md#safe-intake-and-manual-fallback) owns that intake and confirmation contract.
An ordinary Claude Code or Codex session started in a connected Herdr pane receives the same conditional guidance as a Hide-managed session when Hide's hook is installed in that runtime's configuration on that machine.
A disconnected pane, a cwd outside every registered checkout, an unavailable renderer, or an unsupported Browser surface receives no Workspace capability claim.
The guidance scopes every command to the caller's checkout Workspace and lists only capabilities returned by the daemon; `hide workspace info` remains the live check and `hide --help` gives the full syntax.
With `browser.open` it also lists the `hide browser` page commands in one line and points at `hide browser help`, their agent guide ([BROWSER_DISPLAYS.md](BROWSER_DISPLAYS.md#agent-page-commands)).
The hook creates an owner-only, finite-lived credential reference and includes its path as a shell environment prefix for the listed commands.
The path is a credential reference and should be handled as private session context, even though it contains no bearer bytes itself.
The credential's bearer bytes and file contents never enter hook stdout, arguments, or the agent context, and each command rechecks the caller's checkout membership and renderer availability.
The prefix is a convenience, not the only way in: a bare `hide` command bootstraps its own one-shot credential, and a caller the daemon cannot place in a pane is bound to the registered checkout holding its working directory (`docs/ARCHITECTURE.md`, hided and the WebSocket boundary).
A prefix whose reference has expired or been revoked keeps working, because the command falls back to that same bare bootstrap; a session start that inherits a `HIDE_CAP_REF` whose file is gone bootstraps a fresh reference for its context instead of repeating the gone one.
That covers Codex 0.157 with `daemon_auto_start`, where the tool shell and this hook both run inside the shared `codex app-server` daemon under launchd rather than in the pane: the hook may run with the daemon's environment instead of the pane's, so its Workspace guidance can be missing, and the bare Workspace commands still reach the checkout the tool shell runs in.
Letters and agent commands do not: they need a pane-bound caller, so such a session's hook gets `agent_pane_required` (recorded as the `identity` cause) and takes no letters until the session runs in its pane ([delivery.md](delivery.md#commands-and-caller-identity)).
Every Codex Hide starts passes `--no-daemon`, so a Codex Hide starts runs its hooks in its pane.
The kit no longer turns the daemon off on its own: it only reads whether the machine's Codex has the setting, for that flag, and a machine where an earlier Hide turned it off keeps it off.
`src/codex_daemon.rs` is the one place that changes the setting or stops the daemon, through `codex features disable daemon_auto_start` and `codex app-server daemon stop`, and only for the operator's own confirmed request from a not connected Codex pane's popover (`docs/ARCHITECTURE.md`, The install kit; `docs/status-model.md`, Not connected, and what fixes it).
The kit reads the setting with the capability (`KitReport.codex_daemon_on`) and whether a daemon answers (`KitReport.codex_daemon_running`), which is what tells a Codex session on the shared server from one that started before the hook.
The SessionStart command hook has an eight-second timeout, including one bounded two-second CLI probe (`workspace info --hook`), which it asks beside Memory; a failed probe leaves the existing purpose and Memory context intact.
A persistent credential (the reference SessionStart hands the session as `HIDE_CAP_REF`) remains unclaimed for at most 30 seconds until a CLI receives and acknowledges a Workspace response.
A one-shot Workspace command is never claimed: the daemon revokes its credential when the command's connection ends, and the CLI removes its reference; a `hide browser` relay claims its credential, one-shot or not, because the daemon starts a relay only for a claimed one.
The CLI writes the claimed marker only after the daemon acknowledges that claim, so a caller killed before acknowledgement leaves an unclaimed reference that expires.
A repeated SessionStart in the same attested pane reuses its live persistent reference, including when an earlier hook stopped after claiming but before delivering context.
The SSH bridge tracks every reference it issues and revokes the corresponding daemon token when its file disappears, its unclaimed period expires, its eight-hour lifetime ends, its pane shell exits, or its one-shot caller exits.
A failed bootstrap reply revokes only a newly issued reference; it leaves a reused live reference intact.
Late bridge replies are matched by request ID so one timed-out attestation cannot poison the next bootstrap.
On `UserPromptSubmit`, it parses at most 256 KiB of runtime input and asks the core for Memory (`hide workspace memory`, below) with the session and the prompt's first 8 KiB on stdin; the core resolves the Project of the caller's checkout through that checkout's node and performs a read-only lookup against the materialized active projection of the one store it owns.
The prompt text is the search input only; the original prompt is never replaced.
A card is eligible only when `hide_memory::matcher` scores it above its minimum: a whole word, a stem with its Korean particle taken off, an alias, or enough two-syllable Hangul pieces ([ARCHITECTURE.md: Project sessions and Memory](ARCHITECTURE.md#project-sessions-and-memory)); extraction confidence never stands in for relevance and only breaks ties after it (B11, B12).
At most three whole cards and 600 estimated tokens are returned in the same `additionalContext` envelope, leaving out the cards waiting to be checked against the repository, the cards an agent reported stale and still held, and the cards the session was already given in full, by an earlier prompt or by `hide memory show`, as its recorded receipts say.
The cards follow one `<hide-memory-instruction>` line, outside the untrusted `<hide-memory-context>` block, that says to run `hide memory stale <id> "<what changed>"` when what the operator says now contradicts a card; every runtime receives the same text, and the session-start index has no such line ([ARCHITECTURE.md: A card an agent reports stale](ARCHITECTURE.md#a-card-an-agent-reports-stale)).
Only a body injection carries a receipt; the session-start index carries none, because an index line is no delivery.
The core authenticates the receipt with the Project-scoped key in the same SQLite store, binding the runtime, session, hook event, and exact ordered card revisions without creating another persistence surface.
The core accepts it only from provider-owned transcript records, including Claude Code's `hook_additional_context` attachment and actual Codex developer messages, and verifies the authentication tag before recording it.
A receipt not yet recorded when the next prompt arrives can repeat a card once; the session goes on either way, and no sidecar or second store is written.
When a Project's receipts are at their cap and the session has none recorded, its prompts get no Memory rather than repeating cards; the cap is logged ([ARCHITECTURE.md](ARCHITECTURE.md#project-sessions-and-memory), B26).
Claude Code and Codex currently accept the same envelope, but the installed runtime argument keeps that protocol choice explicit.
`SubagentStart`, `SubagentStop`, and `Stop` write nothing to stdout, preserving their existing silent behavior.
This stdout is advisory context for the agent and is independent of the best-effort metadata report described below.

For local and connected device agent delivery, `UserPromptSubmit` reads the submitted prompt from the runtime payload (waiting at most 0.5 seconds for it) and sends its SHA-256 and the native session it runs in to `hide inbox --hook --prompt-digest <sha256> --session <id>`; the core decides from that digest whether the prompt is the line its doorbell typed ([delivery.md: Recognizing the bell](delivery.md#recognizing-the-bell)), and the letter intake never sends the prompt text; only Memory's request carries the prompt's first 8 KiB, as a search input.
The session id goes to `hide inbox --hook --session`, which is how the hook counts as a submission only in its own pane; an unreadable or truncated payload has none, so that hook clears no draft.
The bell turn pulls at most five pending letters and 8 KiB of letter context from the sibling `hide` CLI within one total two-second budget; any other prompt receives only a one-line count of waiting letters.
Only a successful stdout flush permits confirmation, so pre-confirm interruption can repeat an ID and confirmed letters do not repeat.
The session guidance says that a prompt beginning with `🦉` is Hide's bell and that `hide inbox` shows the letter when the prompt did not bring it, and a Codex bell names `hide inbox` itself ([delivery.md: The bell line](delivery.md#the-bell-line)); a missing or failed hook leaves pending letters available through `hide inbox` and `hide request show`.
The same guidance says that Codex CLI on Windows may show the line without the owl; the core and hook recognize the saved line after removing non-BMP characters ([delivery.md: Recognizing the bell](delivery.md#recognizing-the-bell)).
This delivery path has its own bounded private diagnostics; Memory's request runs beside it and ends where it does.
[delivery.md](delivery.md#safe-intake-and-manual-fallback) owns these intake, failure and confirmation rules.

Memory has one store, `project-memory.sqlite3` in the core's state folder, which only the core opens, whichever machine the agent runs on (PRD core-host-node-move D-12, B14).
The hook asks for it through `hide workspace memory --event <event> --runtime <runtime> --session <id>`, the same pane credential and node link its letters come by, on its own thread beside the letter pull; on another machine the request goes through that machine's node to the core.
These reads and SessionStart guidance use one typed proof-and-answer request through the local bootstrap service, forwarded with the node's fresh caller proof when the core is elsewhere (NodeLink protocol 31).
The service accepts only the `LetterPull`, `Memory` and read-only `WorkspaceInfo` variants of `HookCommand`, never Workspace actions or letter confirmation, and reuses the ordinary core executors after credential validation.
A letter or Memory one-shot token stays inside the core and is revoked before the answer is sent, even if the caller ended or the request unwound; no reusable credential is returned by those two reads.
The guidance read returns the same persistent reference and scoped capabilities that bootstrap followed by info returned, in one child and one proof exchange.
The node owns that reference file; a failed answer removes only a newly issued reference, and the core claims a guidance credential only after its proof answer is accepted.
A live `HIDE_CAP_REF` must agree with the fresh proof and, on a node, its own link; a letter or Memory read does not claim or extend that persistent reference.
An absent named reference falls back to the fresh proof and records `hook.reference_missing` with the request identity, never its path or token.
Local and device refusals include the same `next_action` recovery advice.
The selected route receives one request with no retry through another transport; ambiguous node bridges are refused before any command is sent.
The request's stream carries its deadline ([ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)), so the connect and every write and read wait only for the time left, and partial responses cannot renew the deadline.
Each step that can fail answers its own reason, which the hook records under a cause of its own ([What a hook records when it fails](#what-a-hook-records-when-it-fails)).
Requests and responses are capped at 16 KiB and 64 KiB, and the existing bootstrap and node proof worker caps still apply.
The letter and Memory children run independently and in parallel under their existing caller deadlines and fail-open outcomes.
The core takes the Project from the caller's attested checkout, read by that checkout's own node, never from anything the hook sends.
The lookup performs no provider or embedding call, transcript scan, child-process launch, or database write, and runs off the core's runtime lock.
Missing, locked, corrupt, stale, over-limit, unresolved-Project, and over-deadline stores return no Memory context, and the hook still exits zero.
Memory waits for its answer until 1,250 ms after the inner hook starts; the 1,650 ms intake budget reserves its last 400 ms for letter confirmation after a successful stdout flush.
The prompt supervisor bounds the inner helper at 1,850 ms from the outer helper's start, so the Memory read budget is not the entire prompt hook duration.
SessionStart has the same Memory read budget, a separate two-second Workspace CLI probe asked beside it when its thread starts, and later count-report calls with their own two-second timeouts; its installed runtime entry has an eight-second timeout.
The core sets a one-second Memory answer deadline, but owner-scope reply waiting has no receive timeout and a remote call applies its timeout separately to admission, write and answer.
That deadline is not an enforced end-to-end execution cutoff: the caller can continue without Memory while an admitted daemon job still runs.
Candidate, item and token counts remain hard bounded; [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#project-memory-cost-contract) owns the measurement conditions and this limit.
A node whose link to the core is down refuses at once with `hide_unavailable`; the hook then records `memory.unavailable`, or `memory.deadline` for an answer that did not come in time, at most once per ten minutes in its private diagnostics, and the turn goes on without Memory.
A panic on the thread that asks is recorded the same way as `memory.panicked`.
Stdin is read through a nonblocking descriptor until EOF, the size cap, or the 500 ms payload wait.
Windows has no nonblocking read of a pipe, so there a reader thread collects stdin and the helper stops waiting for it at the same point.
Current Claude Code and Codex `UserPromptSubmit` input and output shapes are fixed by sanitized fixtures in `hide-agent-hooks/tests/fixtures/`.
The app probes the installed runtime binaries with a 750 ms bounded version check and currently requires Claude Code 2.1.278 or Codex 0.155.1 for Memory injection.
A runtime below that capability is diagnosed as `Update required` without disabling a supported installed runtime or Sessions browsing.

The helper is stateless, as a hook script must be.
The count lives in `~/.hide/agent-hooks/panes/`, keyed by `$HERDR_PANE_ID`, and is republished after every event through the `pane.report_metadata` socket method, which Herdr defines as display-only pane metadata.
The request goes through `hide-herdr-client`, the same client the core's Herdr connections use, to the socket in `HERDR_SOCKET_PATH` or Herdr's default (`$XDG_CONFIG_HOME/herdr/herdr.sock`, otherwise `~/.config/herdr/herdr.sock`), with a two-second timeout so a Herdr that does not answer costs the agent's turn that long and no more.
The metadata source is `hide-subagents`, the marker name without its version: Herdr limits a source to ASCII letters, digits, `:`, `.`, `_` and `-`, and refuses the marker's `@` with `invalid_metadata_source`.
The core reads the tokens back out of the pane tokens its ordinary snapshot already carries, so no subscription or poll exists for any of this.

The first release shelled out to `herdr pane report-metadata` with the pane id after the options and the versioned marker as the source.
The pinned CLI refuses both, the helper swallowed the exit status, and every pane on the machine read as `session_predates_install` with advice to restart that changed nothing.
Two things keep that from recurring: `report::tests` sends one report at a fake socket and asserts the exact request, and a report that fails is written down (next section) rather than dropped.

| Token | Meaning |
| --- | --- |
| `hide_hooks` | The hook version this session reported with. Its presence is the proof that the session is instrumented at all |
| `hide_sub_working` | Subagents currently running |
| `hide_sub_done` | Subagents this session has finished |

`hide_sub_blocked` is defined and never written: neither shipped adapter can observe a blocked subagent from the four available events, so the count stays unknown rather than being drawn as a zero.
A token that is not a count is dropped rather than coerced.

A record belongs to one session.
The first event of another session that the pane's agent now is starts the record over, so a new session in a reused pane never inherits the last one's numbers.
A `SessionStart` of the same session (a resume or a compaction) keeps them, since its background subagents still run; a runtime whose input names no session resets at `SessionStart`.
A pane Herdr has stopped listing has its record swept on the next session bootstrap.

#### Which session is the pane's

A session that is not the pane's agent runs the same hooks in the pane's environment: the operator's own Stop hook that starts `claude -p` is one, and it reset the pane's count to zero after every turn of a session that ran subagents (twenty-one of the forty panes on the operator's machine on 2026-10-15 held such a session's record).
The hook therefore asks Herdr, before it changes anything, which session it reports for the pane's agent (`pane.get`, `agent_session`).
An event of any other session changes nothing and reports nothing, and an event that names no session counts as another one.
A pane for which Herdr reports no session, or does not answer, takes every event as its own, as before; an answer that cannot be had is written down as the pane's last report failure (`doctor` and Settings show it), so a pane that fell back to counting every session says so.
Herdr is the authority and the hook's environment is not: `CLAUDE_CODE_ENTRYPOINT` says `sdk-cli` inside the nested session, but a user's own program can start a session that says `cli`.

The rule that makes Herdr's answer the right one is Herdr's, in `src/terminal/state.rs` of the pinned tag (`session_report_allows_session_replacement` and `conflicting_same_owner_session_ref`, called from `set_agent_session_ref_for_session_start`).
For `herdr:claude`, a `pane.report_agent_session` of another session replaces the pane's only when its `session_start_source` is `clear`, `resume` or `compact`; a `startup` of another session is dropped.
Every `claude -p` session reports `startup`, and so does the pane's own first session, which has no earlier one to conflict with.
The Socket API reference does not document this, so `real_herdr::the_pinned_herdr_keeps_a_panes_session_against_a_nested_startup_and_follows_clear_resume_and_compact` pins it against the pinned binary in the `os contract` lane, and a Herdr bump that changes the rule fails there.
Two limits follow from the rule:

- A nested session that reports `resume` (a `claude -p --resume`) replaces the pane's session in Herdr itself, and Hide follows Herdr.
- For `herdr:codex` the rule lets a `startup` replace the session; Herdr's own Codex hook script ignores a session whose id differs from `CODEX_THREAD_ID`, which is how a nested Codex session stays out, and Hide relies on that script for Codex.

A pane's own `SessionStart` can reach the socket before Herdr's own hook has reported the new session (Claude Code runs a start's hooks in parallel).
Hide then reads it against the old session and ignores it, and the new session's next counted event starts its record from zero, so the old session's numbers are never added to (`claude_subagent_counts::a_new_session_whose_start_beats_herdrs_own_report_starts_a_fresh_record_at_its_next_event`).
Until that event the pane's tokens still show the old session's counts.

What an event costs beyond the count it always made: one `pane.get` over the local socket, before the lock is taken, for `SessionStart`, `SubagentStart`, `SubagentStop` and a `Stop`, and none for an event that changes no count and finds the record already naming its session.
`UserPromptSubmit` and `PreToolUse` never reach the count (the prompt hook and the spawn guard take them).
`PostToolUse` and the supported failure events feed passive observations only and add nothing to the count.

#### Which subagents run

Claude Code names every subagent (`agent_id`) and sends three facts that the count follows (Claude Code 2.1.300, measured in an interactive session):

- The parent's `Stop` fires while background subagents still run, and both `Stop` and `SubagentStop` list them in `background_tasks` with `type` `subagent` and `status` `running`.
- A subagent that waits on its own background command ends its turn, which sends a `SubagentStop`; when the command ends it runs again and sends a `SubagentStart` with the same `agent_id`. Every pause is a stop, so counting stops counted one subagent as done at every pause.
- A subagent stopped with the `TaskStop` tool sends no `SubagentStop` at all; the next `Stop`'s list no longer holds it.

So the record keeps the running ids and the ids that finished, per session.
A start adds the id (and takes a resumed subagent back out of `done`), a stop of a running id moves it to `done` once, and a repeated stop, or the stop of an id that never started, changes nothing.
The parent's `Stop` replaces the running ids with the ones it lists: a subagent it no longer lists left without a stop reaching the hook and stops working, without being counted done.
A `Stop` that lists nothing (an input cut off, or a Claude Code that sends no `background_tasks`) changes nothing; nothing sweeps, since no signal says a running subagent ended.
The running ids are capped at 256 per session, the remembered finished ids at 256 (the oldest go first, and the done count keeps them); crossing the running cap fails the event and is written down as the pane's last report failure.
Codex and the runtimes whose input carries no subagent id keep the older rule: a start adds one, a stop moves one to done, and the turn's `Stop` sets `working` to what the runtime lists, none for Codex.

### What a hook records when it fails

A hook that cannot get its answer goes on without it and records why in `~/.hide/agent-hooks/delivery-diagnostics.json`, private to the account, at most once per cause every ten minutes, with one JSON line on its standard error when it records.
The causes are a fixed list (`CAUSES` in `hide-agent-hooks/src/delivery.rs`), so the file stays small and holds no path or text.
A hook's request to its daemon (`hide inbox --hook`, `hide workspace memory`) answers a reason for the step it stopped at, and each step has a cause of its own:

| Step | `hide` reason | Cause |
| --- | --- | --- |
| No daemon published its bootstrap socket for the state folder, and no device's node answers | `hide_unavailable` | `unavailable` |
| The record naming that socket is not the account's own and private, or names a folder that is gone | `invalid_bootstrap_socket_record` | `record` |
| Nothing took the connection at the socket the record or the pane's reference names | `hide_unreachable` | `connect` |
| The request was not written whole, because its time ran out or the stream broke; nothing ran | `request_unsent` | `unsent` |
| No answer came before the deadline; the command may have run | `request_timeout` | `timeout` |
| The answer ended, broke off, or was not one bounded JSON line for this request | `invalid_response` | `answer` |

The daemon's own refusals keep the causes a hook acts on (`ledger`, `capacity`, `identity`), and any other reason is `refused`; an answer with no reason is `format`, and `cli` means the helper could not run the `hide` beside it.
The `hide` whose request failed also writes `hook.request_failed` on its standard error, with the request id, the reason and the system's error kind.
Memory's request records every failure but a deadline as `memory_unavailable` and keeps the step's cause beside it under `step`, so the file says where it stopped; a file an earlier helper wrote has no `step` and is read as it is.
Each step has its own reason because they used to share one: every step answered `hide_unavailable`, recorded as `cli`, so a Windows hook that never wrote a byte to its daemon read like a missing `hide`.

### After Herdr hands off or restarts

Herdr keeps a pane token only as long as the server that took it.
A live handoff (what an update runs) and a restart both start the new server with no tokens, while the pane ids stay, so the pane's record is still there and the pane still has its name (measured on the pinned Herdr 0.9.3 in a private server, issue 799).
The agent session Herdr reports for a pane (`agent_session`) survived the live handoff in the same measurement.
Until the agent's next hook event, which for an idle session can be a long time, every instrumented pane then read as a session that predates Hide's setup, with a Reopen that restarted a healthy session.
hided puts the tokens back, so the pane reads as it did before.

A pane id alone does not say that the agent in the pane now is the one the record describes: a restart can leave the same id with another process in it.
So each record names, besides its counts, who wrote it (`counters::Record`): the hook version of the helper, the adapter id of the runtime that reported, and the agent's own id of the session, read from the `session_id` of the hook's input when the helper read that input whole.
Every pane that has reported has a record: an event that changes no count writes it when the pane has none or when it names another version, agent or session.
An event that changes no count keeps the session the record names for the same agent, and any other event names its own, or none when its input named none.
Grok names its agent and no session, because their hook input is not read for one.
The OpenCode and Pi plugins' count calls carry no session id today, so their records name none either (the helper records one it is given, after the same validity check).
A record that names no session never has its counts put back; the version still is.

The core's restorer (`herdr-core/src/coordination/hook_tokens.rs`) reads the agent list the coordinator already reads each second.
For each listed agent that carries no `hide_hooks` token and that Hide has an adapter for, it reads that pane's record on a worker thread, under the shared side of the same lock, and sends the report the helper sends (`report::report_params`):

| The record and what Herdr says now | What is put back |
| --- | --- |
| Names the agent Herdr detects, and the session Herdr reports | `hide_hooks` with the version the record names, and the counts |
| Names the same agent, and Herdr reports no session or another one, or the record names none | `hide_hooks` only; the counts are left out, so they read as unknown and never as a zero |
| Names another agent | nothing: the pane was taken over |
| Is missing, or was written by an older helper (no version, no agent) | nothing; the pane waits for its next hook event, and no record is migrated |

The version is put back for the same agent because a hook is installed in a runtime's configuration and not in a session.
The counts are put back only for the same session, and nothing relies on `SessionStart` having reset the record: a session that started after the record was written and before the connection is the case the session id tells apart.
What the counts can still be is the record's last word for that session: a subagent that ended with no `SubagentStop` reaching the helper still counts as working until the session's next `Stop` lists the subagents that run (or, for a runtime that lists none, ends the turn), exactly as it did before the loss.
A record that changed while the report was on its way is sent again, as the helper does, so Herdr ends on the latest count.
A report merges token by token, so when the second read finds the counts no longer vouched for (the session changed, or another agent's record replaced the file) the restorer clears the two count tokens with `null` instead of leaving the first report's values on the pane.

What else it does not do:

- A pane that already has its token is left alone, so a live helper's report is never raced.
- The record sweep (`sweep_subagent_counters`, which drops the records of panes Herdr no longer lists) cannot take a record the restorer is about to read: both work from the same bootstrap snapshot, a pane the restorer asks about is listed in it, and the sweep keeps every listed pane's record.
  After a server restart the first answer on the socket already lists every restored pane (measured on the pinned Herdr 0.9.3: five panes before a stop, the same five in the first snapshot after it), so a connect cannot see an empty pane list while the session is still being restored.
  With a hided attached, a stop and start of the private Herdr left the pane's record in place after hided's reconnect.
- A pane is asked once for each connection to Herdr, because every connect is a bootstrap and a handoff or a restart ends the connection.
  A pane with no record, or one an older helper wrote, is not asked again: the helper's next event reports the token and writes the record in the same step.
- It covers this machine's panes only.
  A device's record is the device's file, and the core reaches a machine's files through its node, so a device's pane waits for its next hook event as before.

The snapshot-rate work is a comparison of the tokens each listed agent already carries.
The only file read and the only `pane.report_metadata` call are on the worker, at most 256 panes in one batch, one batch in flight and one waiting, a failed pane tried three times for a connection, and each failure is a `hook_tokens` diagnostic naming the pane (`restore.failed`, `restore.gave_up`, `restore.batch`, `restore.deferred`).

The helper always exits zero and reads no more than its bounded standard-input prefix before writing applicable context.
A producer that never closes stdin is released after the 500 ms payload wait.
A hook that fails must never be what breaks the operator's agent.

Exiting zero is not the same as saying nothing.
The outcome of every report is recorded in `~/.hide/agent-hooks/last-report-failure.json`: a failure writes the pane, the event, the socket and Herdr's answer, and the next success removes the file, so it describes the hook's current state rather than its history.
`Diagnosis` reads it back as `last_report_failure`, `doctor` prints it as a `Last report failed:` line, and the Settings group shows it as an error note above the restart advice, because with a refused report on record a restart is not the fix.
The Settings screen learns of it through the existing kit worker, with the next diagnosis due one second after the previous read finishes while a window observes Settings (`settings_observed`, the same flag the Hide AI tab sets).
The same worker reads once at startup and after a successful install; ordinary refreshes stop when Settings is unobserved.
Diagnosis and installation are serialized, so a pre-install answer cannot overtake a later install, and neither read makes the session coordinator wait.
Shutdown stops admission, closes the fixed node to end its current call and joins the worker; a retiring answer is not published.

## Native questions from Factory workers

The same `PreToolUse` helper refuses Claude Code's `AskUserQuestion` and `ExitPlanMode`, and Codex's `request_user_input` and `request_user_input_async`, only after the core proves the caller is the current Task-held Factory worker.
The model receives a deny reason directing it to `hide factory ask`; an operator-started session and every other pane retain their native question UI.
OpenCode's `question` tool and omp's `ask` are refused the same way through Hide's plugin and extension ([OpenCode: Hide's plugin](#opencode-hides-plugin), [Pi and omp: Hide's extension](#pi-and-omp-hides-extension)).
Pinned Codex 0.160.1 was exercised with a trusted Bash positive control and a genuine plan-mode `request_user_input`: both reached PreToolUse and the question's native tool result and model reply carried the denial.
Codex 0.162.1's `request_user_input_async` was exercised the same way in an isolated `CODEX_HOME`: a hook that denied it reached the model as the tool result "Tool call blocked by PreToolUse hook", so the redirect holds for the async tool too.
The ordinary launch mode remains unchanged.

`hide workspace factory-question-guard --session <native-id> --runtime <runtime>` uses the authenticated pane-scoped route, a bounded fresh native identity read and the already-open Factory's current Task worker.
Environment, display name, cwd, descendants and a historical registration alone confer no worker authority.
An ended registration retained across cancel/revive counts only when its accepted spawn still joins the current independently attested native execution.
Missing identity, an unopened Factory, a changed connection, overload, timeout or any other unavailable proof allows the tool call and records a bounded diagnostic.
Factory workers run on the core's machine and on the nodes that dial the core; a caller on a device the core dials cannot borrow a worker identity, while a device running its own Factory uses the same local route.
This read starts no Factory, judgment, tick or store write and publishes no snapshot.

## The spawn guard

`PreToolUse` carries the spawn guard: a shell call that starts an agent through Herdr is refused before it runs because it bypasses Hide's responsibility record.
The reason offers two filled commands with the same name, kind, repository, branch and native arguments: `hide agent spawn --parent here …` delegates work the caller supervises and starts an automatic watch; `hide agent spawn …` hands independent work to the operator as a root without an automatic watch.
Both record the caller as origin and open a separate tab without changing the current screen or keyboard focus.
The agent fills missing values and chooses responsibility before rerunning the call.
What is refused, in a pane of a registered checkout (the call is `herdr`, or `$HERDR_BIN_PATH`, after any `VAR=value` words and the prefix words `time`, `exec`, `command`, `nohup`, `env` and, at the start of a command in a shell line, `if`, `then`, `elif`, `else`, `while`, `until`, `do`, `{` and `!`):

- `herdr agent start <name> --kind <kind> ...`;
- `herdr pane run <pane> <command>...` and `herdr pane send-text <pane> <text>`, when the command's first word is one of Herdr's agent kinds (the 24 the pinned Herdr lists in `herdr agent start --help`, pinned by a unit test; `cursor-agent` is read as `cursor`).

Everything else runs untouched, with no output: other `herdr` calls (`pane split`, `agent list`), `--help`, a start with no `--kind`, a call that does not mention `herdr`, any call outside a Herdr pane or a registered checkout, and a call that names another Herdr: `--session`, `--machine`, or a `HERDR_SOCKET_PATH` or `HERDR_SESSION` assignment that differs from the pane's own, whether inline before the call, through `env`, by an earlier `export`, `declare`, `typeset` or bare assignment in the same call, or by an `unset`, because the `hide agent spawn` it would be sent to starts the child in the pane's own Herdr, and an isolated Herdr server of a verification run is reached that way; a `source` of a script that sets them cannot be read, so that form is still refused, and the exact text is compared, so `HERDR_SOCKET_PATH=$HERDR_SOCKET_PATH` counts as another Herdr.
The guard reads the whole call as a shell would read it, one top-level simple command at a time (`&&`, `||`, `;`, `|`, `&`, newline), with quotes, backslashes, heredoc bodies and comments set aside, so a commit message or a heredoc that quotes `herdr agent start` is not a launch.
A heredoc inside `$(...)` is read as text to its delimiter line, so a quote or a parenthesis in its body does not count, and text that still ends inside a quote, a backtick or a `$(` is a syntax error a shell would not run, so it holds no launch.
A `send-text` or `pane run` whose first word is an agent's name is refused whatever the words after it say, so a plain message that starts with `claude` or `pi` is a known false positive of that rule, and `herdr agent prompt` is the call that sends text to a running agent.
A launch chained behind another command (`cd x && herdr agent start ...`) is refused as a whole and nothing in the chain runs, since the hook sees the tool call and not one command of it; the reason says to run the other commands separately.
A launch inside `bash -c`, `$(...)`, a script or an alias is not found (PRD non-goal).

The registered-checkout test is `hide workspace bootstrap` through the sibling `hide`, the same daemon-owned call `SessionStart` makes, needing no renderer.
Only the daemon's own "this caller is not in a Hide checkout" answers (`checkout_not_registered`, `caller_unavailable`, `pane_not_connected`, `pane_unavailable`, `pane_changed`, `caller_not_in_pane`) mean the call is not Hide's to guide, and it runs silently.
Any other outcome, a `hide` that is missing, a bridge to a device that is gone, an answer that does not come within the guard's 2.5 second budget, an unreadable answer and a reason this build does not know, lets the call run and appends one `daemon.unreachable` line with its cause to the guard's log, throttled to one per ten minutes through the delivery diagnostics' store (cause `guard`), because a refusal Hide cannot explain would be worse than the untracked child.
The line goes to the log only: a hook's standard error reaches nobody the operator or the agent could act on.
A pane of a Herdr server Hide does not attach to, whose working directory is inside a registered checkout, is bound by `hide workspace bootstrap` to that checkout and so reads as registered too; the guard is guidance, not a boundary, and the Hide commands it offers cannot spawn from such an unattached pane (known limitation, PRD D-03 reads the pane and the checkout, not whose Herdr it is).
`--repo` is the repository's main root and `--branch` the caller's current branch, both read from the call's `cwd` through the repository's own files (`hide_project::git`), with no git process; a detached HEAD leaves `<branch>` for the agent to fill.
Inside a Grok session Claude Code's hook says nothing at all and Grok's own hook guards in its own answer shape ([Grok's own hooks](#groks-own-hooks)); inside OpenCode it ends silently as well and OpenCode gets the same guard from Hide's own plugin.
Claude Code and Codex share one deny envelope, `hookSpecificOutput.permissionDecision = "deny"` with `permissionDecisionReason`; every other path prints nothing and exits 0, and the whole entry runs inside `catch_unwind`, so a defect allows the call rather than refusing an ordinary one; a failed owner handshake in the environment ends the outer hook with 0 as well, since exit 2 from a pre-tool hook would refuse the call.

Each refusal appends one JSON line (pane id, agent kind, shape, runtime; never the command text, and the same file holds the `daemon.unreachable` lines) to `~/.hide/agent-hooks/spawn-guard.log` (private, capped at 256 KiB with one rotation) and echoes it on standard error, where a hook run by hand shows it.
The per-call cost is what an ordinary shell call pays: the payload is read within 0.5 seconds and a byte test for `herdr` over the whole payload runs before anything is parsed or spawned, so a call whose payload does not mention `herdr` (its working directory and transcript path included) costs one process start and the [project guardrails](#project-guardrails)' policy read; a path containing `herdr` pays one JSON parse and the lexer as well (numbers in [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-spawn-guard-hook-on-a-shell-call)).
`hide-agent-hooks/tests/it/spawn_guard.rs` runs the helper beside a stand-in `hide` for each outcome above, and `src/spawn_guard.rs` holds the parser's table tests.

## Folder trust for the worktrees Hide makes

Claude Code, Codex and agy each ask, the first time they start interactively in a folder they have not been told to trust, whether to trust it, and a delegated child stopped at that question waits for an answer nobody is there to give.
So before Hide starts one of them in a worktree it made for that start (`hide agent spawn` into a new worktree, a task's agent), it records that agent's own trust of the worktree on the machine that will run it: the core sends `agent_trust` to the node of the machine that runs the worktree's pane, this Mac's or a connected device's, and the node writes it there (`hide_agent_hooks::folder_trust`).
That node is the pane's, not the one the agent's PATH is checked on (none for a device, whose own Herdr answers for its PATH), so a worktree made on a device from the screen or the phone gets its trust like one made here; a pane whose machine cannot be reached refuses the start with `agent_trust_unavailable`.
A start in a checkout that already existed, the operator's own or an earlier worktree, records nothing, and the agent asks there as it would outside Hide.

| Agent | Where its trust is kept | What Hide adds |
| --- | --- | --- |
| Claude Code | `~/.claude.json`, or `~/.claude/.config.json` while Claude Code still keeps that older file and it is not empty | `projects["<worktree>"].hasTrustDialogAccepted: true` |
| Codex | `~/.codex/config.toml`, written by Codex's own app-server | `[projects."<worktree>"] trust_level = "trusted"` |
| agy | `~/.gemini/antigravity-cli/settings.json` | the worktree in `trustedWorkspaces` |

Claude Code's and Codex's files are found from the same folder as their hook files (`AgentRuntime::trust_path` beside `config_path`), so the trust and the hooks always land in one place.
Like the hooks, they are the account's default folders: an agent run with `CLAUDE_CONFIG_DIR` or `CODEX_HOME` pointing elsewhere reads neither Hide's hooks nor its trust, and asks.

The worktree is named by its resolved path, spelled as the agent compares it: with `/` between names for Claude Code on Windows, and in lower case for Codex on Windows, which compares folders there without case.
The entry names the worktree itself and never the repository it was made from.
Answering an agent's own question in a linked worktree trusts the repository's main checkout, which is the operator's and would trust every worktree of that repository with it; Claude Code 2.1.302 and Codex 0.163.0 read an entry for the exact folder before the repository's, and trust given to a folder above a repository does not reach into it (measured on 2026-10-17).

Hide keeps which entries it added in its own record, `~/.hide/agent-hooks/<agent>-trust.json` (`claude-trust.json`, `codex-trust.json`, `antigravity-trust.json`), at most 4096 folders that exist per agent; a trust past that is refused with the cap in its reason.
The record is written before the entry and taken back when the entry could not be written, so Hide never leaves an entry its record does not name.
An entry that is already there is the operator's: Hide writes nothing and records nothing, so removing the worktree leaves it.
Removing a worktree Hide made takes out the entries Hide added for it, for each agent, and switching an agent off, or removing the machine from Hide, takes out every entry Hide added for that agent; an entry the operator added, before Hide or after it, stays, and so does what else the agent keeps under the same folder (Claude Code's project settings, a Codex project's other keys).
The removal runs once the folder is gone, so it finds the entry by the key Hide recorded, compared as the system compares folders: the folder's parent resolved, and on Windows without case and with either separator, so Git's `C:/x` finds Codex's `c:\x`.
A worktree removed outside Hide (by hand, or by a Factory cleanup that found the folder already gone) leaves its entries until the next trust Hide records for that agent, which takes out every entry of Hide's whose folder is gone in the same write.
Codex's entries are taken out through its app-server too; with no Codex left to ask, they and Hide's record of them stay until a pass that finds one, and nothing reads them meanwhile.

Every change holds one account lock per agent, and a change waits for it as long as the change holding it can take: one Codex app-server session's whole bound (`codex_trust::SESSION_BOUND`, 21 s) and a second, so a second start while Codex records the first one's trust waits instead of failing.
The core gives the node twice that and five seconds for the call (`folder_trust::Agent::longest_change`); a Codex that answers, as it does in well under a second, takes about a second.
The JSON files are rewritten through the same lossless read and compare-before-replace as the hook files, so their other keys and the operator's formatting keep their bytes.
Claude Code rewrites `~/.claude.json` while it holds its own lock on it, the `.lock` folder beside the file's real path (proper-lockfile resolves a link, so a linked `~/.claude.json` is locked beside its target), and reads the file again under that lock, so Hide changes the file only while it holds the same lock and neither overwrites the other.
Hide waits up to two seconds for it and takes over a lock older than ten seconds, which Claude Code treats as left by a process that ended, removing only the folder it saw stale; it removes its own lock only while the folder there is still the one it made, and does not write once Claude Code has taken the lock over.
Hide's own changes never meet at Claude Code's lock, since each holds Hide's lock first; between Hide and Claude Code, a takeover of a stale lock has the moment between the look and the removal that Claude Code's own takeover has.
Codex's file is never written by Hide: `hide_agent_hooks::codex_trust` starts `codex app-server`, reads the user layer's `projects` with `config/read`, writes the one key with `config/batchWrite` against the version it read, and reads it back, as it does for [Hide's own hooks](#codex-trusts-hides-own-hooks).

A trust that cannot be recorded refuses the start before the agent runs, with `agent_trust_failed: <agent>'s trust of <worktree> could not be recorded (<reason>)`, so no agent waits unseen on its question (PRD antigravity-first-class B20).
The node refuses to trust a folder that is not a Git worktree, a path written with `.` or `..`, the filesystem root, the home folder or a folder above it, whoever asks.
When the operator marked the worktree, or the repository's main checkout it was made from, `untrusted` in Codex, Hide does not override that answer: the reason names that folder and says to trust it in Codex or to start the agent there by hand.
A Codex configuration the operator's organization manages that decides the folder's trust instead, a machine with no Codex Hide can find, a Claude Code lock held past two seconds and a settings file that does not parse refuse the start too, each with its reason; a machine that is not connected refuses it with `agent_trust_unavailable`.

`hide-agent-hooks/src/folder_trust/tests.rs` checks each agent's format, spelling, ownership and removal in a fixture home on every pull request, on Linux in `rust` and on Windows in `windows check`, and `tests/it/codex_folder_trust.rs` runs Codex's through a stand-in app-server on Linux.
The nightly `agent trust contract` (`.github/workflows/agent-trust-contract.yml`) installs the pinned Claude Code and Codex on all three systems and checks that the real ones read what Hide wrote: Codex's app-server loads a worktree's own settings once Hide trusted it and stops once Hide took the trust out (`tests/it/codex_binary.rs`), and Claude Code in a pane of the pinned Herdr starts at its prompt in the worktree Hide trusted and asks its question in the repository's other worktree (`herdr-core/src/agent_start/real_agent_trust_tests.rs`).
The pull request runs no registry install, so a failure that is not Hide's never holds one; a change to the trust writer is proven on all three systems with `gh workflow run nightly.yml --ref <branch> -f lane=agy`.

## Project guardrails

The pre-tool hook also refuses what a project's protections name, the four switches of Project Settings > Working style ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#project-settings), PRD agent-guardrails).
They are protections, not a security boundary: they stop the ordinary tool calls and shell words below, and a write a script, `sed -i`, `bash -c`, `$(...)`, an alias or an MCP server makes is not seen (D-18).

The core publishes one policy file per machine, `~/.hide/agent-hooks/guardrails.json` (`hide_agent_hooks::guardrails::policy_path`), naming each project of that machine with a protection on: its main worktree root, the base branch the operator chose (none for Auto, which reads `origin/HEAD`), its four switches, the merge option and its protected paths.
This Mac's file is written by its own node and a connected device's by that device's node, through `Call::GuardrailPolicy` ([ARCHITECTURE.md](ARCHITECTURE.md#project-guardrails)); a saved switch shows Saved only after that machine has the new file, and every daemon start and device connection writes the file again from the current settings, empty when nothing is on.
The hook reads that file and the repository's own Git files on the machine the agent runs on and asks the daemon nothing, so a saved change applies from the agent's next tool call without a restart.
A project removed from Hide loses its protections, and a project turns them on again only from Project Settings.

What each protection refuses, in any checkout of the project and in any agent session there, a Herdr pane or not (B22): outside a pane the hooks and scripts run the guardrails and nothing else.

- Guard working folders: `git switch`, and a `git checkout` or `gh pr checkout` that changes branch, in the project's original checkout; a file tool's write into another checkout of the same repository than the one the agent works in.
- Guard the base branch: a file tool's write, `git commit`, `merge` (a fast-forward too), `rebase`, `cherry-pick`, `revert`, `am` and `pull` (each but its `--abort`, `--quit` or `--skip` forms, and `pull --ff-only`, which takes only the remote's own commits) in a checkout that has the base branch checked out, and a `git push` whose destination is the base branch (`--all`, `--mirror`, an explicit refspec, a pattern refspec such as `refs/heads/*:refs/heads/*`, or the current branch); with its merge option, `gh pr merge` as well (`-R` and `--repo` read as the option they are).
- Block risky Git commands: `git push --force` or `--mirror` without a lease and `--no-verify` on a push or commit, always; `git reset --hard`, `git checkout -- <path>`, `git checkout <tree> -- <path>` or `-f`, `git restore` of the worktree and `git switch -f`, only while the checkout has uncommitted changes to tracked files, and `git clean -f` only while it has untracked ones (`git -c core.fsmonitor=false --no-optional-locks status --porcelain`, the one Git process the guardrails start, once per checkout and call, with a 2 second budget, no `GIT_*` variable of the hook's and no fsmonitor command the repository configures, since the hook runs with the operator's rights).
- Guard files: a file tool's read or write of a path on the project's list, and a shell call that names one as a word or a redirect target; `.env.*` leaves out `.env.example` and `.env.sample` unless the list names them too.
  A word with a space in it that is text, the value of an option that takes text in `git`, `gh`, an interpreter, a search or a stream editor (`-m`, `--message`, `--title`, `--body`, `-c`, `-e` and their `=` forms, never another program's `touch -m` or `curl -c`), what `echo` or `printf` prints, or the pattern or script `grep`, `rg`, `sed`, `awk` or `jq` takes first, is read as a path only when it names something that is there; any other such word is a path, which `cp`, `touch` or `tee -a` may be about to make.
  Programs are known by their exact file name, so one spelled otherwise (`python3.12`, `sudo git`) has its text read as a path, which refuses a protected name rather than letting one through.
  The list holds at most 32 entries of at most 200 characters, each a gitignore pattern relative to the checkout or a `~/` path under the account's home, which the agent's own project lists wherever a `cd` in the line went; the default is `.env`, `.env.*`, `*.pem`, `*.key` and `~/.ssh/`.
  The first 512 candidate words of one shell call are checked; past them the words go unchecked, the `git` and `gh` commands after them are still judged, and one `guard.word_limit` line per ten minutes says so.

A plain folder project takes only Guard files.
`git checkout <word>` reads `<word>` as Git does: a local branch of that name first, then a path that is there, then a commit; anything after `--` is paths.
A repository nested in a checkout, a submodule or a clone, is a folder of the checkout around it, so its files take that checkout's protections and no protection locks an agent out of it; a `git` or `gh` command run there works on the nested repository, which no protection guards; only a project whose own repository cannot be read refuses, and then only a write or a `git` command a protection that is on needs the repository for.
Paths are compared after links are followed, and with or without case as the folder compares names, judged from the entries the folder already holds so the hook writes nothing there, and taken as ignoring case when no entry tells (B24).
A `cd` or `pushd` earlier in a shell line moves where the commands after it run (a bare `cd` goes home), and `git -C <dir>` moves one command; the line is read as the [spawn guard](#the-spawn-guard) reads it, a POSIX shell's words one simple command at a time with quotes, heredoc bodies and comments set aside.
A PowerShell command's backslash path (`.\.env`, `C:\repo`) is therefore not resolved: on Windows the file tools and the `git` and `gh` commands that name no backslash path are guarded, and the rest is the gap [INSTALL.md](INSTALL.md#what-each-system-supports) records.
In Git Bash, the shell Claude Code runs on Windows, a backslash path is read as that shell runs it: quoted (`git -C 'C:\repo'`) it is the path, unquoted its backslashes are escapes, there and in the hook alike; a drive mount (`/c/work/...`, `/cygdrive/c/...`) is the drive's path (`hide_platform::path::shell_drive_path`).

The refusal reaches the agent in English, in its runtime's deny shape, and names the protection, what the call did and what to do instead: `Not run: the "Guard files" protection is on for this project in Hide. This call reads or changes a path on the project's protected list. Ask the operator ...`.
Each refusal appends one `guard.refused` line (rule, whether a fact was unread, runtime, pane; never the command, its paths or a file's contents) to `~/.hide/agent-hooks/spawn-guard.log`, and the operator's screen shows nothing for it.

How it fails (D-27):

- A fact a protection that is on needs cannot be read (the checkout's repository, its branch, the base branch, whether it has changes): the call is refused and the reason says which fact; with no base branch to read (Auto, and no `origin/HEAD`), the reason asks the operator to choose one in Project Settings.
- The policy file is missing, unreadable, corrupt or of another version: the call runs, and one `policy.unreadable` line with its cause is appended to the same log, throttled to one per ten minutes through the delivery diagnostics' store (cause `guardrails`).
- The hook panics, or the payload does not arrive within 0.5 seconds, while some project has a protection on: the call runs, and one `guard.failed` line with its cause (`panic`, `payload_deadline`) is appended, throttled the same way (cause `guardrails_failed`).
  A hook the runtime kills at its own timeout leaves no line, since the process is gone, and the call takes that runtime's default.
- A payload past the helper's 256 KiB read is judged by the tool name and the paths that arrived, a patch's file lines included; a shell command cut short is not judged.
- agy sorts its payload's keys, so a long file write is cut inside its new text, before the tool's name and target; such a write is refused in a project where a protection judges writes, since its target may be one the protection guards.

The payload each runtime sends, observed on the versions Hide installs into (`hide_agent_hooks::guardrails::payload`):

| Runtime | Reads | Writes | Shell |
|---|---|---|---|
| Claude Code 2.1.300 | `Read` | `Edit`, `Write`, `MultiEdit`, `NotebookEdit` (`file_path`, `notebook_path`) | `Bash` |
| Codex 0.162.1 | no read tool; the shell words only | `apply_patch` (the patch's file lines) | `Bash` |
| Grok 1.0.52 | `read_file`, `Read` | `search_replace`, `write`, `Edit`, `Write` | `Bash` |
| OpenCode 1.18.30 | `read` | `edit`, `write`, `apply_patch`, `multiedit` (`filePath`, `patchText`) | `bash` |
| OpenCode 2.0.29 | `read` | `edit`, `write`, `patch` (`path`, `patchText`) | `shell` |
| Pi 1.0.4, omp 18.7.0 | `read` | `edit`, `write` (`path`) | `bash` |

A key another version renames reads as no path, and the call runs.
Claude Code's and Codex's hooks select the file tools from matcher version eight (`hide-subagents@8`) and Grok's from `hide-guidance@5` (from `@6` their entries carry no matcher at all, so every tool call reaches the hook for the letters due while the agent works); OpenCode's plugin (version 2) and the Pi and omp extension (version 4) send a file tool to the helper only while some project of the machine has a protection on, read again when the policy file changes, so a machine with none starts no helper for a file tool (B27); a missing or unreadable policy sends the call in a Herdr pane, where the helper records why, and starts nothing outside one.
A session outside a pane, or an agent started in another agent's shell, says so (`guard_only`), and the helper then runs the guardrails alone, whatever pane variables the shell inherited.
A command hook pays one policy read per call and parses the payload only when some protection is on ([PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-guardrails-on-a-pre-tool-call)).
`hide-agent-hooks/src/guardrails/tests.rs` holds each runtime's payload and each protection's table tests.

## OpenCode: Hide's plugin

OpenCode has no command hook; its official extension point is a JavaScript plugin whose hooks the SDK types (`@opencode-ai/plugin`) declare ([plugins](https://opencode.ai/docs/plugins/)).
Hide writes into OpenCode's global plugin folder `~/.config/opencode/plugins/`, beside Herdr's `herdr-agent-state.js`, and nothing else of OpenCode's: `hide.js`, the plugin OpenCode 1.x loads, and the folder `hide-opencode/` with `server.js` and `tui.js`, the plugin OpenCode 2 loads ([OpenCode 2](#opencode-2)).
OpenCode 1.x loads only the files of the folder, so it never sees `hide-opencode/`, and OpenCode 2 loads `hide.js` as a plugin that does nothing.
`hide-agent-hooks/src/opencode.rs` declares the three files and `hide-agent-hooks/src/plugin.rs`, the owned script files they share with Pi's and omp's extension, writes, judges and removes them together; their sources are `hide-agent-hooks/src/opencode/{plugin,server,tui}.js` with this build's helper path filled in.
Each file's first line, `// <hide-opencode-plugin|hide-opencode-server|hide-opencode-tui>@<version> sha256=<hash of the rest>`, proves the file is Hide's, carries its version and shows whether it was edited; `hide.js` and `server.js` hold `const HELPER = "<absolute path of this build's hide-agent-hooks>";`, and `tui.js` takes it from `server.js`.
OpenCode's configuration folder is never created: with no `~/.config/opencode` the row reads Absent and says OpenCode has not created it yet, and only the `plugins/` folder inside an existing one and Hide's `hide-opencode/` inside that are made.

On OpenCode 1.x the plugin does nothing outside a pane Hide manages: with no `HERDR_ENV=1`, `HERDR_PANE_ID` and `HERDR_SOCKET_PATH`, or no helper at its path, it registers no hook and starts no process, so OpenCode in an ordinary terminal runs as if the file were not there.
Inside one it uses three hooks, each calling `hide-agent-hooks opencode <start|prompt|confirm|tool|subagents>` with one JSON object on stdin and one answered on stdout; the helper always exits 0 and a failure answers `{}`:

- `chat.message` adds Hide's text to a root session's prompt as one synthetic text part wrapped in `<system-reminder>`: the session guidance (the same text the `SessionStart` hook gives, read once when OpenCode loads the plugin) on the session's first prompt, then Project Memory under Claude Code's rules (the session-start index with the first prompt's cards on the first prompt, the prompt's cards after), then the pane's waiting letters ([docs/delivery.md](delivery.md)).
  OpenCode's TUI does not show a synthetic part, and the title and label readers skip it, so the operator's text stays theirs; a subagent's prompt and a prompt of only synthetic text get nothing.
  The part's id is made the way OpenCode makes its own (`prt_` and a time-ordered id), so it sorts after the operator's text.
- `event` confirms letters once `message.part.updated` reports Hide's part stored, and counts subagents: a `session.created` with a parent is a child, `session.status` busy and idle move it between working and done, and a root that goes idle while a child still runs in the background keeps it counted (OpenCode's `session.status` list is read then).
  The counts go to the pane's counter file and to Herdr as the Claude Code and Codex hooks report theirs.
- `tool.execute.after` asks, for the pane's root session at every result except while letters it handed over wait for their receipt, for the letters due while the agent works (`letters`, which runs `hide inbox --hook --session <id> --tool`), appends them to the tool's `output` inside `<system-reminder>`, and confirms them through `event` once `message.part.updated` reports that session's completed tool part stored with the letters still in its output; a part stored without them leaves the letters pending in Hide for the next ask ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); nothing due, a failed helper and a child session leave the result as it is.
- `tool.execute.before` sends a `bash` call whose command mentions `herdr` or `HERDR_BIN_PATH` to the spawn guard and every `question` call to the Factory question guard (runtime `opencode`), and throws the deny reason; OpenCode hands a thrown error to the model as the tool's error, so the model reads the same reason Claude Code's model does.
  While some project of the machine has a protection on, every `bash` call and the file tools `read`, `edit`, `write`, `apply_patch` and `multiedit` go to the helper too, with only the arguments that name a file (`filePath`, a patch's `*** ` lines), for the [project guardrails](#project-guardrails).
  The plugin reads the policy file again only when its modification time or size changes, so a machine with every protection off starts no helper for them (PRD agent-guardrails B27).
  Any other tool call never reaches the helper.

Each hook has a budget: 1.85 seconds for a prompt (the Claude Code prompt hook's), 2.5 seconds for a tool call (the spawn guard's), 1.5 seconds for a tool result's letters, 2 seconds for a confirmation or a count and 8 seconds for the one guidance read at load.
OpenCode waits on a hook without a limit, so the plugin enforces them: past the budget the helper is killed, the prompt goes on unchanged and the tool call runs.
At most eight helpers run at once; a call skipped at that cap or given up on is counted and passed on the next call, which records it in the private delivery diagnostic (cause `plugin`), since the plugin has no log of its own and a hook's output reaches nobody who could act on it.

### OpenCode 2

OpenCode 2 (2.0.29) refuses a plugin module without a default definition (`{ id, setup }`), and runs its plugins in one shared server that every terminal attached to it talks to, so a plugin there is no pane's: the server carries the environment of whichever terminal started it.
`hide.js` exports one default definition both versions read: OpenCode 1.x runs its `server`, the plugin above, and OpenCode 2 finds a `setup` that does nothing.
OpenCode 2 loads `hide-opencode/` as one plugin of two halves, which talk over OpenCode's plugin RPC (`hide.opencode`: the methods `attach`, `detach` and `answer`, and the event `ask`):

- `server.js`, in the server, holds the hooks (the session's `prompt` hook, the tool's `execute.before` and `execute.after`, and OpenCode's event stream) and runs no helper for a pane: it asks the terminal half and waits for the answer within the hook's budget.
- `tui.js`, in the terminal, carries its pane's environment: it attaches to the root session the terminal shows (a subagent's session shows its root's) and lets it go when the terminal shows none, as on the home screen, answers each question by running the helper in its pane, and keeps the pane's subagent counts from OpenCode's events as 1.x's `event` does.
  It attaches again every 5 seconds while it shows the session; a terminal that opens a session takes it over, and one that only renews never takes it from another terminal still attached, so of two terminals showing a session the one that opened it last answers.
  Outside a pane Hide manages it attaches nothing.

OpenCode 2 runs one instance of the server half per project folder and sends a plugin call to the instance of the folder it names, so the terminal half attaches and answers at the folder of the session (`location`); an answer sent without it would reach no question.

- A root session's prompt asks the terminal attached to it, or any terminal now showing it (the prompt that opens a session reaches the plugin before the terminal's route is read again), and writes the answer, the guidance on the session's first prompt, Memory and the letters due, as one synthetic message (`session.synthetic`) inside `<system-reminder>`, which OpenCode admits ahead of the prompt and the operator's screen leaves out; its letters are confirmed once `session.inbox.delivered` names that message.
  A confirmation still on its way is waited for first, within the prompt's budget, and an answer carrying a letter Hide already took writes the guidance alone, so a letter is not written twice; a letter whose confirmation failed stays pending in Hide and comes with a later prompt, as in 1.x.
- An attached root's completed tool result asks for the letters due while it works, appends them to the result's content, and confirms them once `session.tool.success` reports the result stored with them; a result stored without them leaves them pending in Hide, as in 1.x.
- A tool call asks the attached terminal for the spawn guard (a `shell` call whose command mentions `herdr` or `HERDR_BIN_PATH`), the Factory question guard (`question`, asked as its root session's) and, while some project has a protection on, the [project guardrails](#project-guardrails) for `shell`, `read`, `edit`, `write` and `patch` (only `path` and a patch's `*** ` lines); the deny reason is thrown, as in 1.x.
  A `shell` call's relative `workdir` is resolved against the session's folder, where OpenCode runs it (1.x's against its project folder too), never the shared server's own.
  A session whose line OpenCode does not tell within 300 ms is taken as its own root, as 1.x takes it, and any terminal showing its root is asked.
  With no terminal attached, or none answering in time, the server half runs the guardrails alone (`guard_only`), with the pane variables it inherited removed, so a protected project stays protected in `opencode run` and outside Herdr; the spawn and question guards then let the call run, as a timed-out helper does in 1.x, and the miss reaches the helper's diagnostic with the next call.
  A slow answer never detaches a terminal; one that stopped attaching for 10 seconds (two renewals; a killed terminal says nothing on its way out) is asked no more.

The budgets and the cap of eight running helpers are 1.x's: the server half's wait spends the hook's budget, and the terminal half keeps 150 ms of it for the answer's way back.

No bell rings for OpenCode: on 2026-10-07 its previous-session picker read `done` and a bell's Enter there opened another session, so its letters ride its next prompt instead and are confirmed only once OpenCode stored that prompt ([docs/delivery.md](delivery.md#safe-intake-and-manual-fallback)); while it works they ride a tool result (above).
Claude Code's hook run inside OpenCode, through an operator plugin that bridges `~/.claude/settings.json`, ends silently (`ForeignOrigin::OpenCode`), so letters and counts are never taken twice.

The kit row reads the plugin as Claude Code's hook part is read: Installed when every file is this build's and unedited; Outdated with the reason when one is another build's, names a helper that is gone, is an older version or was edited; Removed when it was installed and taken out; Off when OpenCode is switched off.
The three files are judged, written and removed together, so a pass never leaves one version's half beside another's: the row reads as its worst file, an older or another build's unedited plugin is replaced by the next pass (a `hide.js` an earlier build wrote alone gets the `hide-opencode/` folder beside it), an edit in any file keeps all three until Reinstall, and a switch-off removes the unedited files and then `hide-opencode/` unless something else is in it.
A file without Hide's marker at one of the three paths is not Hide's: it is never replaced or removed, and Hide writes none of the other two beside it.
The plugin is written on Windows too, where OpenCode runs on Bun as it does elsewhere.
On a `windows-latest` runner on 2026-10-11, OpenCode 1.18.35 (Bun 1.3.14) loaded a plugin from `%USERPROFILE%\.config\opencode\plugins`, and the plugin's own call, `spawn` of an absolute `.exe` path with no shell, wrote its stdin, read its stdout and killed it with `SIGKILL` there.
`hide-agent-hooks/tests/it/windows_scripts.rs` runs this build's plugin, `hide.js` and OpenCode 2's two halves with one terminal, in Node on Windows against the real helper in a folder whose name has a space, a quote, brackets and a `$`, requires the session guidance to reach the first prompt, and runs a tool result through it: the helper's `letters` ask reaches for the `hide` beside it, records in the home's delivery diagnostics that none is there, and the result goes on unchanged; a letter reaching a result is proved by the Unix fake-host test, since no daemon runs on the runner.
Memory receipts are read back from the OpenCode session's synthetic user parts by the conversation read (`hide-session`, `LabelTranscript.memory_receipts`): the label worker's for a session a live pane holds, the Memory pass's for any other (the same read shaped as a conversation page, where each is a provider-injected event at its message's index), and recorded off the core lock by the one function the Claude and Codex transcripts' go through.

## Pi and omp: Hide's extension

Pi and omp have no command hook; their extension point is a module in the user extensions folder whose default export registers handlers for the host's events ([Pi extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [omp extensions](https://omp.sh/docs/extension-authoring)).
omp is a fork of Pi with the same extension shape, so Hide writes one source, `hide-agent-hooks/src/pi_extension/extension.js`, as `hide.ts` into `~/.pi/agent/extensions/` and `~/.omp/agent/extensions/`, beside Herdr's `herdr-agent-state.ts` and `herdr-omp-agent-state.ts`, and nothing else of either agent's (PRD pi-omp-extension D-01).
`hide-agent-hooks/src/pi_extension.rs` declares both files on the owned script file of [OpenCode's plugin](#opencode-hides-plugin), so they are written, judged and removed by the same code.
The first line, `// hide-extension@<version> sha256=<hash of the rest>`, works as the plugin's does; the body holds `const HELPER`, `const AGENT = "pi"` or `"omp"` and `const VERSION`, and the two files differ only in `AGENT`, so the other agent's file copied into a folder reads Outdated and is replaced.
The file is plain JavaScript that is also TypeScript, as Herdr's own extension for these agents is, so both hosts load it without a build step.
It is written on Windows too: on a `windows-latest` runner on 2026-10-11, Pi 1.0.4 (Node 22) and omp 18.7.0 (Bun 1.3.14) loaded an extension from `%USERPROFILE%\.pi\agent\extensions` and `%USERPROFILE%\.omp\agent\extensions`, emitted `session_start` to it, and its `spawn` of an absolute `.exe` path with no shell crossed stdin and stdout and ended with `SIGKILL`; `hide-agent-hooks/tests/it/windows_scripts.rs` runs this build's file for both on Windows as it does OpenCode's plugin, with the session file named by a `C:\` path as Windows reports it.
A session file counts as saved when it is absolute in either system's spelling (`path.posix.isAbsolute` or `path.win32.isAbsolute`, the check Herdr's own Pi integration makes), so the same string is judged alike on every system; a relative path is an unsaved session.
The agent's own folder (`~/.pi/agent`, `~/.omp/agent`) is never created, and only those default folders get the file: an omp profile or a `PI_CODING_AGENT_DIR` folder gets none.

The extension does nothing outside a Herdr pane (no `HERDR_ENV=1`, `HERDR_PANE_ID` and `HERDR_SOCKET_PATH`), in an agent started from another Pi's or omp's shell (`PI_SESSION_ID`, `OMPCODE=1`, which those hosts set for every shell they start), or with no helper at its path; like OpenCode's plugin it does not ask whether a Hide daemon owns that Herdr, and the helper answers `{}` when none does.
Its guidance and letters go only to the pane's own agent, the host's TUI (`ctx.mode` `tui`): both hosts' RPC modes report a UI too but draw nothing in the pane, and another program there may start one, so they, print runs, unsaved sessions and omp's subagents, which run in the same process with their own binding of the module, get none.
The spawn guard holds for every session in the pane, and omp's `ask` guard for the pane's TUI session and its subagents.
It names the session by its file, as Herdr's integration does, which a deep checkout makes longer than 250 bytes; the helper, the CLI and the daemon take a session of up to 4096 bytes for a letter pull and a Factory question (`hide_agent_hooks::delivery::SESSION_LIMIT`), and an agent registration keeps the same ([delivery.md: Agent registration and spawning](delivery.md#agent-registration-and-spawning)).
Each handler calls `hide-agent-hooks <pi|omp> <start|prompt|confirm|tool|subagents>` with one JSON object on stdin, which carries the extension's version, and one answered on stdout; the helper always exits 0, a failure answers `{}`, and a version this build does not write also answers `{}`, so a file another build left does nothing until the kit replaces it.

- `session_start` and `session_switch` read the session guidance once, as OpenCode's plugin does at load; a read that failed is asked again by the next prompt.
- `before_agent_start` returns one custom message for the prompt, wrapped in `<system-reminder>`: the guidance until a message that carried it has been written, then Project Memory under Claude Code's rules (a session-start index, asked for on each prompt until one is given and the message carrying it is written, tracked apart from the guidance so it is neither repeated nor held back by it, then the prompt's cards on each later prompt; unlike Claude Code's one `SessionStart`, a session whose first prompt found Memory off or busy gets its index once Memory answers), then the pane's waiting letters ([docs/delivery.md](delivery.md)).
  Letters know the session by its file and Memory by the host's own session id (`sessionManager.getSessionId()`), which is what Pi's and omp's session reader keys a receipt by; the extension sends both.
  The message is `display: false`, so the host keeps it off the screen and out of the title while the model reads it, and a letter cannot close Hide's reminder tags.
  Its `details` carry a fresh id per attempt, because omp can prepare one submission more than once and keeps only the attempt it delivers.
- `message_end` confirms the letters once the host has written Hide's message and then the reply after it, and only for the attempt it wrote; a turn that ends before any reply leaves them pending for the next prompt or `hide inbox`, and a confirmation Hide did not take is made again when the letter rides the next prompt.
  A tool result's letters (`toolCallId`) join the written list when the host finishes that message with Hide's text still in its `content` and are confirmed by the reply that follows, since a later `message_end` handler may replace the result; a result finished without the text leaves them pending in Hide for the next ask.
- `tool_result` asks, for the pane's own agent at every result except while letters it handed over wait for their receipt, for the letters due while it works (`letters`, which runs `hide inbox --hook --session <file> --tool`) and adds them to the result's `content` as one more text block inside `<system-reminder>`, keeping a tool's `structuredContent` ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); nothing due, a failed or late helper, a print run, an unsaved session and a subagent leave the result as it is.
- `tool_call` sends a `bash` call whose command mentions `herdr` or `HERDR_BIN_PATH` to the spawn guard and, in omp, every `ask` call to the Factory question guard (runtime `omp`, with the pane's root session also for a subagent's call), and returns `{block: true, reason}`, which the host hands to the model as the tool's result.
  While some project of the machine has a protection on, every `bash` call and the `read`, `edit` and `write` tools (their `path`) go to the helper too, gated on the policy file as OpenCode's plugin is, for the [project guardrails](#project-guardrails).
  Pi has no question tool, so its Factory question is the Hide command's alone.
- In omp, `before_subagent_spawn` counts a subagent as working from its dispatch (its spawn key is the subagent's id), and that subagent's own `agent_start` and `agent_end` move it between working and done; the counts go to Herdr as the other hooks report theirs, zero when the session starts and again at each turn's end.

The budgets, the cap of eight running helpers and the `plugin` diagnostic cause are OpenCode's plugin's; the `plugin` cause also records a prompt whose host gave no usable session id for Memory.
Both hosts wait on a handler far longer (omp blocks the tool call after 30 seconds), so the extension enforces them: past the budget the helper is killed, the prompt goes on unchanged and the tool call runs.
Every callback catches its own error, since a throw from a timer callback ends an omp session.
No bell rings for Pi or omp: on 2026-10-07 their resume pickers read `done`, as OpenCode's did, so their letters ride the next prompt, or a tool result while they work ([docs/delivery.md](delivery.md#safe-intake-and-manual-fallback)).
The host stores Hide's message in the session file as a `custom_message` entry of `customType` `hide`; the shared Pi and omp reader reads that entry, and no other extension's, as provider-injected context, so the Memory pass finds the receipt in it as it finds the Claude and Codex transcripts' (PRD pi-omp-extension D-09).
The kit row reads the file as it reads OpenCode's plugin, and Settings names the piece Extension.

## Other agents: skill and guidance hook

On a machine where the kit has never run (no `~/.hide/kit/installed.json`), the default-on agents (Claude Code, Codex and agy) are recorded off in the same pass and the record is marked `awaiting_choice`, so nothing is written to any agent until the operator answers the first-run agent choice, the first step of the first-run wizard (UI_BEHAVIOR.md, First run).
The hold records the three default-on agents off explicitly, whether or not they are installed (operator decision D7): an agent installed later appears in Settings, Agents off and is never turned on by a pass, and the operator switches it on there.
An explicit agent choice in the same pass wins over the hold, and any explicit choice (a scope that names an agent, which the first-run answer always does, naming the unchosen default agents off) clears the mark; an existing install, a record without the field, is never held.
The mark lives in the same record as the choices, so it outlasts a quit between the hold and the answer, and `held_for_onboarding` in the report reads it on every pass.
The core follows it (`ui_state.agent_onboarding`), applies the answer to this Mac and to every device whose own record waits, and sends the saved choice once per run to a device that reports waiting later; one that still waits afterwards is logged, not asked again on every report.
The same step reopened from Help > Set up Hide, once the question is answered, sends only the agents whose switch it changed, as Settings > Agents does, so it never rewrites an agent the operator left alone.

Claude Code and Codex are the agents the kit has always had a hook for; OpenCode has Hide's plugin, Pi and omp Hide's extension, and agy (the Antigravity CLI) Hide's block in its `hooks.json`.
Hide supports seven agents: Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity, in that order.
Every agent is one row of `hide-agent-adapter/src/declarations.rs` (`ADAPTERS`), and one switch per agent per machine turns its pieces on and off, in Settings, Agents and in each device's row.
`hide-kit/src/agents.rs` projects those declarations into installation pieces; it does not maintain a second support table.
A row carries the agent's program names (`executables`), the folder it reads skills from, whether Hide writes a hook for it, Herdr's integration name for it, and the official page the row's answers come from (`doc_url`).
A test fails a row with no `https` `doc_url`, and a row with no program, so a claim in the table below always has a page behind it and every agent can be found.

### Which agents Hide supports

Hide supports an agent only when the pinned Herdr ships an integration for it (`herdr integration install <target>`, listed by `herdr integration status`).
Lineage, the mailbox identity, labels and sleep all read the session id that integration gives Herdr, and an agent without one could only have its state judged from its screen, a second kind of row that behaves unlike every other; so every supported agent has a Herdr target, but a Herdr integration alone does not grant Hide support (Gemini CLI left for that reason and is retired below).
`AgentAdapter::herdr` is therefore required, not optional.
Among the supported agents, one gets the multi-agent collaboration tier (letters, the spawn guard, Memory and the subagent count) when its official documentation or SDK types give a hook or plugin that can both put text into the prompt and refuse a tool call; today that is Claude Code and Codex, through their native hooks, OpenCode, through Hide's plugin, Pi and omp, through Hide's extension (Pi without a subagent count, since Pi runs none), and Antigravity, through Hide's `hide` block ([Antigravity: Hide's hook block](#antigravity-hides-hook-block)), with no bell yet.
Grok keeps the Basic tier: its hook carries the spawn guard and subagent count, takes letters through its tool and turn-end events and is a bell target at rest.
It carries no new operator prompt's letters or Memory.
Adding an agent to the list checks, in order: the pinned Herdr lists a target for it and which folder that target needs; the vendor's documentation confirms the folder it reads skills from and the name of the program it installs; the vendor publishes a mark (`docs/BRAND.md`) or the row draws a monogram; and whether its hooks or plugins meet the collaboration tier above.
It is then one shared declaration, its contract fixtures, the logo manifest, and the expected kit case; the web shell reads the generated `contracts/agent-adapters.json` rather than maintaining its own order, names or links.

### Adding an agent

Prove it on the real agent first, before any code, and pin what you measure with a test:

- A vendor document is a lead, not a fact.
  On 2026-10-16 agy 1.4.0 ignored `ANTIGRAVITY_CLI_CONFIG_DIR`, and its `/skills` listed a folder whose skills its model never received.
  Prove a skill by asking the model for its own list of skills, not by reading the agent's listing.
- Measure the hook failure matrix for the agent: a missing helper, a helper that is not executable, a crash, a non-zero exit, empty or malformed output, and a timeout.
  agy refuses the tool call on every one of them but an empty answer with exit 0 (measured 2026-10-16 and 2026-10-17), so each case has a test that the installed command still answers (`guidance::tests::agys_tool_command_leaves_the_call_to_agy_when_the_helper_is_gone_cannot_run_or_fails`).
- Measure how the pinned Herdr reads the agent's permission and folder-trust prompts before relying on Needs You or the bell.
  On 2026-10-16 a probe outside the app read agy's permission question as `idle`, and the running app then showed Herdr 0.9.3 reading it as an approval; measure in the app, and its folder-trust question is still unmeasured, which is why agy has no bell.
- Record the agent version every fact was proven on.
  agy moved from 1.3.6 to 1.4.0 on 2026-10-16, during the work, so every fact proven on 1.3.6 had to be proven again on 1.4.0.

Add the declaration in `hide-agent-adapter/src/declarations.rs`, keeping its canonical id stable across records, hook commands and device reports.
Aliases belong on that row: every caller uses the same borrowed, case-insensitive lookup, including surrounding-whitespace normalization.
Select hook and installation dialects implemented by `hide-agent-hooks`, session formats implemented by `hide-session`, and launch, find and usage dialects in their existing owners; the leaf crate contains data and typed selectors only.
Declare prompt intake, spawn refusal, subagent counts, Memory and bell eligibility independently.
A bell target requires a prompt hook and measured menu behavior on the pinned, isolated Herdr; adding prompt intake alone never enables its bell.
Declare the six Factory capabilities explicitly as available, unavailable or unconfirmed; declaring a slot does not implement a question guard, parse structured questions or change Factory's runtime selection.
Add independent hook and session samples, or an explicit absence reason when that format is unsupported, in `hide-agent-adapter/tests/fixtures/support.json`, and add the logo or monogram to its manifest.
Add the owner's dialect tests and feature-gate cases, including aliases, without changing the pre-migration hook-byte fixtures.
Generate `contracts/agent-adapters.json` with the `export_web_contract` example as described in [contracts/README.md](../contracts/README.md#agent-adapters).
The leaf contract tests reject missing samples, links, logos, inconsistent capability combinations and generated-contract drift, and the existing feature and isolated-Herdr e2e suites must still pass.

### What the kit puts down

Two pieces are written per agent into the agent's own files, and nothing else; a third, Herdr's integration, is put in through Herdr's own CLI (below):

- The skill stub `hide-browser/SKILL.md` in the folder the agent reads.
  It is a few lines that point at `hide browser help`, so it stays right as the CLI's guide changes.
  Its folder is `~/.agents/skills` for the agents that read it, `~/.claude/skills` for Claude Code (which documents that it does not read the shared folder), and `~/.gemini/config/skills` for agy, whose model is given neither of the others: agy 1.4.0 lists skills in its documented `~/.gemini/antigravity-cli/skills` under `/skills` but does not give them to the model, while those in `~/.gemini/config/skills` reach it (measured on 2026-10-16), so Hide writes there and never in `~/.gemini/antigravity-cli/skills`, where the operator's own links stay; agy's desktop app shares `~/.gemini/config`, so it sees the skill too.
  A shared folder is written while any agent that reads it is on and installed, and it is removed only when none is.
  A file is Hide's only when its marker line, `<!-- hide-skill@<version>: ... -->`, is the first line after the front matter; a file that merely mentions `hide-skill@` is never replaced or removed, and the agent's row says a skill that Hide did not write is already there.
  A stub with Hide's marker over text that is not Hide's was edited by the operator: no pass rewrites it and a switch-off leaves it, the row reads Outdated with that reason, and only Reinstall puts Hide's text back.
  A stub of an older marker version is Hide's own and is replaced by the next pass.
  An agent's own folder (`~/.claude`, `~/.gemini/config`) is never created for the stub: the hook code reads that folder as the agent's settings being there, so a pass that made it would write a hook on the next pass that it did not write on this one, and a second apply would not be a no-op; the row says the agent has not created its folder yet, and the stub goes in on the pass after it has.
- The agent hook, for the agents below marked as done; for OpenCode this piece is Hide's plugin, and for Pi and omp Hide's extension, which carry the guidance with the rest ([OpenCode: Hide's plugin](#opencode-hides-plugin), [Pi and omp: Hide's extension](#pi-and-omp-hides-extension)).
  It is one entry per event the agent's documentation names, in the agent's own format, whose command is `hide-agent-hooks hook --runtime <agent id> --event <event>`.
  `hide-agent-hooks` writes it (`src/guidance.rs`) and nothing else does, under the per-agent marker (`hide-guidance` with the active agent's version) that proves an entry is Hide's and separates a current one from an older one.
  These entries capture the installing daemon's state route; an earlier owned entry is replaced by the next pass.
  Antigravity retains the encoded Windows command introduced in version 7 across its Go-to-cmd boundary.
  Where the session start adds context (Antigravity), its output is the worktree-purpose instruction, including the sentence that points at `hide factory add`, one fixed line that points at `hide browser help`, and the live Workspace guidance when the daemon answers, in the field the agent documents.
  A second delivery of the same session prints the same guidance: the output is a pure function of the daemon's answer, and a test runs the hook twice and compares.
  The other events carry the spawn guard, and Grok's subagent count ([Grok's own hooks](#groks-own-hooks)).

Cursor is retired from support, onboarding, new installation and provider selection.
The kit removes only its recorded, Hide-marked hook entries, including older event layouts; operator and other tools' entries stay.
A recorded `herdr:cursor` integration is uninstalled through the existing Herdr owner boundary before ownership is forgotten.
An uninstall failure retains that record for the next pass, including full kit removal.
The shared `hide-browser` stub stays while any live enabled agent reads it, and stored Cursor conversations remain readable.

Cursor also loads Claude Code's hooks from `~/.claude/settings.json` (and the project's `.claude/settings*.json`) when "Include Third-Party Plugins, Skills, and Other Configs" is on, which is its default, and merges them with its own at the lowest priority without removing a duplicate ([third-party hooks](https://cursor.com/docs/reference/third-party-hooks)).
So on a machine where Claude Code is on in Hide a Cursor session would also run Hide's instrumented Claude Code hook (the pane counters and reports as `claude-code`, Memory), which is not what a Cursor session is.
The compatibility exclusion remains after retirement: `hide-agent-hooks hook --runtime claude-code` prints nothing and counts nothing when `CURSOR_VERSION` is in its environment, the variable Cursor documents as set for every hook it runs.
Cursor's page on third-party hooks does not say whether those get the variable, so this rests on the documented one; a session where it is missing would run both hooks, which print different fields and count only for `claude-code`.
Grok runs `~/.claude/settings.json` and `~/.cursor/hooks.json` beside its own hook files, so inside Grok (`GROK_HOOK_EVENT`, which Grok sets for every hook it runs and for nothing else; `GROK_SESSION_ID` can also reach a Claude Code started from a Grok shell, which keeps counting) Hide's Claude Code hook and Hide's Cursor entries end without a word or a count, and Hide's Grok file is the one that counts and refuses (PRD grok-cursor-hooks D-05).
Grok's session start is passive, so the guidance Claude Code's hook printed there never reached the model and nothing is lost.
OpenCode can run the hooks in `~/.claude/settings.json` too, through an operator's bridge plugin; it has Hide's own plugin, so Claude Code's hook ends silently as a whole there, as it does in Cursor, and nothing is counted or delivered twice (PRD opencode-plugin D-10).
What Claude Code's hook never does inside another agent's session is take or confirm letters: a letter is addressed to the pane's own session and is confirmed once that session has seen it, so a hook that runs inside another agent's session would take the letter and confirm it to nobody who reads it (PRD settings-cleanup D-25).
`hide_agent_hooks::runtime::ForeignOrigin` finds such a session by what its agent sets for the processes it starts: `CURSOR_VERSION` for Cursor, `OPENCODE` or `OPENCODE_PID` for OpenCode, and `GROK_HOOK_EVENT` or `GROK_SESSION_ID` for Grok (for letters; only `GROK_HOOK_EVENT` silences the hook).
`hide-agent-hooks/tests/it/letter_origin.rs` runs the built helper beside a stand-in `hide` that answers `inbox` with one letter and records its calls, once outside and once inside each of them.
On Windows both agents run the hook's command as PowerShell, so Hide writes a PowerShell form for each ([Installing](#installing)).

### Grok's own hooks

Grok's native hooks carry the spawn guard, subagent count and letter routes.
Its events follow the hook guide Grok ships; retired Cursor layouts are retained only for ownership cleanup.
Grok's `SessionStart` does not fire for a subagent, so it is where the pane's count starts over and the pane first reports itself instrumented.

Grok's file is Hide's own, `~/.grok/hooks/hide.json`, beside Herdr's and Orca's files in that folder (`{"hooks": {"<Event>": [{"matcher"?, "hooks": [{"type": "command", "command", "timeout"}]}]}}`, seconds); a switch-off deletes the file and nothing else there.
The folder `~/.grok/hooks` is created when `~/.grok` exists, and nothing is created when it does not: the row says Grok has not made its folder yet.
An older build's entries are replaced by the next pass; entries the operator edited read Outdated and stay until Reinstall, as OpenCode's edited plugin does.

The helper (`src/bin/hide-agent-hooks.rs`, `run_basic_hook`; the payload and answer shapes in `src/basic.rs`) reads Grok's documented payload:

| | Grok |
| --- | --- |
| Shell call | `toolName` `run_terminal_command`, `toolInput.command`, `cwd` |
| Refusal | Claude Code's envelope, `hookSpecificOutput.permissionDecision = "deny"`, which Grok takes |
| Nothing to refuse | no output |
| Letters due while the agent works | `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": <letters>}}`, which Grok's guide says the model reads with the batch's tool results (`sessionId` names the session) |
| Prompt submission | `UserPromptSubmit`: `prompt`, `sessionId` and/or `session_id`; observe only, no output |
| Turn end | `Stop`: `subagentType` marks a subagent's own stop, `backgroundTasks` lists what still runs, `stopHookActive` prevents another letter pull; due letters go in `{"decision":"block","reason":<letters>}` |

The refusal's reason and every rule of what is refused are the spawn guard's above, through the same `guard_refusal`.
A call that runs, in a Herdr pane and with a session in its payload, then asks `hide inbox --hook --session <id> --tool` for the letters due while the agent works and answers them as the row above shows, confirming them once the answer is printed ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); a refused call carries none, and the Pre entry has no matcher so every tool reaches that path.
The Post and failure entries select shell tools only and print no output; they read `toolUseId`, `toolInput`, `toolResult` and the native truncation flags for passive observations.
Grok's `UserPromptSubmit` reads `prompt` and the native session, sends only the trimmed prompt's digest and session to the core, and prints and confirms nothing, because Grok discards its stdout.
Its `Stop` reads the payload once for both the subagent count and turn-end letters; a child stop, missing session or malformed payload takes none.
It asks `hide inbox --hook --session <id> --turn-end`, prints `{"decision":"block","reason":<letters>}` when something is due, and confirms after the flush.
A Stop with `stopHookActive: true` makes no letter pull and allows the turn to end even if confirmation failed; it still updates the subagent count.
Prompt, tool and Stop intake share the session parser: either `sessionId` or `session_id` is accepted, equal pairs are accepted, and conflicting pairs take no letters and record `session_conflict` without either value.
The core recognizes a bell's turn and keeps the normal five-minute wait for letters arriving during an operator's turn ([delivery.md](delivery.md#safe-intake-and-manual-fallback)).
`GuidanceAgent::version()` is per agent: Grok and Antigravity write `hide-guidance@9`, retired Cursor layouts stay at 7 and the other retired layouts at 6 for removal only.
An older active entry reads Outdated until the kit replaces it with the captured state route.
The bell line names `hide inbox` for a session still running older hook entries.
Grok 1.0.62 keeps the loaded entries after the file changes; `/hooks`, then `r`, reloads them without restarting the session (measured with separate command tags before and after reload in an isolated session).

Legacy Cursor entries read output that is not a valid answer from `preToolUse` and `subagentStart` as a refusal even when the hook exits 0, and only the CLI's own code, not its documentation, says empty output proceeds, so for those two events (`subagentStart` only from a `@2` entry an earlier build installed and the next pass has not yet removed) every path that does not refuse prints `{"permission":"allow"}`: outside a pane or a registered checkout, an unreadable payload, a daemon that is down or slow, a panic, and a failed owner handshake before anything else runs (D-04).
Grok fails open on everything but a refusal, so its hook prints nothing on those paths, as Claude Code's does.
A failure goes to the guard's log, never to the agent (B5).
Cursor CLI 2026.10.01 reads a refusal on two paths: its local tool path (every shell call) shows the agent `user_message`, or "<tool> blocked by preToolUse hook" when there is none, and gives it `additional_context`, while only a `preToolUse` step the agent server asks for passes `agent_message` on; the reason is written to all three so it reaches the agent whichever path ran (issue 910, read from the installed CLI bundle).

The count is the same per-pane record and the same `pane.report_metadata` report as Claude Code's (above), changed under an exclusive lock on `~/.hide/agent-hooks/panes.lock` (private to the account), so subagents started in parallel are all counted; a report that finds the record changed by another event while it was sent sends the record again, so Herdr ends on the latest count, and a lock not taken within one second is written down as the pane's last report failure.
A subagent start adds one working, a subagent stop moves one from working to done.
Grok's `Stop` from inside a subagent leaves the count alone, and the main session's `Stop` sets working to the number of `subagent` entries in `backgroundTasks`, so a background subagent stays counted past the turn and its own `SubagentStop` ends it (D-06); a payload that is cut off or unreadable leaves the count alone.
The core judges a Grok pane instrumented by that agent's hook piece in the kit (`herdr_core::agent_hooks::kit_hook_status` for the agent's `HookDialect`, as for OpenCode's plugin), not by a Claude Code or Codex component.

Grok's question tools `ask_user_question` and `exit_plan_mode` are in its `PreToolUse` matcher, but the hook routes them to the Factory question guard ([Native questions from Factory workers](#native-questions-from-factory-workers)) only for an agent whose `direct_ask` the adapter table declares, and Grok's stays Unconfirmed until a live Grok shows `PreToolUse` firing for them; until then Grok's questions stay native.

`hide-agent-hooks/tests/it/grok_cursor_hooks.rs` runs the built helper with each agent's documented payloads beside a stand-in `hide` and a stand-in Herdr socket, and asserts the answers, the counts Herdr is told (none for Cursor), and a single count when Grok runs all three of Hide's hooks for one subagent.

The record `~/.hide/kit/installed.json` keeps the operator's choice per agent (`agents`) and the pieces Hide installed (`hook:<agent>`, `skill:<folder>`, `herdr:<agent>`), and an older build ignores them.
With no choice on record Claude Code and Codex are on, as they have been since their hooks became part of the kit, and every other agent is off.
A piece that was installed and is gone stays gone until Reinstall, and an agent switched off keeps nothing of Hide's and gets nothing back from a later pass.
Whether the agent is installed only decides whether a switch can work: an agent that is not installed on a machine has no switch there, and switching it on is not recorded.
A guidance hook also needs the agent's own settings folder, which Hide does not create: an installed agent that has not made it yet reads that, and its hook goes in on the pass after it has.

### Antigravity: Hide's hook block

agy reads every named block of `~/.gemini/config/hooks.json` beside the others, so Hide writes one block of its own, `hide`, and touches no other (Herdr's `herdr`, Orca's `orca-status` and the operator's stay byte for byte; PRD antigravity-first-class D-04).
The block holds five events in agy's own format (`{"hide": {"<Event>": [{"type": "command", "command", "timeout"}]}}`, seconds, and a group with a `matcher` for `PreToolUse`), with the original four-event behavior read from agy 1.4.0 and measured live on 2026-10-16; the added Post slot follows the documented contract:

| Event | What Hide answers |
| --- | --- |
| `SessionStart` (a conversation's first prompt, never a resume) | the guidance and the Memory index, as an injected system message (`injectSteps`) |
| `PreInvocation` (every model call; `invocationNum` 0 is a prompt's first) | on the prompt's first call the prompt's Memory cards and the pane's letters, the same way; on a later call of the turn, the gap after a tool, the letters due while the agent works ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)), the same way, confirmed once printed |
| `PreToolUse` (matcher `run_command\|view_file\|write_to_file\|replace_file_content\|multi_replace_file_content\|ask_question`) | `{"decision":"deny","reason"}` for a refused call, `{"decision":"ask"}` on every other path |
| `PostToolUse` (matcher `run_command`) | no output; passive observation with unavailable result because the native event supplies no stdout |
| `Stop` (one per turn of each conversation) | nothing; a subagent's own stop counts it done |

agy refuses a tool call whose `PreToolUse` hook fails (a nonzero exit, output that is not its JSON, a timeout, a missing program), unlike every other agent Hide supports; empty output with exit 0 lets the call run (agy 1.4.0, measured 2026-10-17).
In agy 1.4.0 only `deny` changes what agy does: `allow`, `ask` and an empty answer each leave agy's own permission question exactly as with no hook, asking before a command (`hide inbox` included) or a file write and not before a read (measured 2026-10-17 with no other block installed).
Hide answers `ask` because agy's hook contract documents `allow` as running the call without its question, and Hide never makes agy ask less than agy's own rules; for the same reason a hook cannot spare the operator agy's question before Hide's own commands, and agy 1.4.0's settings hold no command allow list (only `trustedWorkspaces`), so an agy pane asks before each `hide` command until the operator answers agy's own "always allow".
So the helper prints one of the two answers on every path, and the installed command prints the ask answer itself when the helper is gone or ends without answering (a binary that cannot run, a crash): `if [ -x <helper> ]; then <helper> ... || printf '%s\n' '{"decision":"ask"}'; else ...; fi`, and on Windows a PowerShell guard sent through `powershell.exe -NoProfile -NonInteractive -EncodedCommand` (D-05).
agy 1.4.2 starts Windows hooks through `cmd /c` with Go's argument escaping: embedded double quotes arrive as backslash-quote, which cmd does not unescape.
The earlier `if exist "<helper>"` guard therefore found no file and ran nothing, even though the hook block loaded; a direct cmd test without that escaping missed the defect.
Hide encodes the PowerShell script as UTF-16LE Base64, so paths and JSON answers cross that boundary without embedded quotes, and decodes it when checking ownership and the helper path.
The guard leaves stdin attached to the helper and prints the ask answer with exit 0 if the helper is absent, cannot start or exits unsuccessfully.
It adds no PowerShell input pipeline, so it does not add the BOM that Windows PowerShell 5.1 puts on a pipe.
The helper's existing ASCII JSON serializer escapes non-ASCII text before PowerShell sees stdout; JSON decoding restores Korean titles and non-BMP characters.
Only agy uses this Go-to-cmd boundary: Grok uses its PowerShell form directly, Claude Code uses the exec form and Codex its session shell, and the JavaScript plugins start the helper directly.
The Windows crate-boundary tests reproduce agy's escaping and assert guidance injection and conversation parsing from a real helper, Korean and non-BMP JSON answers, the missing and failed helper answers, and migration from the old block; the fake-model journey checks Memory and confirmed letters through the real agy CLI.
The helper's own waits end far inside the block's eight-second timeout: half a second for the payload, two and a half for the daemon's answer to a start or a question, two for `git status` before a discarding command, and a daemon that never answers lets the call run (`a_hide_that_never_answers_lets_the_call_run_well_inside_the_blocks_timeout` in `hide-agent-hooks/tests/it/agy_hooks.rs`).
agy is on by default like Claude Code and Codex, so these guarantees, not the operator's switch, are what keep Hide from ever being what blocks an agy tool call.
A subagent is a conversation of its own in the same process and pane, with its own `SessionStart` and `Stop` and no parent field; its transcript opens with the message its parent sent it, and that is how the helper tells a subagent's start and stop from the pane's own conversation; a subagent a subagent started is counted under the pane's own conversation too, found by following those first messages up at most eight levels (`hide_agent_hooks::antigravity::root_of`).
An entry the operator edited reads Outdated and stays until Reinstall, and a `hooks.json` that is not JSON fails agy's row alone and is left as it is.
`hide-agent-hooks/src/antigravity.rs` and `src/guidance.rs` test the answers and the block's install and removal in a fixture home, and the `agy contract` CI lane (`.github/workflows/agy-contract.yml`) also runs the vendor's current agy against the block in the runner's throwaway home.

agy 1.4.0 loads a workspace's own `.agents/hooks.json` only for a folder in `trustedWorkspaces` of `~/.gemini/antigravity-cli/settings.json` (1.3.6 also loaded it in print mode without trust), trust is per exact path and is not inherited from a parent folder, and the first interactive start in a folder it has not been told to trust asks, which would leave a delegated child stopped at that question.
Hide therefore adds the exact path of a worktree it made to that list before it starts agy there, as it records Claude Code's and Codex's trust ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes); D-08).
Hide AI's own folder for agy is trusted the same way ([AI_PROVIDERS.md](AI_PROVIDERS.md#antigravity)), and switching agy off removes it too.

agy's tool-permission question reaches Needs You as an approval: Herdr 0.9.3 reports the pane as waiting on an approval while it is open (seen in the app on 2026-10-16; an earlier probe outside the app had read it as idle).
Its folder-trust question has not been measured, and agy has no bell (D-06): an Enter from the bell into a dialog Herdr reads as idle would answer that dialog, and a letter that arrives meanwhile reaches agy on its next prompt through `PreInvocation`, or, once its wait is over, on the turn's next model call through the same hook.
The bell is turned on for agy once the pinned Herdr is shown to read the folder-trust question as blocked too, and this paragraph goes ([INSTALL.md](INSTALL.md#what-each-system-supports)).
Claude Code's hook needs no silencing inside agy, because agy reads neither `CLAUDE.md` nor `~/.claude` hooks (D-22).

### Installed means the program is found

An agent is installed on a machine when one of its programs is found there, the way the operator's terminal would find it; a folder the agent creates does not count, because a CLI that was removed leaves its folder behind (`~/.pi/agent` without `pi`).
The program names are the ones each vendor's install documentation and install script give the command: `claude`, `codex`, `grok`, `opencode`, `pi`, `omp` and `agy`.
The program names live in the shared adapter declaration alone; Hide AI keeps no list of them, and an agent it cannot use yet is listed as not installed from what the kit found (`AiRequest.cli_found`).

The search is, in order, the folders the account's login shell puts on its `PATH`, the daemon's own `PATH`, and the usual install folders: `~/.local/bin`, pnpm's global folder (`~/Library/pnpm` on macOS and `~/.local/share/pnpm` on Linux, and the `bin` folder inside it from pnpm 11), `~/.npm-global/bin`, `/opt/homebrew/bin` and `/usr/local/bin` (`hide_platform::programs`, the one search; Hide AI's backends find their CLIs with it too, so the kit's "installed" and Hide AI's "can be asked" cannot disagree about a program only the shell's `PATH` or an install folder reaches).
The login shell is asked because many installers put their own folder on the `PATH` by editing a startup file rather than using one of those folders: Grok's `~/.grok/bin`, OpenCode's `~/.opencode/bin` and Pi's `~/.pi/agent/bin` are written into `~/.zshrc`, `~/.bashrc` or `config.fish`, and a Node CLI installed under nvm lives in nvm's folder.
No daemon sees those folders on its own: an app opened from the Dock gets the system folders and the usual install folders from the desktop host, and a device helper started over an SSH exec channel gets the system folders alone (a non-interactive zsh reads only `~/.zshenv`).
`hide_platform::host::login_shell_path` runs `$SHELL -ilc` with the account's login variables and the home being searched, and reads the `PATH` it prints between two marks, past anything the startup files print; it gives the shell ten seconds, the same as the desktop host's own ask, and ends it and everything it started after that or when Hide quits.
The kit asks, on the kit worker on this Mac and in the helper on a device, and Hide AI's backends ask through the same cached answer on their own worker threads (the Settings reader, the label analyzer and Memory's analysis, never under the runtime lock); the hook diagnosis, Memory's version probe and the read of Codex's daemon setting keep the search without the shell, so nothing else that reads `cli_path` starts a shell.
The answer is kept until one of the shell's startup files changes (zsh's, bash's and sh's in the home and in `~/.config/zsh`, fish's, and the system's in `/etc`), which is how an installer adds a folder, so Settings re-reading the kit every few seconds starts no shell; a file those files read in turn is not watched, and is read again when `hided` or a device's helper next starts.
A shell that does not answer is logged as `platform.login_shell_unread` with its error kind, asked again after a minute, and the search goes on without its folders meanwhile; Windows has no login shell, and its search is the account's own `Path` plus the usual folders, which there include agy's own `%LOCALAPPDATA%\agy\bin`.
A program found by the search is also run with it, because a CLI installed as a script starts its interpreter by name (pnpm's `codex` runs `node`).
The Memory version probe does, and so does every child Hide AI starts: a model-list or sign-in probe as much as a request, and the Codex app-server, each given the `PATH` its program was found on (`hide-ai/src/program.rs`, one `Program` type that every backend resolves through and `runner::run` and the Codex session apply).
A CLI named by file, which only a test does, keeps the process's own `PATH` unless the backend's `search_path` is set.

### When the program is gone

An agent that is on and whose program is no longer found (uninstalled, or moved where the search does not reach) keeps everything: the operator's choice stays on record, the stub and the hook Hide wrote stay where they are, and no pass installs, replaces or removes anything for it.
Taking them out on a guess would remove what the operator may want back, and an unused stub or a guarded hook does nothing; this is the kit's rule that only the operator takes a piece away (D-20, D-26).
Its row stays under Installed in Settings with its switch only when the record holds the operator's own choice for it (`AgentReport.chosen`, `KitAgentSnapshot.chosen`); Claude Code, Codex and agy are on by default with no recorded choice, so one of them with no program is listed under Not installed with no switch, and never reads Ready.
A kept row reads `Not on this machine` with the reason that its program is not found, and offers no Reinstall, since Reinstall cannot bring a CLI back; switching it off takes out Hide's pieces as for any agent, and once the program is found again the agent is whole with nothing asked.
Claude Code's and Codex's hook parts follow the kit-part rule instead: they are written while the agent is on and its folder (`~/.claude`, `~/.codex`) is there, found or not, because a CLI the search misses still runs the hook.

### Support table

Hook "done" means Hide writes the guidance hook; "none" rows have no command hook that Hide can write to put text into a session's context, and the row says why and where the documentation says so.
Every row gets the skill stub on every system Hide runs on.
On Windows `~/.agents/skills` is `%USERPROFILE%\.agents\skills`; no vendor page says so for Windows, so the folder rests on each agent's source at the version Hide installs into, none of which gates it by system: Codex `codex-rs/ext/skills/src/host_roots.rs` (`rust-v0.155.1` and `rust-v0.162.1`, the profile folder from `dirs::home_dir`), Grok `xai-grok-shell/src/extensions/skills.rs` (1.0.50, `std::env::home_dir`, which reads `USERPROFILE`), OpenCode `packages/opencode/src/skill/index.ts` (1.18.35, `os.homedir()`), omp `src/discovery/agents.ts` (18.7.0, `os.homedir()`), and Pi `src/core/package-manager.ts` (1.0.4, `HOME` when set, else `os.homedir()`).

| Agent | Skill folder | Hook | Herdr integration | Why, and the page that says so |
| --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/skills` | done: native hooks, a kit part | `claude` | [skills](https://code.claude.com/docs/en/skills) |
| Codex | `~/.agents/skills` | done: native hooks, a kit part | `codex` | [skills](https://learn.chatgpt.com/docs/build-skills) |
| Grok | `~/.agents/skills` | done: spawn guard and subagent count in `~/.grok/hooks/hide.json`; no guidance, because `SessionStart` cannot add context, tool-call context and a Stop block reason can | `grok` | [skills](https://docs.x.ai/build/features/skills-plugins-marketplaces) |
| OpenCode | `~/.agents/skills` | done: Hide's plugin in `~/.config/opencode/plugins/`, not a command hook; OpenCode has none, only JS plugins (<https://opencode.ai/docs/plugins/>), whose `chat.message` hook adds a synthetic part to the prompt and whose `tool.execute.before` refuses a call by throwing ([OpenCode: Hide's plugin](#opencode-hides-plugin)) | `opencode` | [skills](https://opencode.ai/docs/skills/) |
| Pi | `~/.agents/skills` | done: Hide's extension in `~/.pi/agent/extensions/`, not a command hook; Pi has none, only TS extensions, whose `before_agent_start` adds a hidden message to the prompt and whose `tool_call` refuses a call ([Pi and omp: Hide's extension](#pi-and-omp-hides-extension)) | `pi` | [skills](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/skills.md) |
| omp | `~/.agents/skills` | done: the same extension in `~/.omp/agent/extensions/`, with omp's subagent counts and `ask` guard | `omp`, in `~/.omp/agent` | [skills](https://omp.sh/docs/skills) |
| Antigravity | `~/.gemini/config/skills` | done: the `hide` block in `~/.gemini/config/hooks.json`, with guidance, letters, Memory, the spawn guard and the subagent count ([Antigravity: Hide's hook block](#antigravity-hides-hook-block)) | `antigravity-cli`, in `~/.gemini/config` | [skills](https://antigravity.google/docs/skills/) |

Each row also carries what Hide can do for that agent as a list of features (`hide_agent_adapter::Feature`, re-exported by `hide_kit::agents`, and `KitAgentSnapshot.features`).
An agent without both prompt intake and spawn refusal wears the Basic chip, regardless of other missing features.
Its popover groups the feature table into Herdr basics, session reading and multi-agent collaboration, retaining a mark and a supported/unavailable word for every feature:

| Feature | Supported when | Claude Code, Codex | OpenCode | Pi | omp | Grok | Antigravity |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `skill` | always | yes | yes | yes | yes | yes | yes |
| `guidance` | Hide's hook, plugin or extension prints the session guidance | yes | yes | yes | yes | no | yes |
| `letters` | its prompt dialect declaration (a new prompt's letters; while the agent works, `tool_letters` names the dialect whose tool event hands them over: OpenCode, Pi, omp, Grok and Antigravity; a kind with this path is belled only at rest) | yes | yes | yes | yes | no | yes |
| `spawn_guard` | its refusal dialect declaration | yes | yes | yes | yes | yes | yes |
| `memory` | its Memory dialect declaration | yes | yes | yes | yes | no | yes |
| `bell` | the core rings the doorbell for that agent (`AgentAdapter::bell`, tied to `delivery::doorbell::bell_target`) | yes | no | no | no | yes (at rest) | no |
| `herdr_integration` | always: every supported agent has a Herdr target | yes | yes | yes | yes | yes | yes |
| `start` | its launch declaration | yes | yes | yes | yes | yes | yes |
| `sleep` | its Sleep declaration | yes | yes | yes | yes | yes | yes |
| `fork` | its Fork declaration | yes | yes | yes | yes | yes | yes |
| `titles` | its conversation declaration enables generated labels | yes | yes | yes | yes | yes | yes |

Claude Code, Codex, Grok, Pi, omp and Antigravity are the agents whose session files Hide reads for these features, so only they get a per-agent session count, and the count is per machine and only of the sessions running now: one number of open panes holding an awake agent, never an accumulation of warnings.
OpenCode's sessions live in its database rather than in session files, so it has no session count; its database reader gives it titles, sleep, fork and its conversation ([ARCHITECTURE.md](ARCHITECTURE.md)).
`herdr-core/src/runtime/tests/agent_features.rs` ties each flag to the gate in the core that decides it (`runtime_of`, `sleeps_kind`, `ForkableAgent`, `AGENT_KINDS`, `conversation_agent_kind`), so a flag cannot say yes where the core says no, and holds each row's Herdr target to the kind Herdr reports its panes as.

An agent is a row only when its documentation confirms where it reads skills and the name of the program it installs.
The other fifteen agents earlier builds supported are no longer rows (see Retired agents below).
Hide does not write an agent's `AGENTS.md` or `CLAUDE.md`, its MCP configuration or its model settings, and it runs no installer for an agent.
The things it records in an agent's own state are Codex's trust for Hide's own hooks, below, and Claude Code's, Codex's and agy's folder trust for the worktrees Hide makes ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)).

### Herdr's integration for an agent

Herdr learns an agent's session id, and for some agents its lifecycle, from a hook script or plugin that `herdr integration install <target>` puts into the agent's own configuration.
Lineage, the mailbox identity, labels and sleep all read that session id, so the kit installs the integration with the agent's other pieces, as a third piece of the agent's row, `herdr:<agent>` in the record.
It runs through the machine's own Herdr CLI (`KitTarget::herdr_bin`): the Herdr bundled in the app on this Mac, and the device's own Herdr on a device.
Every call is one owned child with a deadline and only the account's login variables and `HOME` (`hide_platform::process::restrict_to_login_environment`), on the kit worker and never under the runtime lock.
The integration is put in only when the agent is on, installed and has made its own folder, since Herdr refuses an agent whose configuration folder is missing.
For Codex the kit also keeps what Herdr's install wrote into `~/.codex/hooks.json`, because Codex asks the operator to review a hook before it runs and Hide records that trust for the entries it installed ([Codex trusts Hide's own hooks](#codex-trusts-hides-own-hooks)); it goes from the record with the `herdr:codex` piece.
Hide takes out only what it put in: an integration that was already in place when Hide first looked is the operator's and stays, whether the agent is switched on or off or the machine leaves Hide, and an older one of the operator's is not replaced.
One of Hide's own that is older is replaced, and one the operator removed by hand stays removed until Reinstall, as for every other piece.
A failed install shows on that agent's row alone and the other agents are untouched; an agent that has not made its folder yet (omp's `~/.omp/agent`, Pi's `~/.pi/agent`, agy's `~/.gemini/config`) reads that on its row, and neither Hide nor Herdr makes the folder.
The integration goes in on the first pass after the folder exists that switches the agent on, the launch pass included.

### Which agents ask for a review

Only Codex asks the operator to approve a hook before it runs.
A survey on 2026-10-07 read each agent's official documentation and the executables installed on the maintainer's machine: Claude Code, Grok and OpenCode run a hook or plugin from the user folder with no approval screen, and the check a project folder's hook gets is a folder trust, not a hook review.
Pi 1.0.4 and omp 18.7.0 loaded Hide's extension from a private user folder with no approval screen on 2026-10-09.
agy 1.3.6 and 1.4.0 ran the user folder's `~/.gemini/config/hooks.json` with no approval screen on 2026-10-16, and neither its folder-trust dialog nor `/hooks` has a step that approves one hook; its folder trust, which Hide records for its own worktrees, is the check a workspace's hooks get.
Hide therefore writes no hook trust for any other agent (the folder trust it records for its own worktrees is a separate step, [Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)), and an agent that starts asking for approval of a user-folder hook is a new row for this section and a new trust step beside Codex's, not a change to Codex's.

### Retired agents

Earlier builds supported fifteen more agents: Cursor, GitHub Copilot CLI, Amp, Factory Droid, Kiro, Qwen Code, Goose, Cline, Kilo Code, Crush, Junie, Augment, Kimi Code, Mistral Vibe and Gemini CLI.
Gemini CLI left later than the others, when the support list became the agents the pinned Herdr ships an integration for; its retirement takes Hide's `hide-guidance` group out of `~/.gemini/settings.json` and leaves the operator's hooks, settings, sign-in and sessions there as they are.
The kit takes out what it put on a machine for them, once, by ownership: only a piece the record names is looked at, and then only what carries Hide's own marker, the skill stub's marker line and the hook entry's `hide-guidance` source.
A file the operator wrote or edited stays, and a file that does not parse is left as it is and reported on the machine's retirement line; its record entry stays too, so the next pass tries again.
Each piece leaves the record once it is out and an agent with nothing left in the record is not looked at again, so a second pass asks and rescans nothing.
The shared skill stub stays while a supported agent that reads it is on and installed, or was switched on by the operator and has lost its program.
Its record entry is what is retried, so a stub that could not be removed (a folder the account cannot write to) is removed by a later pass even after the retired agents that owned it have left the record.
`hide-agent-hooks` keeps the layouts of the seven retired guidance hooks only so this removal can find them, and a hook entry that an earlier build left behind prints nothing and writes nothing when it runs.
This is a transition path: it goes with the release after the one that ships Gemini CLI's retirement.

### Codex trusts Hide's own hooks

Codex keeps a content hash for every entry of `~/.codex/hooks.json` in `~/.codex/config.toml` (`hooks.state`) and starts no entry whose hash it does not hold.
A new or changed entry opens a "Hooks need review" screen at the next start, and until the operator approves it Hide hears nothing from that Codex: no session, no letters, no subagents, and no mark on the pane saying so.
That happened on a machine's first install, whenever Hide's hook changed, and whenever another tool put a hook in front of Hide's and moved its position, which is part of the key.

Installing Hide is the operator's agreement to Hide's hooks, so Hide records Codex's trust for them itself (PRD codex-hook-trust D-01).
It does it through Codex's own interface and never by hand: `hide_agent_hooks::codex_trust` starts `codex app-server` over stdio for one short check, reads the entries with `hooks/list` (each entry's key, its hash and its status) and stores `trusted_hash` for the ones that need it with one `config/batchWrite` to `hooks.state`, then reads the list again and counts a write Codex did not keep as a failure.
A listing that shows Codex could not use the file Hide wrote is a failure too, not "nothing to record": codex-cli 0.160.0 reports a `hooks.json` it cannot parse as a `failed to parse hooks config <path>` warning with an empty list, and a broken `config.toml` as an error naming the Codex home; a warning about another tool's hook is not Hide's to fail on.
Hide computes no hash and writes no `config.toml`, so a change in how Codex hashes is followed, and every other setting in that file stays as Codex wrote it; the folder trust for Hide's worktrees goes through the same interface ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)).

The kit runs the check in every pass that finds the Codex hook part in place, whether the pass wrote it or found it current: at launch, when a device connects, and on Reinstall.
It runs once, after the pass has put in every agent's pieces, because Herdr's integration is only in place then, so one `codex app-server` session trusts Hide's entries and Herdr's together and a first install is trusted before the operator's first Codex start.
A pass that finds every entry trusted writes nothing, so a repeat leaves `config.toml` as it was.
An entry another tool's hook displaced is trusted at its new position by the next pass; a Codex started before that pass can still show the screen for Hide's entry.
A Codex without hook trust (its app-server does not know `hooks/list`, which codex-cli 0.160.0 shows as a `-32600` "unknown variant" error rather than JSON-RPC's `-32601`; both are read as unknown), a Codex that ends before it answers the handshake (no `app-server` command, or a program that is not a Codex) and a machine with no Codex are left alone and show nothing new; a Codex that answers the handshake and then ends, hangs or refuses is a failure.
Starting the app-server also makes Codex do its own bookkeeping in `~/.codex` (its databases, `installation_id`, `skills/`), which is Codex's and not Hide's.

What is trusted is exactly the entry Hide wrote, or the entry the kit recorded Herdr writing, and nothing else (`select_targets`, one function for both):

- Codex lists it from this account's `~/.codex/hooks.json` as a user hook that is not managed, so a project's hook, a plugin's and a managed one with the same command are not it;
- its event, handler type, `command` and matcher are, byte for byte, those of one of Hide's runtime-specific entries (the command Hide writes for that event with this kit's helper, a command hook, and `install::hook_matcher` for that runtime/event; the writer and this check read it from one function, so a matcher that changed is a `modified` entry trusted again, never one trusted blind), or of one entry in the kit record's Herdr entries for Codex (below);
- Codex does not trust it yet (`untrusted`, or `modified` after a change).

The Herdr entries are learned, never written down in Hide: the kit reads the command entries of `~/.codex/hooks.json` before its own `herdr integration install codex` call and again as soon as the call returns, under the account lock, and records the entries that call added (event, matcher, handler type, command) as `herdr_hooks.codex` in `~/.hide/kit/installed.json`, once Herdr reports the integration `current`.
The window another writer could land an entry in is that one call.
The file may only have gained entries, or lost entries the kit already recorded as Herdr's (Herdr replacing its own older command); the removal or edit of any other entry, or an install that adds more than four, records nothing and leaves the record as it was (`herdr_hook_not_learned` on the kit's standard error), and Codex shows its screen for what that install wrote.
A `hooks.json` the kit cannot read around the call leaves the record as it was (`herdr_hook_unread`).
A call that added nothing keeps the recorded entries that are still in the file: Herdr leaves an entry it already wrote alone when it reinstalls (observed on Herdr 0.9.1: an outdated script is replaced and the entry is not duplicated).
A Herdr that changes its command is followed because the kit reads what Herdr wrote at its next install of the integration, and the same pass trusts the new command; the trust recorded for the old command is not removed, as for Hide's own.
The recorded entries count only while the record also holds `herdr:codex` and Herdr reports the integration `current` in that pass, so an integration the operator installed first (the kit never installed it, and the record has neither) and a command that is merely like Herdr's, in another folder or with another argument, are not byte-equal to anything recorded and stay for the review screen.
What stays judgment rather than a rule: an entry another tool wrote in that one call, as a pure addition, would be taken for Herdr's, and a `herdr` that a device finds on its `PATH` is trusted for what it writes as it is already run.
A machine whose record holds `herdr:codex` from a build that did not keep the entries learns none, and does not heal unless Herdr adds a new entry without removing the old one: Herdr's reinstall of an entry it already wrote adds nothing for the kit to see, and an in-place replacement of its command removes an entry the kit never recorded, which an install may not do.
Such a machine shows the review screen once for Herdr's hook and, in those two cases, again for each later change of Herdr's command; approving it trusts exactly that hash.

One other tool's entries join that list, and only through the kit's record: the entries Herdr's own integration (`herdr integration install codex`) added to `hooks.json` when the kit ran it (PRD codex-herdr-hook-trust).
The kit installs that integration with Hide's hooks (see [Herdr's integration for an agent](#herdrs-integration-for-an-agent)), so the operator's agreement to Hide's install covers it: a Mac or device the kit set up opens Codex with no "Hooks need review" screen for Herdr's hook either, and Herdr knows the Codex session from its first turn.
Another tool's hook, an entry that carries Hide's marker over a different command, and any entry that is not like this are neither read nor changed: Codex still shows the review screen for them, and lists only them.
That includes an integration the operator installed before Hide did, which the kit never installed and never recorded, so on a machine where it is the only hook Codex shows "Hooks need review" with that one hook while Hide's are already trusted and run; approving that screen once covers it only, and the review screen lists no entry for Hide's spawn guard.
Hide writes `trusted_hash` and never `enabled`, so a hook the operator switched off in Codex's hook list stays off, and switching Hide's hook off there is how the operator opts out of one; switching Codex off in Settings, Agents takes Hide's entries out and Hide asks Codex for nothing.
Removing Hide's hooks leaves its trust records in `config.toml`; each one matches only the same command and starts nothing by itself.
Codex's `--dangerously-bypass-hook-trust` and `bypass_hook_trust` skip the review for every hook and are not used.

The app-server is one child per check, started through the one spawn helper, bounded by a 15 second overall and 5 second per-request deadline, by caps on what it may print, and by the kit's stop flag, and ended with its whole process tree on success, failure and timeout alike (a process that left that tree and holds its output open is not waited for: the reader thread ends when it lets go); it runs on the kit worker or the device helper, never under the runtime lock.
When Codex is there and the trust cannot be recorded, the Codex hook part reads Failed with one line, "Codex has not trusted Hide's hook: …; it will ask you to review it", the cause class goes to the diagnostic log with the pass's record (`kit apply.completed` names each part's reason), Codex's own words go to the kit's standard error as `codex_trust_failed` (seen where the kit runs in a terminal, and in a device helper's log, but not from the packaged app's detached daemon), the other parts are installed as usual, and the next pass tries again.
`status` runs every few seconds while Settings is open and starts no process, so it repeats what the last pass in this process found; a failure the operator fixed by approving the hook in Codex clears at the next pass or Reinstall, which Failed offers.
`hide-agent-hooks/tests/it/codex_trust.rs` runs the module against a stand-in app-server (`tests/fixtures/fake-codex.py`), and `hide-kit`'s `codex_trust_cases` runs the passes.
Codex's app-server interface is marked experimental; a method or field that changes ends as the Failed line above, and Codex shows its screen, never a wrong trust.

## Judging what is installed

`hide_agent_hooks::diagnosis` resolves one reason, in a fixed order, and the first match wins:

1. `config_unreadable` - the file could not be read or parsed, so nothing was installed into it.
2. `hooks_not_installed` - the runtime is here and carries no hook of Hide's.
3. `session_predates_install` - the hook is installed and this pane carries none of Hide's tokens, so the session was already running when it was installed. Restarting the agent instruments it (the pane's Reopen does that in place); on a Codex whose machine has the shared server on, the same finding is read as the shared server instead (`PaneConnectionReason`). A pane whose reports Herdr refuses lands here too; `last_report_failure` is what tells the two apart.
4. `hook_outdated` - the session is reporting through an older hook than this Hide writes.
5. `unknown` - genuinely unknown, and said to be.

Both the pane's own mark and the Settings diagnosis read that one function, and every projection carries the reason's stable code alongside its sentence so no surface has to recognise its own operator-facing text.
A pane on a device is judged by the same function, against that device's hooks as its kit last reported them (`agent_hooks::device_hook_status`), with the pane's tokens carried from the device's Herdr.
A device where Hide may not install, or whose platform this build does not carry, reads `hooks_not_installed`; a device whose kit Hide has not read yet reads `unknown`.

## Installing

Hide's hooks are one part of its install kit (`hide-kit`; [ARCHITECTURE.md, The install kit](ARCHITECTURE.md#the-install-kit)), which installs the same set on this machine at every launch of the packaged app and on every device at every helper connection, without asking: the operator agreed to it once, by installing the app or by adding the device; after a core move it does so on the core's machine at every start of its core and on the machine the core moved off at every link.
Every entry's command is guarded, so a session whose app or helper was moved or deleted carries on without a hook error (PRD device-parity B3), and it is written in the form its runtime runs a hook on that system:

| System | Runtime | Entry |
| --- | --- | --- |
| macOS, Linux | both | `"command": "if [ -x '<kit folder>/hide-agent-hooks' ]; then exec '<kit folder>/hide-agent-hooks' hook ...; fi"` |
| Windows | Claude Code | `"command": "powershell.exe", "args": ["-NoProfile", "-NonInteractive", "-Command", "<guard>"]` |
| Windows | Codex, Grok | `"command": "<guard>"` |
| Windows | Cursor | `"command": "& { <guard, with $input piped to the helper> else { '{\"permission\":\"allow\"}' } }"` (the `else` only for `preToolUse`) |

On Windows `<guard>` is `if (Test-Path -LiteralPath '<kit folder>\hide-agent-hooks.exe' -PathType Leaf) { & '<kit folder>\hide-agent-hooks.exe' hook ... }`, with a quote inside the path doubled, as PowerShell reads it.
The forms follow what each runtime documents or, where its documentation is silent, what its source does:

- Claude Code passes a command string to `sh -c` on macOS and Linux, to Git Bash on Windows, or to PowerShell when Git Bash is not installed, and spawns `command` with `args` directly, with no shell, on every system (hooks reference, "Exec form and shell form", <https://code.claude.com/docs/en/hooks>; `args` since 2.1.139, below the 2.1.278 Memory needs). A string would meet two different shells on two Windows machines, so the Windows entry names PowerShell in exec form and the guard always runs in PowerShell.
- Codex documents `commandWindows` as a Windows override without saying what runs it (<https://developers.openai.com/codex/hooks>). Its source at `rust-v0.160.0`, and the same code at `rust-v0.155.1`, the oldest Codex Hide installs into, runs a command through the session's shell (`core/src/session/mod.rs`, `build_hooks_config`), which on Windows is PowerShell 7 or else Windows PowerShell, started `-NoProfile -Command <command>` (`shell-command/src/shell_detect.rs`, `default_user_shell`; `core/src/shell.rs`, `derive_exec_args`), and through `%COMSPEC% /C` only when the session reports no shell (`hooks/src/engine/command_runner.rs`, `build_command`). The file is that machine's, so the guard is written as `command`.
- A Codex session that falls back to `cmd` cannot read the PowerShell guard: its hooks fail with cmd's syntax error, the turn goes on without Hide's context, and nothing else on the machine changes.
- Grok runs a command that holds a space as `pwsh -NoProfile -NonInteractive -Command <command>`, with Windows PowerShell when PowerShell 7 is not found and Git Bash only when neither is, or when `GROK_SHELL` names it, with the event on stdin (its shipped hooks guide, `10-hooks.md`: "under the detected Windows shell (pwsh, Windows PowerShell, or Git Bash)"; the order is its source's, `xai-grok-config/src/shell.rs`, `detect_windows_shell`, at grok-build `2bdd1d6a`). Windows PowerShell is part of every Windows, so the Git Bash fallback is reached only through `GROK_SHELL`; a `GROK_SHELL=bash` makes the guard a bash syntax error, whose exit 2 Grok reads as a refusal of every shell call, and `GROK_SHELL=cmd` a cmd error, which Grok lets through without Hide's hook. Grok skips a handler that carries `args`, so its entry is the string form Codex's is.
- The Cursor CLI on Windows writes the event to a temporary file and runs `Get-Content <file> -Raw | & { $input | <command> }` as a script under PowerShell 7, else Windows PowerShell, never a POSIX shell (read from its 2026.10.01 Windows package, `executeCommandScript` and its shell executor; the documentation says nothing of Windows).
The command is one element of that pipeline, where an `if` cannot stand, so Hide's is a script block that runs the helper while it is there and, for `preToolUse`, answers allow itself when it is gone.
Inside the block, `$payload = $input | Out-String -Stream` prepares the native pipeline's text before `$payload | & <helper>` starts the helper, using the same mode as PowerShell's native input formatter.
PowerShell otherwise starts the external process before initializing its input formatter ([`NativeCommandProcessor.cs`](https://github.com/PowerShell/PowerShell/blob/master/src/System.Management.Automation/engine/NativeCommandProcessor.cs), `ProcessInputWriter.Start`), so a cold formatter can consume the helper's 500 ms payload budget before writing the event; the helper then answers allow without inspecting the call (issue 987).
Preparing the text first pays the formatter's cold initialization before the helper's unchanged deadline.
PowerShell writes a UTF-8 byte order mark before the event it pipes (Cursor's forum, threads 166794 and 168407), and Windows PowerShell 5.1 can write one at each pipe the event crosses; the earlier lazy pipe produced two there and one under PowerShell 7 (measured on a `windows-latest` runner on 2026-10-11).
The helper drops every leading mark before reading the JSON.
- A Cursor CLI started from Git Bash, or with `MSYSTEM` or `EXEPATH` in its environment, evaluates that PowerShell wrapper with bash, which exits 2, and Cursor then refuses every tool call of a session with any hook in `hooks.json`, Hide's or another's (Cursor's forum, thread 168129, open with staff confirmation). Hide's entry cannot change the wrapper Cursor builds, and Herdr's own Cursor integration, which the kit installs with Hide's hook, puts a PowerShell `sessionStart` entry into the same file on Windows (`powershell -NoProfile -ExecutionPolicy Bypass -File <herdr-agent-state.ps1> session`, observed from the pinned Herdr 0.9.3's `integration install cursor` on a Windows runner), so Hide's entry adds no case Herdr's does not already have.

PowerShell starts before the helper on every Windows hook, which Codex does for any hook there; the 1,250 ms Memory wait above counts from the helper's own start.
PowerShell can read what the helper prints in the console code page and write it out again (no runtime documents whether it does, and Windows PowerShell 5.1 does when the helper's output crosses a pipe inside Cursor's wrapper, as the runner test showed), so on Windows the helper prints every hook's JSON in ASCII, every other character as a `\u` escape that decodes to the same text.
The marker is looked for in an entry's `command` and in each of its `args`, where Claude Code's Windows entry carries it, so a second install on Windows recognises its entries and converges as it does elsewhere.
The current marker also captures the daemon state route; the shell guards and quoting remain the same on each system.
`hide-agent-hooks/tests/it/windows_hook_command.rs` owns the Windows command fixtures in the `windows check` lane.
`the_windows_entries_run_the_helper_through_powershell_only_while_it_is_there` exercises Claude Code and Codex with the real helper in a folder whose name has a space, a quote, brackets and a `$`; the Memory request recorded as `memory_unavailable` proves stdin reached the helper.
The Grok PowerShell fixture proves event intake through the spawn guard's `daemon.unreachable` line.
Antigravity's `agys_windows_session_hook_reads_its_input_and_injects_guidance`, `agys_cmd_tool_command_leaves_the_call_to_agy_when_the_helper_is_gone_or_fails` and `agys_windows_guard_preserves_korean_and_non_bmp_hook_answers` cover its cmd boundary, guidance, fallback and Unicode answers.
Cursor command and byte-order-mark fixtures describe the retired integration's historical format; they are not current installation support.
Version 6 is the first guarded command, so an older entry reads outdated and the next launch or connection replaces it.
The kit folder is a path that survives a rebuild: the installed app bundle's `Contents/Resources` on macOS, the unpacked package's `resources` on Windows/Linux, and the helper root's `current` link on a device, which each new build of the helper points at itself, as a moved core does at the build it runs.

A `hided` outside a desktop package and outside a build Hide placed in the account's helper root installs nothing, and This machine's row in Settings says the packaged app is where the kit comes from.
`hide_kit::bundled_kit_dir` decides the package from the layout: macOS `*.app/Contents/Resources`, or Windows/Linux `resources` with `app.asar` inside and Electron `hide[.exe]` in its parent folder; a build is `~/.hide/host-helper/<16 hex>/` in the account's own home, holding `hide` and `hide-agent-hooks` beside `hided`, the folder a core move and a core update make (`KitPlace::of_program`, held by `a_core_installs_from_its_package_or_a_placed_build_and_never_from_another_copy`).
A folder merely named `resources` grants no install authority.
The rule exists because a hook command outlives the process that wrote it.
It is a path stored in the operator's own configuration file and run by every future session of that agent, so the only path worth writing is one that survives a rebuild.
On 2026-09-10 a development build resolved the helper beside its own executable under `target/debug/deps`, which Cargo deletes on the next build, and wrote that path into both `~/.claude/settings.json` and `~/.codex/hooks.json`.
Every Claude and Codex session on the machine then failed four hooks per turn with `No such file or directory` until the entries were taken out by hand.
Resolving "beside the executable" was the defect; the package layout is the boundary, and `only_a_daemon_inside_an_app_bundle_has_a_kit_folder` plus `unpacked_package_requires_both_the_shell_archive_and_electron` keep it.

The kit records what it installed in `~/.hide/kit/installed.json`.
A hook it installed that the operator then removed is not put back by the next launch or connection: its part reads Removed and comes back only through Reinstall (D-26).
The `~/.hide/agent-hooks/installed-once` marker from before the kit is not such a record, so a machine that has it and no Hide hook gets one (B20).
A configuration file that could not be read is not written to on a guess: its part reads Failed with the reason, and the kit's other parts are installed anyway (B6).
A runtime whose CLI answers with a version older than the hook needs is not installed into and its part says to update the CLI; a runtime whose home folder does not exist reads Not on this machine.
The write itself runs off `Mutex<Runtime>` (on the core's own node for this Mac, on the helper for a device), and the diagnosis is read back from the file afterwards so the screen shows what the file now says rather than what was asked for.

Settings shows each machine's hook parts under Agents and its whole kit under Devices, This Mac first and then each device.
Reinstall, the `kit_reinstall` event, is offered only where a part is outdated, not installed, removed or failed, repairs only those parts, and leaves the ones in place untouched (B8); pressing it twice is one install.
Project Memory's "update hooks" sends the same Reinstall for this Mac's hook parts.
Reinstall repairs what is on: the hook part of an agent that is switched off reads Off whatever its file says, the machine row does not offer Reinstall for it, and a pane of that agent says its hook is switched off in Settings, Agents (`hooks_switched_off`), not that it was never installed.
A switch pressed while an earlier press for the same agent is still queued replaces it, so the latest press is what the machine ends up with.
The kit looks for the agents once per pass, asks the login shell for its `PATH` only when a startup file changed, and asks a CLI for its version once per version of its file, so Settings re-reading the kit every few seconds runs no subprocess.

Removal does not need the helper, and must not: it reads the configuration file and takes out the entries carrying Hide's marker, and nothing else.
Removing a device from Hide does that on the device while its helper is connected (D-16); the operator removes a hook on their own machine by editing the file, and the kit then leaves it removed.

`hide-agent-hooks doctor [--json]` prints the same judgement in a terminal, because a broken hook shows on screen only as a Not connected chip on a Claude Code or Codex pane and nowhere for any other agent, and the output of that command is the evidence.
Install and remove are deliberately not CLI subcommands: writing to the operator's configuration is the kit's decision, agreed to when the app was installed or the device added, not something a stray command line performs.

## Testing

The operator's real `~/.claude/settings.json`, `~/.claude.json`, `~/.codex/hooks.json` and `~/.codex/config.toml` are never a test target.
Every test in this crate builds its own `HOME` fixture and asserts against that.

OpenCode's plugin is tested at three layers, none of which runs the operator's OpenCode.
`hide-agent-hooks/src/plugin/tests.rs` writes, judges and removes the files in a fixture `HOME`, and `tests/it/opencode_helper.rs` runs the built helper beside a stand-in `hide` that records each call.
`tests/it/opencode_plugin.rs` runs `tests/opencode/plugin.test.mjs` in Node 22, which loads the generated `hide.js` as OpenCode 1.x does and calls its hooks with OpenCode 1.18.30's event and hook shapes (`tests/fixtures/opencode/events-1.18.30.json`) against a stand-in helper; the rust lane installs Node for it.
It runs `tests/opencode/plugin2.test.mjs` the same way, which loads the generated `hide-opencode/` halves on a stand-in for OpenCode 2.0.29's shared server, with the hook and event shapes a 2.0.29 capture showed: an instance per folder with a call routed by the folder it names, every event to every instance, and terminals in two panes; it checks that each prompt, guard and letter is answered in the pane showing its session, a letter's confirmation once OpenCode delivered or stored it, the guardrails alone with no terminal or a vanished one, and the counts of the pane showing a subagent's root.
`web/e2e/opencode-plugin.spec.ts` lets the kit write the plugin into a private `HOME`, runs a small OpenCode stand-in (`web/e2e/opencode-host.ts`) that loads it in two panes of the pinned Herdr, reports itself the way Herdr's OpenCode integration does, and checks a letter on the next prompt and its confirmation once stored, the spawn guard's refusal, and the subagent counts Herdr receives.

Pi's and omp's extension is tested at the same three layers, none of which runs the operator's Pi or omp.
`hide-agent-hooks/src/plugin/tests.rs` holds the file tests for all three script files, and `tests/it/pi_extension.rs` pins both files' bytes and runs `tests/pi-extension/extension.test.mjs` in Node once per agent, which loads the generated file and replays the event and context shapes captured from Pi 1.0.4 and omp 18.7.0 (`tests/fixtures/pi-extension/`) against a stand-in helper: what stays out, the hidden message, confirmation after the reply and once across omp's re-entry, the budgets, omp's counts and `ask`, and that no handler throws.
`tests/it/opencode_helper.rs` checks through the built helper, beside a stand-in `hide`, which session Memory is asked for (Pi's and omp's host id, never the session file; OpenCode's own id), which Memory context a prompt asks for (the index with the first prompt, cards after), its prompt on stdin, and that the letters ride whatever Memory answers.
Which session a receipt is signed for and what the core answers are `hide-memory`'s tests (`src/hook.rs`) and the core's (`herdr-core/src/memory_hook.rs`, `hided/tests/it/remote_core.rs`).
`web/e2e/pi-omp-extension.spec.ts` lets the kit write both files into a private `HOME`, runs a Pi stand-in and an omp stand-in (`web/e2e/pi-omp-host.ts`) in the two panes of the pinned Herdr, reporting the session file the way Herdr's integrations do, and checks a letter in each direction on the next prompt and its confirmation once written, the spawn guard's refusal in both, and omp's subagent counts with a subagent prompt that takes no letter.

## Passive direct prompt observations

The Post adapter recognizes a bounded, single canonical `herdr agent prompt <target> <text>` call and reads its terminal provider result without rewriting the command or result.
Claude Code and Grok register Post and failure events, Codex registers Post only, OpenCode 1 reads after results and terminal error parts, OpenCode 2 reads completed/error after events, and Pi/omp read tool results with the actual native session ID.
Antigravity registers Post but its documented event has no stdout or result, so it retains a weak conversation-step correlation and an unavailable result.
Ordinary shell results are filtered locally before any observation transport starts; command-hook runtimes still pay the native Post helper startup.
No observation adds a Pre request, persistent command spool, polling thread, or detached process.

Only a complete pinned Herdr `agent_prompted` response from a no-wait canonical call supplies a server-reported target candidate.
Success means PTY submission was observed, never model intake, reply, work completion, a Hide letter receipt, lineage or a watch.
Missing native references remain incomplete; wait snapshots, compounds, redirection, foreign routes, truncation, interruption and background results cannot create a confirmed recipient.
Only the exact `agent_blocked` server refusal is classified as an observed rejection.
The parser shares the spawn guard lexer and emits typed metadata containing runtime, native session, tool correlation, observation time, status, reason and any returned pane/terminal/native reference.
Prompt bodies, commands and result bodies never cross the observation admission boundary.
The existing owned helper transport sends at most 8 KiB within 1.2 seconds, with no original-operation retry.
The daemon validates the current sender and independently checks recipient identity before a graph endpoint can be admitted; caller JSON and timestamps are claims, not proof.
Plugin helper concurrency and shutdown reuse their existing eight-child cap, while command hooks use their existing input deadline and owned child cleanup.

Provider fixtures prove payload parsing and original-result preservation, not that a native runtime loaded the candidate hook or emitted an event.
Native event delivery, provider versions and performance evidence are reported separately in the implementation run.
