# Agent Hooks

How Hide learns what an agent has spawned inside its own process, and what it writes to the operator's machine to learn it.

The engine lists panes.
An agent that spawns a subagent in-process creates no pane, so nothing about it reaches the engine's pane list, and a pane running six subagents is indistinguishable from one working alone.
This is the only part of the tree Hide cannot read from the session snapshot, and it is why anything is written to the operator's configuration at all.

## What is written, and where

`hide-agent-hooks/` is the only code path in the product that writes a file the operator owns outside the app's own state.
Its own records are the exception: they stay in the state folder of the daemon that reads them ([Where the hooks keep their records](#where-the-hooks-keep-their-records)), so a candidate Hide started with a state folder of its own writes there rather than into the operator's `~/.hide`.

| Runtime | File |
| --- | --- |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/hooks.json` |
| OpenCode | `~/.config/opencode/plugins/hide.js` and `~/.config/opencode/plugins/hide-opencode/` (see [OpenCode: Hide's plugin](#opencode-hides-plugin)) |
| Pi | `~/.pi/agent/extensions/hide.ts` (see [Pi and omp: Hide's extension](#pi-and-omp-hides-extension)) |
| omp | `~/.omp/agent/extensions/hide.ts` (same section) |
| Grok | `~/.grok/hooks/hide.json`, Hide's own file ([Grok's own hooks](#groks-own-hooks)) |

On Windows `~` is the account's profile folder (`%USERPROFILE%`), where Claude Code, Codex, Grok keeps these files and OpenCode, Pi and omp load their plugins and extensions from.
Beside the hooks, Hide records Claude Code's, Codex's, agy's, Grok's and Pi's own folder trust for a worktree it made, in each agent's own file ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)).

One entry is appended per registered event, and nothing else in the file is touched.
The common writer retains JSON source spans through `serde_json::value::RawValue`, splicing only owned changes so unrelated members, groups and handlers retain their literal whitespace, ordering and escapes.
Removing a Hide handler from a mixed group keeps every unowned sibling.
Duplicate decoded member names, malformed JSON, nesting beyond serde's limit, or input/output beyond 16 MiB are refused without replacement.
The write is atomic - a temporary file beside the target, created private to the account (0600 on macOS and Linux, an access list naming only the account on Windows), that replaces the target in one step (`hide_platform::fs::atomic::write_file`).
The kit holds its account lock around the pass, and the writer refuses an observed source edit or resolved-link change before replacement.
That comparison is not an operating-system compare-and-swap: an arbitrary editor can still race after it.
The file keeps its mode, a new one is created 0600, and a file that is a symlink stays one: the write lands at the file the link resolves to, so a settings file kept in a dotfiles repository is edited there.
Entries belonging to other tools are counted before and after, and a regression test asserts they survive.

Both runtimes register `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `SubagentStart`, `SubagentStop`, `Stop`, `PostToolUse` and `PermissionRequest`.
Claude Code also registers `PostToolUseFailure` and `Notification`, the latter for `permission_prompt|elicitation_dialog|idle_prompt` only, and Codex registers `Interrupt`; Codex reports nonzero outcomes through Post and declares no failure event and no notification, and Claude Code declares no interrupt event, so neither gets an event it would not send.
`PermissionRequest`, `Notification` and `Interrupt` only report the session's turn ([The turn the hook reports](#the-turn-the-hook-reports)): they carry no Memory, print nothing and decide no permission.
Codex declares both of its events from 0.155.1, its minimum, in `codex-rs/config/src/hook_config.rs` of `rust-v0.155.1`, and the installed 0.166.0 lists them among its hook events.
The Post entries have no matcher in Claude Code, so the end of every call reaches the helper (the end of the call a permission waited on is what says the turn runs again), and select `Bash|PowerShell` in Codex; they produce no replacement output or permission decision, and only a shell call's result is observed.
Codex runs an `Interrupt` hook for at most three seconds and warns at its start when an entry asks for more (Codex 0.166.0), so that entry's timeout is three seconds and every other entry's eight (`install::hook_timeout`); an entry of Hide's with another timeout or matcher than an install writes reads Outdated.
`SessionEnd` is not registered by either: a session's record is read only while the engine records that session for the pane, so a session that ends needs no event to be forgotten.
`PreToolUse` has no matcher in Claude Code and selects `Bash|request_user_input|request_user_input_async|apply_patch|Edit|Write` in Codex (`install::hook_matcher`, the one function the writer and Codex's trust check both read).
In Claude Code every tool call reaches the helper, an MCP tool, a search, a subagent's `Task` or a `Skill` included, because an approval sends no event of its own: the next tool call is what says the turn runs again ([The turn the hook reports](#the-turn-the-hook-reports)); the guards judge the shell, the native questions and the file tools the [project guardrails](#project-guardrails) read, and refuse nothing a call of another tool asks.
In Codex the matcher keeps to the shell, the native questions and those file tools.
Codex matches a tool by its whole name: with a hook on `request_user_input` alone, the async question tool of 0.162.1 never reached the helper (measured 2026-10-15 in an isolated `CODEX_HOME`), so each question tool is named.
An entry of Hide's whose matcher differs from the one written today reads Outdated at its own version, so the next kit pass rewrites it and records its Codex trust.
The version-twelve marker (the Windows guard's .NET check, [Installing](#installing)) makes earlier entries Outdated until the normal kit pass at launch, connection or Reinstall replaces Hide's entries and records their exact Codex trust.
Already-running sessions pick up the changed hooks when restarted.
The installed-command version (`HOOK_ENTRY_VERSION`, now 12) is separate from the pane-report protocol (`HOOK_VERSION`, still 8).
A pane already reporting protocol 8 keeps its connection and child counts both before and after that kit pass; updating a command does not invalidate a readable report.

Every entry carries `--runtime claude-code|codex` and `--source hide-subagents@<version>` inside its command (in Claude Code's Windows entry, inside its `args`).
The runtime argument selects that runtime's stdout envelope; the current marker also replaces commands not guarded against a missing helper.
The source name proves the entry is Hide's, and the version after the `@` separates a current hook from an outdated one.
The kit also compares the generated command, including its helper and state route; an unedited command for another state folder is stale, while an operator edit stays until Reinstall.
The helper does not pass the marker on: it is an install marker only.

Every active agent's hook or extension carries the installing daemon's state folder as `--state-dir-b64`, a URL-safe base64 argument that preserves spaces, quotes and non-ASCII names across POSIX shells, PowerShell and JavaScript argument arrays.
The helper sends its own hook requests to the daemon of this state folder when `HIDE_STATE_DIR` is absent; an explicit pane route and the pane's capability reference retain their existing priority and authentication checks.
The installer rejects relative, non-UTF-8, NUL-containing or oversized state paths before writing an entry.
Without a nonempty explicit `HIDE_STATE_DIR`, a malformed, relative or duplicate captured route fails the call rather than falling back to another daemon.
This covers an app opened from the Dock without `XDG_STATE_HOME` while an agent's shell exports it, and agents whose hooks do not inherit Hide's environment.
The current Grok and agy entries, Pi and omp extensions, and both OpenCode server plugins use the same rule; the kit updates their owned entries through its normal pass and records the exact route-bearing Codex command as trusted.
The install versions are Claude Code and Codex 11, Grok 9, agy 10, Pi and omp 12, OpenCode V1 11, OpenCode's server plugin 5 and its TUI plugin 3.
Every kit pass compares the captured folder with the running daemon's folder, so a state move or a daemon started with another environment rewrites an unedited generated command without another version bump.
The account kit records which daemon state folder owns those commands; a different daemon sharing that HOME cannot change them while the recorded owner's lifetime lock is held.
File identity treats aliases or case changes of the same state folder as the same owner.
After that daemon ends, the next pass may take ownership and refresh the entries.
Development and QA candidates use a private HOME as well as a private state folder, because changing only the state folder still shares the account's hook files.
Before that pass, an unavailable old folder leaves mail pending and records the existing throttled `intake.failed` diagnostic with code `unavailable` in the helper's private delivery diagnostics; an invalid captured argument records `record` instead.

The shared SessionStart guidance is English and begins with an intent table that turns what the operator asks for into its command: showing the operator a file, diff or web page (`hide file|diff|browser open …`, which opens as a tab of the active View area and waits in their Needs You as To look at until they look at it), checking a page for yourself with your own browser tool such as `aside` and not `hide browser open`, collecting what a delegated child opened (`hide view list`, a read with `hide view status` or `hide browser snapshot`, and an open again from your own pane to show it), an agent beside you (`hide agent spawn --beside …`, own checkout only), work handed to an agent whose result you collect (`--parent here`), complex work (planning or review first, multiple PRs or parallel work, another agent reviewing, or work likely to fill the conversation: first propose a mission in one line and follow the operator’s answer; an explicit mission request or `/hide-mission` goes straight to `hide mission help`, and work without these signals proceeds directly without asking) a bug in Hide itself (a crash, a command that fails or answers wrongly for no reason of the agent's, a pane or screen that breaks: `hide issue help`), and independent work (no `--parent`), with a line that forbids starting an agent with `hide pane open` or by typing its program into a pane.
The table is one literal (`hide_agent_hooks::intent_table!`) at the front of `PURPOSE_CONTEXT`, so every runtime's SessionStart, `hide guide` and the `hide` skill (version 10) carry the same text, and an older version is replaced at the next kit pass.
The guidance names Hide's commands only, never Herdr's program, its variables or its commands (PRD engine-bundle-1 D-6): a pane is read and driven with `hide pane read|send|open` and `hide agent read`, a worktree's purpose is set with `hide workspace purpose` and a worktree is removed with `hide workspace remove`, and input for another agent goes as a letter, `hide request send`, with `--force` to put it into the agent's input now ([delivery.md](delivery.md#pane-commands)).
`hided/src/guide.rs` holds the session note, `hide guide` and `hide mission help` to that: none names Herdr, and every command of the contract's topics they teach is one `hide contract --json` names.
Supervised delegation uses `hide agent spawn --parent here` to record responsibility and start the automatic watch.

## What the hook returns and reports back

On `SessionStart`, the helper writes one runtime JSON envelope whose `hookSpecificOutput.additionalContext` combines the worktree-purpose instruction with the Project Memory index when Memory is enabled: at most 20 rule, decision and lesson cards, one line each (`m3 Tests run with nextest`), under a line that says to read a card in full with `hide memory show <card>` and to find others with `hide memory search <words>`, and nothing when the Project has no such card (PRD project-memory-v2 D-03, B10); a card waiting to be checked against the repository is left out of the index (PRD memory-card-quality D-06).
The index ends with a receipt line, `<hide-memory-receipt event="SessionStart" ...>`, that records only that the session was shown those lines, never that it was given a card (PRD memory-self-improve B16); an archived card is left out like a waiting one.
A card whose promotion merged into the repository keeps its index line but points at its file (`m3 Tests run with nextest -> docs/TESTING.md`) or its pull request, and is never given in full (B10, B12).
The instruction explains supervised delegation (`hide agent spawn --parent here …`, with an automatic watch) and independent operator handoff (`hide agent spawn …`, without parent or automatic watch); both preserve focus and record origin.
It also says that `--machine <device id>`, used from the machine that runs Hide, starts the agent on a connected device (an agent on a device is refused with `machine_not_permitted`), where the id is the `device_id` `hide workspace info` shows and the `machine` `hide agent list` shows for agents there, and that `--repo` and `--path` are then that device's paths.
The same envelope adds Workspace commands only after `hide workspace info --hook` confirms a renderer-connected Workspace for the caller and report its actual capabilities.
The probe does not need a pane id: the daemon binds a caller inside a pane to that pane, and any other local caller, such as a tool shell or hook inside Codex's shared app-server daemon or a plain terminal, to the registered checkout holding its cwd (`docs/ARCHITECTURE.md`, the Workspace CLI).
The guidance names that checkout path, so a session can read which Workspace its commands reach.
The helper asks the daemon of its route itself, by the hook request `hide workspace info --hook` sends (`WorkspaceInfo`); on a device it asks that device's node, which forwards it with its fresh caller proof to this Mac's core (PRD device-parity B15).
The hook starts nothing and installs nothing; when the daemon does not answer within its two-second bound the session starts without Workspace guidance and the next one tries again (B26).
A session on any linked device receives its Project Memory index and prompt context from the core's one store through the same pane-scoped return route as letters.
The device's node resolves the attested checkout to its own Project identity; the store is never copied to the device.
An unavailable link or a helper without a Memory reader refuses the read immediately with a diagnostic reason; hooks continue without context within their existing budget.
A connected device pane can still receive pending letters through its node and the return route within the same two-second caller budget; [delivery.md](delivery.md#safe-intake-and-manual-fallback) owns that intake and confirmation contract.
An ordinary Claude Code or Codex session started in a connected pane receives the same conditional guidance as a Hide-managed session when Hide's hook is installed in that runtime's configuration on that machine.
A disconnected pane, a cwd outside every registered checkout, an unavailable renderer, or an unsupported Browser surface receives no Workspace capability claim.
The guidance scopes every command to the caller's checkout Workspace and lists only capabilities returned by the daemon; `hide workspace info` remains the live check and `hide --help` gives the full syntax.
With `browser.open` it also lists the `hide browser` page commands in one line and points at `hide browser help`, their agent guide ([BROWSER_DISPLAYS.md](BROWSER_DISPLAYS.md#agent-page-commands)).
It ends with one line that points at `hide guide`, the agent's guide to Hide, and names `hide browser help` and `hide mission help`; since this text exists only when the daemon answered for the caller, the line reaches a session inside Hide and no other, in every agent with session guidance (Grok, which has none, gets the guide through the `hide` skill).
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
The SessionStart command hook has an eight-second timeout, including one bounded two-second Workspace probe (the request `hide workspace info --hook` sends), which it asks beside Memory; a failed probe leaves the existing purpose and Memory context intact.
A persistent credential (the reference SessionStart hands the session as `HIDE_CAP_REF`) remains unclaimed for at most 30 seconds until a CLI receives and acknowledges a Workspace response.
A one-shot Workspace command is never claimed: the daemon revokes its credential when the command's connection ends, and the CLI removes its reference; a `hide browser` relay claims its credential, one-shot or not, because the daemon starts a relay only for a claimed one.
The CLI writes the claimed marker only after the daemon acknowledges that claim, so a caller killed before acknowledgement leaves an unclaimed reference that expires.
A repeated SessionStart in the same attested pane reuses its live persistent reference, including when an earlier hook stopped after claiming but before delivering context.
The SSH bridge tracks every reference it issues and revokes the corresponding daemon token when its file disappears, its unclaimed period expires, its eight-hour lifetime ends, its pane shell exits, or its one-shot caller exits.
A failed bootstrap reply revokes only a newly issued reference; it leaves a reused live reference intact.
Late bridge replies are matched by request ID so one timed-out attestation cannot poison the next bootstrap.
On `UserPromptSubmit`, it parses at most 256 KiB of runtime input and asks the core for Memory (the request `hide workspace memory` sends, below) with the session and the prompt's first 8 KiB; the core resolves the Project of the caller's checkout through that checkout's node and performs a read-only lookup against the materialized active projection of the one store it owns.
The prompt text is the search input only; the original prompt is never replaced.
A card is eligible only when `hide_memory::matcher` scores it above its minimum: a whole word, a stem with its Korean particle taken off, an alias, or enough two-syllable Hangul pieces ([ARCHITECTURE.md: Project sessions and Memory](ARCHITECTURE.md#project-sessions-and-memory)); extraction confidence never stands in for relevance and only breaks ties after it (B11, B12).
At most three whole cards and 600 estimated tokens are returned in the same `additionalContext` envelope, leaving out the cards waiting to be checked against the repository, the cards an agent reported stale and still held, and the cards the session was already given in full, by an earlier prompt or by `hide memory show`, as its recorded receipts say.
The cards follow one `<hide-memory-instruction>` line, outside the untrusted `<hide-memory-context>` block, that says to run `hide memory stale <id> "<what changed>"` when what the operator says now contradicts a card; every runtime receives the same text, and the session-start index has no such line ([hide-private project-sessions-and-memory.md: A card an agent reports stale](ARCHITECTURE.md#project-sessions-and-memory)).
Only a body injection carries a receipt; the session-start index carries none, because an index line is no delivery.
The core authenticates the receipt with the Project-scoped key in the same SQLite store, binding the runtime, session, hook event, and exact ordered card revisions without creating another persistence surface.
The core accepts it only from provider-owned transcript records, including Claude Code's `hook_additional_context` attachment and actual Codex developer messages, and verifies the authentication tag before recording it.
A receipt not yet recorded when the next prompt arrives can repeat a card once; the session goes on either way, and no sidecar or second store is written.
When a Project's receipts are at their cap and the session has none recorded, its prompts get no Memory rather than repeating cards; the cap is logged ([ARCHITECTURE.md](ARCHITECTURE.md#project-sessions-and-memory), B26).
Claude Code and Codex currently accept the same envelope, but the installed runtime argument keeps that protocol choice explicit.
`SubagentStart`, `SubagentStop`, and `Stop` write nothing to stdout, preserving their existing silent behavior.
This stdout is advisory context for the agent and is independent of the best-effort metadata report described below.

For local and connected device agent delivery, `UserPromptSubmit` reads the submitted prompt from the runtime payload (waiting at most 0.5 seconds for it) and sends its SHA-256 and the native session it runs in to `hide inbox --hook --prompt-digest <sha256> --session <id>`; the core decides from that digest whether the prompt is the line its doorbell typed ([delivery.md: Recognizing the bell](delivery.md#recognizing-the-bell)), and the letter intake never sends the prompt text; only Memory's request carries the prompt's first 8 KiB, as a search input.
The session id goes with the pull (`hide inbox --hook --session`), which is how the hook counts as a submission only in its own pane; an unreadable or truncated payload has none, so that hook clears no draft.
The bell turn pulls its pending letters as one bundle ([delivery.md](delivery.md#one-bundle-per-handover)) of at most 8 KiB of letter context within one total two-second budget; any other prompt receives only a one-line count of waiting letters.
Only a successful stdout flush permits confirmation, so pre-confirm interruption can repeat an ID and confirmed letters do not repeat.
The session guidance says that a prompt beginning with `🦉` is Hide's bell and that `hide inbox` shows the letter when the prompt did not bring it, and a Codex bell names `hide inbox` itself ([delivery.md: The bell line](delivery.md#the-bell-line)); a missing or failed hook leaves pending letters available through `hide inbox` and `hide request show`.
The same guidance says that Codex CLI on Windows may show the line without the owl; the core and hook recognize the saved line after removing non-BMP characters ([delivery.md: Recognizing the bell](delivery.md#recognizing-the-bell)).
This delivery path has its own bounded private diagnostics; Memory's request runs beside it and ends where it does.
[delivery.md](delivery.md#safe-intake-and-manual-fallback) owns these intake, failure and confirmation rules.

Memory has one store, `project-memory.sqlite3` in the core's state folder, which only the core opens, whichever machine the agent runs on (PRD core-host-node-move D-12, B14).
The hook asks for it with the request `hide workspace memory --event <event> --runtime <runtime> --session <id>` sends, the same pane credential and node link its letters come by, on its own thread beside the letter pull; on another machine the request goes through that machine's node to the core.

The hook sends every request it makes of the daemon itself (`hide_agent_hooks::hook_request`) and starts no `hide`, on any system.
Memory, the letter pull, the SessionStart Workspace probe and the status report are the four requests `hide workspace memory`, `hide inbox --hook`, `hide workspace info --hook` and `hide workspace status --hook` send through the same function, so the two never differ on the wire, the pane reference they name or its check; a letter's confirmation (`letter_confirm`) asks what `hide inbox --confirm` asks, and the opener guard's registered-checkout test (`registration`) is the proof `hide workspace bootstrap` gives.
It starts no program to ask, on any system, because a program start is not something a deadline can bound: on Windows the system holds a new process until the antivirus has scanned its image, and for `hide` (about 130 MB) that took about one second on every start and 8.4 seconds the first time a new copy ran, measured on a Windows 11 laptop with McAfee in 2026-11 (`hide-agent-hooks` itself, 9 MB, took 78 ms and 1.7 s).
One start was Memory's whole 1,250 ms budget, and SessionStart's three together on the first session after an install outlasted Codex's eight-second limit for the hook, which then discarded the hook's whole answer.
The confirmation and the guard's test started a `hide` until with-hide/hide-private#663: inside the confirmation's last 400 ms and the guard's 2.5 seconds, that start took 18 to 25 seconds on the same laptop the first time the 1.2.2 copy ran (2026-11-07), and Codex dropped the hook's answer at its eight-second limit.
Every one of these is one typed proof-and-answer request through the local bootstrap service, forwarded with the node's fresh caller proof when the core is elsewhere (NodeLink protocol 31).
The service accepts only these variants of `HookCommand`: the reads `LetterPull`, `Memory`, `Registration` and the read-only `WorkspaceInfo`, the reports `Status` and `AgentExec` (what `hide agent exec` asks of and did with a profile's start, its `profile` ask answered with how the profile runs, taken only from that start's executor as the kernel names the connection's process; docs/ARCHITECTURE.md, Starting an agent with a profile), and one write, `LetterConfirm`, which the core takes only from the letters' recipient, its own pane and session, as it takes `hide inbox --confirm` (`mailbox::recipient_of_all`); never a Workspace action.
It reuses the ordinary core executors after credential validation.
Every variant but `WorkspaceInfo` is one-shot, and a request whose `one_shot` does not match its variant is `invalid_request`: a one-shot token stays inside the core and is revoked before the answer is sent, even if the caller ended or the request unwound, and no reusable credential is returned by a one-shot request.
The guidance read returns the same persistent reference and scoped capabilities that bootstrap followed by info returned, in one proof exchange.
The node owns that reference file; a failed answer removes only a newly issued reference, and the core claims a guidance credential only after its proof answer is accepted.
A live `HIDE_CAP_REF` must agree with the fresh proof and, on a node, its own link; a one-shot request does not claim or extend that persistent reference.
An absent named reference falls back to the fresh proof and records `hook.reference_missing` with the request identity, never its path or token.
Local and device refusals include the same `next_action` recovery advice.
The selected route receives one request with no retry through another transport; node bridges that may each be the pane's are refused as ambiguous before any command is sent, and a bridge that declares another installation is not one of them ([More than one Hide on an account](#more-than-one-hide-on-an-account)).
The request's stream carries its deadline ([ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)), so the connect and every write and read wait only for the time left, and partial responses cannot renew the deadline.
Each step that can fail answers its own reason, which the hook records under a cause of its own ([What a hook records when it fails](#what-a-hook-records-when-it-fails)).
Requests and responses are capped at 16 KiB and 64 KiB, and the existing bootstrap and node proof worker caps still apply.
The letter and Memory children run independently and in parallel under their existing caller deadlines and fail-open outcomes.
The core takes the Project from the caller's attested checkout and already-prepared catalog facts, never from anything the hook sends.
Only an explicit checkout purpose or branch description contributes at most 1024 bytes of prepared task context; absent facts fail open without discovery.
Native hooks have no continuation flag, so a prompt ranks by its own text first; only a nonempty brief prompt of at most 80 characters that matches no card at all is asked again with that context, on the same open store, so an unrelated short prompt that finds its own cards never spends the budget or its receipts on the task's, and one whose matches were all given earlier gets nothing rather than the task's cards.
The combined ranking input is capped at 8 KiB on a UTF-8 boundary after composition.

The lookup performs no provider or embedding call, transcript scan, child-process launch, or database write, and runs off the core's runtime lock.
Missing, corrupt, corrupt, stale, over-limit, unresolved-Project, and over-deadline stores return no Memory context, and the hook still exits zero.
A Memory write never blocks the read, because the store is in WAL mode; what can is another connection opening alone (rebuilding the WAL index) or closing last (checkpointing), which each short-lived writer process does, so the read waits for that lock until its invocation expires and a lock that outlasts it ends as over-deadline (`MemoryStore::open_hook_read_only_with_expiry`).
Memory waits for its answer until 1,250 ms after the inner hook starts, its connect, write and read each bounded by the time left; the 1,650 ms intake budget reserves its last 400 ms for letter confirmation after a successful stdout flush.
The prompt supervisor bounds the inner helper at 1,850 ms from the outer helper's start, so the Memory read budget is not the entire prompt hook duration.
SessionStart has the same Memory read budget, a separate two-second Workspace probe asked beside it when its thread starts, and later count-report calls with their own two-second timeouts; its installed runtime entry has an eight-second timeout.
The core sets a one-second Memory answer deadline: owner-scope reply waiting uses the remaining expiry and remote admission, write and answer share an absolute deadline.
Retrieval expires at the earlier of that invocation expiry and its own 100 ms allowance.
The escaped index or body envelope, including receipts, shares a 1200 estimated rendered-token cap; this is not a provider billing count.
On a plugin first prompt, the combined session index and prompt bodies share the same 1200 estimated-token Memory quota.
If both envelopes do not fit, the complete signed prompt-body envelope takes priority and the index is retried on a later prompt; letters remain outside that Memory quota.
The local estimator counts ASCII letters and spaces at one token per four characters (never fewer than one per word), ASCII digits and symbols at one per two, so ids, paths and escapes are not counted as prose, each other character of the Basic Multilingual Plane at one, and a character past it, such as an emoji, at two, then adds a tenth as margin; unspaced Korean, Chinese and Japanese text is not discounted as English words.

Receipts suppress only a current revision delivered in the same Project/runtime/session/context generation.
The existing background receipt reader advances context generation from Claude Code's native `SessionStart:clear` and `SessionStart:compact` hook attachments, Codex's native top-level `compacted` records, and Pi/omp native compaction records.
A Codex replacement context can retain some content, but its metadata does not certify which delivered Memory bodies survived, so the boundary permits bounded revision-aware recall again without reading the retained payload.
A normal Claude Code resume preserves delivery history because it restores the conversation rather than proving context loss.
Other runtimes retain revision-aware delivery but have no proven context-reset signal here; prompt text never implies compaction, and delivery history does not prove present model context.
Generation-bound tags preserve old generation-zero receipt authentication and never assign old deliveries to a new context.
Native evidence becomes usable after the existing reader observes it; the submitting prompt does not scan a transcript.
Candidate, item and token counts remain hard bounded; [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#project-memory-cost-contract) owns the measurement conditions and this limit.
A node whose link to the core is down refuses at once with `hide_unavailable`; the hook then records `memory.unavailable`, or `memory.deadline` for an answer that did not come in time, at most once per ten minutes in its private diagnostics, and the turn goes on without Memory.
A panic on the thread that asks is recorded the same way as `memory.panicked`.
Stdin is read through a nonblocking descriptor until EOF, the size cap, or the 500 ms payload wait.
Windows has no nonblocking read of a pipe, so there a reader thread collects stdin and the helper stops waiting for it at the same point.
Current Claude Code and Codex `UserPromptSubmit` input and output shapes are fixed by sanitized fixtures in `hide-agent-hooks/tests/fixtures/`.
The app probes the installed runtime binaries with a 750 ms bounded version check and requires each agent's declared minimum for Memory injection, Claude Code 2.1.278 and Codex 0.155.1 ([The oldest version of each agent](#the-oldest-version-of-each-agent)).
A runtime below that capability is diagnosed as `Update required` without disabling a supported installed runtime or Sessions browsing.

The helper is stateless, as a hook script must be.
It reports to the core through hided (PRD engine-bundle-1 D-7): it sends one JSON report as a hook request (`HookCommand::Status`, what `hide workspace status --hook` sends), which hided proves came from the pane like every other hook request (`hided/src/pane_auth.rs`); a device's node forwards it with the pane's proof.
The report names the runtime's adapter id, the hook version (`HOOK_VERSION`, 8), the session id the hook's input named, the session's turn when the event says one, its subagent counts for an agent that counts them, and whether the event is a session start (`hide_agent_hooks::report::HookStatusReport`).
A session start also carries the account variables of the agent's `[account]` that are set in its process, empty for its default account (`report.account`, `hide_agent_hooks::account::reported_variables`), each exactly as set, as the node lists a profile's (D-15); Claude Code's hook under `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` reads the folder above the session's `CLAUDE_ENV_FILE` instead, and Pi's, omp's and OpenCode's helper reads its agent's own environment.
The core reads the pane in the profile account those variables name ([ARCHITECTURE.md](ARCHITECTURE.md#a-profile-accounts-usage-sign-in-and-the-account-a-hook-reports)); a core on node protocol 90 or older refuses a report with the field, so a device's helper upgrades with its core.
The call has a two-second bound, so a hided that is absent or does not answer costs the event that long and no more (B13), and its outcome is recorded with the pane and the agent ([below](#what-the-report-records-when-it-fails)); nothing reaches the screen (B14).
The bound also ends inside the time the agent gives the hook, counted from the hook's start rather than from the Memory and letter work before the report (`report::deadline`, from the same `install::hook_timeout` the entry is written with, less half a second for the helper to end), so Codex's three-second `Interrupt` entry is never cut by its own report.
The core keeps the latest report of each session of each pane, at most eight sessions a pane and 1,024 panes (`hide-core/src/agent_hooks/facts.rs`), and lays the one for the pane's agent on the session it projects; [status-model.md](status-model.md#where-each-axis-comes-from) owns what it decides.
A report that changes nothing a projection would show wakes nothing, and one that does wakes the coordinator of the pane's machine to publish again.
What the core keeps besides the turn (runtime, version, session and counts) is saved to `hook-facts.json` in its state folder on a thread of its own, so a restart of hided keeps every pane's connection and counts; the turn is not kept, since the session's next hook event says it again.
A restart of the engine loses none of this, because the core keeps it itself (D-8).
A core that starts with no `hook-facts.json` takes the pane records below as its facts once and saves them, so an agent that last reported before the update stays connected.

Each pane's records live in the hooks' records folder's `panes/` ([Where the hooks keep their records](#where-the-hooks-keep-their-records)), one file per pane id, `$HIDE_PANE_ID`; every reader of a pane's id reads it through `hide_agent_hooks::pane`.
The file holds a record per session, at most eight, the session heard from last at the end: the counts, the hook version and adapter id of the helper that wrote it, the session id the hook's input named, and the turn its last report said, so a tool event reports only a turn it moves.
A file an earlier helper wrote, with one record, is read as that record, and an event that changed no record leaves the file unwritten.
A pane the engine has stopped listing has its file swept on the next session bootstrap.

| Report field | Meaning |
| --- | --- |
| `version` | The hook version this session reported with. Its presence is the proof that the session is instrumented at all |
| `counts.working` | Subagents currently running |
| `counts.done` | Subagents this session has finished |
| `turn` | `working`, `done`, `waiting` or `idle`, when the event says one ([The turn the hook reports](#the-turn-the-hook-reports)) |

No adapter can observe a blocked subagent from the events it has, so the blocked count stays unknown rather than being drawn as a zero.

A record belongs to one session.
A new session in a reused pane starts a record of its own, so it never inherits the last one's numbers.
A `SessionStart` of the same session (a resume or a compaction) keeps them, since its background subagents still run, and forgets the session's turn; a runtime whose input names no session resets at `SessionStart`.

#### Which session is the pane's

Nothing but Hide's integration names a pane's session, where Herdr's integration did before the core's own engine replaced it, so each agent's integration reports it when the session starts or the pane switches to another, before its first turn, and letters and the inbox reach the agent from then on (PRD engine-bundle-4-herdr-removal, mission c3).
Claude Code's and Codex's `SessionStart` hook, Grok's `SessionStart` (its input's `sessionId`) and Cursor's `sessionStart` (its `conversation_id`) report it as a session start; Pi's and omp's extension at `session_start` and `session_switch` (the session's file, as Herdr's integration named it), OpenCode 1.x's plugin at a root `session.created`, and OpenCode 2's terminal half when the session it shows changes ask the helper's `session` operation, which reports the same.
Only the pane's own session reports so: the integrations pass over a subagent's or a nested session's start (Grok reads no other event's session, since a subagent's events name the subagent's), and the engine takes a reported session only for the pane's agent of that kind, or, with no agent known in the pane, as the record of the agent a program it cannot name runs there, keeping one that comes while the shell still holds the terminal until a program shows (`engine/own/topology.rs`, `hook_session`; [ARCHITECTURE.md](ARCHITECTURE.md#agents-on-the-cores-own-engine)).

A session that is not the pane's agent runs the same hooks in the pane's environment: the operator's own Stop hook that starts `claude -p` is one, and it reset the pane's count to zero after every turn of a session that ran subagents (twenty-one of the forty panes on the operator's machine on 2026-10-15 held such a session's record).
So the hook does not decide which session is the pane's: every session reports under its own id into a record of its own, and the core lays on the pane's agent only the report of the session the engine records for it (`agent_session` of kind `id`); an agent the engine records no session for, or names by a path, takes the session of its runtime that reported last.
The engine can keep naming the session of an agent that ended in the pane when the operator starts it again there, so where no session reported under the id the engine names and a hook reported since the engine last changed that id, the session that reported last speaks for the pane: the hook's report is proven to come from the pane, and it is the later word (`agent_hooks::facts::tests::the_session_the_hook_reported_since_the_engine_last_changed_its_id_speaks_for_the_pane`).
An agent restarted quickly enough that the engine first names the ended session after the new one already reported is the edge: that naming reads as newer than the report, so the new session's turn shows from its next hook event, and the engine's state before it.
A nested `claude -p` session therefore keeps its own numbers and changes nothing the pane shows (`claude_subagent_counts::the_counts_follow_background_subagents_through_pauses_resumes_a_kill_and_a_nested_session`, `agent_hooks::facts::tests::a_nested_session_never_takes_the_panes_own_sessions_place`).

#### The turn the hook reports

The turn is the first thing the core reads for a pane's activity, completion and approval (D-9, [status-model.md](status-model.md#where-each-axis-comes-from)), and each agent reports it from the events it gives (D-10):

| Agent | Working | Done | Waiting on the operator | At rest without finishing |
| --- | --- | --- | --- | --- |
| Claude Code | `UserPromptSubmit`; a tool event of its own after a turn that ended, rested or waited on no named call | the session's own `Stop` | an open approval of a call `PermissionRequest` named, until that call ends or the session file holds its result; `Notification` `permission_prompt` or `elicitation_dialog` while none is open | `Notification` `idle_prompt`, which comes after an interrupt, where no `Stop` does, when it comes; otherwise the session file's interruption (below) |
| Codex | `UserPromptSubmit`; a tool event after a turn that ended, waited or rested | `Stop` | `PermissionRequest` | `Interrupt` |
| OpenCode 1.x | the root session's prompt, its `session.status` busy, and `permission.replied` | the root's `session.status` idle | `permission.asked` of the root or a child of it | the root's idle after a `session.error` named `MessageAbortedError` |
| OpenCode 2 | the root's prompt and its `session.execution.started` | `session.execution.succeeded` or `failed` | not reported | `session.execution.interrupted` |
| Pi | the pane session's `agent_start` | `agent_settled`, since Pi may retry, compact or take a queued message after `agent_end` | not reported | an `agent_end` whose last message has `stopReason` `aborted` |
| omp | `agent_start` | `agent_end` | not reported | an `agent_end` whose last message has `stopReason` `aborted` |
| Grok | its prompt, and a tool event that moves the turn | `Stop` of the session | not reported | not reported |
| Antigravity | its prompt, and a tool event after a gap | `Stop` of the root session | not reported | not reported |

A tool event of Claude Code or Codex reads the pane file and starts no process unless the session's last report was not working, so an ordinary tool call costs a helper run at its start and, in Claude Code, one at its end, each reading one small file; the first tool after a turn ended, waited or rested, as when a background task wakes the agent without a prompt, reports it working.
A subagent's tool call (its input names an `agent_id`) moves only the subagent counts, never the session's turn.
An approval is a call's, not the turn's: a Claude Code `PermissionRequest` opens the approval of the call it names (its tool and a digest of its input with each object's keys in order, `hide_session::calls::key`, kept in the pane file as `waiting` and reported as `waiting`), and the session waits on the operator while any approval is open, whatever its turn says (`HookFacts`, `counters::Said`).
Claude Code 2.1.314 runs an Agent in the background and its subagents ask under the main session's `session_id`, so the main session's `Stop` can come while a subagent's dialog is open: the `Stop` ends the turn and leaves the approval open, and the end of that call, whoever made it, closes it (`claude_subagent_counts::a_background_subagents_dialog_outlives_the_main_stop_and_ends_at_its_call`, `runtime::tests::agent_connection::an_open_approval_outlives_the_main_turns_end_and_closes_with_its_call`).
A call that runs beside it, whose hook may finish after the request's, and a notification of the open dialog leave the approval open; a prompt and an interrupt close every one, since the operator is back at the prompt; at most 16 are open per session, and a seventeenth is a failed report.
A call refused in the dialog sends no hook event (`PermissionDenied` runs only for auto mode's refusals, not a dialog's), and Claude Code 2.1.312 writes the refusal into the session file as the call's result (`is_error`, `permissionDecision.source` `user_reject`); two readers close its approval from there.
The helper, at the session's next hook event whose input names the session file (`transcript_path`) while an approval is open, reads the file's last 256 KiB (`hide_session::calls::answered_in_claude_file`), so the next call or the turn's `Stop` reports it closed (`claude_subagent_counts::a_stop_after_a_refusal_the_session_file_records_leaves_no_approval_open`).
It looks for the last call with the waited input, and for a call known by its tool alone, the tool's last call: a background subagent writes its calls to its own file while its events name the main session's, so the main session's earlier call of the same tool must not close the subagent's approval (`claude_subagent_counts::a_subagents_approval_asked_after_the_main_stop_stays_open_until_its_call_ends`).
The core, when the turn after the refusal only writes text and sends no hook event, takes it from the session read the label worker makes on the engine's state change: the read keeps, of the last 64 calls, each key whose last call has a result (`TurnTracker::answered_calls`, laid as `RowFacts.answered_calls` for that state), so a command run twice waits on its second run's own result, and an approval first laid under an earlier engine state closes, with no I/O under `Mutex<Runtime>` (`runtime::tests::labels::an_approval_ends_when_the_session_file_reads_the_refused_calls_result`).
The same read's interruption, which ends the turn as resting, closes every open approval with it.
A waited call older than either window, or a file that cannot be read, leaves the approval to the call's end, the next prompt or an interrupt.
The core keeps open approvals in memory only, so after hided restarts a dialog still open is judged from the session file and the engine, as before hooks, until the session's next hook report names its approvals again.
A call whose input a hook rewrote (`updatedInput`) ends with an input whose digest differs from the one the request named, so its own end does not close the approval: the session file's result for it does, read as above.
Every other event that names a turn (a prompt, `Stop`, a permission, a notification, an interrupt) reports it each time, with one `status` hook request, as do a session start and a change of the subagent count.
`UserPromptSubmit`'s report is sent by the outer hook process, beside the inner one that pulls the prompt's letters and Memory and within the same budget: the outer process ends the inner one at the budget, which once took a report and its outcome with it under load, and the outer process outlives it.
The outer process reads the prompt's input itself and passes the same bytes to the inner one, saying whether the read was cut or not made by its budget (`--payload`), so the report names the session the prompt came from, not a nested `claude -p` session that reported last in the pane; a tool event's report likewise names its input's session, and an input that names none falls back to the session the pane file last recorded for the runtime.
A plugin or extension sends one turn report at a time and lets a newer turn replace one still waiting, so a burst of events costs at most two helper runs.
A turn stands until the same session's next hook event, or until the session file says the turn ended after it; no timer ends it (design principle 13).
Claude Code writes an Esc as a `user` record whose text is `[Request interrupted by user]` (`… for tool use]` when a tool call was cut; Claude Code 2.1.314), and sends no `Stop` and not always `idle_prompt` (none came in 199 s on 2026-10-30): the session read under a later engine state than the one the hook's turn was first laid under, with the engine reading the agent at rest, finds the turn interrupted and ends the hook's turn as resting (`TurnTracker::ended`, `agent_hooks::facts`, `runtime::tests::labels::a_hook_turn_ends_when_the_session_file_reads_an_interruption_after_it`); a session file that records completed turns (Codex) ends it as done the same way.
An Esc before the first output leaves less: Claude Code 2.1.314 puts the prompt back in the composer and keeps its `user` record and the hooks' attachments in the session file with nothing of the agent's after them (the next prompt takes the same `parentUuid`), and records no interruption, `Stop` or notification; a prompt whose turn has no agent record after it reads as unanswered (`TurnMark::Prompt`, `TurnMark::Reply`, `TurnEnded::Unanswered`), and the same later read at rest ends the hook's turn as resting (`runtime::tests::labels::a_hook_turn_whose_prompt_the_session_file_shows_unanswered_ends_at_rest`).
A finished turn whose `Stop` never reached the core (one lost with a device's link) ends the same way: Claude Code writes a `system` record of subtype `turn_duration`, with `isSidechain` false, as each turn ends after its Stop hooks ran, and the session reader reads that form, and only that form, as the turn completed (`TurnMark::Completed`, `hide-session/src/turns/native.rs`).
In the 2.1.319 sessions of one Mac (2026-11-08: 130 files, 1376 turns, 1269 such records) each turn had at most one, after the last Stop hook summary when the agent went on after the first, and none was written after a later prompt; that is what the scan found, not a promise Claude Code makes.
It names no turn and no reason, and it also followed 2 of the 5 interruptions of a tool call there, so an interrupted turn stays interrupted whichever record comes last (`TurnTracker`); a turn that a compaction, a queued prompt or a closed session cut short has none and is not guessed complete.
The end clears no question, approval or background-task record the tracker holds (`hide-session/tests/it/adapters.rs`, `a_claude_turn_end_leaves_an_open_question_and_the_answered_calls`).
A device's engine goes with its link, and the next link's engine counts its states from the start again ([status-model.md](status-model.md#across-an-engine-restart)), so a later state there is not a larger sequence.
When a device's link ends, every turn and open approval its hooks reported is held as laid before any state of the next engine (`HookFacts::engine_ended`, called where every link end goes, `retire_own_device_engine`): the first read of the next engine that finds the agent at rest, with a session read under that state that finds the turn ended, ends it, and a read that finds a call's result closes its approval, whatever sequence the engine reads.
A turn a hook stopped reporting while the link was down, such as a `Stop` lost with it, therefore ends once the link is back, and the restore's `unknown` step ends nothing (`runtime::tests::device_reconnect`).
A report the hook makes after the link ended speaks for the next engine: the turn or approvals it states, even unchanged, are laid again under that engine's state, so a session read of the turn before them never ends them.
A turn the agent wrote anything for is not unanswered, so after output only the interruption record or the next hook event ends it, and only Claude Code's reader marks replies, so no other agent's turn reads unanswered.
The read is the status's, not the labels': the agent-summary switch stops analysis and takes the AI fields off the row, and the label worker still reads each session on the agent's engine state change and lays what it finds (`labels::tests::an_interruption_is_read_from_the_session_file_with_summaries_off`, `runtime::tests::labels::a_hook_turn_ends_on_the_session_files_interruption_with_summaries_off`).
The read runs on the label worker's read thread, once per engine state change and never per hook report or tick; under `Mutex<Runtime>` the core only compares the read's engine state with the turn's.
Where a cell says not reported, the core reads the engine's state for that part until the agent gives an event for it: an OpenCode 2, Pi or omp permission prompt, and a Grok or Antigravity interrupt, show as the engine reads the screen.
A Claude Code or Codex approval sends no event of its own, so the turn reads waiting while the approved call runs: in Claude Code until that call ends, and in Codex until the session's next tool call; a plain denial in Claude Code interrupts the turn, which the session file then says, and a denial with feedback lets the turn go on, which the session file's result for the call says (above).
An answered MCP `elicitation_dialog` names no call, so it reads waiting until the session's next call starts or ends, since the tool that asked is still running.
Codex's tool hooks keep to its shell, question and file tools, so an approval of an MCP tool in Codex reads waiting until the next of those or the turn's end.

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

### The Hide global instruction

A session start also carries the Hide global instruction (PRD memory-self-improve D-23, D-29, D-31): `global-instruction.md` in the core's state folder (`hide_kit::layout::global_instruction`), Hide's own file of instructions the operator approved for every Project and every agent, never one of an agent's own global files.
The core reads it on the same `workspace memory --event SessionStart` answer as Memory, in its own `global` field, whether Memory is on, off, unresolved for the checkout or unreachable, so a session in any Project and on any connected device gets it (`hide_core::global_instruction`, `hided/src/hook_request.rs`).
It goes in Hide's own `<hide-global-instruction trust="operator-approved">` block, before Memory's untrusted block, and under its own cap of 4 KiB and 1,600 estimated tokens, kept apart from Memory's 1,200, so it never shortens Memory's index or bodies (B4).
A file over the cap or not UTF-8 is given to no session, never cut, and each start it would have reached leaves a `global_instruction.not_given` diagnostic (B5); an empty file gives nothing.
The read is one bounded file read on hided's request task, outside Memory's one-second deadline, and the hook's own 1,250 ms Memory wait is unchanged (B31).
Claude Code, Codex and Antigravity get it at `SessionStart`; OpenCode, Pi and omp get it with their session's first prompt, beside the start guidance, once a session.
Grok's `SessionStart` cannot add context and an Antigravity session's own child sessions run no Hide hook, so neither gets it ([INSTALL.md](INSTALL.md#what-each-system-supports)).
A session on a device whose link is down gets no answer from the core, so it starts without the instruction and the hook records `memory.unavailable`, as it does for Memory; the next session after the link returns gets it (B3).
A session started outside a Hide pane has no Hide hook identity and gets nothing.

Because every session takes the file as the operator's own instruction, no agent writes it (D-07): the pre-tool hook refuses a file tool's write to it, a shell call with a word or redirection naming it, and a shell call that names it by its bare name beside a state folder or runs in one (`cd ~/.hide/state && ... >> global-instruction.md`, a `bash -c` script, an `eval`), in every session and Project and whatever the project's guardrails say (`hide_agent_hooks::global_guard`).
It looks for the file in every state folder the account's core can have, the folder the kit installed the hook for, `HIDE_STATE_DIR`, `$XDG_STATE_HOME/hide` and `~/.hide/state` (`STATE_FOLDERS`), and for the file a link at any of them points to; names are compared without case, as macOS and Windows find files.
Only a payload that names `global-instruction.md`, or the name of the file a link there points to, is parsed, so other calls cost a byte search and a link read per state folder; a payload that names it but was cut at the hook's read limit before Hide could read what the call does is refused rather than guessed.
OpenCode's plugin and the Pi and omp extension send the helper the same calls, with no protection on as with one, through a prefilter the guard writes into each of them (`global_guard::prefilter_constants`: the file's name, the state folders from `STATE_FOLDERS`, and `namesGlobal` from `src/global_guard/names_global.js`), so none of the three keeps a list of its own.
One corpus (`tests/fixtures/global-guard-corpus.json`) is run by the guard and by each script's own tool hook: the guard refuses exactly the calls it marks refused, and every call it marks named, a refused one always among them, reaches the helper from all three.
A file tool's read passes; a shell call cannot be told apart from a write, so one naming the file is refused, read or not.
Each refusal writes one `global_instruction` `guard.refused` line to `spawn-guard.log` with the runtime and the pane, never the command or its paths.
OpenCode, Pi and omp take the same refusal through the plugin helper: their plugin and extension send a call naming the file to the helper whether or not a project guardrail is on.
It reads a shell call in the [syntax of the shell that runs it](#which-shell-reads-a-call), so in a PowerShell call `state\global-instruction.md` run in `~/.hide` names the file, as PowerShell runs it, where Git Bash would run `stateglobal-instruction.md`.
Like the guardrails it is a guard, not a security boundary: a script that spells the name another way is not seen, and the file stays a plain file the operator's own processes can change; a confirm before a hand edit counts is a follow-up, not this change.
The core follows a link at the file only to an existing regular `.md` or `.txt` file: through a link it never makes a file or a folder, and a link to anything else is read as an unreadable file, given to no session.
A move of the core carries the text the core reads through such a link as a regular file, since the link would point nowhere on the receiving machine, and leaves a link whose file the core reads as too large or unreadable behind with a `global_instruction.link_left` diagnostic and its reason.

### Where the hooks keep their records

The hooks' records are the pane counts and turns (`panes/`), the pane's plan (`plans/`), the failed reports (`last-report-failure.json`, `spawn-guard.log`) and what a hook could not deliver (`delivery-diagnostics.json`).
They live in the folder of the daemon whose state folder the hook asks (`hide_agent_hooks::route::records_root`), so a daemon reads what its own hooks wrote and no other Hide's:

| The daemon's state folder | Records folder |
| --- | --- |
| The account's own, `~/.hide/state` | `~/.hide/agent-hooks`, where every build so far wrote them |
| Any other, chosen by `HIDE_STATE_DIR` or `XDG_STATE_HOME`, as a test or a QA candidate's is | `<state folder>/agent-hooks` |

The choice is made by the folder and not by which variable named it: a daemon `hide connect` starts is handed its state folder through `HIDE_STATE_DIR`, and one naming the account's own moves nothing.
A folder `XDG_STATE_HOME` chose counts as a move, and is judged from the resolved state folder, so a hook that lost Hide's environment and one that kept it agree.
Hook, node, core and `doctor` all resolve it the same way, and the folder a daemon uses is the one its `hook-facts.json` first-run seed, its pane sweep and its diagnosis read (`route::records_in`, `route::records_root`).
An older build wrote a moved daemon's records under the account's `~/.hide/agent-hooks`, since it had no other place; they stay there and nothing deletes them.
The daemon reads none of them after the update and writes its own from its panes' next events, so what they held is read once more by no one: a pane's counts still come back from the state folder's `hook-facts.json`, its plan record is written again by its next letter pull or `hide task`, and a confirmation it owed ends at its deadline unsent as it would have anyway.
The one reading that is lost is the turn the older build last recorded, so a restore after the update may not name a cut-off turn once.

The guardrails policy (`guardrails.json`) and the folder-trust records are not among them: they hold what the operator chose for a machine, and stay at `~/.hide/agent-hooks/guardrails.json` (`guardrails::policy_path`) and each agent's own `-trust.json`.

### What a hook records when it fails

A hook that cannot get its answer goes on without it and records why in the hooks' `delivery-diagnostics.json`, private to the account, at most once per cause every ten minutes, with one JSON line on its standard error when it records.
The causes are a fixed list (`CAUSES` in `hide-agent-hooks/src/delivery.rs`), so the file stays small and holds no path or text.
A hook's request to its daemon (the letter pull, Memory) answers a reason for the step it stopped at, and each step has a cause of its own:

| Step | `hide` reason | Cause |
| --- | --- | --- |
| No daemon published its bootstrap socket for the state folder, and no device's node answers | `hide_unavailable` | `unavailable` |
| The record naming that socket is not the account's own and private, or names a folder that is gone | `invalid_bootstrap_socket_record` | `record` |
| Nothing took the connection at the socket the record or the pane's reference names | `hide_unreachable` | `connect` |
| The request was not written whole, because its time ran out or the stream broke; nothing ran | `request_unsent` | `unsent` |
| No answer came before the deadline; the command may have run | `request_timeout` | `timeout` |
| The answer ended, broke off, or was not one bounded JSON line for this request | `invalid_response` | `answer` |
| An answer came, but it does not name the pane's installation (an earlier build's, or another installation's); it is dropped ([More than one Hide on an account](#more-than-one-hide-on-an-account)) | `install_unconfirmed` | `unconfirmed` |
| Only a daemon or node of another installation answered, refusing the pane before anything ran | `other_install` | `other_install` |

The daemon's own refusals keep the causes a hook acts on (`ledger`, `capacity`, `identity`), and any other reason is `refused`; an answer with no reason is `format`, and `cli` means the helper could not form its request.
A letter confirmation the hook owes its pane's next turn end and could not keep is recorded as `owed_record` (its file could not be written, or would pass 64 KiB) or `owed_capacity` (64 panes already owe one) ([delivery.md](delivery.md#a-confirmation-that-missed-its-hook)).
The `hide` whose request failed also writes `hook.request_failed` on its standard error, with the request id, the reason and the system's error kind.
Memory's request records an answer that did not come in time (the hook's deadline or the request's `timeout`) as `memory_deadline` and every other failure as `memory_unavailable`, with the step's cause kept beside it under `step`, so the file says where it stopped; a file an earlier helper wrote has no `step` and is read as it is.
Beside either it keeps under `stages` when each stage of that request ended, in milliseconds from the start of Memory's budget: `asked_ms` (the hook's input read), `connected_ms` (the stream to the daemon), `answered_ms` (the answer line) and `ended_ms`, a stage the request did not reach left out, so the next failure names the stage its time went to; the same object is on the standard-error line.
Each step has its own reason because they used to share one: every step answered `hide_unavailable`, recorded as `cli`, so a Windows hook that never wrote a byte to its daemon read like a missing `hide`.

### What the report records when it fails

The helper always exits zero and reads no more than its bounded standard-input prefix before writing applicable context.
A producer that never closes stdin is released after the 500 ms payload wait.
A hook that fails must never be what breaks the operator's agent.

Exiting zero is not the same as saying nothing.
The outcome of every report is recorded in the hooks' `last-report-failure.json`: a failure writes the pane, the agent, the event and the cause `hide` answered (the causes of [What a hook records when it fails](#what-a-hook-records-when-it-fails)), and the next success removes the file, so it describes the hook's current state rather than its history.
A report whose pane record could not be written is that same failure: the report still goes to the core with the session it names, and the refused write is the pane's recorded cause, so a start waiting for the session's identity is not left without both the identity and the reason.
Every failure is also a `report.failed` line in the hooks' bounded log, `spawn-guard.log`, with the pane, the agent, the event and the cause, so a report that did not arrive stays on record after a later one clears the file (engineering rule 10).
A failure record and the log line it writes are both written, and one that cannot be is printed on the hook's standard error rather than dropped, since a failure record that is lost is the one `doctor` and Settings exist to show.
A report the core refuses is also written to hided's diagnostic log as `agent_hooks` `status.refused` with the pane, the device, the agent and the reason.
`Diagnosis` reads it back as `last_report_failure`, `doctor` prints it as a `Last report failed:` line, and the Settings group shows it as an error note above the restart advice, because with a refused report on record a restart is not the fix.
The Settings screen learns of it through the existing kit worker, with the next diagnosis due one second after the previous read finishes while a window observes Settings (`settings_observed`, the same flag the Hide AI tab sets).
The same worker reads once at startup and after a successful install; ordinary refreshes stop when Settings is unobserved.
Diagnosis and installation are serialized, so a pre-install answer cannot overtake a later install, and neither read makes the session coordinator wait.
Shutdown stops admission, closes the fixed node to end its current call and joins the worker; a retiring answer is not published.

## Native questions

Agent question tools keep their native UI.
This release has no Factory question guard or private command route.

## The opener guard

`PreToolUse` carries the opener guard: a shell call that hands a result to the system opener is refused before it runs, because the page, image or document lands in an app outside Hide, where nothing reaches Hide's View and the operator's Needs You never says which agent opened something to look at.
The session guidance already names `hide file open` and `hide browser open`, and says never to use the system opener, but a skill's or a habit's `open` wins over guidance, so the call is refused where it is made (`hide_agent_hooks::opener_guard`).
The reason gives one filled command per target, through the `hide` beside the helper as the session guidance names it: `hide browser open <target>` for an HTML file or a URL, `hide file open <target>` for the rest.
It says the view opens as a tab of the View beside the agent and that the operator's Needs You tells them to look at it, that this holds for a file outside the checkout too, and that a path Hide refuses is reported in the reply rather than handed to the system opener.

What is refused, in a pane of a registered checkout, is an opener command whose operands include a viewable target (the call is read after any `VAR=value` words and the prefix words `time`, `exec`, `command`, `nohup`, `env` and, at the start of a command in a shell line, `if`, `then`, `elif`, `else`, `while`, `until`, `do`, `{` and `!`):

- macOS `open`, also with `-a <app>`, `-b <bundle>`, `-e`, `-t`, `-u <url>`, `-n`, `-g`, `-j`, `-F` and `-W`; the words after `--args` are the app's own;
- `xdg-open`, `gio open`, `sensible-browser`, `x-www-browser`, `www-browser`, `wslview` and the desktops' `gnome-open`, `kde-open`, `kde-open5`, `exo-open` and `gvfs-open`;
- cmd's `start` (and Git Bash's `start` script, which calls it), PowerShell's `Start-Process` with its aliases `start` and `saps`, `Invoke-Item` and its alias `ii`, and `explorer`, matched without case as Windows matches them; cmd's `/` switches and an empty window title are skipped, and any viewable operand is a target, since `start chrome x.html` shows `x.html` as `start x.html` does;
- `python -m webbrowser <url>`, the system opener through Python;
- each of these as the line `cmd /c` (`/k`, Git Bash's `//c`) or `powershell -Command` (`pwsh`, `-c`) is handed, one level deep: PowerShell's line as PowerShell reads it, and cmd's, whose own syntax Hide does not read, in the syntax of the call it came in.

A viewable target is an `http:`, `https:` or `file:` URL, or a path whose extension Hide's View shows: `html` and `htm`; `png`, `jpg`, `jpeg`, `gif`, `webp`, `bmp`, `avif`, `svg` and `ico`; `pdf`; `md`, `markdown` and `mdx`; `txt`, `log`, `csv`, `json`, `yaml` and `yml`; `mp4`, `webm` and `mov`.
A pattern (`out/*.png`) is refused with a `<path>` to fill once per file it matches, since `hide file open` takes one path.
Everything else runs untouched: a folder (`open .`), a reveal (`open -R`, `explorer /select,`), a header search (`open -h`), `open -f`, an app with no target (`open -a Simulator`), an app bundle, a project (`.xcodeproj`), a source file, `--help`, a browser program run directly (`google-chrome --headless`, `firefox x.html`), which is a QA tool as often as a viewer, and an opener hidden in `bash -c`, `$(...)` or a script.
The guard reads the whole call as the shell that runs it would read it (`hide-shell`, in the syntax [Which shell reads a call](#which-shell-reads-a-call) names), one top-level simple command at a time (`&&`, `||`, `;`, `|`, `&`, newline, and in PowerShell a `{ }` block), with quotes, escapes, heredoc bodies and here-strings and comments set aside, so a commit message or a heredoc that quotes `open x.html` is not an opening.
In PowerShell `\` is a plain character and `` ` `` the escape, which before a line end, `\n` or `\r\n`, continues the line, and the typographic double quotes (U+201C to U+201E) quote as `"` does (`hide_shell::quote::POWERSHELL_DOUBLE_QUOTES`), so `ii .\out.png` names `.\out.png`; in Git Bash, a POSIX shell, the same text names `.out.png`, as Git Bash would run it.
The command the refusal offers is written for the same shell, so its path is the one the agent wrote: `hide file open .\out.png` for PowerShell, `hide file open 'C:\a b\x.png'` for `Invoke-Item 'C:\a b\x.png'`, and a quoted `hide` called with `&` there.
A heredoc inside `$(...)` is read as text to its delimiter line, so a quote or a parenthesis in its body does not count, and text that still ends inside a quote, a backtick or a `$(` (in PowerShell also a block, a here-string, a `<# #>` comment, a stray closing bracket or a `<`) is a syntax error the shell would not run, so it holds no opening and the call runs.
An opening chained behind another command (`npm run build && open dist/index.html`) is refused as a whole and nothing in the chain runs, since the hook sees the tool call and not one command of it; the reason says to run the other commands separately.

The registered-checkout test is the hook's own `registration` request, the proof `hide workspace bootstrap` and `SessionStart` give, needing no renderer and starting no program, and it is asked only for a call about to be refused.
Only the daemon's own "this caller is not in a Hide checkout" answers (`checkout_not_registered`, `caller_unavailable`, `pane_not_connected`, `pane_unavailable`, `pane_changed`, `caller_not_in_pane`) mean the call is not Hide's to guide, and it runs silently.
Any other outcome, a daemon that is not running, a bridge to a device that is gone, an answer that does not come within the guard's 2.5 second budget, an unreadable answer and a reason this build does not know, lets the call run and appends one `daemon.unreachable` line with its cause to the hooks' log, throttled to one per ten minutes through the delivery diagnostics' store (cause `guard`), because a refusal Hide cannot explain would be worse than a result shown outside Hide.
The line goes to the log only: a hook's standard error reaches nobody the operator or the agent could act on.
Outside a pane Hide manages (no `HIDE_PANE_ID`) the call runs without asking anything.
Inside a Grok session Claude Code's hook says nothing at all and Grok's own hook guards in its own answer shape ([Grok's own hooks](#groks-own-hooks)); inside OpenCode it ends silently as well and OpenCode gets the same guard from Hide's own plugin.
Claude Code and Codex share one deny envelope, `hookSpecificOutput.permissionDecision = "deny"` with `permissionDecisionReason`; every other path prints nothing and exits 0, and the whole entry runs inside `catch_unwind`, so a defect allows the call rather than refusing an ordinary one; a failed owner handshake in the environment ends the outer hook with 0 as well, since exit 2 from a pre-tool hook would refuse the call.

Each refusal appends one `opener.refused` line to the hooks' log, `spawn-guard.log` in the records folder of its daemon ([Where the hooks keep their records](#where-the-hooks-keep-their-records); private, capped at 256 KiB with one rotation, and named for the guard that first wrote it), and echoes it on standard error, with the pane, the runtime, the openers and the kinds of their targets (`html`, `image`, `url` and so on); neither the command nor a path is written, since either can hold a name the operator keeps private.
The per-call cost is what an ordinary shell call pays: the payload is read within 0.5 seconds, and a call is parsed only when it holds both a word of an opener (`open`, `start`, `saps`, `invoke-item`, `explorer`, `browser`, `wslview`, or `ii` as a word) and a viewable mark (a URL scheme, or a `.` followed by one of the extensions above and no further letter or digit), so any other call costs one process start and the [project guardrails](#project-guardrails)' policy read.
OpenCode's plugin and Pi's and omp's extension apply the same test in JavaScript before they start the helper.
`hide-agent-hooks/tests/it/opener_guard.rs` runs the helper for Claude Code, Codex and Grok beside a stand-in `hide`, `tests/it/agy_hooks.rs` for agy and `tests/it/opencode_helper.rs` for the OpenCode, Pi and omp helper, the plugins' host tests check that the call reaches the helper, and `src/opener_guard.rs` holds the reading's table tests.

## Which shell reads a call

A shell call is read in the syntax of the shell that runs it, decided once, by `hide_session::call_shell::syntax` from the runtime, its tool and the system, and carried in the call (`pre_tool::Call::syntax`); a guard reads the call in that syntax and never guesses it, and every difference between the syntaxes is read in `hide-shell` (`hide_shell::Syntax`).
The tool decides, not the shell of the pane: an agent starts a shell of its own for each call.
The session readers read a call a session file recorded by the same table: each reader names its runtime and the record's tool (Claude Code's `tool_use` name, `Bash` or `PowerShell`), the file is read on the machine whose agent wrote it, so that machine's system is the session's, and the call's pull requests, commits, failed-call kind, reads and searches and `hide` command are read in that syntax (`hide_session::named_prs`, `tools`, and `thread::hide_call` through the shell a thread's command call carries, [ARCHITECTURE.md](ARCHITECTURE.md)), so a PowerShell `cd .\app; gh pr view 5` names pull request 5 in `app` (`hide-session/tests/it/links.rs`, `a_recorded_call_is_read_in_the_syntax_of_the_shell_that_ran_it`).

| Runtime | Tool | macOS, Linux | Windows | Source |
| --- | --- | --- | --- | --- |
| Claude Code | `Bash` | POSIX | POSIX (Git Bash) | tools reference, "PowerShell tool"; hooks reference: on Windows without Git Bash Claude Code "doesn't register the Bash tool at all" |
| Claude Code | `PowerShell` | PowerShell | PowerShell | tools reference, "PowerShell tool": on by default on Windows, `pwsh` on macOS and Linux when enabled; its payload has the `Bash` tool's fields |
| Codex | `Bash` | POSIX | PowerShell | `codex-rs/shell-command/src/shell_detect.rs`, `default_user_shell`: PowerShell on every Windows |
| Grok | `run_terminal_command` | POSIX | PowerShell | Grok's hooks guide and `detect_windows_shell` (above); Git Bash only through `GROK_SHELL` |
| agy | `run_command` | POSIX | PowerShell, unconfirmed | no capture of a Windows call yet |
| OpenCode, Pi, omp | `bash`, `shell` | POSIX | POSIX | their bash tool runs a POSIX shell; unconfirmed on Windows |

No payload names its shell beyond the tool: Claude Code's `Bash` and `PowerShell` calls carry the same fields, and no hook variable says which shell a call runs in.
Claude Code's Windows hook entry has no matcher, so a `PowerShell` tool call reaches the helper as a `Bash` call does, and every guard judges it: the opener guard through `pre_tool::Call`, and the guardrails, the global instruction's guard and the plan record through the same syntax carried in the call they read (`guardrails::Action::Shell`, set by `guardrails::payload::action` from `hide_session::call_shell::syntax`).
What a PowerShell line means beyond its words is read in `hide-shell` too, so no guard spells it on its own: a word's `\` separates names on every system, as PowerShell reads a path (`Syntax::path_word`: `.\.env` is `./.env`, `C:\repo` is `C:/repo`), a word passes a path as a `-Name:value` parameter value and as each item of a comma-separated array as well as itself (`Syntax::path_values`: `-LiteralPath:.\.env`, `a.txt,.env`, beside both shells' `--name=value`), a command name is found without case (`Syntax::program`, the one place a command's name is read: `Set-Location`, `GIT.EXE`), `Set-Location` with its aliases `sl` and `chdir`, and `Push-Location`, move the folder as `cd` and `pushd` do (`Syntax::changes_folder`, which also says where a bare one goes), and an assignment in front of a command is not part of it, since what runs is the command after it: when a command's first word starts with `$` or `[` and an assignment operator (`=`, `+=` and the others) stands outside its brackets, everything up to the first one is the target and the rest is the command, and without one the words stay as they are (`$r == 1`, `$r -eq 1`, `[IO.File]::Exists`).

## Folder trust for the worktrees Hide makes

Claude Code, Codex, agy, Grok and Pi each ask, the first time they start interactively in a folder they have not been told to trust, whether to trust it, and a delegated child stopped at that question waits for an answer nobody is there to give.
Pi 1.0.4 asks only in a folder whose project files it would load: a `.pi/` resource of the folder (`settings.json`, `mcp.json`, `extensions`, `skills`, `prompts`, `themes`, `SYSTEM.md`, `APPEND_SYSTEM.md`), or `.agents/skills` in the folder or one above it other than the account's own (`hasTrustRequiringProjectResources` in its `dist/core/trust-manager.js`); Hide records its trust for every worktree it makes for Pi, since the files can arrive with any commit.
So before Hide starts one of them in a worktree it made for that start (`hide agent spawn` into a new worktree, a task's agent), it records that agent's own trust of the worktree on the machine that will run it: the core sends `agent_trust` to the node of the machine that runs the worktree's pane, this Mac's or a connected device's, and the node writes it there (`hide_agent_hooks::folder_trust`).
That node is the pane's, not the one the agent's PATH is checked on (none for a device, whose own helper answers for its PATH), so a worktree made on a device from the screen or the phone gets its trust like one made here; a pane whose machine cannot be reached refuses the start with `agent_trust_unavailable`.
A start in a checkout that already existed, the operator's own or an earlier worktree, records nothing, and the agent asks there as it would outside Hide.

| Agent | Where its trust is kept | What Hide adds |
| --- | --- | --- |
| Claude Code | `~/.claude.json`, or `~/.claude/.config.json` while Claude Code still keeps that older file and it is not empty | `projects["<worktree>"].hasTrustDialogAccepted: true` |
| Codex | `~/.codex/config.toml`, written by Codex's own app-server | `[projects."<worktree>"] trust_level = "trusted"` |
| agy | `~/.gemini/antigravity-cli/settings.json` | the worktree in `trustedWorkspaces` |
| Grok | `~/.grok/trusted_folders.toml` | `[folders."<main checkout>"] trusted = true` and `decided_at`, for the repository the worktree was made from |
| Pi | `~/.pi/agent/trust.json` | `"<worktree>": true` |

Claude Code's and Codex's files are found from the same folder as their hook files (`AgentRuntime::trust_path` beside `config_path`), so the trust and the hooks always land in one place.
Like the hooks, they are the places of the agent's default account for a node that honours no variable (`AgentAdapter::account_in`, [ARCHITECTURE.md, Agent profiles and accounts](ARCHITECTURE.md#agent-profiles-and-accounts)): an agent run by hand with `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `GROK_HOME` pointing elsewhere reads neither Hide's hooks nor its trust, and asks.
A start with a profile whose account is not the default has its trust written in that account's file instead: `agent_trust` names the profile, and the node resolves its account from the profile file as the start does, once the kit has judged the account's trust file Hide's to write (`hide_kit::trust_writable`, [Profile accounts](#profile-accounts)); a profile that is gone, being removed or whose file is refused refuses the start with that reason.
What Hide reads as the operator runs the agent (Codex's sign-in for usage, Hide AI and Desk) is the same default account with the process's `CODEX_HOME` handed in, so it follows a set `CODEX_HOME`.

The worktree is named by its resolved path, spelled as the agent compares it: with `/` between names for Claude Code on Windows, and in lower case for Codex on Windows, which compares folders there without case.
For Claude Code, Codex, agy and Pi the entry names the worktree itself and never the repository it was made from.
Answering an agent's own question in a linked worktree trusts the repository's main checkout, which is the operator's and would trust every worktree of that repository with it; Claude Code 2.1.302 and Codex 0.163.0 read an entry for the exact folder before the repository's, and trust given to a folder above a repository does not reach into it (measured on 2026-10-17).

Grok is the exception: Grok 1.0.67 keeps the trust of every worktree of a repository under the repository's main checkout and reads no other entry for it, so an entry for the worktree itself, or for a folder above the repository, leaves Grok asking (measured on 2026-10-23 in an isolated `GROK_HOME`; `workspace_key` in Grok's published folder trust source says the same).
Hide's Grok entry therefore names the main checkout, by its resolved path as Grok spells it (`dunce::canonicalize`: no `\\?\` prefix on Windows), and while it is there Grok also starts in the main checkout and the repository's other worktrees without asking.
Hide keeps it only while a worktree it made for Grok in that repository is there: the record holds each worktree with the entry it needed, and the entry leaves with the last of them.
A worktree whose repository has no main checkout (a bare repository's) is refused, since Grok keeps its trust under no folder Hide can name.

Pi 1.0.4 keeps one JSON object of resolved folders, each `true`, `false` or `null`, and reads the nearest decision from the folder up: the folder's own entry, else the nearest folder above it that has one, `null` being none (`findNearestTrustEntry`, measured 2026-11-02 by reading its source and answering its question in an isolated HOME).
So Hide's entry names the worktree by its resolved path as Pi resolves it (`realpath`), a folder above it the operator already answered for in Pi leaves Hide nothing to add, "Do not trust" included: Pi asks nothing once a decision is saved, so Hide writes nothing and the start goes on as it would without Hide.
Switching Pi off, like removing Hide from the machine, takes Hide's entries out of the file with Pi's extension (`Plugin::trust`), and leaves the operator's.
Hide writes the file under Pi's own lock, proper-lockfile's `trust.json.lock` folder beside the file as spelled, which Pi takes around every read and write and gives up on after ten tries 20 ms apart, so Hide holds it only across its own read and write and Pi's start does not meet it; a lock left more than ten seconds is taken over as Claude Code's is.
Hide writes the folder the agent folder would be if Pi has not made it yet, as Pi does, so a Pi started there for the first time finds the trust.

Hide keeps which entries it added in its own record, `~/.hide/agent-hooks/<agent>-trust.json` (`claude-trust.json`, `codex-trust.json`, `antigravity-trust.json`, `grok-trust.json`, `pi-trust.json`), by the worktree each was added for and, for a profile account, the account it was added in, at most 4096 folders that exist per agent; a trust past that is refused with the cap in its reason.
An entry in a profile account is written in a form an earlier build reads as the default account's, so an earlier build that removes the worktree takes the default account's entry out and leaves the profile account's.
The record is written before the entry and taken back when the entry could not be written, so Hide never leaves an entry its record does not name.
An entry that is already there is the operator's: Hide writes nothing and records nothing, so removing the worktree leaves it.
Removing a worktree Hide made takes out the entries Hide added for it, for each agent and in every account it was added in, unless another worktree Hide trusted still needs the same entry in that account (Grok's main checkout), and switching an agent off, or removing the machine from Hide, takes out every entry Hide added for that agent, the default account's with its hook and each profile account's with that account's pieces; an entry the operator added, before Hide or after it, stays, and so does what else the agent keeps under the same folder (Claude Code's project settings, a Codex project's other keys).
The removal runs once the folder is gone, so it finds the entry by the key Hide recorded, compared as the system compares folders: the folder's parent resolved, and on Windows without case and with either separator, so Git's `C:/x` finds Codex's `c:\x`.
A worktree removed outside Hide (by hand) leaves its entries until the next trust Hide records for that agent, which takes out every entry of Hide's whose folder is gone in the same write.
Codex's entries are taken out through its app-server too; with no Codex left to ask, they and Hide's record of them stay until a pass that finds one, and nothing reads them meanwhile.

Every change holds one account lock per agent, and a change waits for it as long as the change holding it can take: one Codex app-server session's whole bound (`codex_trust::SESSION_BOUND`, 21 s) and a second, so a second start while Codex records the first one's trust waits instead of failing.
The core gives the node twice that and five seconds for the call (`folder_trust::Agent::longest_change`); a Codex that answers, as it does in well under a second, takes about a second.
The JSON files are rewritten through the same lossless read and compare-before-replace as the hook files, so their other keys and the operator's formatting keep their bytes.
Claude Code rewrites `~/.claude.json` while it holds its own lock on it, the `.lock` folder beside the file's real path (proper-lockfile resolves a link, so a linked `~/.claude.json` is locked beside its target), and reads the file again under that lock, so Hide changes the file only while it holds the same lock and neither overwrites the other.
Hide waits up to two seconds for it and takes over a lock older than ten seconds, which Claude Code treats as left by a process that ended, removing only the folder it saw stale; it removes its own lock only while the folder there is still the one it made, and does not write once Claude Code has taken the lock over.
Hide's own changes never meet at Claude Code's lock, since each holds Hide's lock first; between Hide and Claude Code, a takeover of a stale lock has the moment between the look and the removal that Claude Code's own takeover has.
Grok replaces `trusted_folders.toml` whole while it holds an exclusive lock on `trusted_folders.toml.lock` beside it (`fs2`: `flock` on Unix, `LockFileEx` on Windows) and reads the file again under it, so Hide reads and replaces the file only while it holds the same lock, waiting up to two seconds for it; Grok rewrites the file from its `folders` alone, and Hide changes only the entries it adds or takes out.
Codex's file is never written by Hide: `hide_agent_hooks::codex_trust` starts `codex app-server`, reads the user layer's `projects` with `config/read`, writes the one key with `config/batchWrite` against the version it read, and reads it back, as it does for [Hide's own hooks](#codex-trusts-hides-own-hooks).

A trust that cannot be recorded refuses the start before the agent runs, with `agent_trust_failed: <agent>'s trust of <worktree> could not be recorded (<reason>)`, so no agent waits unseen on its question (PRD antigravity-first-class B20).
The node refuses to trust a folder that is not a Git worktree, a path written with `.` or `..`, the filesystem root, the home folder or a folder above it, whoever asks.
When the operator marked the worktree, or the repository's main checkout it was made from, `untrusted` in Codex, or answered Grok's question for that main checkout with No (`trusted = false`), Hide does not override that answer: the reason names that folder and says to trust it in that agent or to start the agent there by hand.
A Codex configuration the operator's organization manages that decides the folder's trust instead, a machine with no Codex Hide can find, a Claude Code or Grok lock held past two seconds and a settings file that does not parse refuse the start too, each with its reason; a machine that is not connected refuses it with `agent_trust_unavailable`.

`hide-agent-hooks/src/folder_trust/tests.rs` checks each agent's format, spelling, ownership and removal in a fixture home on every pull request, on Linux in `rust` and on Windows in `windows check`, and `tests/it/codex_folder_trust.rs` runs Codex's through a stand-in app-server on Linux.
The nightly `agent trust contract` (`.github/workflows/agent-trust-contract.yml`) installs the pinned Claude Code and Codex on all three systems and checks that the real ones read what Hide wrote: Codex's app-server loads a worktree's own settings once Hide trusted it and stops once Hide took the trust out (`tests/it/codex_binary.rs`), and Claude Code in a pane of the core's own engine starts at its prompt in the worktree Hide trusted and asks its question in the repository's other worktree (`hide-core/src/agent_start/real_agent_trust_tests.rs`).
Grok's real reading is measured by the agent journey's `start_untrusted` cell ([VERIFICATION.md](VERIFICATION.md)), on macOS only so far; no nightly lane installs Grok.
Pi's is measured by the same cell on Linux, macOS and Windows: the never-trusted repository holds `.pi/settings.json`, so the untrusted start reaches its prompt only through the trust Hide recorded.
The pull request runs no registry install, so a failure that is not Hide's never holds one; a change to the trust writer is proven on all three systems with `gh workflow run nightly.yml --ref <branch> -f lane=agy`.

## Project guardrails

The pre-tool hook also refuses what a project's protections name, the four switches of Project Settings > Working style ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#project-settings), PRD agent-guardrails).
They are protections, not a security boundary: they stop the ordinary tool calls and shell words below, and a write a script, `sed -i`, `bash -c`, `$(...)`, an alias or an MCP server makes is not seen (D-18).

The core publishes one policy file per machine, `~/.hide/agent-hooks/guardrails.json` (`hide_agent_hooks::guardrails::policy_path`), naming each project of that machine with a protection on: its main worktree root, the base branch the operator chose (none for Auto, which reads `origin/HEAD`, or in a repository with no remote the branch its main worktree has out, as the worktree reader names the default branch), its four switches, the merge option and its protected paths.
This Mac's file is written by its own node and a connected device's by that device's node, through `Call::GuardrailPolicy` ([ARCHITECTURE.md](ARCHITECTURE.md#project-guardrails)); a saved switch shows Saved only after that machine has the new file, and every daemon start and device connection writes the file again from the current settings, empty when nothing is on.
The hook reads that file and the repository's own Git files on the machine the agent runs on and asks the daemon nothing, so a saved change applies from the agent's next tool call without a restart.
A project removed from Hide loses its protections, and a project turns them on again only from Project Settings.

What each protection refuses, in any checkout of the project and in any agent session there, a pane or not (B22): outside a pane the hooks and scripts run the guardrails and nothing else.

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
A `cd` or `pushd` earlier in a shell line (in PowerShell also `Set-Location`, `sl`, `chdir` and `Push-Location`) moves where the commands after it run (a bare `cd` goes home), and `git -C <dir>` moves one command; the line is read in the [syntax of the shell that runs it](#which-shell-reads-a-call), one simple command at a time with quotes, heredoc bodies and comments set aside, and a line that shell would refuse to parse holds no command and runs.
In PowerShell a backslash path is the path as written, on every system: `Get-Content .\.env`, `Get-Content -LiteralPath:.\.env` and `Get-Content a.txt,.env` read the protected `.env`, and `git -C C:\repo` and `Set-Location ..\repo` aim at that checkout; a command name is matched without case (`Git push --force`), and an assigned command is judged as itself, whatever the target (`$r = git push --force`, `[int] $r = git push`, `$r.x = git push`, `$h['k'] = git push`, `$a, $b = git push`).
An array item is read wherever the comma stands, inside quotes too, so `echo 'a,.env'` is refused: the reading refuses more than PowerShell would pass a path, never less.
A word holding a variable (`$PWD\.env`, `$env:USERPROFILE\.ssh`) is not read in either shell, since its path is known only when it runs.
In Git Bash, the shell Claude Code runs on Windows, a backslash path is read as that shell runs it: quoted (`git -C 'C:\repo'`) it is the path, unquoted its backslashes are escapes, there and in the hook alike; a drive mount (`/c/work/...`, `/cygdrive/c/...`) is the drive's path (`hide_platform::path::shell_drive_path`).

The refusal reaches the agent in English, in its runtime's deny shape, and names the protection, what the call did and what to do instead: `Not run: the "Guard files" protection is on for this project in Hide. This call reads or changes a path on the project's protected list. Ask the operator ...`.
Each refusal appends one `guard.refused` line (rule, whether a fact was unread, runtime, pane; never the command, its paths or a file's contents) to `spawn-guard.log` in the hooks' records folder, and the operator's screen shows nothing for it.

How it fails (D-27):

- A fact a protection that is on needs cannot be read (the checkout's repository, its branch, the base branch, whether it has changes): the call is refused and the reason says which fact; with no base branch to read (Auto, no `origin/HEAD`, and a remote, so the main worktree's branch does not count), the reason asks the operator to choose one in Project Settings.
- The policy file is missing, unreadable, corrupt or of another version: the call runs, and one `policy.unreadable` line with its cause is appended to the same log, throttled to one per ten minutes through the delivery diagnostics' store (cause `guardrails`).
- The hook panics, or the payload does not arrive within 0.5 seconds, while some project has a protection on: the call runs, and one `guard.failed` line with its cause (`panic`, `payload_deadline`) is appended, throttled the same way (cause `guardrails_failed`).
  A hook the runtime kills at its own timeout leaves no line, since the process is gone, and the call takes that runtime's default.
- A payload past the helper's 256 KiB read is judged by the tool name and the paths that arrived, a patch's file lines included; a shell command cut short is not judged.
- agy sorts its payload's keys, so a long file write is cut inside its new text, before the tool's name and target; such a write is refused in a project where a protection judges writes, since its target may be one the protection guards.

The payload each runtime sends, observed on the versions Hide installs into (`hide_agent_hooks::guardrails::payload`):

| Runtime | Reads | Writes | Shell |
|---|---|---|---|
| Claude Code 2.1.300 | `Read` | `Edit`, `Write`, `MultiEdit`, `NotebookEdit` (`file_path`, `notebook_path`) | `Bash`, `PowerShell` |
| Codex 0.162.1 | no read tool; the shell words only | `apply_patch` (the patch's file lines) | `Bash` |
| Grok 1.0.52 | `read_file`, `Read` | `search_replace`, `write`, `Edit`, `Write` | `Bash` |
| OpenCode 1.18.30 | `read` | `edit`, `write`, `apply_patch`, `multiedit` (`filePath`, `patchText`) | `bash` |
| OpenCode 2.0.29 | `read` | `edit`, `write`, `patch` (`path`, `patchText`) | `shell` |
| Pi 1.0.4, omp 18.7.0 | `read` | `edit`, `write` (`path`) | `bash` |

A key another version renames reads as no path, and the call runs.
Claude Code's and Codex's hooks select the file tools from matcher version eight (`hide-subagents@8`) and Grok's from `hide-guidance@5` (from `@6` their entries carry no matcher at all, so every tool call reaches the hook for the letters due while the agent works); OpenCode's plugin (version 2) and the Pi and omp extension (version 4) send a file tool to the helper only while some project of the machine has a protection on, read again when the policy file changes, so a machine with none starts no helper for a file tool (B27); a missing or unreadable policy sends the call in a pane, where the helper records why, and starts nothing outside one.
A session outside a pane, or an agent started in another agent's shell, says so (`guard_only`), and the helper then runs the guardrails alone, whatever pane variables the shell inherited.
A command hook pays one policy read per call and parses the payload only when some protection is on ([PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-guardrails-on-a-pre-tool-call)).
`hide-agent-hooks/src/guardrails/tests.rs` holds each runtime's payload and each protection's table tests.

## The pane's plan record

The prompt hook and the pre-tool hook know a pane's plan and mission work from one small file, `plans/<pane>.json` in the hooks' records folder (`hide_agent_hooks::plan_record`), written whole and private to the account: `{"version":1,"session":<native session>,"context":<plan_context>}`, under 16 KiB (PRD hide-plan D-14, D-15, D-30).
`plan_context` is the summary the core sends ([delivery.md: Plans](delivery.md#plans)), fixed in `contracts/delivery-envelope.json`; the prompt hook writes it from its letter pull's answer, which already asks the core, and every `hide task` command writes it from its own answer, so an approval lets the next write through in the same turn.
A pane with no record, and a record of another session than the call's, gives neither a line nor a refusal, so a new session in a reused pane starts clean.
A record written before node criteria still reads: a mission's missing `role` is a worker's, and missing `line`, `criteria` and `unmet` are empty, so it gives the lines it gave before.
The other way round, a hook from before node criteria reads its records with unknown fields refused, so a record a newer core wrote reads as no record there: that hook gives no plan or mission line and refuses nothing until the kit installs the hook of the running Hide, which a Hide update does.

The prompt hook adds the record's lines to the session context:

- the plan's, `Task: now <step> (n/m) · next <step> · updated <N> min ago`, with the review's rule after it while the plan waits for review, or, when it was not updated for 30 minutes, the `hide task node <key> --status done` command; a finished plan has none;
- a mission worker's, `Mission <id> · node <key> · range <its own lines or title> · criteria <met>/<total> · left: <keys> · no merging · no starting agents`, without `no merging` for a mission that goes through the merge, and without the criteria for a mission made before node criteria (PRD mission-map-git-criteria D-19, B22); while a line is left, a second line names `hide mission record <node> <line> --pass|--fail --evidence <one line>`, the check line with `--commit <sha>`;
- a checker's, `Mission <id> · checking node <key> (<review or verify>) · range <its own lines or title> · criteria …`, followed by the one `hide mission record` command for its own line;
- a Conductor's, `Mission <done>/<nodes> · working <n> · letters <n> · decision waiting: <yes/no>`, with ` · reported with lines left: <nodes>` when a worker sent its report while its node still has a line open.

The lines only tell: no hook refuses a call or ends a turn because a line is open, and the core alone refuses a node's `done` (D-12).

The pre-tool hook reads only the record, after the project guardrails and before the opener guard, and never asks the daemon; a tool call refused here is logged in the hooks' log as `plan.refused` with the rule, the runtime and the pane, never the command.
It refuses three things and lets everything else run, reading and searching included:

- a file write, `git commit` or `git push` while the plan waits for the operator's review (`plan_proposed`), with `hide task approve` as the way on and `hide task set --json '<plan>' --ask` as the way to send a revised plan without writing a file;
- `gh pr merge` by a worker of a mission that goes up to pull requests (`mission_merge`);
- `git push` with `--force`, `--force-with-lease`, a short flag holding `f` or a `+` refspec, and `git rebase` other than `--abort` or `--quit`, in any pane with a record (`history_rewrite`).

The shell words are read the way the opener guard reads them (`hide_shell::shell_line`), each command of a line on its own, so `cd repo && git commit` is seen; a write a script or `bash -c` makes is not, as the guardrails say of theirs.
OpenCode's plugin and the Pi and omp extension send a file tool to the helper only while a project guardrail is on, so on those agents the write refusal of a plan under review holds only then; their shell calls reach the helper and are refused the same way ([INSTALL.md](INSTALL.md#what-each-system-supports)).
`tests/it/plan_record.rs` runs the compiled hook beside a stand-in `hide` that records every call: the prompt hook's one letter pull writes the record, and the pre-tool hook then refuses a write and a commit and lets `git status` and a read run with no further call.

## OpenCode: Hide's plugin

OpenCode has no command hook; its official extension point is a JavaScript plugin whose hooks the SDK types (`@opencode-ai/plugin`) declare ([plugins](https://opencode.ai/docs/plugins/)).
Hide writes into OpenCode's global plugin folder `~/.config/opencode/plugins/`, beside any `herdr-agent-state.js` an earlier build's Herdr integration left there, and nothing else of OpenCode's: `hide.js`, the plugin OpenCode 1.x loads, and the folder `hide-opencode/` with `server.js` and `tui.js`, the plugin OpenCode 2 loads ([OpenCode 2](#opencode-2)).
OpenCode 1.x loads only the files of the folder, so it never sees `hide-opencode/`, and OpenCode 2 loads `hide.js` as a plugin that does nothing.
`hide-agent-hooks/src/opencode.rs` declares the three files and `hide-agent-hooks/src/plugin.rs`, the owned script files they share with Pi's and omp's extension, writes, judges and removes them together; their sources are `hide-agent-hooks/src/opencode/{plugin,server,tui}.js` with this build's helper path filled in.
Each file's first line, `// <hide-opencode-plugin|hide-opencode-server|hide-opencode-tui>@<version> sha256=<hash of the rest>`, proves the file is Hide's, carries its version and shows whether it was edited; `hide.js` and `server.js` hold `const HELPER = "<absolute path of this build's hide-agent-hooks>";`, and `tui.js` takes it from `server.js`.
The current plugin, server and TUI payload versions are 12, 6 and 3; Pi and omp use extension version 13.
OpenCode's configuration folder is never created: with no `~/.config/opencode` the row reads Absent and says OpenCode has not created it yet, and only the `plugins/` folder inside an existing one and Hide's `hide-opencode/` inside that are made.

On OpenCode 1.x the plugin does nothing outside a pane Hide manages: with no `HIDE_PANE_ID`, or no helper at its path, it registers no hook and starts no process, so OpenCode in an ordinary terminal runs as if the file were not there.
Inside one it uses three hooks, each calling `hide-agent-hooks opencode <start|prompt|confirm|tool|subagents|turn>` with one JSON object on stdin and one answered on stdout; the helper always exits 0 and a failure answers `{}`:

- `chat.message` adds Hide's text to a root session's prompt as one synthetic text part wrapped in `<system-reminder>`: the session guidance (the same text the `SessionStart` hook gives, read once when OpenCode loads the plugin) on the session's first prompt, then Project Memory under Claude Code's rules (the session-start index with the first prompt's cards on the first prompt, the prompt's cards after), then the pane's waiting letters ([docs/delivery.md](delivery.md)).
  OpenCode's TUI does not show a synthetic part, and the title and label readers skip it, so the operator's text stays theirs; a subagent's prompt and a prompt of only synthetic text get nothing.
  The part's id is made the way OpenCode makes its own (`prt_` and a time-ordered id), so it sorts after the operator's text.
- `event` confirms letters once `message.part.updated` reports Hide's part stored, and counts subagents: a `session.created` with a parent is a child, `session.status` busy and idle move it between working and done, and a root that goes idle while a child still runs in the background keeps it counted (OpenCode's `session.status` list is read then).
  The counts and the root session's turn go to the pane's record and to the core as the Claude Code and Codex hooks report theirs (`subagents` and `turn`, [The turn the hook reports](#the-turn-the-hook-reports)).
- `tool.execute.after` asks, for the pane's root session at every result except while letters it handed over wait for their receipt, for the letters due while the agent works (`letters`, which asks what `hide inbox --hook --session <id> --tool` asks), appends them to the tool's `output` inside `<system-reminder>`, and confirms them through `event` once `message.part.updated` reports that session's completed tool part stored with the letters still in its output; a part stored without them leaves the letters pending in Hide for the next ask ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); nothing due, a failed helper and a child session leave the result as it is.
- `tool.execute.before` sends a `bash` call that can hand a result to the system opener to the [opener guard](#the-opener-guard), and throws the deny reason; OpenCode hands a thrown error to the model as the tool's error, so the model reads the same reason Claude Code's model does.
  While some project of the machine has a protection on, every `bash` call and the file tools `read`, `edit`, `write`, `apply_patch` and `multiedit` go to the helper too, with only the arguments that name a file (`filePath`, a patch's `*** ` lines), for the [project guardrails](#project-guardrails).
  The plugin reads the policy file again only when its modification time or size changes, so a machine with every protection off starts no helper for them (PRD agent-guardrails B27).
  Any other tool call never reaches the helper.

Each hook has a budget: 1.85 seconds for a prompt (the Claude Code prompt hook's), 2.5 seconds for a tool call (the opener guard's), 1.5 seconds for a tool result's letters, 2 seconds for a confirmation or a count and 8 seconds for the one guidance read at load.
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
- A tool call asks the attached terminal for the [opener guard](#the-opener-guard) (a `shell` call that can hand a result to the system opener), while some project has a protection on, the [project guardrails](#project-guardrails) for `shell`, `read`, `edit`, `write` and `patch` (only `path` and a patch's `*** ` lines); the deny reason is thrown, as in 1.x.
  A `shell` call's relative `workdir` is resolved against the session's folder, where OpenCode runs it (1.x's against its project folder too), never the shared server's own.
  A session whose line OpenCode does not tell within 300 ms is taken as its own root, as 1.x takes it, and any terminal showing its root is asked.
  With no terminal attached, or none answering in time, the server half runs the guardrails alone (`guard_only`), with the pane variables it inherited removed, so a protected project stays protected in `opencode run` and outside a Hide pane; the opener guard then lets the call run, as a timed-out helper does in 1.x, and the miss reaches the helper's diagnostic with the next call.
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
omp is a fork of Pi with the same extension shape, so Hide writes one source, `hide-agent-hooks/src/pi_extension/extension.js`, as `hide.ts` into `~/.pi/agent/extensions/` and `~/.omp/agent/extensions/`, beside any `herdr-agent-state.ts` and `herdr-omp-agent-state.ts` an earlier build's Herdr integration left there, and nothing else of either agent's (PRD pi-omp-extension D-01).
`hide-agent-hooks/src/pi_extension.rs` declares both files on the owned script file of [OpenCode's plugin](#opencode-hides-plugin), so they are written, judged and removed by the same code.
The first line, `// hide-extension@<version> sha256=<hash of the rest>`, works as the plugin's does; the body holds `const HELPER`, `const AGENT = "pi"` or `"omp"` and `const VERSION`, and the two files differ only in `AGENT`, so the other agent's file copied into a folder reads Outdated and is replaced.
The file is plain JavaScript that is also TypeScript, as Herdr's own extension for these agents is, so both hosts load it without a build step.
It is written on Windows too: on a `windows-latest` runner on 2026-10-11, Pi 1.0.4 (Node 22) and omp 18.7.0 (Bun 1.3.14) loaded an extension from `%USERPROFILE%\.pi\agent\extensions` and `%USERPROFILE%\.omp\agent\extensions`, emitted `session_start` to it, and its `spawn` of an absolute `.exe` path with no shell crossed stdin and stdout and ended with `SIGKILL`; `hide-agent-hooks/tests/it/windows_scripts.rs` runs this build's file for both on Windows as it does OpenCode's plugin, with the session file named by a `C:\` path as Windows reports it.
A session file counts as saved when it is absolute in either system's spelling (`path.posix.isAbsolute` or `path.win32.isAbsolute`, the check Herdr's own Pi integration makes), so the same string is judged alike on every system; a relative path is an unsaved session.
The agent's own folder (`~/.pi/agent`, `~/.omp/agent`) is never created, and only those default folders get the file: an omp profile or a `PI_CODING_AGENT_DIR` folder gets none.

The extension does nothing outside a pane Hide manages (no `HIDE_PANE_ID`), in an agent started from another Pi's or omp's shell (`PI_SESSION_ID`, `OMPCODE=1`, which those hosts set for every shell they start), or with no helper at its path; like OpenCode's plugin it does not ask whether a Hide daemon owns that pane, and the helper answers `{}` when none does.
Its guidance and letters go only to the pane's own agent, the host's TUI (`ctx.mode` `tui`): both hosts' RPC modes report a UI too but draw nothing in the pane, and another program there may start one, so they, print runs, unsaved sessions and omp's subagents, which run in the same process with their own binding of the module, get none.
The opener guard holds for every session in the pane; Pi and omp question tools keep their native behavior.
It names the session by its file, as Herdr's integration did, which a deep checkout makes longer than 250 bytes; the helper, the CLI and the daemon take a session of up to 4096 bytes for a letter pull (`hide_agent_hooks::delivery::SESSION_LIMIT`), and an agent registration keeps the same ([delivery.md: Agent registration and spawning](delivery.md#agent-registration-and-spawning)).
Each handler calls `hide-agent-hooks <pi|omp> <start|prompt|confirm|tool|subagents>` with one JSON object on stdin, which carries the extension's version, and one answered on stdout; the helper always exits 0, a failure answers `{}`, and a version this build does not write also answers `{}`, so a file another build left does nothing until the kit replaces it.

- `session_start` and `session_switch` read the session guidance once, as OpenCode's plugin does at load; a read that failed is asked again by the next prompt.
- `before_agent_start` returns one custom message for the prompt, wrapped in `<system-reminder>`: the guidance until a message that carried it has been written, then Project Memory under Claude Code's rules (a session-start index, asked for on each prompt until one is given and the message carrying it is written, tracked apart from the guidance so it is neither repeated nor held back by it, then the prompt's cards on each later prompt; unlike Claude Code's one `SessionStart`, a session whose first prompt found Memory off or busy gets its index once Memory answers), then the pane's waiting letters ([docs/delivery.md](delivery.md)).
  Letters know the session by its file and Memory by the host's own session id (`sessionManager.getSessionId()`), which is what Pi's and omp's session reader keys a receipt by; the extension sends both.
  The message is `display: false`, so the host keeps it off the screen and out of the title while the model reads it, and a letter cannot close Hide's reminder tags.
  Its `details` carry a fresh id per attempt, because omp can prepare one submission more than once and keeps only the attempt it delivers.
- `message_end` confirms the letters once the host has written Hide's message and then the reply after it, and only for the attempt it wrote; a turn that ends before any reply leaves them pending for the next prompt or `hide inbox`, and a confirmation Hide did not take is made again when the letter rides the next prompt.
  A tool result's letters (`toolCallId`) join the written list when the host finishes that message with Hide's text still in its `content` and are confirmed by the reply that follows, since a later `message_end` handler may replace the result; a result finished without the text leaves them pending in Hide for the next ask.
- `tool_result` asks, for the pane's own agent at every result except while letters it handed over wait for their receipt, for the letters due while it works (`letters`, which asks what `hide inbox --hook --session <file> --tool` asks) and adds them to the result's `content` as one more text block inside `<system-reminder>`, keeping a tool's `structuredContent` ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); nothing due, a failed or late helper, a print run, an unsaved session and a subagent leave the result as it is.
- `tool_call` sends a `bash` call that can hand a result to the system opener to the [opener guard](#the-opener-guard), and returns `{block: true, reason}`, which the host hands to the model as the tool's result.
  While some project of the machine has a protection on, every `bash` call and the `read`, `edit` and `write` tools (their `path`) go to the helper too, gated on the policy file as OpenCode's plugin is, for the [project guardrails](#project-guardrails).
- In omp, `before_subagent_spawn` counts a subagent as working from its dispatch (its spawn key is the subagent's id), and that subagent's own `agent_start` and `agent_end` move it between working and done; the counts go to the core as the other hooks report theirs, zero when the session starts and again at each turn's end.

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
A row carries the agent's program names (`executables`), the folder it reads skills from, whether Hide writes a hook for it, the kind name Herdr's integration gave it, and the official page the row's answers come from (`doc_url`).
A test fails a row with no `https` `doc_url`, and a row with no program, so a claim in the table below always has a page behind it and every agent can be found.

### Which agents Hide supports

The support list began as the agents Herdr shipped an integration for (`herdr integration install <target>`), and a Herdr integration alone never granted Hide support (Gemini CLI left for that reason and is retired below).
Lineage, the mailbox identity, labels and sleep all read the session id the agent's own Hide piece reports ([Which session is the pane's](#which-session-is-the-panes)), and an agent without one could only have its state judged from its screen, a second kind of row that behaves unlike every other; so every supported agent reports one.
`AgentAdapter::herdr` is required, not optional: its `name` is the kind the product knows the agent by, and its `target` the `herdr integration install` name an earlier build used.
Among the supported agents, one gets the multi-agent collaboration tier (letters, the opener guard, Memory and the subagent count) when its official documentation or SDK types give a hook or plugin that can both put text into the prompt and refuse a tool call; today that is Claude Code and Codex, through their native hooks, OpenCode, through Hide's plugin, Pi and omp, through Hide's extension (Pi without a subagent count, since Pi runs none), and Antigravity, through Hide's `hide` block ([Antigravity: Hide's hook block](#antigravity-hides-hook-block)), with no bell yet.
Grok keeps the Basic tier: its hook carries the opener guard and subagent count, takes letters through its tool and turn-end events and is a bell target at rest.
It carries no new operator prompt's letters or Memory.
A new agent follows [Adding an agent](#adding-an-agent), which starts from these facts; the web shell reads the generated `contracts/agent-adapters.json` rather than maintaining its own order, names or links.

### ACP servers

The [Agent Client Protocol](https://agentclientprotocol.com) is how an editor drives a coding agent over stdio and receives its conversation as typed updates (messages, tool calls with their kind, diffs and output, plans).
Hide runs each agent in a pane and reads the agent's own session files (`hide-session/`); the one agent it drives over ACP is Desk's, through the pinned adapters in `contracts/desk-adapters/` ([ARCHITECTURE.md: Desk](ARCHITECTURE.md#desk)), with no pane and no hook.
An agent's ACP server is still the vendor's own answer to how its conversation is shaped, so it is the reference to read before mapping that agent's session records onto a conversation view.

Find an agent's server in the ACP registry: [zed.dev/acp](https://zed.dev/acp) lists every registered agent with its page, and the machine-readable list is `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`, which names each server's version, launch command and per-system download with its sha256.
A server built into the CLI can be missing from the registry, so also read the CLI's own help.

As measured on 2026-10-30:

| Agent | ACP server |
| --- | --- |
| Claude Code | `@agentclientprotocol/claude-agent-acp` (registry `claude-acp`), a separate package |
| Codex | `@agentclientprotocol/codex-acp` (registry `codex-acp`), a separate package |
| OpenCode | built in, `opencode acp` (registry `opencode`, `opencode-v2`) |
| Pi | `pi-acp` (registry `pi-acp`), a separate package |
| omp | built in, `omp acp`; not in the registry |
| Grok | built in, `grok agent stdio` (registry `grok-build`); the top-level `grok --help` does not name ACP |
| Antigravity | built in since 1.4.12 as `agy acp` (`Antigravity ACP Server`, measured 2026-11-01); before it `agy_acp_server.par` (registry `antigravity-acp`), a separate program Google publishes for macOS, Linux and Windows |

`session/load` is not a read-only way to look at a session: on 2026-10-30 claude-agent-acp 0.97.0 appended a `cost-state` record and codex-acp 2.3.0 a `thread_settings_applied` record to the session file it loaded, and codex-acp refuses to load a sub-agent's session without its parent.
Replay quality differs by server: pi-acp 0.0.34 replayed tool calls titled only `bash`, `read` or `edit`, with no input, location, diff or command output, all of which Pi's own session file keeps.
An ACP server can keep sessions of its own: Antigravity's `agy_acp_server.par` 1.4.5 looks for `~/.gemini/antigravity-acp/conversations/<id>.db` and refused a conversation the `agy` CLI wrote under `~/.gemini/antigravity-cli/brain/`, and it asks for a sign-in of its own (its `authenticate` method or `auth.type` in `~/.gemini/antigravity-acp/settings.json`) rather than the CLI's.

### Adding an agent

Prove it on the real agent first, before any code, and pin what you measure with a test:

- A vendor document is a lead, not a fact.
  On 2026-10-16 agy 1.4.0 ignored `ANTIGRAVITY_CLI_CONFIG_DIR`, and its `/skills` listed a folder whose skills its model never received.
  Prove a skill by asking the model for its own list of skills, not by reading the agent's listing.
- Measure the hook failure matrix for the agent: a missing helper, a helper that is not executable, a crash, a non-zero exit, empty or malformed output, and a timeout.
  agy refuses the tool call on every one of them but an empty answer with exit 0 (measured 2026-10-16 and 2026-10-17), so each case has a test that the installed command still answers (`guidance::tests::agys_tool_command_leaves_the_call_to_agy_when_the_helper_is_gone_cannot_run_or_fails`).
- Measure how the engine's screen rules read the agent's permission and folder-trust prompts before relying on Needs You or the bell.
  On 2026-10-16 a probe outside the app read agy's permission question as `idle`, and the running app then showed Herdr 0.9.3 reading it as an approval; measure in the app, and its folder-trust question is still unmeasured, which is why agy has no bell.
- Record the agent version every fact was proven on.
  agy moved from 1.3.6 to 1.4.0 on 2026-10-16, during the work, so every fact proven on 1.3.6 had to be proven again on 1.4.0.

Then add it in this order.
Each line says what to add, where, and the check that fails while it is missing; "no check" marks what only a person can see.
`python3 -m unittest scripts/tests/test_agent_journey_contract.py` reads every piece a script or a guide holds and names, in one verdict per agent, all it still lacks; the lines below call it the journey contract.

1. Vendor facts: the page that names the skills folder and the program the installer puts down, and a published mark (`docs/BRAND.md`) or a monogram; no check, so the row's `doc_url` cites that page.
   Note its ACP server too, from the registry or the CLI's help, in [ACP servers](#acp-servers); no check.
2. Declaration: an `AgentId` variant and one row of `ADAPTERS` in `hide-agent-adapter/src/declarations.rs`, with a canonical id that stays stable across records, hook commands and device reports, and the aliases on the row; the compiler and `declarations_match_independent_support_samples`.
   Its definition file's `[account]` names its account's places and the variables that move them ([ARCHITECTURE.md, Agent profiles and accounts](ARCHITECTURE.md#agent-profiles-and-accounts)); every reader and installer of those places asks `hide_agent_adapter::resolve`; the definition reader, `hide-agent-adapter/tests/it/account.rs` and `scripts/check-agent-folders.sh`.
3. Capabilities: `guidance`, `memory` and `titles` as `Support::Supported` with the typed route or reader, or `Support::Unsupported(CapabilityAbsence)` with a reason, evidence and what would close it, and prompt intake, tool-call refusal (`guard`), subagent counts and bell each declared on its own; `context_and_title_support_require_matching_routes_or_documented_absences` and the bell assertions of `declarations_match_independent_support_samples`.
4. Hook, plugin or extension: a `HookDialect` or `PluginDialect` implemented in `hide-agent-hooks`, with its Windows form and its installed-command fixtures; the compiler's exhaustive matches, `adapter_contract` and `windows_hook_command`.
5. Session reader: a `SessionFormat` implemented in `hide-session`, and the agent's hook and session samples (or an absence reason) and feature list in `hide-agent-adapter/tests/fixtures/support.json`; `declarations_match_independent_support_samples` and the core's `all_core_gates_accept_the_same_aliases_without_enabling_other_agents`.
6. Thread: a converter for its `SessionFormat` in `hide-session/src/thread/` that draws the session as ACP session updates, mapping each tool's kind, title and content as the agent's ACP server does ([ACP servers](#acp-servers)); `thread` on the row as `Support::Supported(ThreadReading)`, which lists each part (an image, a file change, a command's output) its records cannot show with the reason, or `Support::Unsupported` with its absence; and a sample of the same task every agent records (read an image, write a new file, edit a file, run a command with output, and keep a plan where the agent has one) under `hide-session/tests/fixtures/thread/<agent>-<version>/`, registered in `SAMPLES` of `hide-session/tests/it/thread.rs`; the compiler (the exhaustive match of `thread::reader`), `every_declaration_matches_its_converter_and_has_a_sample` and `every_agents_sample_reads_to_the_task_it_recorded`.
7. Composer: its declaration and six captures in `hide-agent-adapter/tests/fixtures/composer/`; `tests/it/composer.rs`.
8. Usage: every cell of its `UsageDeclaration`, then `contracts/usage-coverage.json` regenerated; `every_usage_cell_has_an_independent_declaration_and_unshipped_absences` and `usage_coverage_is_generated_from_the_complete_registry`.
9. Launch, resume, fork and sleep: a `LaunchDialect` in its owners, with its folder-trust answer (`LaunchDialect::trusts_worktrees`, [Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)) and its full-access option (`LaunchOptions::full_access`: `FullAccess::Options` with the option its `--help` names for running without permission questions, plus any option that keeps the CLI from stopping to confirm it, or `FullAccess::NotAsked` for a CLI that asks none; start the CLI with it once in an empty HOME and a trusted folder to see whether it stops to confirm, as Claude Code does), which Settings › Agents › 권한 adds to every start and restart Hide makes; read in the CLI's help where the option goes relative to its resume and fork arguments; the compiler (every arm of `LaunchDialect::options()` fills the field), the start-dialect tests of `hide-agent-adapter/tests/it/contract.rs` (`every_started_agent_declares_how_it_starts_without_permission_questions`), the `agent CLIs` lane, which finds the declared option in the pinned CLI's `--help` on every system (or, for a CLI whose help cannot prove it, such as Codex's `-c` overrides, requires the access the words give in what the CLI renders with them, `FULL_ACCESS_PROOF` in `scripts/install-agent-clis.py`), the core gates test, and the fake-model journey's `start_untrusted` cell.
   Its first input is typed by default; declare the launch prompt in its `input` only for the systems the journey's `start` and `launch_brief` cells prove, under the one standard of [delivery.md](delivery.md#the-input-ladder).
10. Title measurement: automatic or operator-named for its session format, in the exhaustive match of `hide-agent-adapter/src/journey.rs`; the compiler.
11. Skills folder and kit case: a `SkillLocation`, and the agent's expected features in `hide-kit/src/tests/agent_cases.rs`; the compiler and the kit tests, which list every id.
12. Program: the vendor's program names in `executables`, found only through `hide_platform::programs`, with a vendor install folder outside the usual ones added there; `no_product_code_outside_the_platform_looks_for_a_program_on_its_own` refuses a search of its own, and no check sees a missing folder, so install the CLI the vendor's way on each system and see Settings find it.
13. Minimum version: `minimum` with its evidence on the row, proven on macOS, Linux and Windows; `every_supported_agent_declares_a_comparable_minimum_version_with_its_proof`, and the journey contract for its row in [The oldest version of each agent](#the-oldest-version-of-each-agent).
    Its `updates` name each documented way of installing the CLI that Hide can tell from the program's path, with the vendor's update command and the page it is from ([Updating an agent that is too old](#updating-an-agent-that-is-too-old)); the journey contract, for the declaration and for its rows in that section.
14. Logo and links: its entry in `web/src/assets/agents/manifest.json`, and an `https` `doc_url` and `install_url`; `generated_web_contract_and_logo_document_sources_are_current`.
15. Generated contracts: `contracts/agent-adapters.json` with the `export_web_contract` example and `contracts/agent-journey.json` with `export_journey_contract` ([contracts/README.md](../contracts/README.md#agent-adapters)); their drift checks in `hide-agent-adapter/tests/it/contract.rs`.
16. CLI pin: its row in `contracts/agent-clis.json`, at or above the minimum, with an installer `scripts/install-agent-clis.py` runs on Linux, macOS and Windows; the journey contract, `test_the_installed_cli_set_matches_the_registered_agents`, `scripts/tests/test_agent_clis.py`, and the `agent CLIs` lane on all three systems.
17. Canary: a latest-release source for its installer in `scripts/agent_journey/latest.py`; `test_every_pinned_cli_has_a_latest_source`.
18. Journey routes: local isolation (`scripts/agent_journey/local.py`, `ROUTES`), the disposable CI home (`ci.py`, `AGENT_ENV`) and the fake model (`fake_lane.py`, `PROVIDERS`); the journey contract, then the fake-model lane, which fails a missing row or a declared cell that did not pass on any system ([VERIFICATION.md: The CI lanes](VERIFICATION.md#the-ci-lanes)).
19. Operator watch: the agent's folder in `scripts/agent_journey/fake_private.py`, in `WATCH` or as its files in `CONFIGURATION`; the journey contract.
20. Hide AI: its declaration's `ai` (a provider id, or the absence with its reason), its default model in `BINDINGS` and its arm in `build_backend` (`hide-ai/src/registry.rs`, [AI_PROVIDERS.md](AI_PROVIDERS.md)); the compiler for `ai`, `every_declared_provider_is_bound_and_nothing_else_is` in hide-ai, and `every_supported_agent_is_a_hide_ai_provider_in_the_adapter_order` in `adapter_contract`. Whether first run checks its sign-in (`SIGN_IN_CHECKED`, `web/src/firstRun.ts`) waits on its login files being measured; no check.
21. Guides: its row in [Support table](#support-table) and its column in that section's feature table, its row in [The oldest version of each agent](#the-oldest-version-of-each-agent), and, in [INSTALL.md: What each system supports](INSTALL.md#what-each-system-supports), its name in the agents row and its own row under Letters to an agent while it works; the journey contract. The other sentences that name each agent (this guide's program names, Herdr integration names, hook reviews, folder trust and guardrails, and INSTALL.md's other rows) have no check; read them through for the agent.
22. Every system: a Windows or Linux gap only as a cell of INSTALL.md, What each system supports, with its reason and what would close it ([AGENTS.md](../AGENTS.md)); no check.

What some lines take on:

- `memory` means automatic injection into the model's context, independently of storing, extracting or searching Memory cards.
- `ContextRoute` distinguishes a native hook, a guidance hook whose output reaches the model, and a plugin's prompt event; the latter carries the start guidance and Memory index on the first prompt rather than a native `SessionStart`.
- The journey export refuses a startable guidance-only declaration until a probe for that combination is implemented: today's combined probe proves injection through a private Memory-card token, so it cannot mark guidance-only support `N/A` or attest Memory support.
- The Settings `Feature::Titles` describes readable session features; `titles` selects the native-title reader used before a background label is available, and is a separate decision.
- A bell target requires a prompt hook and measured menu behavior in an isolated app; adding prompt intake alone never enables its bell.
- The owner's dialect tests and feature-gate cases, aliases included, go in without changing the pre-migration hook-byte fixtures.

### What the kit puts down

Two pieces are written per agent into the agent's own files, and nothing else:

- The skill stubs Hide puts down (`hide_kit::agents::SKILLS`), each as `<name>/SKILL.md` in the folder the agent reads, plain copies written by every pass into every folder: `hide`, the guide to Hide, `hide-mission` v3, proposing a mission before complex work, following the operator’s answer, sending an explicit mission request or `/hide-mission` straight to `hide mission help`, and leaving small work with the current agent without asking, and `hide-issue` v1, which points at `hide issue help` (`hided/assets/issue/help.md`): search `with-hide/hide` first, collect the raw error, versions, steps and `Logs/core.jsonl` lines into the bug form's sections under 6,000 characters (the public repository closes longer reports), strip private paths and tokens, add no root cause or fix, show the operator the report and run `gh issue create --repo with-hide/hide` only after their yes, falling back to the form's URL when `gh` is missing; a test pins the template's headings to the labels of `.github/ISSUE_TEMPLATE/bug.yml`; `hide` v10 carries the same session-start table; a further stub is one more entry of that array, and the observe, write and remove passes read the array.
  Each is a few lines that point at the CLI's own guide, `hide guide` or `hide mission help`, so it stays right as that guide changes; the `hide` guide sends browser work to `hide browser help`, so there is no separate browser skill, and it names `hide mission help` too, so an agent that did not pick the Conductor's skill still finds it.
  The folders are the agent's global ones, read in every terminal, so each description says the skill is only for a session in a pane Hide manages, and the body first has the agent check that (Hide's session guidance, or `hide workspace info` answering) and stop when it is not.
  The description is a quoted YAML string: Pi 1.0.4 parses front matter strictly and drops a skill whose plain description holds `: `, which is how the Conductor's first stub never reached Pi's model (measured on 2026-10-22).
  The `hide-browser` stub an earlier build wrote is retired by ownership (`RETIRED_SKILLS`): a pass takes it out when its bytes are exactly what that build wrote, with its folder when nothing else is in it, and keeps one the operator edited; a folder holding a retired stub reads Outdated until that pass, so a folder an earlier build filled gets the current stubs even though the kit had recorded it as written. An edited one decides nothing: a recorded folder that holds only edited ones reads as the operator's removal of Hide's stubs, which Reinstall undoes, so a stub the operator deleted is not written back on every pass.
  The Conductor's stub was named `hide-conductor` (versions 1 to 3) until 2026-11-02, when it took the name of `hide mission`; those versions are retired the same way, so a `hide-conductor` Hide wrote gives way to `hide-mission` and one the operator edited or wrote themselves stays, and their `RETIRED_SKILLS` entry goes once the oldest build that may update in place already writes `hide-mission`.
  The session's `hide-plan` v1 was renamed `hide-task` on 2026-11-04; `hide-task` v1 was folded into `hide-mission` v2 on 2026-11-09.
  Both are retired by exact bytes too, including on uninstall, while edited files stay.
  Keep the `hide-plan` retirement until the oldest supported updater writes `hide-task`, and keep the `hide-task` retirement until that updater writes only the two current stubs.
  A folder that holds one of Hide's stubs and lacks another is stale and gets the missing one.
  The row's skill state: unreadable, then missing, then outdated, then edited by the operator, then another tool's file, then current.
  Grok and omp read `~/.claude/skills` as well as `~/.agents/skills`; with Hide's `hide` stub in both, each listed it once in its model's request (Grok 1.0.62, omp 18.7.0, 2026-10-22), as did Claude Code 2.1.307, Codex 0.163.0, Pi 1.0.4 and agy from their one folder, each in an isolated HOME.
  OpenCode 2.0.29 also listed it once, but it read the account's own skill folders whatever `HOME` and `OPENCODE_TEST_HOME` said, so whether it lists a stub found in both folders twice is unmeasured.
  Its folder is `~/.agents/skills` for the agents that read it, `~/.claude/skills` for Claude Code (which documents that it does not read the shared folder), and `~/.gemini/config/skills` for agy, whose model is given neither of the others: agy 1.4.0 lists skills in its documented `~/.gemini/antigravity-cli/skills` under `/skills` but does not give them to the model, while those in `~/.gemini/config/skills` reach it (measured on 2026-10-16), so Hide writes there and never in `~/.gemini/antigravity-cli/skills`, where the operator's own links stay; agy's desktop app shares `~/.gemini/config`, so it sees the skill too.
  A shared folder is written while any agent that reads it is on and installed, and it is removed only when none is.
  A file is Hide's only when its marker line, `<!-- hide-skill@<version>: ... -->`, is the first line after the front matter; a file that merely mentions `hide-skill@` is never replaced or removed, and the agent's row says a skill that Hide did not write is already there.
  A stub with Hide's marker over text that is not Hide's was edited by the operator: no pass rewrites it and a switch-off leaves it, the row reads Outdated with that reason, and only Reinstall puts Hide's text back.
  A stub of an older marker version is Hide's own and is replaced by the next pass.
  An agent's own folder (`~/.claude`, `~/.gemini/config`) is never created for the stub: the hook code reads that folder as the agent's settings being there, so a pass that made it would write a hook on the next pass that it did not write on this one, and a second apply would not be a no-op; the row says the agent has not created its folder yet, and the stub goes in on the pass after it has.
  The read that finds the folder made, for an agent that is on, queues that pass without a press (`Runtime::install_updated_agents`, logged as `kit.agent_folder_created`), as an updated agent's read does ([Updating an agent that is too old](#updating-an-agent-that-is-too-old)), and its hook goes in with the stub.
- The agent hook, for the agents below marked as done; for OpenCode this piece is Hide's plugin, and for Pi and omp Hide's extension, which carry the guidance with the rest ([OpenCode: Hide's plugin](#opencode-hides-plugin), [Pi and omp: Hide's extension](#pi-and-omp-hides-extension)).
  It is one entry per event the agent's documentation names, in the agent's own format, whose command is `hide-agent-hooks hook --runtime <agent id> --event <event>`.
  `hide-agent-hooks` writes it (`src/guidance.rs`) and nothing else does, under the per-agent marker (`hide-guidance` with the active agent's version) that proves an entry is Hide's and separates a current one from an older one.
  These entries capture the installing daemon's state route; an earlier owned entry is replaced by the next pass.
  Antigravity retains the encoded Windows command introduced in version 7 across its Go-to-cmd boundary.
  Where the session start adds context (Antigravity), its output is the worktree-purpose instruction, one fixed line that points at `hide browser help`, and the live Workspace guidance when the daemon answers, in the field the agent documents.
  It prints no Memory index, because a Memory receipt is read back only from a session Hide reads it from: Claude Code's, Codex's, Pi's and omp's files and OpenCode's database.
  A second delivery of the same session prints the same guidance: the output is a pure function of the daemon's answer, and a test runs the hook twice and compares.
  The other events carry the opener guard, and Grok's subagent count ([Grok's own hooks](#groks-own-hooks)).

Cursor is retired from support, onboarding, new installation and provider selection.
The kit removes only its recorded, Hide-marked hook entries, including older event layouts; operator and other tools' entries stay.
A recorded `herdr:cursor` piece is retired like any agent's ([Herdr's integration an earlier Hide installed](#herdrs-integration-an-earlier-hide-installed)) before ownership is forgotten.
An uninstall failure retains that record for the next pass, including full kit removal.
The shared `hide` stub stays while any live enabled agent reads it, and stored Cursor conversations remain readable.

Cursor also loads Claude Code's hooks from `~/.claude/settings.json` (and the project's `.claude/settings*.json`) when "Include Third-Party Plugins, Skills, and Other Configs" is on, which is its default, and merges them with its own at the lowest priority without removing a duplicate ([third-party hooks](https://cursor.com/docs/reference/third-party-hooks)).
So on a machine where Claude Code is on in Hide a Cursor session would also run Hide's instrumented Claude Code hook (the pane counters and reports as `claude-code`, Memory), which is not what a Cursor session is.
The compatibility exclusion remains after retirement: `hide-agent-hooks hook --runtime claude-code` prints nothing and counts nothing when `CURSOR_VERSION` is in its environment, the variable Cursor documents as set for every hook it runs.
Cursor's page on third-party hooks does not say whether those get the variable, so this exclusion rests on the documented one; a session where it is missing cannot be identified as foreign through that variable.
Grok runs `~/.claude/settings.json` and `~/.cursor/hooks.json` beside its own hook files, so inside Grok (`GROK_HOOK_EVENT`, which Grok sets for every hook it runs and for nothing else; `GROK_SESSION_ID` can also reach a Claude Code started from a Grok shell, which keeps counting) Hide's Claude Code hook and Hide's Cursor entries end without a word or a count, and Hide's Grok file is the one that counts and refuses (PRD grok-cursor-hooks D-05).
Grok's session start is passive, so the guidance Claude Code's hook printed there never reached the model and nothing is lost.
OpenCode can run the hooks in `~/.claude/settings.json` too, through an operator's bridge plugin; it has Hide's own plugin, so Claude Code's hook ends silently as a whole there, as it does in Cursor, and nothing is counted or delivered twice (PRD opencode-plugin D-10).
What Claude Code's hook never does inside another agent's session is take or confirm letters: a letter is addressed to the pane's own session and is confirmed once that session has seen it, so a hook that runs inside another agent's session would take the letter and confirm it to nobody who reads it (PRD settings-cleanup D-25).
`hide_agent_hooks::runtime::ForeignOrigin` finds such a session by what its agent sets for the processes it starts: `CURSOR_VERSION` for Cursor, `OPENCODE` or `OPENCODE_PID` for OpenCode, and `GROK_HOOK_EVENT` or `GROK_SESSION_ID` for Grok (for letters; only `GROK_HOOK_EVENT` silences the hook).
`hide-agent-hooks/tests/it/letter_origin.rs` runs the built helper beside a stand-in `hide` that answers `inbox` with one letter and records its calls, once outside and once inside each of them.
On Windows active command hooks run through PowerShell, so Hide writes that form for Grok ([Installing](#installing)); retired Cursor layouts are read only for cleanup.

### Grok's own hooks

Grok's native hooks carry the opener guard, subagent count and letter routes.
Its events follow the hook guide Grok ships; retired Cursor layouts are retained only for ownership cleanup.
Grok's `SessionStart` does not fire for a subagent, so it is where the pane's count starts over and the pane first reports itself instrumented.

Grok's file is Hide's own, `~/.grok/hooks/hide.json`, beside other tools' files in that folder (`{"hooks": {"<Event>": [{"matcher"?, "hooks": [{"type": "command", "command", "timeout"}]}]}}`, seconds); a switch-off deletes the file and nothing else there.
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

The refusal's reason and every rule of what is refused are the opener guard's above, through the same `guard_refusal`.
A call that runs, in a pane and with a session in its payload, then asks for the letters (`hide inbox --hook --session <id> --tool`) due while the agent works and answers them as the row above shows, confirming them once the answer is printed ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)); a refused call carries none, and the Pre entry has no matcher so every tool reaches that path.
Grok's `UserPromptSubmit` reads `prompt` and the native session, sends only the trimmed prompt's digest and session to the core, and prints and confirms nothing, because Grok discards its stdout.
Its `Stop` reads the payload once for both the subagent count and turn-end letters; a child stop, missing session or malformed payload takes none.
It asks for them as `hide inbox --hook --session <id> --turn-end` does, prints `{"decision":"block","reason":<letters>}` when something is due, and confirms after the flush.
A Stop with `stopHookActive: true` makes no letter pull and allows the turn to end even if confirmation failed; it still updates the subagent count.
Prompt, tool and Stop intake share the session parser: either `sessionId` or `session_id` is accepted, equal pairs are accepted, and conflicting pairs take no letters and record `session_conflict` without either value.
The core recognizes a bell's turn and keeps the normal five-minute wait for letters arriving during an operator's turn ([delivery.md](delivery.md#safe-intake-and-manual-fallback)).
`GuidanceAgent::version()` is per agent: Grok writes `hide-guidance@10` and Antigravity `hide-guidance@11`, both raised when their Windows check stopped using a cmdlet, retired Cursor layouts stay at 7 and the other retired layouts at 6 for removal only.
An older active entry reads Outdated until the kit replaces it with the captured state route.
The bell line names `hide inbox` for a session still running older hook entries.
Grok 1.0.62 keeps the loaded entries after the file changes; `/hooks`, then `r`, reloads them without restarting the session (measured with separate command tags before and after reload in an isolated session).

An old Cursor entry reaches no active adapter and exits successfully without output, daemon calls or pane reports.
Its recorded layout stays only so retirement can remove Hide-owned entries without changing operator entries.
Grok fails open on everything but a refusal, so its hook prints nothing on those paths, as Claude Code's does.
A failure goes to the guard's log, never to the agent (B5).

The count is the same per-pane record and the same report to the core as Claude Code's (above), changed under an exclusive lock on `panes.lock` in the hooks' records folder (private to the account), so subagents started in parallel are all counted, and a lock not taken within one second is written down as the pane's last report failure.
A subagent start adds one working, a subagent stop moves one from working to done.
Grok's `Stop` from inside a subagent leaves the count alone, and the main session's `Stop` sets working to the number of `subagent` entries in `backgroundTasks`, so a background subagent stays counted past the turn and its own `SubagentStop` ends it (D-06); a payload that is cut off or unreadable leaves the count alone.
The core judges a Grok pane instrumented by that agent's hook piece in the kit (`hide_core::agent_hooks::kit_hook_status` for the agent's `HookDialect`, as for OpenCode's plugin), not by a Claude Code or Codex component.


`hide-agent-hooks/tests/it/grok_cursor_hooks.rs` runs the built helper with Grok's documented payloads and historical Cursor payloads beside a stand-in `hide` that keeps the status reports.
`web/e2e/grok-cursor-hooks.spec.ts` runs Grok's hooks from inside a Grok stand-in's own process in its pane (`agentRunner` in `web/e2e/agent-pane.ts`), because hided takes a status report only from a process it proves runs in the pane.
It asserts Grok's answers and counts, retired Cursor's silence and lack of daemon calls or counts, and a single count when Grok runs all three of Hide's hooks for one subagent.

The record `~/.hide/kit/installed.json` keeps the operator's choice per agent (`agents`) and the pieces Hide installed (`hook:<agent>`, `skill:<folder>`, and `herdr:<agent>` from a build that put in Herdr's integration, until [its retirement](#herdrs-integration-an-earlier-hide-installed) takes it out), and an older build ignores them.
With no choice on record Claude Code and Codex are on, as they have been since their hooks became part of the kit, and every other agent is off.
A piece that was installed and is gone stays gone until Reinstall, and an agent switched off keeps nothing of Hide's and gets nothing back from a later pass.
Whether the agent is installed only decides whether a switch can work: an agent that is not installed on a machine has no switch there, and switching it on is not recorded.
A guidance hook also needs the agent's own settings folder, which Hide does not create: an installed agent that has not made it yet reads that, and its hook goes in on the pass after it has.

### Antigravity: Hide's hook block

agy reads every named block of `~/.gemini/config/hooks.json` beside the others, so Hide writes one block of its own, `hide`, and touches no other (other tools' blocks and the operator's stay byte for byte; PRD antigravity-first-class D-04).
The block holds four events in agy's own format (`{"hide": {"<Event>": [{"type": "command", "command", "timeout"}]}}`, seconds, and a group with a `matcher` for `PreToolUse`), with the behavior read from agy 1.4.0 and measured live on 2026-10-16:

| Event | What Hide answers |
| --- | --- |
| `SessionStart` (a conversation's first prompt, never a resume) | the guidance and the Memory index, as an injected system message (`injectSteps`) |
| `PreInvocation` (every model call; `invocationNum` 0 is a prompt's first) | on the prompt's first call the prompt's Memory cards and the pane's letters, the same way; on a later call of the turn, the gap after a tool, the letters due while the agent works ([docs/delivery.md: Letters while the agent works](delivery.md#letters-while-the-agent-works)), the same way, confirmed once printed |
| `PreToolUse` (matcher `run_command\|view_file\|write_to_file\|replace_file_content\|multi_replace_file_content`) | `{"decision":"deny","reason"}` for a refused call, `{"decision":"ask"}` on every other path |
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
`hide-agent-hooks/src/antigravity.rs` and `src/guidance.rs` test the answers and the block's install and removal in a fixture home, and the `agy contract` CI lane (`.github/workflows/agy-contract.yml`) also runs the agy `contracts/agent-clis.json` pins against the block in the runner's throwaway home.

agy 1.4.0 loads a workspace's own `.agents/hooks.json` only for a folder in `trustedWorkspaces` of `~/.gemini/antigravity-cli/settings.json` (1.3.6 also loaded it in print mode without trust), trust is per exact path and is not inherited from a parent folder, and the first interactive start in a folder it has not been told to trust asks, which would leave a delegated child stopped at that question.
Hide therefore adds the exact path of a worktree it made to that list before it starts agy there, as it records Claude Code's and Codex's trust ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes); D-08).
Hide AI's own folder for agy is trusted the same way ([AI_PROVIDERS.md](AI_PROVIDERS.md#antigravity)), and switching agy off removes it too.

agy's tool-permission question reaches Needs You as an approval: Herdr 0.9.3 reported the pane as waiting on an approval while it is open (seen in the app on 2026-10-16; an earlier probe outside the app had read it as idle).
Its folder-trust question has not been measured, and agy has no bell (D-06): an Enter from the bell into a dialog the screen rules read as idle would answer that dialog, and a letter that arrives meanwhile reaches agy on its next prompt through `PreInvocation`, or, once its wait is over, on the turn's next model call through the same hook.
The bell is turned on for agy once the engine's screen rules are shown to read the folder-trust question as blocked too, and this paragraph goes ([INSTALL.md](INSTALL.md#what-each-system-supports)).
Claude Code's hook needs no silencing inside agy, because agy reads neither `CLAUDE.md` nor `~/.claude` hooks (D-22).

### Installed means the program is found

An agent is installed on a machine when one of its programs is found there, the way the operator's terminal would find it; a folder the agent creates does not count, because a CLI that was removed leaves its folder behind (`~/.pi/agent` without `pi`).
The program names are the ones each vendor's install documentation and install script give the command: `claude`, `codex`, `grok`, `opencode`, `pi`, `omp` and `agy`.
The program names live in the shared adapter declaration alone; Hide AI keeps no list of them, and an agent it cannot use yet is listed as not installed from what the kit found (`AiRequest.cli_found`).

The search is, in order, the folders the account's login shell puts on its `PATH`, the daemon's own `PATH`, and the usual install folders: `~/.local/bin`, pnpm's global folder (`~/Library/pnpm` on macOS and `~/.local/share/pnpm` on Linux, and the `bin` folder inside it from pnpm 11), `~/.npm-global/bin`, `/opt/homebrew/bin` and `/usr/local/bin` (`hide_platform::programs`, the one search; Hide AI's backends find their CLIs with it too, so the kit's "installed" and Hide AI's "can be asked" cannot disagree about a program only the shell's `PATH` or an install folder reaches).
The login shell is asked because many installers put their own folder on the `PATH` by editing a startup file rather than using one of those folders: Grok's `~/.grok/bin`, OpenCode's `~/.opencode/bin` and Pi's `~/.pi/agent/bin` are written into `~/.zshrc`, `~/.bashrc` or `config.fish`, and a Node CLI installed under nvm lives in nvm's folder.
No daemon sees those folders on its own: an app opened from the Dock gets the system folders and the usual install folders from the desktop host, and a device helper started over an SSH exec channel gets the system folders alone (a non-interactive zsh reads only `~/.zshenv`).
`hide_platform::host::login_shell` runs `$SHELL -ilc` with the account's login variables and the home being searched, and reads the `PATH` it prints between two marks, past anything the startup files print, and the environment `env -0` lists after it, which Desk's agent starts with ([ARCHITECTURE.md](ARCHITECTURE.md#desk)); it gives the shell ten seconds, the same as the desktop host's own ask, and ends it and everything it started after that or when Hide quits.
The kit asks, on the kit worker on this Mac and in the helper on a device, and Hide AI's backends, the hook diagnosis a node answers and a node's folder trust ask through the same cached answer on their own worker threads (the Settings reader, the label analyzer and Memory's analysis, never under the runtime lock).
Nothing that checks or runs an agent's CLI searches without the shell: whether an agent is installed, whether its version takes Hide's hook, and the Codex asked about hook trust, folder trust and its shared daemon, are all found on this one search.
A search that skipped the shell once found an older Codex left in `~/.local/bin` before the nvm Codex the operator's terminal ran, and refused the hook as needing an update the operator had already made; a refusal now names the program it judged.
`no_product_code_outside_the_platform_looks_for_a_program_on_its_own` (`hide-platform/tests/it/programs.rs`) fails when product code outside `hide-platform` looks for a program by the primitive, builds a search without the shell, or names a program by file instead of finding it.
The answer is kept until one of the shell's startup files changes (zsh's, bash's and sh's in the home and in `~/.config/zsh`, fish's, and the system's in `/etc`), which is how an installer adds a folder, so Settings re-reading the kit every few seconds starts no shell; a file those files read in turn is not watched, and is read again when `hided` or a device's helper next starts.
A shell that does not answer is logged as `platform.login_shell_unread` with its error kind, asked again after a minute, and the search goes on without its folders meanwhile; Windows has no login shell, and its search is the account's own `Path` plus the usual folders, which there include agy's own `%LOCALAPPDATA%\agy\bin`.
A program found by the search is also run with it, because a CLI installed as a script starts its interpreter by name (pnpm's and nvm's `codex` run `node`).
What finds an agent's CLI hands on a `hide_platform::programs::Program`, the program and the `PATH` it was found on, so the version probe, Codex's trust and daemon commands and the kit run the program they checked with that `PATH`; every child Hide AI starts does the same through its own resolver: a model-list or sign-in probe as much as a request, and the Codex app-server (`hide-ai/src/program.rs`, which `runner::run` and the Codex session apply).
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

| Agent | Skill folder | Hook | Earlier Herdr integration | Why, and the page that says so |
| --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/skills` | done: native hooks, a kit part | `claude` | [skills](https://code.claude.com/docs/en/skills) |
| Codex | `~/.agents/skills` | done: native hooks, a kit part | `codex` | [skills](https://learn.chatgpt.com/docs/build-skills) |
| Grok | `~/.agents/skills` | done: opener guard and subagent count in `~/.grok/hooks/hide.json`; no guidance, because `SessionStart` cannot add context, tool-call context and a Stop block reason can | `grok` | [skills](https://docs.x.ai/build/features/skills-plugins-marketplaces) |
| OpenCode | `~/.agents/skills` | done: Hide's plugin in `~/.config/opencode/plugins/`, not a command hook; OpenCode has none, only JS plugins (<https://opencode.ai/docs/plugins/>), whose `chat.message` hook adds a synthetic part to the prompt and whose `tool.execute.before` refuses a call by throwing ([OpenCode: Hide's plugin](#opencode-hides-plugin)) | `opencode` | [skills](https://opencode.ai/docs/skills/) |
| Pi | `~/.agents/skills` | done: Hide's extension in `~/.pi/agent/extensions/`, not a command hook; Pi has none, only TS extensions, whose `before_agent_start` adds a hidden message to the prompt and whose `tool_call` refuses a call ([Pi and omp: Hide's extension](#pi-and-omp-hides-extension)) | `pi` | [skills](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/skills.md) |
| omp | `~/.agents/skills` | done: the same extension in `~/.omp/agent/extensions/`, with omp's subagent counts | `omp`, in `~/.omp/agent` | [skills](https://omp.sh/docs/skills) |
| Antigravity | `~/.gemini/config/skills` | done: the `hide` block in `~/.gemini/config/hooks.json`, with guidance, letters, Memory, the opener guard and the subagent count ([Antigravity: Hide's hook block](#antigravity-hides-hook-block)) | `antigravity-cli`, in `~/.gemini/config` | [skills](https://antigravity.google/docs/skills/) |

Each row also carries what Hide can do for that agent as a list of features (`hide_agent_adapter::Feature`, re-exported by `hide_kit::agents`, and `KitAgentSnapshot.features`).
An agent without both prompt intake and spawn refusal wears the `일부 기능` chip (`Some features`), regardless of other missing features.
Its popover groups the feature table into basics, session reading and multi-agent collaboration, retaining a mark and a supported/unavailable word for every feature:

| Feature | Supported when | Claude Code, Codex | OpenCode | Pi | omp | Grok | Antigravity |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `skill` | always | yes | yes | yes | yes | yes | yes |
| `guidance` | Hide's hook, plugin or extension prints the session guidance | yes | yes | yes | yes | no | yes |
| `letters` | its prompt dialect declaration (a new prompt's letters; while the agent works, `tool_letters` names the dialect whose tool event hands them over: OpenCode, Pi, omp, Grok and Antigravity; a kind with this path is belled only at rest) | yes | yes | yes | yes | no | yes |
| `memory` | its Memory dialect declaration | yes | yes | yes | yes | no | yes |
| `bell` | the core rings the doorbell for that agent (`AgentAdapter::bell`, tied to `delivery::doorbell::bell_target`) | yes | no | no | no | yes (at rest) | no |
| `start` | its launch declaration | yes | yes | yes | yes | yes | yes |
| `sleep` | its Sleep declaration | yes | yes | yes | yes | yes | yes |
| `fork` | its Fork declaration | yes | yes | yes | yes | yes | yes |
| `titles` | its conversation declaration enables generated labels | yes | yes | yes | yes | yes | yes |

Claude Code, Codex, Grok, Pi, omp and Antigravity are the agents whose session files Hide reads for these features, so only they get a per-agent session count, and the count is per machine and only of the sessions running now: one number of open panes holding an awake agent, never an accumulation of warnings.
OpenCode's sessions live in its database rather than in session files, so it has no session count; its database reader gives it titles, sleep, fork and its conversation ([ARCHITECTURE.md](ARCHITECTURE.md)).
`hide-core/src/runtime/tests/agent_features.rs` ties each flag to the gate in the core that decides it (`runtime_of`, `sleeps_kind`, `ForkableAgent`, `AGENT_KINDS`, `conversation_agent_kind`), so a flag cannot say yes where the core says no, and holds each row's kind name to the kind the engine reports its panes as.

An agent is a row only when its documentation confirms where it reads skills and the name of the program it installs.
The other fifteen agents earlier builds supported are no longer rows (see Retired agents below).
Hide does not write an agent's `AGENTS.md` or `CLAUDE.md`, its MCP configuration or its model settings, and it runs no installer for an agent.
The things it records in an agent's own state are Codex's trust for Hide's own hooks, below, and Claude Code's, Codex's, agy's, Grok's and Pi's folder trust for the worktrees Hide makes ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)).

### The oldest version of each agent

Each supported agent's declaration (`hide-agent-adapter/src/declarations.rs`, `AgentAdapter::minimum`) names the oldest version of its CLI that Hide's hook, plugin or extension was proven on, and where the proof is; nothing else keeps a copy.
It is one version per agent: the oldest proven on every system Hide runs on, macOS, Linux and Windows, not the oldest proven on any one of them, and there is no minimum per system; proven means run on that version, and a reading of the agent's source sets no minimum (operator decision, 2026-10-27).
The kit puts that piece in place only for a CLI that answers this version or a newer one to `--version`.
An older CLI is refused on every system with a reason that names the agent, the version it answered, the program's path and the minimum, such as `Pi 1.0.3 at ~/.local/bin/pi is older than 1.0.4, which Hide's extension needs; update it`, and nothing of Hide's is written for it; the screens say what to update and how ([Updating an agent that is too old](#updating-an-agent-that-is-too-old)).
A piece already current stays as it is, as Claude Code's and Codex's hook parts always have, and a CLI whose version Hide cannot read within the probe's 750 ms is not judged too old.
The program asked is the one the operator's login shell runs ([Installed means the program is found](#installed-means-the-program-is-found)), run with the `PATH` it was found on, and it is asked once per version of its file: `hide_agent_hooks::program_version` remembers the answer by the file's path, search, write time and length, at most 32 programs, and asks again after a minute only when no version was read.
`contracts/agent-journey.json` carries each minimum as `minimum_version` for scripts.

| Agent | Minimum | The proof |
| --- | --- | --- |
| Claude Code | 2.1.278 | The hook output shape Memory injection needs (2026-09-21), pinned by `hide-agent-hooks/tests/fixtures/claude-user-prompt-submit.json`; the agent journey passes on 2.1.302 |
| Codex | 0.155.1 | The oldest Codex Hide installs into: its hook runner and skill folder read from its source at `rust-v0.155.1` ([Support table](#support-table), [Installing](#installing)), and the Memory hook shape of `hide-agent-hooks/tests/fixtures/codex-user-prompt-submit.json` |
| Grok | 1.0.52 | The agent journey ran on 1.0.52 on macOS, Linux and Windows (run 39152230964) with Hide's kit installed, its letter cell passing through Grok's hook; how Grok runs a hook on Windows (`xai-grok-hooks/src/runner/command.rs`, `xai-grok-config/src/shell.rs`, pinned by `hide-agent-hooks/tests/it/windows_hook_command.rs`) and its skill folder were read from its source at 1.0.50, which is a reading and not a run, so it sets no minimum |
| OpenCode | 1.18.35 | Windows: on a `windows-latest` runner on 2026-10-11, OpenCode 1.18.35 loaded Hide's plugin from `%USERPROFILE%\.config\opencode\plugins` and its `spawn` of the helper crossed stdin and stdout ([OpenCode 2](#opencode-2)); macOS and Linux: the plugin's event and hook shapes captured from 1.18.30 (`hide-agent-hooks/tests/fixtures/opencode/events-1.18.30.json`), and the journey passes on 2.0.29 on all three |
| Pi | 1.0.4 | The extension's event and context shapes captured from 1.0.4 (`hide-agent-hooks/tests/fixtures/pi-extension/pi-1.0.4.json`), loaded on Windows on 1.0.4, and the journey on 1.0.4 |
| omp | 18.7.0 | The same for omp 18.7.0 (`hide-agent-hooks/tests/fixtures/pi-extension/omp-18.7.0.json`) |
| Antigravity | 1.4.2 | Windows: agy 1.4.2 starts a hook through `cmd /c` with Go's argument escaping, which broke the earlier guard, and Hide's encoded PowerShell guard was proven on it ([Antigravity: Hide's hook block](#antigravity-hides-hook-block)); the journey passes on 1.4.2 on macOS, Linux and Windows (run 39152230964); the block and skill folder were first measured live on 1.4.0 on macOS |

A minimum moves only with new proof: a feature that needs a newer CLI raises it in the same change, with the fixture or measurement that shows the older one fails, and a lower one needs the integration proven on that version.

### Updating an agent that is too old

An agent whose copy is refused as too old is shown with the version it answered, the one it needs, and the one command that updates that copy, never a Reinstall, which would be refused again for the same reason.
`AgentReport::update` (`hide_kit::AgentUpdate`, `KitAgentSnapshot.update`) carries it: present for an agent that is off whose found copy is older than its minimum, since switching it on would put nothing in (the first-run choice shows it), and for one that is on only while one of its pieces is refused as `update_agent`, so an old copy under a piece still current from a newer one says nothing.
The command is chosen in `hide-kit/src/agent_update.rs` from the path of the program the kit judged, the copy the operator's shell runs, and each agent's ways of installing are declared once, with their command and the vendor page it is from, in `AgentAdapter::updates` (`hide-agent-adapter/src/declarations.rs`).
A route is recognised by where the program really is, its links followed, in this order: a Homebrew keg (`Cellar/<formula>` or `Caskroom/<cask>`), a WinGet package (`WinGet/Packages/<id>_…`), the vendor installer's own place under the home folder, bun's global folder (`.bun/install/global/node_modules/<package>`), then npm's: the program inside `node_modules/<package>`, or beside a prefix that holds the package (`<prefix>/bin` with `<prefix>/lib/node_modules`, as nvm, fnm, Homebrew's Node and any npm prefix lay it out; `%AppData%\npm` with its `node_modules`; Volta's `bin` with `tools/image/packages`).
A route documented for one system is not offered on another.
A copy no route recognises, such as a system package, pnpm's global folder or a build of one's own, gets the agent's install page instead of a command, and the core shows a helper's command only when this build's declaration names it for the agent.
A command the vendor does not document is not written: where the vendor names a package but documents no update for it, the row reruns the documented install, which installs the newest release, or uses that package manager's own upgrade of it (operator decision, 2026-10-27).

| Agent | Installed by | Where the program is | Systems | Command | Source |
| --- | --- | --- | --- | --- | --- |
| Claude Code | its installer | `~/.local/share/claude/versions/`, or `~/.local/bin/claude.exe` | all | `claude update` | [setup](https://code.claude.com/docs/en/setup), [troubleshooting](https://code.claude.com/docs/en/troubleshoot-install) |
| Claude Code | Homebrew cask `claude-code` or `claude-code@latest` | `Caskroom/claude-code*/` | macOS | `brew upgrade claude-code`, `brew upgrade claude-code@latest` | [setup](https://code.claude.com/docs/en/setup) |
| Claude Code | WinGet `Anthropic.ClaudeCode` | `WinGet/Packages/Anthropic.ClaudeCode_…` | Windows | `winget upgrade Anthropic.ClaudeCode` | [setup](https://code.claude.com/docs/en/setup) |
| Claude Code | npm `@anthropic-ai/claude-code` | npm's global folder | all | `npm install -g @anthropic-ai/claude-code@latest` | [setup](https://code.claude.com/docs/en/setup) |
| Codex | its installer | `~/.codex/packages/standalone/` | macOS, Linux | `curl -fsSL https://chatgpt.com/codex/install.sh \| sh` | [Update Codex](https://learn.chatgpt.com/docs/codex/cli) |
| Codex | npm `@openai/codex` | npm's global folder | all | `npm install -g @openai/codex` | [README](https://github.com/openai/codex#installing-and-running-codex-cli) |
| Codex | Homebrew cask `codex` | `Caskroom/codex/` | macOS | `brew upgrade --cask codex` | [README](https://github.com/openai/codex#installing-and-running-codex-cli), [Homebrew](https://docs.brew.sh/Manpage#upgrade-options-installed_formulainstalled_cask-) |
| Grok | its installer | `~/.grok/downloads/`, or `~/.grok/bin/grok.exe` | all | `grok update` | [CLI reference](https://docs.x.ai/build/cli/reference) |
| Grok | npm `@xai-official/grok` | npm's global folder | all | `npm install -g @xai-official/grok` | [enterprise](https://docs.x.ai/build/enterprise) |
| OpenCode | its installer | `~/.opencode/bin/opencode` | macOS, Linux | `opencode upgrade` | [CLI](https://opencode.ai/docs/cli/#upgrade) |
| OpenCode | npm or bun `opencode-ai`, Homebrew `opencode` | npm's or bun's global folder, `Cellar/opencode/` | all; Homebrew on macOS, Linux | `opencode upgrade` | [CLI](https://opencode.ai/docs/cli/#upgrade) |
| Pi | its installer | `~/.pi/agent/bin/` | macOS, Linux | `pi update` | [quickstart](https://pi.dev/docs/quickstart) |
| Pi | npm or bun `@earendil-works/pi-coding-agent` (Windows' installer uses npm) | npm's or bun's global folder | all | `npm install -g --ignore-scripts @earendil-works/pi-coding-agent`, `bun add -g --ignore-scripts @earendil-works/pi-coding-agent` | [README](https://github.com/earendil-works/pi#installation), [pi.dev](https://pi.dev/) |
| omp | its installer | `~/.local/bin/omp`, or `~/AppData/Local/omp/omp.exe` | all | `omp update` | [CLI](https://omp.sh/docs/cli) |
| omp | bun `@oh-my-pi/pi-coding-agent` | bun's global folder | all | `bun install -g @oh-my-pi/pi-coding-agent` | [README](https://github.com/can1357/oh-my-pi#installation) |
| omp | Homebrew `can1357/tap/omp` | `Cellar/omp/` | macOS, Linux | `brew upgrade can1357/tap/omp` | [README](https://github.com/can1357/oh-my-pi#installation), [Homebrew](https://docs.brew.sh/Manpage#upgrade-options-installed_formulainstalled_cask-) |
| Antigravity | its installer | `~/.local/bin/agy` | macOS, Linux | `curl -fsSL https://antigravity.google/cli/install.sh \| bash` | [getting started](https://antigravity.google/docs/getting-started?tab=cli) |
| Antigravity | its installer | `~/AppData/Local/agy/bin/agy.exe` | Windows | `irm https://antigravity.google/cli/install.ps1 \| iex` | [getting started](https://antigravity.google/docs/getting-started?tab=cli) |

Claude Code's apt, dnf and apk packages, Codex's Windows installer, which documents no update, Codex under pnpm, and OpenCode's Chocolatey and Scoop packages get the install page; which of these rows were proven on which system is in [INSTALL.md: What each system supports](INSTALL.md#what-each-system-supports).
`hide-kit/src/agent_update.rs`'s tests lay out each place in a private folder; `every_supported_agent_declares_how_each_documented_install_is_updated` (`adapter_contract`) and the journey contract refuse an agent with no route.

After the operator updates, nothing is pressed.
The version is asked once per version of the program's file ([The oldest version of each agent](#the-oldest-version-of-each-agent)), so the next kit read asks the new file, and a read that finds an agent which was too old in the machine's last report and is new enough now, with a piece still to put in, queues the automatic pass that puts it in (`Runtime::install_updated_agents`, logged as `kit.agent_updated`).
A read happens at launch and on each device connection, every 5 seconds while a Settings tab showing the kit is open, and once when a pane newly wears the update badge: an agent started in a pane, perhaps after its update, has the kit of its machine read (`Runtime::request_machine_kit_read`).
A session that was already running keeps the old process, which Hide's piece never reached, so it reads `started_before_hide` once instead of the badge ([status-model.md](status-model.md#not-connected-and-what-fixes-it)); Hide judges the copy the operator's shell runs, and a pane that runs another copy by its full path is judged by that one only once the shell finds it.

### Herdr's integration an earlier Hide installed

Earlier builds put Herdr's own integration into each agent's configuration as a third piece of the agent's row, `herdr:<agent>` in the record.
This build runs no Herdr and installs nothing of Herdr's.
It takes nothing of Herdr's out of an agent's files either: the record still reads its `herdr_hooks` field and a `herdr:<agent>` piece, so an earlier Hide's record loads and the piece is forgotten without any write to the agent's configuration (`hide_kit::record`, `forget_piece`).
Codex's record that it trusted an entry stays in `~/.codex/config.toml`, where it trusts nothing that is not there.

### Which agents ask for a review

Only Codex asks the operator to approve a hook before it runs.
A survey on 2026-10-07 read each agent's official documentation and the executables installed on the maintainer's machine: Claude Code, Grok and OpenCode run a hook or plugin from the user folder with no approval screen, and the check a project folder's hook gets is a folder trust, not a hook review.
Pi 1.0.4 and omp 18.7.0 loaded Hide's extension from a private user folder with no approval screen on 2026-10-09.
agy 1.3.6 and 1.4.0 ran the user folder's `~/.gemini/config/hooks.json` with no approval screen on 2026-10-16, and neither its folder-trust dialog nor `/hooks` has a step that approves one hook; its folder trust, which Hide records for its own worktrees, is the check a workspace's hooks get.
Hide therefore writes no hook trust for any other agent (the folder trust it records for its own worktrees is a separate step, [Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)), and an agent that starts asking for approval of a user-folder hook is a new row for this section and a new trust step beside Codex's, not a change to Codex's.

### Retired agents

Earlier builds supported fifteen more agents: Cursor, GitHub Copilot CLI, Amp, Factory Droid, Kiro, Qwen Code, Goose, Cline, Kilo Code, Crush, Junie, Augment, Kimi Code, Mistral Vibe and Gemini CLI.
Gemini CLI left later than the others, when the support list became the agents Herdr shipped an integration for; its retirement takes Hide's `hide-guidance` group out of `~/.gemini/settings.json` and leaves the operator's hooks, settings, sign-in and sessions there as they are.
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
It runs once, after the pass has put in every agent's pieces, so one `codex app-server` session trusts every entry Hide wrote and a first install is trusted before the operator's first Codex start.
A pass that finds every entry trusted writes nothing, so a repeat leaves `config.toml` as it was.
An entry another tool's hook displaced is trusted at its new position by the next pass; a Codex started before that pass can still show the screen for Hide's entry.
A Codex without hook trust (its app-server does not know `hooks/list`, which codex-cli 0.160.0 shows as a `-32600` "unknown variant" error rather than JSON-RPC's `-32601`; both are read as unknown), a Codex that ends before it answers the handshake (no `app-server` command, or a program that is not a Codex) and a machine with no Codex are left alone and show nothing new; a Codex that answers the handshake and then ends, hangs or refuses is a failure.
Starting the app-server also makes Codex do its own bookkeeping in `~/.codex` (its databases, `installation_id`, `skills/`), which is Codex's and not Hide's.

What is trusted is exactly the entry Hide wrote, and nothing else (`select_targets`):

- Codex lists it from this account's `~/.codex/hooks.json` as a user hook that is not managed, so a project's hook, a plugin's and a managed one with the same command are not it;
- its event, handler type, `command` and matcher are, byte for byte, those of one of Hide's runtime-specific entries (the command Hide writes for that event with this kit's helper, a command hook, and `install::hook_matcher` for that runtime/event; the writer and this check read it from one function, so a matcher that changed is a `modified` entry trusted again, never one trusted blind);
- Codex does not trust it yet (`untrusted`, or `modified` after a change).

Another tool's hook, an entry that carries Hide's marker over a different command, and any entry that is not like this are neither read nor changed: Codex still shows the review screen for them, and lists only them; approving that screen once covers them only, and the review screen lists no entry for Hide's opener guard.
Hide writes `trusted_hash` and never `enabled`, so a hook the operator switched off in Codex's hook list stays off, and switching Hide's hook off there is how the operator opts out of one; switching Codex off in Settings, Agents takes Hide's entries out and Hide asks Codex for nothing.
Removing Hide's hooks leaves its trust records in `config.toml`; each one matches only the same command and starts nothing by itself.
Codex's `--dangerously-bypass-hook-trust` and `bypass_hook_trust` skip the review for every hook and are not used.

The app-server is one child per check, started through the one spawn helper, bounded by a 15 second overall and 5 second per-request deadline, by caps on what it may print, and by the kit's stop flag, and ended with its whole process tree on success, failure and timeout alike (a process that left that tree and holds its output open is not waited for: the reader thread ends when it lets go); it runs on the kit worker or the device helper, never under the runtime lock.
When Codex is there and the trust cannot be recorded, the Codex hook part reads Failed with one line, "Codex has not trusted Hide's hook yet. Review it in Codex." (the `codex_untrusted` cause; the kit's own sentence with the cause class stays in its report for the `hide` CLI), the cause class goes to the diagnostic log with the pass's record (`kit apply.completed` names each part's reason), Codex's own words go to the kit's standard error as `codex_trust_failed` (seen where the kit runs in a terminal, and in a device helper's log, but not from the packaged app's detached daemon), the other parts are installed as usual, and the next pass tries again.
`status` runs every few seconds while Settings is open and starts no process, so it repeats what the last pass in this process found, with the `codex_untrusted` cause that pass gave; a failure the operator fixed by approving the hook in Codex clears at the next pass or Reinstall, which Failed offers.
Check again on a machine whose last report holds that cause runs a pass instead of a read, so it clears there too (`Runtime::request_kit_check`), and a spawn of Codex onto that machine runs one before it starts anything ([delivery.md](delivery.md)).
`hide-agent-hooks/tests/it/codex_trust.rs` runs the module against a stand-in app-server (`tests/fixtures/fake-codex.py`), and `hide-kit`'s `codex_trust_cases` runs the passes.
Codex's app-server interface is marked experimental; a method or field that changes ends as the Failed line above, and Codex shows its screen, never a wrong trust.

## Judging what is installed

`hide_agent_hooks::diagnosis` resolves one reason, in a fixed order, and the first match wins:

1. `config_unreadable` - the file could not be read or parsed, so nothing was installed into it.
2. `hooks_not_installed` - the runtime is here and carries no hook of Hide's.
3. `session_predates_install` - the hook is installed and this pane carries none of Hide's tokens, so the session was already running when it was installed. Restarting the agent instruments it (the pane's Reopen does that in place); on a Codex whose machine has the shared server on, the same finding is read as the shared server instead (`PaneConnectionReason`). A pane whose reports the daemon refuses lands here too; `last_report_failure` is what tells the two apart.
4. `hook_outdated` - the session is reporting through an older hook than this Hide writes.
5. `unknown` - genuinely unknown, and said to be.

Both the pane's own mark and the Settings diagnosis read that one function, and every projection carries the reason's stable code alongside its sentence so no surface has to recognise its own operator-facing text.
A pane on a device is judged by the same function, against that device's hooks as its kit last reported them (`agent_hooks::device_hook_status`), with the pane's tokens carried from the device's engine.
A device where Hide may not install, or whose platform this build does not carry, reads `hooks_not_installed`; a device whose kit Hide has not read yet reads `unknown`.

## Installing

Hide's hooks are one part of its install kit (`hide-kit`; [ARCHITECTURE.md, The install kit](ARCHITECTURE.md#the-install-kit)), which installs the same set on this machine at every launch of the packaged app and on every device at every helper connection, without asking: the operator agreed to it once, by installing the app or by adding the device; after a core move it does so on the core's machine at every start of its core and on the machine the core moved off at every link.
Every entry's command is guarded, so a session whose app or helper was moved or deleted carries on without a hook error (PRD device-parity B3), and it is written in the form its runtime runs a hook on that system:

| System | Runtime | Entry |
| --- | --- | --- |
| macOS, Linux | both | `"command": "if [ -x '<kit folder>/hide-agent-hooks' ]; then exec '<kit folder>/hide-agent-hooks' hook ...; fi"` |
| Windows | Claude Code | `"command": "powershell.exe", "args": ["-NoProfile", "-NonInteractive", "-Command", "<guard>"]` |
| Windows | Codex, Grok | `"command": "<guard>"` |

On Windows `<guard>` is `if ([System.IO.File]::Exists('<kit folder>\hide-agent-hooks.exe')) { & '<kit folder>\hide-agent-hooks.exe' hook ... }`, with a quote inside the path doubled, as PowerShell reads it.
The test for the helper is a .NET call, not the `Test-Path` cmdlet entries before command version 12 used: Windows PowerShell loads a cmdlet's module on its first use and, on an account with no module cache, first builds that cache for every installed module ("Preparing modules for first use"), which on a fresh CI runner took four hooks started together 38 to 48 s, past the 8 s a hook is given (with-hide/hide-private#318).
No Windows guard Hide installs, Grok's and agy's included, calls a cmdlet: `install::tests::a_windows_guard_calls_no_cmdlet` and `guidance::tests::the_windows_guidance_commands_call_no_cmdlet` read each one as PowerShell does and find no command but the helper, and the Windows lane runs each with an empty module cache inside its hook timeout (`windows_hook_command::a_windows_guard_with_an_empty_module_cache_answers_inside_its_hook_timeout`).
The forms follow what each runtime documents or, where its documentation is silent, what its source does:

- Claude Code passes a command string to `sh -c` on macOS and Linux, to Git Bash on Windows, or to PowerShell when Git Bash is not installed, and spawns `command` with `args` directly, with no shell, on every system (hooks reference, "Exec form and shell form", <https://code.claude.com/docs/en/hooks>; `args` since 2.1.139, below the 2.1.278 Memory needs). A string would meet two different shells on two Windows machines, so the Windows entry names PowerShell in exec form and the guard always runs in PowerShell.
- Codex documents `commandWindows` as a Windows override without saying what runs it (<https://developers.openai.com/codex/hooks>). Its source at `rust-v0.160.0`, and the same code at `rust-v0.155.1`, the oldest Codex Hide installs into, runs a command through the session's shell (`core/src/session/mod.rs`, `build_hooks_config`), which on Windows is PowerShell 7 or else Windows PowerShell, started `-NoProfile -Command <command>` (`shell-command/src/shell_detect.rs`, `default_user_shell`; `core/src/shell.rs`, `derive_exec_args`), and through `%COMSPEC% /C` only when the session reports no shell (`hooks/src/engine/command_runner.rs`, `build_command`). The file is that machine's, so the guard is written as `command`.
- A Codex session that falls back to `cmd` cannot read the PowerShell guard: its hooks fail with cmd's syntax error, the turn goes on without Hide's context, and nothing else on the machine changes.
- Grok runs a command that holds a space as `pwsh -NoProfile -NonInteractive -Command <command>`, with Windows PowerShell when PowerShell 7 is not found and Git Bash only when neither is, or when `GROK_SHELL` names it, with the event on stdin (its shipped hooks guide, `10-hooks.md`: "under the detected Windows shell (pwsh, Windows PowerShell, or Git Bash)"; the order is its source's, `xai-grok-config/src/shell.rs`, `detect_windows_shell`, at grok-build `2bdd1d6a`). Windows PowerShell is part of every Windows, so the Git Bash fallback is reached only through `GROK_SHELL`; a `GROK_SHELL=bash` makes the guard a bash syntax error, whose exit 2 Grok reads as a refusal of every shell call, and `GROK_SHELL=cmd` a cmd error, which Grok lets through without Hide's hook. Grok skips a handler that carries `args`, so its entry is the string form Codex's is.

Earlier Cursor Windows entries used a PowerShell script block, prepared stdin with `$input | Out-String -Stream` before starting the helper, and answered `{"permission":"allow"}` when a permission hook could not run.
The retained byte-order-mark fixtures describe that historical format and its fail-open transition behavior; the Windows fixture separately proves that a fresh Cursor install is refused and historical owned hooks can be removed while the operator's hooks remain.
Retirement removes only Hide-owned entries and retires a recorded `herdr:cursor` piece before forgetting ownership, preserving operator settings and stored conversations.
Foreign-origin exclusions remain so another runtime loading an old hook cannot emit duplicate reports.

PowerShell starts before the helper on every Windows hook, which Codex does for any hook there; the 1,250 ms Memory wait above counts from the helper's own start.
PowerShell can read what the helper prints in the console code page and write it out again, as the historical Cursor wrapper fixture demonstrated on Windows PowerShell 5.1, so on Windows the helper prints every hook's JSON in ASCII, every other character as a `\u` escape that decodes to the same text.
The marker is looked for in an entry's `command` and in each of its `args`, where Claude Code's Windows entry carries it, so a second install on Windows recognises its entries and converges as it does elsewhere.
The current marker also captures the daemon state route; the shell guards and quoting remain the same on each system.
`hide-agent-hooks/tests/it/windows_hook_command.rs` owns the Windows command fixtures in the `windows check` lane.
`the_windows_entries_run_the_helper_through_powershell_only_while_it_is_there` exercises Claude Code and Codex with the real helper in a folder whose name has a space, a quote, brackets and a `$`; the Memory request recorded as `memory_unavailable` proves stdin reached the helper.
The Grok PowerShell fixture proves event intake through the opener guard's `daemon.unreachable` line.
Antigravity's `agys_windows_session_hook_reads_its_input_and_injects_guidance`, `agys_cmd_tool_command_leaves_the_call_to_agy_when_the_helper_is_gone_or_fails` and `agys_windows_guard_preserves_korean_and_non_bmp_hook_answers` cover its cmd boundary, guidance, fallback and Unicode answers.
Version 6 is the first guarded command, so an older entry reads outdated and the next launch or connection replaces it.
The kit folder is a path that survives a rebuild: the installed app bundle's `Contents/Resources` on macOS, the unpacked package's `resources` on Windows/Linux, and the helper root's `current` on a device (a link to the build on macOS and Linux, a folder of the build's two programs on Windows; [ARCHITECTURE.md](ARCHITECTURE.md#the-install-kit)), which each new build of the helper points at itself, as a moved core does at the build it runs.

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
A runtime whose CLI answers with a version older than its declared minimum is not installed into and its part says to update the CLI ([The oldest version of each agent](#the-oldest-version-of-each-agent)); a runtime whose home folder does not exist reads Not on this machine.
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
Install and remove are deliberately not CLI subcommands: writing to the operator's configuration is the kit's decision, agreed to when the app was installed or a device's helper allowed in Settings › Devices, not something a stray command line performs.
`hide device add` registers a device without that agreement, so it installs nothing, and `hide device remove` takes the kit off only as removing the device in Settings does.

### More than one Hide on an account

An account can hold more than one Hide installation: the desktop app, one per account and named `app`, and a device helper another machine's Hide put under a helper root, or a build Hide placed there, named `root:<helper root>` (`hide_agent_hooks::route::install_of_root`).
Each installation's kit writes its own entries and acts only in its own panes, so a Windows PC that runs Hide and is also a Mac's device keeps both sets of agents reporting.
Every entry carries its installation (`--install-b64`), and OpenCode's, Pi's and omp's plugin files carry it in their names: the app's keep the names earlier builds wrote, and another installation's add a 16-hex mark (`route::mark`).
A pass judges, installs and takes out only its own entries: those that name its installation, those an earlier build wrote without naming one, which read as an older version and are replaced, and those whose helper is gone.
A device helper's first pass takes out the plugin files its earlier build wrote under the app's names.
Codex trusts an exact command, so each installation's pass trusts its own entries.

A pane holder's pane names the installation that started it (`HIDE_INSTALL`) and that installation's own `hide` (`HIDE_INSTALL_CLI`, the `hide` in its `current` for a helper root, which a Windows device pane also finds first on its `PATH`).
An entry another installation wrote exits at once in such a pane, with nothing to say and no report: Cursor's permission hook answers allow and agy's tool hook ask, as when the helper is gone, and a plugin's helper call prints nothing, which the script reads as nothing to do.
Each event therefore also starts the other installation's helper once, which ends before it reads its input.
Claude Code and Codex start an event's matching hooks together, so that costs little time: on a Windows PC, where each entry starts `powershell.exe`, the other installation's entry added about 50 ms to a 1.6 s Stop event at the median (2026-11-04), and on a Mac the helper alone takes about 5 ms.
A pane that names no installation, such as a plain terminal, lets every entry act, as before.

A `hide` that another installation put first on the pane's `PATH` hands the call to `HIDE_INSTALL_CLI` (`hided::pane_install`), so the call speaks its own daemon's protocol.
The procedure around this rule is written once, in `hook_request::install`, and `hide`, the hooks, this account's daemon (`pane_auth`) and a device's node (`hide_host::panes`) all call it: one request (`PaneRequest`), one refusal (`refuse_other`), one way to name the installation in an answer (`stamp`) and in a bridge folder (`declare_bridge`), one asking order (`order`), one rule for the bridges (`may_answer`, `single_bridge`, `settle_bridges`) and the next step each of its refusals names (`next_action`).
In a pane of an installation other than the app, `hide` asks the device bridges first and this account's daemon after (`workspace_cli::bootstrap`), since the daemon would bind the pane by its folder alone.
The call names the pane's installation, and a daemon or device node of another one refuses it with `other_install` before it looks at the folder.
Each node names its installation in its bridge folder (the file `install`, written before its socket exists), so a pane that names an installation never connects to a node declaring another one, or none; a node from before bridges declared theirs may be the pane's and is asked.
The refusal alone cannot hold against an earlier build, which ignores the name it does not know and binds the pane by its folder, so the answer names its installation too and the call takes a reference only from an answer naming the pane's own (`hook_request::install::confirm`, which `workspace_cli::read_bootstrap_answer` applies); an answer naming none or another is `install_unconfirmed`, its reference is never used, and a node that answers so neither succeeds for the pane nor makes it ambiguous.
A refusal of another installation's node (`other_install`) is no answer either, in any bridge order, so it never replaces the reason of the daemon or a node that may be the pane's own.
When only another installation answered, `hide` says to open the Hide that started the pane, or reconnect the device in it, and retry; an earlier build's answer says to quit and reopen that Hide.
A pane whose own Hide does not answer therefore never runs its calls against another, whatever build the other runs.
The same holds against an earlier build of the pane's own installation: in the app's case that is the time between the bundle being replaced and the next app launch, whose `hide connect` replaces the daemon, and the refusal says to quit and reopen Hide.
The added answer field moves no schema or protocol number: an earlier `hide` reads only `ok`, `reason` and `reference`.

An agent hook's request to its daemon (the letter pull, Memory, the status report and the SessionStart probe, `hook_request::send`) follows the same rule, kept once in `hook_request::install`.
It names the pane's installation (`HIDE_INSTALL`, which the hook inherits from the agent), and a daemon or device node of another one refuses it with `other_install` before it proves the caller or runs the command; the daemon's and the node's answers to a hook name their installation as a reference does.
Without a reference naming its route, a hook in a pane of an installation other than the app asks this device's node first and this account's daemon after, as `hide` does, and an app pane asks the daemon first.
Of the device's nodes, it leaves out those whose bridge folder declares another installation, as `hide` does, so another installation's node beside the pane's own neither answers nor makes the pane ambiguous.
A command runs where it lands, so the hook chooses before it sends: with one node left it asks that one, and with more than one, two that declare the pane's installation or one from before nodes declared theirs, it asks none and records `ambiguous_pane`, where `hide`, whose credential runs nothing, asks each and keeps the one answer that confirms the pane.
The request moves to the next route only when nothing ran there: nothing took the connection, or another installation refused it.
An answer that does not name the pane's installation is `install_unconfirmed`: an earlier build ran the command without knowing whose pane it was, so its answer is dropped and the command is not sent again to another route.
The hook then goes on without it, as with any failure, and records the cause ([What a hook records when it fails](#what-a-hook-records-when-it-fails)): no letters are handed to the agent and none is confirmed, so they stay waiting for the next prompt or tool event; the prompt has no Memory; SessionStart gives no `HIDE_CAP_REF`, so the agent is never told another installation's reference; and the status report's failure is kept like any other.
Against an earlier build of the pane's own installation that lasts until the same launch that replaced the kit replaces the daemon (`hide connect`), and a device's node and kit are installed together when its link starts.
The request and the answer each gain one field, `install`, outside the hook request itself; an earlier daemon or node ignores it, and no schema, link or hook version number moves.
The `hide` link in the command folder stays with the installation that holds it while that installation's command is there: the other's part reads Not on this machine, "another Hide on this account holds it", and takes the name back once that command is gone.

The kit record keeps the app's parts, command and hook route where earlier builds read them, and every other installation's under `installs`.
A record without `installs` is an earlier build's, which was the account's, and another installation starts from it once.
A record that cannot be read, damaged or written by a newer build, stays as it is when an installation goes, since it may hold the other's part and the account's choices; removing that installation reports the record as kept.
The operator's agent choices and the retirements stay the account's.
A pane of an installation other than the app keeps its subagent counts and plan record under `installs/<mark>/` in its own records folder, so the app's core neither reads nor sweeps them and equal pane ids never meet.

The two coexist once both installations run a build with this rule.
An installation still on an earlier build treats every Hide entry, plugin file, the `hide` link and the whole record as its own, and can take the other's away until it updates.
What stays shared: the skill stubs and the folder trust Hide records for Grok, agy, Pi and Codex, so switching one of those agents off in one installation removes the trust recorded for both, and the next start Hide makes trusts the folder again.

### Profile accounts

A profile whose account is not its agent's default ([ARCHITECTURE.md](ARCHITECTURE.md#agent-profiles-and-accounts)) gets the agent's pieces in that account too: its hook (the entries in the hook file, Grok's and agy's hook files, or the plugin or extension script) and, for an agent that keeps its skills in its own folder (Claude Code), Hide's skill stubs.
The skill folder every agent shares (`~/.agents/skills`) is the default account's alone, so a profile adds nothing there.
Every place comes from `hide_agent_adapter::resolve` for a node that honours none of the agent's variables, and the hook code takes it as an `AccountAt` (`hide-agent-hooks/src/account.rs`), so neither crate names an account folder.
A pass reads the profiles from the node's profile file ([ARCHITECTURE.md](ARCHITECTURE.md#the-nodes-profile-file)), the ones in use and the ones being removed, once it holds the account's lock (`KitTarget::profiles`, a `ProfileSource`), so it installs and takes out by the file as it is while it writes; every change of the file is followed by a pass of its own. It answers each profile's place and pieces (`ProfileReport`, `hide-kit/src/targets.rs`).

The kit record keeps what each installation put in an account by target.
The default account's pieces stay in `installed`, where every earlier build reads and writes them, and the built-in default profile refers to them for good, so no profile's removal takes them out.
Every other account is a target under the installation's `targets`, keyed by the agent and the agent's own folder there (the `state` place) as the file system names it, with the account as the file system names it too (every link resolved), the pieces put there, the skills folder its stubs went into, and the profiles that refer to it.
Profiles whose folders are one folder, however each spells it, are one target: it is installed into once, its pieces come out when the last profile that refers to it goes, and the record forgets it once they are out.
A target whose pieces could not all come out stays recorded with no profile, and the next pass tries again; the profile being removed reports why meanwhile.
A profile whose folder is the default account's by another spelling, such as a link to `~/.claude`, is the default account's, which only the agent's switch changes.
Taking a target's pieces out goes through the account the record keeps, so it reaches the folder they went into even after a link the profile spelled was moved to another account; a recorded folder that now leads to another account, because a link inside it moved, has nothing taken out and is forgotten, and the log says so.
Every path the kit writes or takes out in a profile account passes one judgment: it must lie in the account's own folder with no link from that folder down to it, the file itself included, so a write or a removal never follows a link out of the account.
The paths come from the account's places the kit writes (its hooks file, its hook trust file, its plugins folder and its skills folder), named once for every place an account has, so a write the agent makes for Hide, such as Codex storing its trust of Hide's hooks in `config.toml`, is judged with the hook, and a refused hook asks Codex for nothing.
A profile account whose hooks file, hook trust file, skills folder, skill folder or `SKILL.md` is a link has that piece left as it is, the link included, and the piece reads Absent with the link named.
A file is taken out only when no one else keeps its real place: a default account (the shared skills folder among its places), a target still in use, or another installation on the account, since the skill stubs every installation writes are alike; a piece kept that way stays and the log says why.
Hook entries carry their installation, so each installation takes out only its own.
A record from before profiles has no `targets` and reads as the default target alone, so an upgrade writes nothing new into it; an earlier build drops `targets` when it saves, and the next pass of this build finds Hide's pieces in place and records them again.

Every rule of the default account holds in a profile account: only Hide's marked entries and unedited files are written or taken out, by the same code with its source check, link keeping and atomic replacement; a piece the operator took out stays out until a Reinstall asks for it (of the agent's part, or of that profile, `ProfileInstall`); switching the agent off takes its pieces out of every account and switching it on puts them back; removing the device takes them out of every target the record holds.
Hide writes in a profile account only while every folder from the one the profile named down to the agent's own folder is a real folder, not a link, that belongs to this account and that no other account can change; otherwise each piece reads Failed with that reason and nothing is written.
The default account keeps the checks it always had, so an account whose `~/.claude` is group-writable, as a Linux umask of 002 leaves it, is not refused by an update.
Codex's trust of Hide's entries is asked of the account's own Codex, in that account's `config.toml`, as for the default account.
The folder trust the node writes for a start with a profile ([Folder trust for the worktrees Hide makes](#folder-trust-for-the-worktrees-hide-makes)) is one of the places the kit judges (`PieceKind::FolderTrust` in `written_by`): the node writes it only when the account's trust file passes the same judgment (`hide_kit::trust_writable`), and the kit takes Hide's entries out of it whenever it takes the account's other pieces out (the last profile going, the agent's switch, the machine's removal), found in Hide's record by the account as the file system names it, so a profile that spelled the folder another way finds them too (`folder_trust::forget_account`).
A trust file that cannot be read, or a removal that fails (Codex's own program missing included), fails the take-out as any piece's does: the account's target stays recorded and the going profile reads `Removing` with the failure, so the next pass, or a reinstall that brings Codex back, takes the entries out; a place Hide may not write is left as it is, and the log says why.
Removing a worktree takes its entries out of a profile account without that judgment: the folder is gone by then, so the entry it takes out trusts nothing.

## Testing

The operator's real `~/.claude/settings.json`, `~/.claude.json`, `~/.codex/hooks.json` and `~/.codex/config.toml` are never a test target.
Every test in this crate builds its own `HOME` fixture and asserts against that.

OpenCode's plugin is tested at three layers, none of which runs the operator's OpenCode.
`hide-agent-hooks/src/plugin/tests.rs` writes, judges and removes the files in a fixture `HOME`, and `tests/it/opencode_helper.rs` runs the built helper beside a stand-in `hide` that records each call.
`tests/it/opencode_plugin.rs` runs `tests/opencode/plugin.test.mjs` in Node 22, which loads the generated `hide.js` as OpenCode 1.x does and calls its hooks with OpenCode 1.18.30's event and hook shapes (`tests/fixtures/opencode/events-1.18.30.json`) against a stand-in helper; the rust lane installs Node for it.
It runs `tests/opencode/plugin2.test.mjs` the same way, which loads the generated `hide-opencode/` halves on a stand-in for OpenCode 2.0.29's shared server, with the hook and event shapes a 2.0.29 capture showed: an instance per folder with a call routed by the folder it names, every event to every instance, and terminals in two panes; it checks that each prompt, guard and letter is answered in the pane showing its session, a letter's confirmation once OpenCode delivered or stored it, the guardrails alone with no terminal or a vanished one, and the counts of the pane showing a subagent's root.
`web/e2e/opencode-plugin.spec.ts` lets the kit write the plugin into a private `HOME`, runs a small OpenCode stand-in (`web/e2e/opencode-host.ts`) that loads it in two panes of an isolated engine, and checks a letter on the next prompt and its confirmation once stored, the opener guard's refusal, and the subagent counts the core receives.

Pi's and omp's extension is tested at the same three layers, none of which runs the operator's Pi or omp.
`hide-agent-hooks/src/plugin/tests.rs` holds the file tests for all three script files, and `tests/it/pi_extension.rs` pins both files' bytes and runs `tests/pi-extension/extension.test.mjs` in Node once per agent, which loads the generated file and replays the event and context shapes captured from Pi 1.0.4 and omp 18.7.0 (`tests/fixtures/pi-extension/`) against a stand-in helper: what stays out, the hidden message, confirmation after the reply and once across omp's re-entry, the budgets, omp's counts and `ask`, and that no handler throws.
Both Node tests start the same stand-in helper (`tests/it/host_helper.rs`), POSIX sh with builtins only, which logs each call and answers from the answers file the test writes (`tests/stand-in-answers.mjs`): the scripts kill a helper past its budget, so the stand-in costs what the helper costs, and one that started a JavaScript runtime per call spent the 1.5 s letters budget on a loaded machine before it logged the call.
`tests/it/opencode_helper.rs` checks through the built helper, beside a stand-in `hide`, which session Memory is asked for (Pi's and omp's host id, never the session file; OpenCode's own id), which Memory context a prompt asks for (the index with the first prompt, cards after), its prompt on stdin, and that the letters ride whatever Memory answers.
Which session a receipt is signed for and what the core answers are `hide-memory`'s tests (`src/hook.rs`) and the core's (`hide-core/src/memory_hook.rs`, `hided/tests/it/remote_core.rs`).
`web/e2e/pi-omp-extension.spec.ts` lets the kit write both files into a private `HOME`, runs a Pi stand-in and an omp stand-in (`web/e2e/pi-omp-host.ts`) in the two panes of an isolated engine, and checks a letter in each direction on the next prompt and its confirmation once written, the opener guard's refusal in both, and omp's subagent counts with a subagent prompt that takes no letter.

`tests/it/records_refused.rs` refuses the pane records' own folder while the rest of the records folder stays writable, which is the narrow form of a records write that fails, and checks that the session still reaches the core and that the refused write is the pane's last report failure, once through the command hook and once through the OpenCode helper.
The journey reads those records where the candidate keeps them (`scripts/agent_journey/hook_records.py`), so a start refused on a missing identity names the hook outcome that explains it, and `scripts/tests/test_agent_journey_hook_records.py` checks that reading with no agent running.
Its write sandbox denies the candidate's whole state folder to the agent, so it opens that records folder and nothing beside it (`allow_hook_records` in `scripts/agent_journey/routes.py`, checked by `scripts/tests/test_agent_journey_trust.py`): without that, a sandboxed hook still reported its session but kept no counts or turn, which is a row that never reads idle after its answer rather than the product's own behavior.
A turn that never read idle names that pane's recorded outcome too (`hook_records.pane_cause` at `Journey.wait_turn`), so a refused record write reaches the cell as `hook_record_refused` with the hook's own sentence rather than as an idle row Hide never showed, and a record of another pane explains no turn.
