# 0019. Agent profiles and accounts

- Status: Accepted
- Date: 2026-11-08
- Code: `hide-agent-adapter/src/account.rs`, `hide-core/src/agent_profiles.rs`, `hide-core/src/agent_profiles/`, `hide-node-link/src/profiles.rs`, `hide-host/src/profiles.rs`, `hide-core/src/launch.rs`, `scripts/check-agent-folders.sh`

## Decision

### Agent profiles and accounts

An operator can run one agent in several accounts or connection setups: another `CLAUDE_CONFIG_DIR`, a gateway wrapper, a shell alias (PRD agent-profiles).
A profile is one way of running one agent: its account (a folder, or omp's own profile name), a program to run instead (a wrapper), and extra variables, none of them secret.
Where an account keeps its folders is computed in one place, from data in the agent's definition, and every reader and installer of an agent's folders uses only that answer.

The data model lives in `hide-agent-adapter/src/account.rs`:

| Type | What it is |
| --- | --- |
| `Profile` | `id` (a stable `ProfileId`, kept across a rename), `kind` (the definition id), `name` (unique within the agent), `account` (`AccountRef::Folder` or `AccountRef::Name`, fixed when added, `None` for the agent's default account), `program`, `env`, `version` (raised by every change) |
| `ProfileId` | `default` is the built-in default profile every agent has: no account, no program, no variable |
| `LaunchTarget` | `kind` and `profile`: what a start carries and an agent's record keeps |
| `AccountKey` | the account's identity: the value set into the agent's account variable, exactly as set, or `Default` |
| `Account` | what `resolve` answers: `key`, `env` (the account's variables, then the profile's own) and each place, read with `Account::path(AccountPath)` |

#### `[account]` in a definition file

A definition's `[account]` table says how its accounts are told apart and where each place is:

- `by`: `folder` (variables name a folder), `name` (omp's `OMP_PROFILE`), or `none` with a `reason` (agy, which only a different `HOME` moves).
- `[account.vars]`: each variable that moves the account, with `default` (where the agent puts the folder when the variable is unset, `~/…`) and `profile` (`{account}`, or a folder below it; the name itself for `by = "name"`).
  `isolated_home = true` marks a variable a node with a home of its own sets to its default below that home (`hide_host::serve::agent_home_variables`; today `CODEX_HOME`).
- `[account.paths]`: every place of the closed list `AccountPath::ALL` (`hooks`, `hook_trust`, `plugins`, `skills`, `folder_trust`, `sessions`, `login`, `state`, `usage`), each a template (`~/…` or `{VARIABLE}/…`), a `{ default, profile }` pair, `false` for a place the agent does not have, or `"shared"` for skills every agent reads (`~/.agents/skills`), which no profile owns.
  `state` is the agent's own folder, whose presence says the agent is set up.
- `[account.forbid] vars`: variables that would move a place behind the account's back, which a profile may not set.

The definition reader checks it with every other field (`src/definition/file.rs`), so a built-in file with a missing, unknown or malformed place fails the build: every place is named, a template names only declared variables and plain folder names, `{account}` appears only on a profile's side, and no variable is `HOME`, `USERPROFILE` or `HIDE_*`.
An operator's file for a new agent may carry its own `[account]`; a `based_on` copy keeps the copied agent's, and a file that replaces a built-in agent's screen rules cannot set one.

The seven agents' accounts (Cursor is retired and keeps none):

| Agent | by | Variables, default to profile | Forbidden |
| --- | --- | --- | --- |
| Claude Code | folder | `CLAUDE_CONFIG_DIR`: `~/.claude` to `{account}`; its global config (`folder_trust`, `login`) is `~/.claude.json` for the default and moves into the folder for another account | none |
| Codex | folder | `CODEX_HOME`: `~/.codex` to `{account}` | `CODEX_SQLITE_HOME` |
| OpenCode | folder | `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`: their defaults to `{account}/config`, `/data`, `/state` | `OPENCODE_CONFIG_DIR`, `OPENCODE_DB` |
| pi | folder | `PI_CODING_AGENT_DIR`: `~/.pi/agent` to `{account}` | `PI_CODING_AGENT_SESSION_DIR` |
| omp | name | `OMP_PROFILE`: unset to the name; its places move from `~/.omp/agent` to `~/.omp/profiles/<name>/agent` | `PI_CODING_AGENT_DIR`, `PI_PROFILE`, `PI_CONFIG_DIR` |
| Grok | folder | `GROK_HOME`: `~/.grok` to `{account}` | none |
| agy | none | none: only a different `HOME` moves its folders | none |

An optional `[account.login]` names the account path (`at = "login"` or `"usage"`) and `any`, 1 to 16 JSON pointers to nonblank credential strings.
An initial `/*/` visits each provider or scope in a JSON object; a pointer has at most eight parts and 256 bytes, and no other wildcard.
The shared reader (`hide_host::credentials`) applies these rules to bounded files for every agent, and the usage readers use its same JSON and credential-string interpretation.
Only Claude Code's macOS keychain presence needs agent-specific code (`hide_host::login::claude`); no token or file contents leave the node's readers.

#### `resolve`

`hide_agent_adapter::resolve(definition, profile, node)` is a pure function: no file, no process, no network.
`NodeEnv` is what the node captured: the account's home, the agent variables that node honours, and its system.
The default account sets no account variable: a place follows the node's own value of a variable when the caller handed one in (a relative value is refused as `NodeVariable`), and the definition's default otherwise.
Claude Code depends on this: naming `~/.claude` in `CLAUDE_CONFIG_DIR` is another login.
For omp, whose variable holds a name, the node's `OMP_PROFILE` is the account the default reads: its places are `~/.omp/profiles/<name>/agent`, as omp itself reads them, and a value that is not one path part is refused as `NodeVariable`.
Another account sets each variable from the account as stored: `~` is expanded once, when the profile is added, and a final separator is kept, because Claude keys its keychain item by the exact spelling.
It refuses an empty account, a folder that is not absolute on the node's system (Windows' `C:x` and `\x` forms included), the default folder itself (use the default profile), a name that is not one path part, an account for an agent of `by = "none"`, and a profile variable that is the account's, a forbidden one, `HOME`, `USERPROFILE` or `HIDE_*`, repeated, not a name, longer than 256 bytes, or whose value holds a NUL or exceeds 32 KiB; a profile adds at most 64 variables.
Windows compares variable names regardless of case.
A refusal names a variable only by a name that passed the name check, never a value.
Checking that a folder exists, whom it belongs to, and making it is the node's, when a profile is added.

A built-in agent's account is asked through its adapter (`AgentAdapter::resolve`, `account_in`, `place_in`), which reads that file's `[account]` tables alone (`definition::builtin_account`): an account is asked inside reads with a time budget, such as a usage step's 10 ms, which cannot wait for every definition to be read and its screen rules compiled.

#### Who does what

| Module | Does | Does not |
| --- | --- | --- |
| `hide-agent-adapter` | the `[account]` schema and its checks, `Profile`, `LaunchTarget`, `resolve`, `Account` | any I/O or check of a path on disk |
| `hide-agent-hooks`, `hide-kit` | each agent's file formats and approval keys, installed at the account's places; the profile file's place (`layout::agent_profiles`), the install record by target and each target's install and removal ([agent-hooks.md](../agent-hooks.md#profile-accounts)) | compute a place |
| `hide-host` (the node) | owns its machine's profile file, checks a profile when it is added and makes its account's folder ([The node's profile file](#the-nodes-profile-file)) | judge whether an agent runs with a profile |
| `hide-session`, `hide-host`, `hide-ai`, `hide-desk`, `hide-core` | read sessions, sign-ins and usage at the account's places; an agent's own module handles what only that agent does there | compute a place |

Installs ask the account of a node that honours no variable, so hooks, plugins, skills and folder trust go to each agent's default folders whatever the launching shell exported (`AgentAdapter::account_in`), and to the account of each profile the node's profile file holds ([agent-hooks.md](../agent-hooks.md#profile-accounts)); the readers that followed a variable before still do, by handing it in: Codex's sign-in for usage, Hide AI and Desk follows `CODEX_HOME`, and OpenCode's database follows the node's `XDG_DATA_HOME`.
A session reader reads only in the accounts its node registered (`hide_session::Accounts`): each agent's default account under the node's home and the account of every profile the node's profile file holds, one being removed included, which `hide_host::profiles::session_accounts` answers and keeps until the file's stamp moves, so a read costs one stat of the file (PRD agent-profiles D-36).
A path a hook, a request or another machine names is read only when it lies in one of those accounts' `sessions` places, and belongs to the innermost account that holds it (`Accounts::account_of`); a request that names an account the node did not register is refused as `ACCOUNT_UNREGISTERED`, and a path never becomes a root of its own (B31).
A path in a profile's account is proven down from the account's folder with no link and no `..` on the way, as Pi's, omp's and Grok's readers prove every account's (`hide_session::inside_session_root`), and a listing walks a profile's `sessions` place only once it is proven so (`session_roots`): a link under a profile's folder never makes an account the node did not register readable.
The reader relates native prefixes with `hide_platform::path::strip_prefix`, so a long Windows session whose resolved spelling keeps `\\?\` remains in the same account as its shorter home; the remainder is not permission to read it, and every component below that home is still checked for `..` and links.
Claude Code's and Codex's default account keeps following every link, as it did before profiles (B29): an operator's `~/.claude/projects` or `~/.codex/sessions` may be a link, and is read and listed through it.
A read of one agent's session names the account its start ran in (`LabelTranscriptRequest::account`, `SessionActivityRequest`, a session thread's reference), and the node reads in that account alone (`Accounts::reading`): the core takes it from the one pure decision over its hook-reported account and matching launch record (`runtime::agent_choice::pane_run`, used by `Runtime::pane_account`, the label reads and the phone's conversation), and the label worker gets those records with its switch only when they changed (`missions.launches_edit`), so a publish adds a counter compare (B19).
A listing that has no one session to name (the session links a resume list reads, usage, the catalog) walks each registered account on its own reader thread.
The phone's conversation asks the core for its pane's account and opens the session in that account alone, for both paths and ids (`hided/src/mobile/conversation.rs`).
A resume whose record kept the profile it ran with (an archive's, a feedback opener's) runs with it, and a record kept before profiles with the built-in default (D-27).
A resume of a session line or record names no account, so before its tab is made the start asks the machine that holds the session which of its registered accounts holds it (`Call::SessionAccount`, `hide_session::session_account::read`, by the file its line names, or OpenCode's id in each account's database, one database read once however many accounts name it) and runs with that account's profile (`Runtime::settle_session_profile`): the built-in default for the default account, else that agent's profile of the account, its default among several first, then the list's order (B19).
An account no profile names now, an id two accounts' databases hold, or a machine that keeps profiles and does not answer starts nothing, never the default account in its place; a node on the older protocol the core still admits keeps no profile, so its sessions are the default account's.
A mission keeps the profile its Conductor started with (`Conductor::profile`), and a take-over that names the Conductor's session by id alone resumes with it: no profile kept, as a mission recorded before profiles is read, is the built-in default, and two missions that name the session with different profiles start nothing.
A Conductor no launch record describes (one started by hand) keeps its profile as unknown (`Conductor::profile_unknown`), and its take-over asks the session's machine as a resume by id alone does.
A Claude Code or Codex resume that names its session by id alone, with no file and no recorded profile, is the start these agents always had, whose own command finds the session and resumes it; as a policy of compatibility, not a proof of the session, its machine answers the default account only while its profile file, read, keeps no other account of that agent, a profile being removed included and a wrapper in the default account adding none (`Accounts::default_alone`).
Another account of the agent refuses it (`session_account_unproven`), as does a profile file the machine could not read or a profile whose account it could not register (`session_accounts_unsettled`), and the core never puts the default account in its place.
Before a delegated start, its one picked request names the account whose hook is judged: another account's hook from its profile report, or the ordinary agent hook for the default account, including a wrapper profile of that account (`runtime::start_hook`).
An unread or missing account hook is never treated as installed; the same pass and refusal policy applies as to the default account.
The judgment keeps that profile's account and version, and the node's launch answer must match both before anything is typed; node calls remain outside the runtime lock.
The start panel reads the final automatic choice from `status.start_profiles`, by project and agent, made by `Runtime::new_start_profile` and the same `launch::pick` as an actual start, to preselect an item (PRD agent-profiles-phase2 D-03); the item the panel then shows is sent as the start's `profile`, the built-in `default` included, so the start is always the item chosen (D-02) and `launch::chosen_id` takes it before the project's choice, the parent's or the agent's default can.
Profile lists, project profile settings and project identity, root or device changes refresh this nonpersistent projection; ordinary pane catalog updates keep it without running the picker again.
An explicit start still validates its chosen profile when it starts.
A profile agent keeps the title its agent has on the built-in default; its row carries the profile's name (`SidebarAgentSnapshot::profile`, absent for the default profile), which the screens show as a mark beside its logo, and `hide agent list` and `show` name it after its agent, `Claude · a` (`AgentView::profile`, `null` for the default profile, `launch::shown`).
agy's module keeps only what lies inside its default account's `sessions` place and beside it, the transcript's form and the index's name.

`scripts/check-agent-folders.sh` keeps it that way: production Rust that names an agent's account folder (`.claude`, `.codex`, `.pi/agent`, `.omp`, `.grok`, agy's `.gemini` folders, OpenCode's folders and database) outside that agent's own modules and the files it lists with a reason fails `verify`, whether one literal names it or a `.join("…")` chain or an array of literals builds it.
`hide-agent-adapter/tests/it/account.rs` holds `resolve`'s contract for the seven agents: the default account, an account of its own, and the inputs refused, with the expected places written by hand from the agents' own documentation and source, on the macOS, Linux and Windows lanes.

Decided in #2271.

#### The node's profile file

Each machine's profiles are its node's, in one file in that node's state folder (`agent-profiles.json`, `hide_kit::layout::agent_profiles`), so a device's profiles name the device's own folders and stay with it (PRD agent-profiles D-04).
Only `hide-host/src/profiles.rs` reads or writes it, one call at a time, replacing it whole with the private access every state file has.
It holds each profile (`Profile`, with `removing` while it is being removed), each agent's default profile where that is not the built-in one, and the file's `version`, which every change raises.
At most 64 profiles, besides the built-in ones, and a file of at most 4 MiB are kept, and no more than that is ever read of it; a profile's own caps are `resolve`'s.
A file written by a newer Hide, or one that cannot be read, is refused and left as it is, and a kit pass then leaves every profile account as it is.

The core asks the node through four calls (`hide-node-link/src/profiles.rs`, protocol 86 for the first three and 88 for `ProfileLaunch`):

| Call | Answers | Does |
| --- | --- | --- |
| `ProfileList { values }` | the list, each profile with its variables' names (their values only with `values`, for the screen that edits it) and its pieces as the kit judges them | nothing |
| `ProfileApply { change, expected_version }` | the list after the change, the id an `add` made, or a refusal under the field it is about (`kind`, `name`, `account`, `program`, `env` with the variable's name, or `profile`) with nothing saved | `add`, `edit` (name, program and variables only: an account is fixed once added, D-24), `default`, `remove`; a list that moved since `expected_version` is refused as `conflict` and read again |
| `ProfileInstall { profile }` | the list | Reinstall of that profile's pieces, even those the operator took out |
| `ProfileLaunch { kind, profile }` | `ready` with how a start of the profile runs now (its name, account, version, the account's variables then its own, its program as the machine finds it, and the `hide` beside the node), `gone`, or `refused` with a sentence that names the profile and, when it is the cause, its program | nothing; asked when a start is planned and again when its executor asks for the profile ([Starting an agent with a profile](#starting-an-agent-with-a-profile)) |

Adding a profile expands a `~` at the start of its folder once, keeping a final separator, has `resolve` check it, then makes the folder it names and the agent's own folder in it private when they are missing and checks that each folder between them is a real folder of this account that no other can change (`hide_kit::prepare_account`, D-20, D-28); only then is it saved and its pieces installed.
A name is unique among the agent's profiles regardless of case, and no profile takes the built-in one's name.
Removing one marks it `removing`, which the list carries so a start can refuse it, and drops it as the agent's default; the kit takes Hide's pieces out of its account once no other profile runs there, and the profile leaves the file when the kit reports that, in the same call or in a later kit pass.
Every kit pass of the node hands the kit the file to read once the pass holds the account's lock (`profiles::source`), so the agent's switch, a Reinstall, an update and the device's removal reach every profile account as the file holds it then ([agent-hooks.md](../agent-hooks.md#profile-accounts)); the module never holds the file while it waits on the kit, so the two locks are always taken in that order.

The core reads the list after each kit pass and after a kit read until it has one, with the runtime lock released (`crate::kit::read_profiles`), and keeps it on the machine's kit snapshot (`KitSnapshot::profiles`): names, accounts, programs and variable names, never a value.
The list carries the node-resolved `AccountKey` beside each profile (`ListedProfile::account_key`, node protocol 92); the core never reconstructs it from the account reference or folder.
Protocol 91 and other unaudited versions are refused at Hello, while the audited protocol 24 transition remains the built-in-default-only boundary.
`ProfilesSnapshot::in_account` owns the profiles of an account in automatic preference order, and `startable` owns whether an automatic default is usable; an explicitly selected or recorded profile is still refused by name when unavailable.
Each later kit read carries the kit's judgment of every profile, which the snapshot takes only for the version it holds, so a late answer about an earlier version is dropped (D-30).
A node that knows the calls and keeps no profiles (one without a state folder) refuses them as `unsupported`, a node on an older protocol the core still admits cannot read them, and either reads as `unsupported`: that machine offers only each agent's built-in default profile.
A list that cannot be read keeps the last one, marked `unreadable`, and why goes to the log.

#### Changing a machine's profiles

Settings › Agents and `hide agent profile` change a machine's profiles through one path, `hide-core/src/agent_profiles.rs` (PRD agent-profiles D-05, D-33, B11).
The runtime plans the change under its lock from the list the machine last gave and the agents that run with each profile (`runtime/agent_profiles.rs`), then the node's calls run with the lock released, and the outcome is settled under the lock again: the new list goes into the machine's kit snapshot and a removal takes the profile out of every project that chose it.
A command runs those calls on the CLI request's own thread and prints the answer; the screen's `agent_profile_change` runs them on a thread of its own and answers in `status.agent_profile_answer`, which carries the screen's request id and, for an edit form, the variables' values the list never carries.
One change runs at a time on the whole core; another meanwhile is refused as `profile_busy`.
Both make the same checks and refuse by the same codes: `profile_invalid` (a malformed name, an agent Hide keeps no profiles for, an account for one that keeps none apart, a variable that is not `KEY=VALUE`), `profile_builtin` (the built-in default is never edited or removed), `profile_gone`, `profile_in_use` (an agent runs with it, a sleeping or dormant one included, listed with where it is, or starts with it, listed as starting with no pane yet), `profile_refused` (the node's refusal, with the field, a `profile_reason` code and bound the screen words in its language, and the English sentence the CLI prints), `profiles_unsupported`, `profiles_unread` and `profile_unanswered`.
An edit is made against the values the node lists then, so a field it does not name keeps its value; a list that moved since the plan was made is read again and the change made once more on it.
A start and a removal of one profile see each other under the runtime lock (`hide-core/src/launch/holds.rs`).
A start with a profile takes a hold when it is planned, before its machine is asked for the profile: a spawn, a task's or the start panel's start, a closed pane's reopen and a fork do, while a wake, a dormant start and a restore bring back an agent whose sleep, dormant or pane record already counts as using it.
A definite non-start releases the hold immediately, even when another copy of the request remains.
A start that ran or may have run hands its launch record through the same bounded hold table; completion wakes the existing session coordinator, and removal planning, removal rechecks, new start admission and publication take pending results under the runtime lock.
That transfer writes the record before releasing the reserved slot, with no protection gap; an unwind after submission transfers an unknown result too.
The record's `pending` flag protects an account before its agent is observed and ends when a matching agent is observed or its pane is authoritatively gone; it is the observed account that counts when a wrapper runs elsewhere, so the original profile is free.
It is deletion protection, never execution evidence: after `ready`, an unknown result with no observed agent conservatively protects its profile until the pane is removed.
Completion incorporates a session already observed while the worker returned, so it cannot restore `pending` or lose that session.
An in-process start sequence prevents an older completion replacing a newer record or its bound conversation, and successful callers write only the built-in default's record, leaving profiled records to the transfer.
Removing a device forgets its outstanding starts and removals.
A removal is refused while a hold stands, listing each as starting; a removal whose job is prepared marks the profile being removed with the list version it names, and a start of a marked profile is refused as `profile_gone`.
The mark goes when the node definitely refused the removal, or when a list at a version newer than the one the removal named shows the profile gone or not being removed, since a change applies only against its version; a list no newer than that, which may predate the profile or a late apply, or an answer that never came, ends nothing.
A removal made again on a list that moved is checked again under the runtime lock as its plan was (`Runtime::recheck_removal`: agents running, sleeping or starting with the profile refuse it) and marked at the new version, with the node's calls still outside the lock.
A hold also keeps a launch record's slot from its plan until its record is written, so starts planned together never take more than `PANE_LAUNCH_LIMIT`: a start with a profile past it is refused as `launch_records_full` before it runs, and a start on the built-in default takes only a slot no hold keeps, else it is recorded nowhere.

#### Starting an agent with a profile

Every start of an agent goes through `hide-core/src/launch.rs` (PRD agent-profiles D-26, D-27, D-32).
A start names a `launch::Request`: the `LaunchTarget` (agent and profile id) and the name the start knew the profile by, which a refusal names.
A new start picks it (`launch::pick`): the profile the start named, else its parent's when the parent is the same agent on the same machine and its hook report or launch record describes its account (`launch::Parent`: the profile the record keeps, the built-in default included, since every start Hide makes is recorded, the built-in default on the CLI's defaults too), else the project's default (`ProjectSettings::agent_profiles`, by agent id, which Project settings › Working style › Agent accounts sets for the project's device and root, so every checkout and worktree of it shares it), else the agent's default on that machine (`ProfileList`'s `defaults`).
A `hide agent spawn` gathers those inputs in one place (`Runtime::spawn_request`: the name `--profile` gives, the caller's profile for a child on its own machine, the project's at `--repo`, the child machine's list), and it is asked once per start: a mission's or plan's candidate admission asks it under the lock and hands its answer to the start (`coordination::Command::Spawn`'s `pick`, which only the core sets), and a spawn no admission judged asks it in the executor, so a worker candidate (`missions::Candidate`, an agent and a profile id of the mission's machine, none for the built-in default) is compared with the profile the worker starts with, an inherited one included, never with the name the spawn gave alone, and a project's choice changed in between cannot start what was not admitted (PRD agent-profiles-phase2 D-08).
The same spawn asked again once its child has started is judged by neither (`coordination::earlier_spawn`): the ledger answers that child, and only the intent's arguments are compared (`intent_conflict`).
A Conductor's spawn of a node's worker (`--intent <mission>:<node>`) that names no `--profile` starts with the item the node's plan names for that worker (`Runtime::planned_request`, the built-in default for a plan that names none), when it starts that agent on the mission's machine, so it never inherits the Conductor's own profile and runs as the map showed it; a profile the machine no longer lists is refused (`profile_gone`).
That item is the start's pick (`coordination::Picked`), not the spawn's `--profile`: the spawn keeps the arguments its caller sent, so the same command asked again is the same spawn, one that finished answers its child before the plan or the profiles are read (`coordination::earlier_spawn`), and the same intent with a `--profile` it did not send is `intent_conflict`.
A plan keeps a worker's and a checker's profile as its id on the mission's machine (`missions::Worker::profile`, written by the name `--profile` takes and turned into the id by `Runtime::plan_profiles`).
A node that started keeps the profile ids its worker and checkers have when the plan names them again for the same agent, so a profile renamed or removed since never refuses a replan of work that ran.
A parent of the same agent with neither a matching record nor a registered account report is `Parent::Unknown`: its account is not known, so the child picks as a start with no parent does, never assuming the built-in default, and `agent_profile.parent_unknown` logs the pane and whether the record was missing or another agent's.
A project's or machine default the machine no longer lists or is removing falls through to the next (`ProfilesSnapshot::startable`, also the start panel's sole preselection source); a parent's profile is asked of the machine like a recorded one, so one that is gone refuses the child by name rather than starting it in another account.
An agent that comes back (a wake in its pane or dormant, a fork, a pane's Reopen, a closed tab's or pane's reopen, an archive's resume, a View's `Reopen and send`, which resumes the View's opener with the profile `view_feedback::Opener` keeps) names the profile its record keeps (`Request::recorded`).
`launch::resolve` turns the request into the `launch::Launch` that `engine::AgentLaunch` carries, and nothing else makes one, so no start function takes an agent's kind alone.
It runs with the runtime lock released, on the thread that makes the start; a spawn, a fork and a closed tab's or pane's reopen ask it before they make a pane or touch one, so a refused profile leaves no pane behind and a reopen keeps its shell with the refusal as its one notice.

The built-in default starts as every agent always has: its line is the agent's program and arguments, nothing is asked of the machine, and no account variable is set (B29).
A start whose first input rides it as the launch prompt is the one exception to the bare line, under the built-in default and any other profile alike: the node keeps the input in a private file (`Call::PromptFile`, which answers the file's path and the `hide` beside the node), and the line runs that `hide` with the file (`agent exec --prompt-file <path> -- <program> <arguments>...`), which reads the file, removes it and hands its text to the program as its last argument, so no shell reads the text ([delivery.md](../delivery.md#the-input-ladder)).
Such a start with no profile asks the node nothing more: the executor finds the program on its shell's `PATH` as the shell would have and hands the pane to it as below, and a file that is not there or cannot be read ends the start with why, never with an empty argument.
Any other profile is asked of the node of the machine that runs the pane (`ProfileLaunch`), so an edit counts from the next start (D-11) and a start of a profile its machine no longer has is refused by name (B20).
Its start line runs the `hide` beside that node, which the node names, with the start's attempt and the agent's own words, nothing of the profile:

```text
<hide> agent exec --attempt <16 hex> [--prompt-file <path>] -- <agent's program> <arguments>...
```

The attempt keeps which profile the start was checked with and at which `version` (`launch::ExecAttempt`), and the executor asks for the rest just before it runs: `HookCommand::AgentExec` with `profile`, answered only while that attempt runs in the pane and has no `ready` (`Engine::exec_attempt`), from the profile as its machine has it then (`launch::exec_start`: `ProfileLaunch` again, asked by the daemon off the owner thread, `ExecAsk`) and only at that version.
The answer (`ExecProfile`) is how the start runs, the account's variables then the profile's own, its program and that program's leading words, with the agent and the profile as the operator reads them for its failures, or `refused` with why: the profile is gone, being removed, changed since the start was checked (a start never runs values its record and its words were not made from), larger than one hook answer takes, or its machine did not answer.
The executor reports a refusal as its check's `refused` and runs nothing; a line whose start has ended is handed nothing and runs nothing either.
So the line, and the shell history that keeps it, carry no account folder, variable or wrapper path; the absolute `hide` stays because the executor has to be that node's own build, which a bare `hide` on the pane's `PATH` (another installation, or none on a device) is not, and the attempt stays because it is what tells a line that runs late from the start its pane runs now.
The attempt's shape is this build's (8 random bytes in lowercase hex, `fresh_attempt`), and an executor refuses a line of another shape before it asks or runs anything, so an older `hide` given this line, or this one given an older line, whose variables rode the line, starts nothing rather than an agent in the default account; a helper on 88 or 89 is not admitted, and a node the core still admits on protocol 24 starts only each agent's built-in default, so the core never types this line on either.
The hold the start took when it was planned stays until the runtime sees what came of it, so a removal through this core is refused all along and the executor's ask refuses one made elsewhere.

`hide agent exec` (`hided/src/agent_exec.rs`, run before anything else in `hide`) adds the variables to its own environment and hands the pane to the program (`hide_platform::process::hand_over`): on Unix it becomes the program, so the agent is the shell's own child and the variables never reach the shell (D-26, B21).
On Windows it runs the program as its one child in a job that ends the program's whole tree with it, passes Ctrl-C and Ctrl-Break to the program while it waits, and ends with the program's exit code; the pane's foreground follows `hide.exe` to the agent behind it (`console_foreground`).
A profile's own program (a wrapper) runs in place of the agent's, with its leading words and the agent's arguments alone: the words the agent's own program starts with on that machine (npm's script on Windows, `engine::StartProgram`) are dropped from the line when the checked profile has one (`Launch::agent_file`), which the pinned version keeps true when the executor runs, and the agent's program still names the agent in a failure.
Before anything is typed the node checks that the program and the `hide` beside it are programs this account can start, a script's interpreter included (`hide_platform::programs::runnable`; B22).
Each such start is an attempt (`engine/own/attempt.rs`): the engine gives it a fresh id, which the line names first so a bounded read of a process's arguments finds it (`hide_agent_hooks::hook_request::is_attempt_executor`), and keeps it on the pane (`PaneLive::attempt`) with the start's claim on the agent's name, one attempt per pane at a time: a second start into that pane is refused as `agent_pane_busy` while one runs.
Once its profile is answered, the executor checks the program again as it will start it, then tells the pane's node through the hook connection (`HookCommand::AgentExec`): `refused` with why the program cannot start, or `ready`, after which it starts the program only once that report is answered ok, so a line whose start has already ended does not run its program late.
A report is taken only from that attempt's executor: the side that receives it (hided for a local pane, the device's `hide-host` for one there, which hands it on in `PaneProof::executor`) reads the connection's process from the kernel, its start time, arguments and start time again, and refuses one whose arguments are not the attempt's executor as `attempt_mismatch` (`executor_process`); the engine keeps that process, pid and start time, from the first `ready`.
A report never says that the program runs.
After `ready` the engine asks the pane's node to read that process fresh (`PaneHolders::process`, `PaneCall::Process`, protocol 88: a process of the pane's shell's tree, its start time, parent and bounded arguments, read with no cache), once per `agent_start::POLL_INTERVAL` with no lock held: on Unix the program runs in the executor's own process, so the same process with the same start time and other arguments than the executor's is the program; on Windows the executor reports the program's process (`spawned`) only once that process is in its job and resumed, and the program runs while a read finds that same process under the executor.
The pid under another start time, or no such process in the pane, ended the attempt without that proof; the pane's own facts never decide it, and an agent the pane's foreground shows is not taken as the attempt's while it is undecided (`agents::apply`), so the executor alone in the pane neither shows the agent nor spends its claim.
The wait sends nothing, so it asks only whether the engine stopped, never the gates of the call that made the start: a wake's gate closes once the woken agent shows and the sleeping record goes, which is no reason to end the attempt (the executor's profile ask would then be refused as `attempt_unknown` and the pane would run no start).
The first outcome taken under the topology lock is the attempt's (`Attempt::settle`).
A program that cannot start, before or after `ready`, fails the start with `profile_refused` and the executor's sentence, frees the agent's name at once, and leaves the pane a shell with the same line and exit code 127 (not found) or 126.
An attempt ended before any `ready` was admitted is a definite `profile_refused`: ending and accepting `ready` share the topology lock, so a late executor is refused and runs nothing.
After `ready`, a start with no outcome by its caller's deadline (the shell wait's bound, or the spawn's), or whose pane, engine or waiter went first, ends as `profile_unconfirmed`: whether the program runs is not known, so nothing starts it again, and a closed agent's reopen that hears it keeps the pane a shell with that notice rather than starting a fresh conversation (`launch::ends_a_start`).
A program that ends before any read sees it can read as unconfirmed too; nothing ever counts it as started or as not started without a read.
A profile's resume after a restart is an attempt too, watched by a thread of its own (`Restore::observe_resume`): one whose program cannot start is named not resumed with the executor's sentence, and one unconfirmed is only logged (`agent.resume_unconfirmed`), leaving the pane to what its facts show.
The built-in default makes no attempt and waits for none.
An OpenCode fork whose session OpenCode's server makes (`Call::OpenCodeFork`) answers its profile first, so a refused profile ends the fork before OpenCode makes anything, and runs that OpenCode with the profile's variables and program, as its agent will start.

A start the machine does not answer for types nothing and is refused with a code its caller words:

| Code | When |
| --- | --- |
| `profile_gone` | the machine has no such profile, or a start named one it does not list (`hide agent spawn --profile`, refused before anything is reserved) |
| `profile_refused` | the node refuses the profile now: being removed, another agent's, an account or variable that no longer resolves, a program it cannot find, or no `hide` beside it |
| `profiles_unsupported` | the node predates profiles: only each agent's built-in default starts there |
| `profile_unanswered` | the node could not be asked, or did not answer within 10 s; why goes to the log |

What started is kept as `launch::LaunchProfile` (the id, the name then, and the account), on the pane's launch record (`PaneLaunch::profile`), on a mission node's proven launch as its id and name without the account (`missions::NodeLaunch::profile`), a sleep's (`SleepRecord`, `DormantRecord`), an archive member's (`ArchivedMember`), a closed agent's (`ClosedAgent`) and a fork's (`ForkRequest`), and on a spawn's record (`SpawnRecord::started_profile`).
None, which every record a build before profiles saved holds, is the built-in default, and the field is left out of what is saved, so a record of the built-in default reads as it always did (`node_migration::upgrade_tests` loads each released state folder and checks that every agent it holds restarts with the built-in default's line).
A Desk job waiting in its queue keeps the profile its spawn names beside the spawn rather than in it (`desk::QueuedJob`), since Desk 1.2.4 reads a spawn that names no field it does not know and starts empty on a store it cannot read.
A restart asks again by id, so a renamed profile still starts and a removed one refuses the restart rather than starting the agent in another account.

#### A profile account's usage, sign-in and the account a hook reports

Each account's limits are read on their own (PRD agent-profiles B23).
The core's usage driver keeps the agent's default reads as they were and, beside them, one read per source for each account of this machine's profiles, made for the first profile in that account in the list's order (`usage::UsageDriver::follow_profiles`); a read names the profile (`Call::UsageLimitFetch::profile`), never a folder, and the node resolves it from its own profile file (`hide_host::profiles::usage_account`), so a read stays inside an account the node keeps.
Claude Code's login in a profile account is its keychain item named by the account's folder exactly as set (`Claude Code-credentials-` and the first eight hex digits of its SHA-256), else the account's `.credentials.json`, and its `/usage` fallback runs with the account's variables set on the CLI alone; a Codex profile reads its own `CODEX_HOME`.
A usage row carries each profile's limits (`AgentUsage::profiles`), and a profile in an account a profile before it or the agent's default already shows (a wrapper or variables-only profile, a second profile of one folder) is `shared`, which Settings › Agents draws as `—`; every profile of a folder account carries that account's limits, so the start panel shows the same percent on each item that runs in it, and a profile in the agent's default account reads the agent's own row; with no profile the row is on the wire as before.
A profile account's reads, its first read included, run only while a window shows its agent's usage row (`UsageActivity::shown`); Memory and the usage limit, which can ask with no window, read the default account alone.
Limits are read on the core's own machine, so a device's profiles show none.

A machine's node lists each profile's sign-in with the list (`ListedProfile::login`, B10): signed in, signed out, or unknown when the place that says so did not answer, by the agent's own check (`hide_host::login`: Claude Code's keychain item, asked whether it is there and never read, or its credentials file; the sign-in file Codex, Pi and Grok write).
A file-based judgment checks the agent's required credential values through `hide_host::credentials`, shared with usage parsing: Codex's API key or access and refresh tokens, Claude Code's OAuth access or refresh token, Pi's provider key or OAuth tokens, and Grok's scope tokens.
Empty or cleared credentials say signed out; unreadable, oversized or malformed files say unknown, without logging their contents.
A profile that runs a program of its own, one in the agent's default account, and an agent whose sign-in Hide has no way to look at (omp's database, OpenCode, agy) show none.
Each answer stands for 30 seconds (`login::LOGIN_TTL`), so a list asked as Settings opens and after each change starts at most one `/usr/bin/security` per profile in that time; no session sync or usage read asks for one.

A hook's session-start report carries the account variables its agent runs with (`HookStatusReport::account`, `hide_agent_hooks::account::reported_variables`; an empty list is the default account), and the node lists the variables each profile's start sets (`ListedProfile::account_variables`), both exactly as set, so the core compares the two strings and never looks at a path, which may be a device's (`runtime/account_reports.rs`, D-03).
An account is its values as set (D-15): Claude Code keys its login by the string, so `/a` and `/a/` are two accounts, and an alias that spells a profile's folder another way (a link, a final separator) runs in an account Hide keeps nothing of and reads as before.
Claude Code drops its variables from a hook's environment under `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1`; its hook then reads the folder above the session's `CLAUDE_ENV_FILE`, which Claude Code keeps at `<folder>/session-env/`.
A pane whose report matches one profile's account is read in that account, its conversation reads included (`Runtime::account_launches` hands the reported accounts to the label reads beside the launch records, taken again when either moves), and its row names that profile (B24); an account several profiles share is read there and the row names the agent alone; the default account, and variables no profile sets (an unregistered folder, the node's own default), read as before.
A pane the core started with a profile whose hook reports another of the machine's accounts is read where it runs, its sleep and reopen records take that account's profile, and the profile it started with is marked `wrapper_folder` until a later start of it reports its own account (D-35, B25): each reported start keeps its place among the machine's starts (`account_start` in the hook facts), the high water mark is retained when a pane closes, and the core owns each profile's last sequence and warning together, so an older surviving pane cannot overwrite it.
A repeated session/account report keeps its sequence; the counter refuses overflow.
The mark is laid again on each list the node sends and removed with the profile on an authoritative list or when its device is removed.
The report is kept with the hook facts across a restart, and the answer per pane is worked out again only when a report, a list or the saved facts come in, so a projection reads one map entry a row.
