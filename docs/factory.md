# Software Factory

The Software Factory takes Tasks whose requirements are clear, runs them through worker agents in dependency order, verifies and merges them, and hands a person only what needs a person.
This guide owns the Factory's engine, its store, its roles, its commands and the read model a screen draws.
`hide-factory/`, the host thread in `herdr-core/src/factory.rs` and `herdr-core/src/runtime/factory.rs`, and the `hide factory` route in `hided/` are its executable authority.
[delivery.md](delivery.md) owns the mailbox the Factory writes through, [AI_PROVIDERS.md](AI_PROVIDERS.md) owns the provider boundary its judgments use, and [ARCHITECTURE.md](ARCHITECTURE.md#the-factory-host) owns the thread and lock rules.

Three deciders work in a fixed order, and a decision that ends at a lower one never rises.
Code runs the rules and enforces every boundary: states, order, slots, merges, reverts, roles and caps.
A tool-less judgment may only make work slower or safer: it adds a question, a dependency or a flag, and never removes, widens or approves.
A person sets intent, loosens a gate, widens a scope and decides what cannot be undone.

## What a Factory is and who owns what

A Factory is one project's work queue.
A project has at most one Factory, named by the project's canonical primary checkout path; its id is `f-` and the first ten hex digits of that path's SHA-256.
A Factory runs on the machine that holds its project: the core's own, or a node that dials the core (`Factory::node`, none for the core's own, set by `init` from the caller's machine or the machine a create sheet names); a device the core dials keeps none (`factory_device_unsupported`).
Its git, quick checks, verify runs, file reads, free-space and memory reads and workers are that machine's; its GitHub work and its decisions are the core's.
While its node is not linked, its work waits in the states it already has, and each step converges when the link is back: a worker start asks the same spawn intent again, which continues the reserved spawn instead of starting a second agent; a merge reads the repository first, so a branch already merged is not merged again; a worktree removal reads it first too, so a folder already gone is left; a revert is made in the Factory's own main worktree and reaches main only through its own merge step; and a verify run the link took with it answers `lost`, which counts as an environment failure and verifies the Task again.
A core move keeps each Factory and worker on its machine: the old owner's become its node's, the new owner's become the core's own (`herdr-core/src/node_migration/reown.rs`).
A Factory whose project has a `github.com` origin takes its Tasks from GitHub issues and merges pull requests (a GitHub Factory).
Any other Factory keeps its issues in the core's local issue store as `L-<number>` and merges into the local default branch (a local Factory); a node's Factory keeps them under that node's id, where its project's backlog in the device Overview reads them and a core move carries them (`IssueBook` takes `Factory::node`).
A GitHub Factory acts as the login `gh` is signed in as, and only after a person approved that account, the repository and the list of reads and writes `init` shows; the Factory keeps that approval as `github_approval`, and a Factory without one refuses every GitHub write (`github.approval`) before calling `gh` or pushing.
Closing a Factory keeps its records, and creating a Factory for the same project again brings them back.

| Owner | What it owns |
| --- | --- |
| `hide-factory` crate | Values (`model.rs`), the store (`store.rs`), the dependency graph (`dag.rs`), roles and permissions (`role.rs`), the command contract and its printing (`command.rs`), the judgment features (`judgment.rs`), the read model (`summary.rs`), the adapter traits (`adapters.rs`), the engine (`engine.rs`, with Factory AI, the recovery schedule, follow-ups and GitHub access in `engine/`), the sentences the engine writes in the operator's language (`words.rs`), the one-way move of an older store (`migrate.rs`) and the real git, `gh` and verify adapters (`project.rs`, `exec.rs`), which ask the node of the machine that holds the project to do the work, and the core's own node for `gh` (`hide_node_link::factory`, served by `hide-host/src/factory.rs`). It does not know the runtime or Herdr, and starts no process and reads no project file itself. |
| Host thread in the core | `herdr-core/src/factory.rs`: the engine thread, its request queue, the worker port (spawn, letters, sleep, wake, worktree removal), the judgment thread and the notifier. `runtime/factory.rs` holds the few reads and requests that take the runtime lock for owned data. |
| `hided` | The pane-capability `factory` request in `server.rs` (`ScopedRequest::Factory`) and the `hide factory` parser and printer in `factory_cli.rs`. |
| `hide-kit` | `hide_kit::layout::{factory_store, factory_files}` name the store and its files folder. |
| `hide-agent-hooks` | The session guidance sentence that sends agents to `hide factory add`. |

A command travels as one scoped request.
`hide factory ...` sends a `factory` request over the pane-capability socket, which needs no open renderer, takes a caller on the core's machine or on a node that dials the core, and refuses a caller on a device the core dials with `factory_local_only`, whose next step sends the agent to the machine that runs Hide, because retrying from that device never succeeds.
The request is at most 256 KiB and is answered within 100 seconds.
The core checks the caller as delivery does, so a moved checkout returns `caller_context_changed`, and queues the command for the engine thread.
The queue holds 32 commands; a full queue answers `factory_busy` and a missed answer window answers `factory_timeout`.
The engine decides the caller's role, runs the command, saves the change, and answers.
A worker's `ask`, `block`, `propose` and `done` take the letter path described under [The mailbox recipient](#the-mailbox-recipient-factoryid).

## The store

The source of truth for every Factory on the machine is one SQLite file, `factory.sqlite3`, in the state folder, with a private folder `factory-files/` beside it.
The file runs in WAL mode with `synchronous=NORMAL`, foreign keys on and a two-second busy timeout.
Both are private to the user: the file is mode 0600 and the folder 0700 on Unix.

| Table | Holds |
| --- | --- |
| `factories` | One JSON record per Factory, with its project, closed flag and creation time. The record carries the settings, the Factory's activity log, its recovery holds, the commands a person has to run and the GitHub block. |
| `tasks` | One JSON record per Task, with indexed state, issue, priority and times. The record carries the card, questions, discoveries, decisions, attempts, worker, pull request, gates, the worker's report and the Task's activity log. |
| `dependencies` | One row per dependency edge, rewritten with its Task. |
| `events` | Ids-and-stage records of transitions, external calls and failures, never card text or secrets. |
| `records` | Each judgment's input and output and each letter's body to or from a worker, kept with its Task and cut at 256 KiB (D-58). |
| `meta` | The machine-wide worker limit and the ids of the last 4,096 applied letters. |

`PRAGMA user_version` carries the schema version, now 2.
A store written by a newer build is refused rather than read: the engine does not start, the host logs `store.open_failed`, and commands answer `factory_unavailable`.
Fields added later load from older rows through serde defaults.
A schema 1 store moves forward once, the first time this build opens it, and never back.
The move first keeps a copy of the store beside it as `factory.v1.sqlite3`, which nothing reads again; a copy an earlier failed move left is kept as it is, since that move changed nothing.
It then rewrites every Task and Factory record in one transaction, so a failure leaves the store as it was, the engine does not start and the host logs `store.open_failed`.
Each notice and each environment proposal leaves the questions and becomes a `note` line of the Task's activity, with its words and its time.
An open proposal that names a command a person runs also becomes a command to-do of its Factory, while one that names a recovery action does not, since those actions run by themselves now.
An open card confirmation is confirmed, each unrelated finding becomes an open follow-up candidate, a worker's `done:` summary moves from the decisions to the Task's report and its activity, and the decision that only recorded a notice's acknowledgement is dropped.
Every recovery action is turned on, and each Factory's events record `store.migrated`.
A schema 1 store kept no recovery clock and no GitHub block, so the move writes none: a Task already stopped, a start already held or a GitHub step already refused opens its hold or its block the first time the engine meets that condition after the move, and its clock starts then.
The engine loads the whole store at start and saves each change before the command that made it answers, so a restart resumes Factories, Tasks, questions, workers and in-flight work.
A restart starts verification again from the recorded attempt, closes as cancelled any earlier attempt still unfinished, and asks a pending review again.
A store write that fails does not stop the engine: it is counted, `hide factory status` says how many failed since start, and the host writes each one to the diagnostic log (`store.write_failed`, with the Factory, Task and stage).
A main recovery the restart cut short is not guessed again, since which merge it was finding or reverting lived only in the old process: it goes to a person with the same choices as a recovery that could not decide.
The engine opens the store when a store file already exists at start, or on the first command, so a machine that never created a Factory opens nothing.

`factory-files/<factory id>/<task id>/` holds each attached PRD as `<sha256>.<extension>`, a read-only copy of at most 4 MiB, and `factory-files/logs/` holds one `<run id>.log` per local verification run.
A run's log keeps only its last 1 MiB when the run ends; a run that writes more than 256 MiB is ended and fails as "output over the log cap".

The Factory deletes no Task, question, decision, discovery, attempt or attachment.
It removes a worker's worktree and pane when the Task is done and main verification has passed, when a cancelled Task or one an outside pull request took passes its keep period, and, only under disk pressure, for finished Tasks.
An outside pull request that takes a Task stops its worker and starts the keep period at that moment; the worktree waits the period out even when that pull request merges and the Task is done, because the worker's own work was never merged.
The removal runs from the repository's main checkout through the host's guarded worktree removal.
A finished Task keeps its branch; a cancelled or outside Task loses its local branch with its worktree, and the Factory never deletes a remote branch.
A finished Task's worktree goes only when it is clean: uncommitted or untracked leftovers keep it, and one line of the Task's activity (`cleanup_kept`) names the folder for a person to look at. Only a cancelled or outside Task past its keep period, or a start abandoned on its way, has what is left in its worktree discarded.
A finished Task folds out of the board's Done column after 3 days and out of every list after 90 days; its page still opens.
Each Factory keeps its newest 20,000 events and drops older ones.
Judgment inputs and answers and letter bodies are kept in `records` and never deleted automatically; `Engine::records` reads a Task's, and the core's diagnostic log records each judgment's feature, ids, outcome and duration.

## Tasks, states and the DAG

A Task is the Factory's unit of execution.
It has a Factory-local id `T-<number>` that is never reused, and, once it is Ready, an issue: `#<number>` on GitHub or `L-<number>` locally.
A Task is shown by its issue number when it has one and by its Task id before.
A command names a Task by Task id, issue, or `<factory id>/<task id>`; a reference that matches Tasks of two Factories answers `task_ambiguous`.

The card has a title (one line, at most 200 characters), a goal, at least one completion criterion, out-of-scope items, open decisions, dependencies (`--after`) and external waits (`--external`).
A person sets four more fields on it: direct review, priority (-100 to 100), merge mode and runtime, and an empty field takes the Factory's default.
Text fields are capped at 4,000 characters, lists at 30 items, and a Factory at 5,000 Tasks.
A card that misses a field, has no criterion, names a dependency that does not exist or is cancelled, or closes a loop is refused at once with a `card_invalid` answer that lists each field and problem.
The producer writes the card; the review may add questions, dependencies and flags and never edits the card.

### States

| State | Label | Meaning | Slot | Board column |
| --- | --- | --- | --- | --- |
| `drafting` | 정리 중 | Open questions remain or the review has not answered. | no | drafting |
| `waiting` | 대기 | Ready; waits for predecessors, a slot or the environment. | no | waiting |
| `running` | 실행 중 | A worker holds a slot. | yes | running |
| `paused` | 일시정지 | A person paused it, or closed its worker's pane in Hide; the worker sleeps. | no | running |
| `blocked` | 막힘 | Waits for an answer or a predecessor; the worker sleeps. | no | running |
| `verifying` | 검증 중 | The worker reported done; checks, verification and the pre-merge steps run. | no | running |
| `merge_waiting` | 머지 대기 | A person merges, requests changes or cancels. | no | running |
| `landed` | 머지됨 | Merged; main verification has not answered yet. | no | running |
| `done` | 완료 | Main verification passed or none exists, or an outside pull request merged. | no | done |
| `stopped` | 멈춤 | The worker went quiet, a start was refused, or a limit was reached. | no | running |
| `relanding` | 다시 올리기 | The merge was reverted; the Task runs again on the latest main. | yes | running |
| `outside` | 밖에서 진행 중 | An outside pull request closes its issue. | no | running |
| `cancelled` | 취소 | Kept for the keep period, then purged. | no | off the board |

The main line is `drafting`, `waiting`, `running`, `verifying`, `merge_waiting` when a person merges, `landed`, `done`.
`blocked`, `paused` and `stopped` branch off `running`, and `relanding` returns to `verifying` through the worker.
A Task stops with one of these reasons: no report, stalled, verification failed three times, new-Task cap, the same environment failure repeated, a refused worker start, a refused publication, or a worker that disappeared again after its automatic restart.

A person's actions depend on the state, and any other action is refused with `action_not_allowed_in_state`, the current state and the allowed list.
An open question adds `answer` to any state's list.

| State | Actions |
| --- | --- |
| `drafting` | `edit` (add again), `cancel` |
| `waiting` | `priority`, `dep-remove`, `cancel` |
| `running` | `pause`, `cancel` |
| `paused` | `resume`, `cancel` |
| `blocked` | `answer` only |
| `verifying` | `cancel` |
| `merge_waiting` | `merge`, `request-changes`, `cancel` |
| `stopped` | `retry`, `cancel` |
| `outside`, and `cancelled` inside its keep period | `revive` |
| `landed`, `done`, `relanding`, and `cancelled` after the keep period | none |

### When a Task may start

A `waiting` or `relanding` Task starts when all of these hold.
Its Factory is open and not paused (see [Factory AI](#factory-ai-the-observer), Pausing a Factory).
Every Task in its `depends_on` is `landed` or `done`: a predecessor must be merged, and a missing one counts as unmerged.
The environment holds no start (see [Environment](#environment)).
A slot is free.
External waits are shown on the Task and never decide a start.
A Task that waits holds back only the Tasks below it in the chain; every other Task keeps running.

Slots are counted per machine, across every Factory.
The machine-wide limit is `max_workers`, 5 by default and between 1 and 64, and it counts Tasks in `running` or `relanding` plus workers still starting.
Free slots go first to `relanding` Tasks, which already hold theirs, then by priority from high to low, then by the older creation time, then by Task number.
A priority change or a removed dependency applies at the next tick.
A person's dependency removal is the only way to loosen an order, and a worker may only add a dependency to its own Task.
Adding a dependency to a running Task whose predecessor is unmerged puts the Task to sleep as `blocked`; it waits for a slot again when the predecessor merges.

## Roles

Each command runs with a role the engine decides from its own store, never from the caller's claim or a file.
A caller is a worker when its pane is the pane of a Task's worker, or when its working folder is inside a Task's worktree.
A `cancelled` or `done` Task's worker stays a worker while its worktree and pane stay, so its pane never gains an operator's commands, and the Task's state refuses its reports; a live Task wins when a pane or folder was reused.
Once a Task's worktree is removed it binds no pane or folder, because Herdr can give a closed pane id to the operator's next pane.
An agent a worker spawned acts as that worker: the host follows the caller's spawn lineage in the delivery ledger (up to 16 agents up) and binds an ancestor by its agent id or by the pane it was registered on, and another pane a pane-bound caller names besides its own only ever makes it a worker, never an operator.
A lineage the host cannot read to its root (the ledger unreadable, a missing record, a loop, more than 16 agents) may hide a worker above, so that caller can only read; anything else is refused with `lineage_unknown`.
The walk starts from the newest record on the caller's pane even when that record has ended, because an agent can end its own record and keep running, and from that same agent's newest record that names a parent when it registered again without one; an ended ancestor lends no pane to bind by, because Herdr reuses pane ids. A record ends through `hide agent end`, and by itself when Herdr no longer has its pane ([delivery.md](delivery.md#agent-registration-and-spawning)), so an ancestor whose pane closed lends no pane id from then on; until the core has read that host's panes again it still does. A caller below a Factory's own agent that no Task holds as its worker (a start abandoned on its way, or a shell on a reused pane id) can only read; anything else is refused with `factory_agent_unbound`.
Every other caller is an operator, recorded by pane id, or `checkout` when the caller has no pane.
A checkout-bound caller (a plain terminal, or a tool shell in Codex's shared app-server daemon) is the operator acting without a pane: the pane it names is never read as its identity or lineage, and only its checkout folder can make it a worker.
The engine itself acts as a third role for deadlines and timers and is never a command caller.
A capability file holds only a token, so editing it cannot change a role.

| Permission | Commands | Worker | Operator |
| --- | --- | --- | --- |
| Read | `status`, `show`, `inbox`, `config` without `--set` | yes | yes |
| Report | `ask`, `block`, `propose`, `done`, `decide` | its own Task | refused |
| Add a dependency | `dep add` | its own Task | yes |
| Intake | `init`, `add`, `check`, `follow-up` | refused | yes |
| Answer | `answer`, `resolve` | refused | yes |
| Loosen | `dep remove`, `priority`, `worker` | refused | yes |
| Merge | `merge`, `request-changes` | refused | yes |
| Control | `pause`, `resume` (a Task or `--factory`), `retry`, `cancel`, `revive`, `close` | refused | yes |
| Configure | `config --set` | refused | yes |

A refusal answers `role_not_allowed` with the role and the verb.
A Task keeps at most 500 questions, decisions and discoveries together; past that a worker's report is refused with `report_limit`, except `done`, which still finishes the Task without storing its report, and `block`, which still stops it for a person.
A worker's `show` reaches its own Factory only.
An operator relays a person's words, and every answer and decision records the relaying pane as `relayed_by`.
Code cannot tell whether an operator pane's agent acted on a person's words, so it records who relayed and does not block.

## The mailbox recipient `factory:<id>`

Each open Factory is a code-owned recipient named `factory:<factory id>` in the delivery ledger.
[delivery.md](delivery.md#the-code-owned-recipient-factoryid) owns the ledger rules; this is how the Factory uses them.

- Only the engine sends as the Factory. The host holds that authority, and the ledger accepts it only while the Factory exists and is open.
- A pane may send to `factory:<id>` and may name it as a watch observer, and it can address the Factory by that name only. A pane or agent can never register under a name or pane that starts with `factory:`.
- The engine reads its letters from the ledger on each tick, up to 16 at a time, applies each once, and confirms it. A request or block letter also gets a reply carrying the engine's JSON answer. No composer input or doorbell is involved.
- The letter's sender must be a live worker of that Factory, or the letter is refused with `sender_not_a_worker` and logged.
- A worker's CLI report is a typed body `{"factory": <command>}` sent under an intent made from the pane, the Task's current state time and the body, so a retry in the same state applies once and the same report after the Task moved (a `done` again once verification sent it back) is a new letter. A harness that follows the letter protocol only is read as a question (a `request` or `block` letter becomes a blocking question with a 24 hour deadline, taking a `Recommendation:`, `Suggestion:`, `추천:` or `제안:` line as the suggestion) or as a completion (a `report` letter becomes `done`, its first line the result and its whole body kept as the report's raw text); a `progress` letter is refused `letter_invalid`, since the Factory reads no interim report.
- A watch whose observer is the Factory warns after the Factory's `stall_minutes` (30 by default) instead of 20; the host hands each open Factory's window to the watch readings, so a change applies at the next reading. The engine turns the first warning into a stalled stop; it does not wait for the second warning or for the human notice, which the ledger skips for a code-owned recipient.
- The Factory writes to a worker with `report` letters, and starts a watch on the worker again each time it does, because a report to its parent ends the sender's watch.

## The worker lifecycle

A worker starts when the Task may start.
The engine asks the core to spawn it through coordination, the same path as `hide agent spawn`, under the Factory's own agent record as parent.
The spawn runs on the host's starter thread, one at a time with at most 16 waiting, because it waits for Herdr to make the worktree and start the agent: the engine keeps the Task's slot, answers commands and worker reports meanwhile, and takes the started worker on the next tick.
A Task cancelled or paused while its worker starts on the starter has that worker ended when it arrives, and a worktree that start made removed; its next start is a new attempt with its own spawn intent.
For Pi and omp, the agents a Factory puts to sleep by closing their pane, this abandoned-start rollback closes only the execution that start created; a resumed start keeps its existing worktree, branch, conversation and other panes.
The unclaimed start retains its original connection generation and file node only until it is claimed or released.
Rollback rechecks the created native session and terminal before and after opening the actual close connection, and refuses a replaced connection or execution before closing or deleting files.
A refused rollback preserves the worktree and branch and records a cleanup failure; it never redirects the old pane ID to the replacement server.
A refused close reports a cleanup failure and preserves the files rather than deleting a folder whose pane may still be running.
A Task cancelled while its agent had not shown its session yet keeps its attempt, so a revive asks the same spawn again and gets the pane and worktree it already made.
A Task relanding after a revert keeps its restart while it waits.
A restart of a gone worker waits on the starter the same way and holds its slot meanwhile, and a spawn lock another spawn holds is asked again in two seconds.
When the host stops, starts still queued are dropped, not run.
The record is registered when needed and converges on the existing one.

- **Name, branch, intent.** The worker is named `factory-<project name>-<task id>` and works on `factory/<issue number>-<slug>` (`factory/l<number>-<slug>` for a local issue, `factory/<task id>` without a slug). The spawn intent is `factory-<factory id>-<task id>`, and each earlier refused start adds `-a<n>`, so a refused start never repeats a spawn that already failed and a retry of the same attempt converges.
- **Worker candidate.** The Factory keeps one to five worker candidates, each an agent, an optional model and effort, and a line saying when to use it; the first is the default. The candidate is, in order, the one a person pinned (`hide factory worker <task> <n>`, or `add --worker <n>`), the one the intake review picked, or the default. A new start whose candidate's agent is at its usage limit takes the next candidate in the list that is not, while a pinned candidate and a resuming worker wait for the reset. A retry, an automatic restart and a resume keep the candidate that started. The model and effort go to the agent only where its adapter declares that start argument, as the spawn's launch choice, so a wake, a fork or a reopen of the worker starts it with them again (`docs/ARCHITECTURE.md`, Agent start); the arguments in `worker_args` for that agent go before the prompt.
  A worker uses only what its agent's adapter declares (D-28): every agent but OpenCode takes a model, and only Claude Code and Codex an effort; Factory sleep follows the sleep the adapter declares, so Claude Code and Codex sleep in their pane and Pi and omp close it and wake in a fresh one; a Grok, OpenCode or Cursor worker keeps working through a pause until a task of its own binds its fresh pane the same way.
  Claude Code, Codex, OpenCode, Pi and omp hear a letter at their next prompt, OpenCode, Pi and omp through Hide's plugin or extension, while a Grok or Cursor worker hears one only by reading its inbox; a worker of any agent but Claude Code and Codex is diagnosed from its screen when it goes quiet.
  An old Factory's `default_runtime` reads as one default candidate on the CLI's defaults until a person changes the list, and `add --runtime <agent>` pins that agent's first candidate.
- **First prompt.** One argument behind the flag the agent's start declares (`--`, or `--prompt` for OpenCode), at most 6 KiB, cut at a character boundary with a pointer to `hide factory show <task>` for the rest. It holds the Task, goal, criteria, out-of-scope items, the read-only absolute path of each attachment, the Task's decision record (what the review assumed and how each question was answered), which the worker follows and whose checks it finishes before it reports, the harness instruction when one is set, and the reporting rules: commit on the branch, never push or merge to the default branch, finish with `hide factory done` and its four parts, never ask the person on screen but send every question through `ask` (with a default) or `block`, each with up to five choices of at most 120 characters, report discoveries, and a turn that ends without a report stops the Task. The prompt text is written in Korean, and a rule after the reporting rules names the operator's language (see [The operator's language](#the-operators-language)) as the one every report a person reads is written in. When a `hide` program sits beside the daemon, the prompt names its absolute path and says every `hide` in the rules means that program, so a worker never reaches an older copy on its `PATH`.
- **Lineage and watch.** The spawn writes lineage and starts a watch whose observer is the Factory.

A worker whose pane exists but whose agent has not shown a session yet is still starting.
It holds its slot, the Task stays `waiting`, and the same spawn is asked again.
Each ask itself waits up to 5 seconds for the session, so a young start, in its first 30 seconds, is asked again on the next tick and the worker is accepted that soon after its agent shows; a start older than that is more likely waiting on a trust or login prompt and is asked every 30 seconds.
After 10 minutes the Task gets a `start` to-do in 결정 필요 asking the person to look at the pane, where a trust or login prompt may be waiting; its button is `hide factory resolve start:<task>`, which asks the same spawn again at once and puts the to-do away; it comes back after another 10 minutes without a session, and goes away for good when the worker shows one.
The starter thread logs why a start has no accepted worker yet as `worker.start_unfinished` (the stage and the reason, such as `native_identity_unavailable`, once per reason), `worker.start_refused` for a start the runtime refused, and `worker.start_accepted` with `waited_ms` once an unfinished start succeeds; a start that works the first time logs nothing.
A woken worker whose letters are still held logs `worker.wake_waiting` with its reason (`agent_absent`, `agent_asleep` or the delivery's reason), once per reason, and `worker.wake_delivered` when they go out.
A Codex worker waits for this Mac's install kit to have read the machine since launch: the first start asks the kit to read and reports the worker as still starting.

A start the runtime refuses, other than for an environment signal, stops the Task with the reason "worker start failed" and the runtime's reason (cut at 300 characters) beside it.
A person fixes the cause and retries; the next attempt uses a new spawn intent, and its spawn closes the pane an earlier attempt left where its agent never started, so a retry leaves no extra tab and Herdr no longer holds the worker's name for it (`docs/delivery.md`).
A start Herdr typed whose agent never showed is not asked again as the same spawn: the next ask stops the Task with `agent_not_started`.
A start that fails with an environment signal leaves the Task `waiting` and is handled by the environment rules.

A Task's worker is put to sleep when the Task blocks, pauses, goes to `verifying`, or waits for a slot, and woken when it runs again.
Sleep goes through agent sleep and is deferred until the agent's turn ends; a worker that never slept is sent the message instead, and a worker whose agent a Factory does not sleep is never counted asleep.
Stopping a worker, when its Task is cancelled or an outside pull request takes it over, ends its ledger record at once and puts its agent to sleep the same way, so its session stays for `revive`; an agent a Factory does not sleep is left awake in its pane.
Claude Code and Codex sleep in their pane, and a wake restarts the agent in the same pane and session.

Pi and omp close their pane when they sleep and wake in a fresh one (`docs/ARCHITECTURE.md`, Agent sleep).
The core binds the Factory's worker to that sleep when it saves it: the dormant record carries the Factory, the worker's agent record, name and worktree.
Hide's own close of that pane is not read as the operator closing it, so the Task is not paused with `PaneClosed`, and the worker is not counted `gone`, which would start a new conversation.
Until the close lands the worker reads as asleep, and a wake asked meanwhile is asked again on the next tick.
A wake resumes the saved conversation in a tab of its own, and the new pane is found only through that record, never by its label, folder, name or native session id alone.
Once the agent in the new pane shows the saved native session, the host binds the worker there before it delivers anything.
The registration the worker held is moved to the new pane by the delivery store with its letters and watch (`docs/delivery.md`), so the worker keeps its agent id.
Only when that registration was ended while it slept does the host end the old one, register the Factory as parent again and register the new pane under the worker's name and worktree.
The watch starts on the new pane either way.
The Task's worker takes the new pane and registration in one engine step that records `worker.rebound`, so a pane Herdr reuses for another agent cannot be taken for it.
The letters held for the worker follow it to the new pane, and none wait in front of the pane that closed.
The record stays `Woken` until the engine has saved the new pane, and a record no one binds within an hour is dropped with the `agent_sleep.woken_unbound` diagnostic.
A restart in between binds the pane from the record.
A wake the core could not finish stops the Task once, as "worker start failed" with the reason, for a person.
A worker whose original pane has a dormant record is never started again as a new conversation; the wake brings its own back.
Letters for a woken worker are held, at most 16, until its agent is back, and are dropped to the diagnostic log after 10 minutes.
A retried Task spawns again in the same worktree and session with a fresh intent.
A worker that is `gone` is detected from its pane, with a three-minute grace after its start; a pane Hide did not close starts again once in the same worktree, counted as an automatic restart once that start begins, and a second disappearance stops the Task as "worker gone" for a person, whose start sets the count back to 0.
A pane Hide is closing is never read as gone, since its close pauses the Task.
A pane the operator closes in Hide pauses its Task (`pause_reason` `pane_closed`) and nothing starts it again until a person resumes it, which continues the session in the same worktree.

A worker reports through `hide factory` and gets its answer in the same call.
`ask` needs a suggestion, a default action and a deadline of 1 to 720 hours, and the worker continues with the default; `block` has no default, so the Task blocks, releases its slot and sleeps.
`done` takes the report in four parts: a one-line result, and lists of what changed, what was verified and what could not be verified.
Without a result it is refused with `result_required`, and the retired one-line `--summary` is refused with `summary_replaced`, both naming the four parts.
The report is kept on the Task (`report`) and as a `report` line of its activity, never as a decision.
`done` answers at once, goes to `verifying`, and puts the worker to sleep.
On a GitHub Factory the next tick pushes the worktree's commits to the Task branch (with a lease, so a rebase goes through), then finds the open pull request for that branch or opens one, and only then reads CI on the pushed commit; every report pushes, so a fix after a failed check reaches the same pull request, and a merged pull request from before a revert is never reused.
The pull request's body carries the issue text, then the worker's report and the Task's decision record, each in a code block, so a closing keyword a worker wrote there stays text.
A Task branch the remote deleted, as automatic branch deletion does after a merge, is pushed again rather than refused by a stale lease.
Only a pull request from the repository's own branch is a Task's or a revert's; a fork's pull request on the same branch name is never adopted.
A push or pull request refused twice in a row with no environment signal in between (a protected branch, a hook; git's transport errors such as a 5xx answer or a timeout count as the network's, while a missing key, a gone repository or a hung-up remote reach a person) stops the Task as "push 거절됨" with the reason; a person fixes the cause and retries. One with an environment signal is asked again every minute.
A failed push or pull request is tried again a minute later.
`decide` records a decision.
A worker whose agent rests for the no-report window (`no_report_minutes`, 2) after a turn that reported nothing is woken once, then diagnosed once, and only then stops the Task as "no report" (see [Factory AI](#factory-ai-the-observer), A quiet worker); a worker that goes quiet for the stall window (`stall_minutes`, 30) stops it as "stalled".
The rest is the one rest start the core saw for the worker's agent, and an agent whose state is unknown is neither woken nor caught as quiet.

## Intake review and judgments

`hide factory add` checks the card's shape first, in milliseconds, then reads what the repository and GitHub say about the card, then asks for an independent review.
The facts are the start of each repository file the card names that git tracks (at most 8, each cut to 4 KiB; an ignored or untracked file, such as a `.env` or `.git/config`, is never read, since the card may come from an issue anyone wrote) and, on GitHub, the issues and pull requests that look related to its title; what cannot be read is left out, and GitHub is not asked while the Factory's access is blocked.
The review sees only the card, its attachment, the repository's file names and guide, those facts and the Factory's other Tasks, never the producer's conversation.
`add` waits up to 90 seconds and answers one of:

- `ready`: no question is open, so the Task is Ready.
- `needs_answers`: the questions to answer, each with a suggestion.
- `split`: the review proposes two or more pieces, each with a goal, criteria and the earlier pieces it follows.
- `pending`: the review has not finished, or could not run; the result goes later to the pane that added the Task when that pane still exists.

Adding again with `--task <id>` or the same issue updates the same Task and reviews it again; the same content changes nothing.
A re-add on a Task that already runs does not rewrite its card: it adds a scope-change question that a person approves or rejects.
A re-add adds dependencies but never drops one the review or a person added, so a producer that resends the card it knows changes nothing; removing a dependency is a person's `dep remove`.
A re-add that leaves out criteria, out-of-scope items, open decisions or external waits keeps the card's.
`dep add` names a Task of the same Factory as the Task it changes, and a watch warning names a Task of the Factory it watched; a warning that names none is kept as a line of the Factory's activity without a Task.
The review completes the card instead of asking.
It checks what a file, an issue or a pull request settles before it raises anything.
A card with no completion criteria gets the criteria the review wrote, and a card with no out-of-scope items gets the review's; criteria and items a person, the producer or the issue's checkboxes gave stay as they are, and the goal is never rewritten.
What the review could not confirm and could decide inside the card's scope, it records as an assumption: a decision of Factory AI with its reason (source `assumption`) that a person can change on the Task page.
Only a permission or a product judgment the card leaves open stays a question.
The Task's activity gets one `intake` line, with the number of criteria and assumptions and whether a label brought the card, and one `ai_decision` line for each assumption.
A review adds at most 12 criteria, 12 out-of-scope items, 12 assumptions and 5 questions, and a review run again for an edited card does not record an assumption the Task already has.
The producer's open decisions become intake questions, every question the review raises goes to Factory AI first (see [Factory AI](#factory-ai-the-observer)), and a Task is Ready only when the review is done and no question is open.
Ready creates the issue: a GitHub Task gets an issue labelled `factory` whose body carries a hidden Task marker, the goal, the criteria and the out-of-scope items, and a local Task gets an `L-<number>`.
A Task added from an existing issue gets the label instead.
A label starts a Task without a card confirmation: the review completes the card from the issue, its checkboxes become the criteria, the issue body is not edited, and the Task waits for a slot when nothing is left open.
The marker lets a retry find the issue it already created.
A split that a person accepts turns the original Task into the first piece, keeping its issue, and drafts the rest as new Tasks that depend on the pieces they follow; choosing to proceed clears the split and the Task becomes Ready when nothing else is open.
A Task is never split or made Ready by silence.

Every judgment is a tool-less, one-shot call whose input the code bundles and cuts to 48 KiB, whose answer must match a `factory.v1` schema, and whose deadline is 180 seconds.

| Feature id | Asked | Input | The answer may |
| --- | --- | --- | --- |
| `factory_intake_review` | On add and re-add, an edited issue body, a label-path card, a split piece, an approved proposal | The card, its attachment (24 KiB), up to 100 other Tasks, up to 400 file names, the repository guide (8 KiB), the files the card names and the related issues and pull requests, the description of an autonomy scope the Task claims, and the worker candidates when there is more than one | Complete the criteria and out-of-scope items a card lacks, record assumptions, add questions, dependencies on listed Tasks, a split, and flags, say whether the card fits the claimed scope, and pick a worker candidate with a reason |
| `factory_drift` | After `done` | The card, the diff against main (24 KiB), the decisions | Pass, send the work back with what to fix, or add questions, with flags and a verdict for each completion criterion |
| `factory_check` | At intake or after `done` for each natural-language check | The instruction, the card, the diff, the decisions | The same as drift |
| `factory_watch` | See [The watch](#the-watch) | The Factory's board summary | Warnings, each with an action of the closed recovery list or none |
| `factory_env_diagnosis` | See [The recovery schedule](#the-recovery-schedule) | The collected facts of one hold, what was tried and the actions that are on | One of those actions, or an exact command with its impact |
| `factory_observer` | See [Factory AI](#factory-ai-the-observer) | The request or the worker's text, the card, its recorded decisions (50) and its PRD | Sort a request into a kind with an answer or a fix, and for a person what it holds up and where each choice leads, read a quiet worker, or approve a risk-path merge |

### The operator's language

Every text the Factory writes for a person is in the operator's language: the language Hide's interface is set to, which the host reads when a judgment is queued or a worker starts, so a change in Settings applies to the next one.
It is the language every Hide AI text follows, `Runtime::output_language` ([AI_PROVIDERS.md](AI_PROVIDERS.md#the-output-language)): the core's explicit choice (`ui_state.interface_language`, English for a stored value that is invalid), else the core machine's primary language resolved as a shell resolves it ([LOCALIZATION.md](LOCALIZATION.md)), else English, with `language.system_fallback` in the diagnostic log.
Each judgment carries it (`Judgment.language`) and its request names it (`AiRequest.output_language`), so the Hide AI system prompt asks for every text a person reads in that language whatever the input's language; the judgment's own instructions name no language.
The worker's first prompt asks for every report (a `done` summary, an `ask` or `block`, a `propose` or `decide` text) in it.
The questions the engine asks itself (a review that could not run, a verification that failed too often, a repeated environment failure, the new-Task cap, an autonomy scope the review did not see fit, a split, an edited issue, a changed card, a scope change, a proposed Task, a main fix and a main recovery that stopped) are composed in it too, in English, Korean, Simplified Chinese or Japanese (`hide-factory/src/words.rs`).
The engine's other fixed sentences, such as the replies it sends a worker, are not composed there.

Questions that a drift or check judgment adds always carry a default action, taking the suggestion when the answer has none, so a check can only slow a Task down.
A drift question keeps the Task in `verifying`, does not wake the worker, and holds auto merge until it is answered or its deadline passes; an answer that differs from the default wakes the worker to apply it.
A drift or check judgment answers one of three: pass, send back, or questions.
A send-back stops the verification in flight, records Factory AI's fix as a decision (source `send_back`) and a `sent_back` line of the Task's activity, counts as a verification failure toward the limit (see [Verification](#verification)), and wakes the worker with the fix and the count; at the limit the Task stops as "verification failed three times" and asks a person to retry or cancel.
A send-back that answers after the Task left `verifying` changes nothing.
Each answer may judge every completion criterion as `met`, `unmet` or `unknown` with a reason, naming each by its place in the numbered criteria list it is given (at most 30), which the Task page shows as its checklist; a place past the list or one named twice makes the answer unreadable.
A verdict keeps the sentence it judged and is shown only beside that sentence in that place, so a card changed while or after a check ran shows the changed criteria without a verdict until the next check.
A judgment that cannot run is never skipped and never read as a pass.
A failed review marks the Task and raises an action question with the choices `retry-review`, `start-as-is` and `cancel`, and `add` answers `pending`.
A review refused as `disabled` asks with two choices instead, `enable-ai` and `start-as-is`, because a retry or a provider fix cannot run while Hide AI is off.
`start-as-is` starts the Task from its card as written, for a label the issue body and its checkboxes, without a review.
While Hide AI is off or no agent is chosen to run it, every judgment fails as `disabled`, and the question says to turn it on in Settings › Hide AI or to start from the issue as written.
While Hide AI is still off, picking `enable-ai` on the screen opens Settings › Hide AI and holds the answer, and the screen sends it once Hide AI is on; sent, `enable-ai` runs the review again as `retry-review` does.
A failed drift or check sends the Task to `merge_waiting` for a person, and so does a check whose answer cannot be read, or one that cannot read the Task's diff.
A judgment that answers after its Task was cancelled, taken outside or finished is dropped and logged as `judgment.dropped`; a request it was sorting and that is still open goes to a person, so a revived Task shows it.
A failed watch or diagnosis changes no Task and is logged.

The Factory has its own judgment queue on its own router (see [AI_PROVIDERS.md](AI_PROVIDERS.md#the-factorys-judgments)): one request in flight, intake reviews before every other judgment, and 16 waiting judgments per Factory.
A judgment submitted to a full queue fails like a provider failure and is escalated the way above.
Natural-language checks are added with `hide factory check --at intake|after-done|periodic`.
A `periodic` check runs over each running Task's card whenever the watch interval comes due; it adds questions or marks, or writes the fix of a send-back to the running worker without counting a failure, it never holds a merge, and one that cannot be queued is logged and asked again at the next interval.

## Factory AI (the Observer)

Factory AI is the Factory's one judgment that answers in a person's place.
It runs as the `factory_observer` judgment on the same queue and Hide AI as the others, and code, never the AI, decides who answers (`hide-factory/src/engine/observer.rs`).
Every Observer failure sends the request to a person with its reason in the log, and the first recorded answer wins: a person who answers while the Observer is still judging settles the question, and the Observer's later verdict is logged as `observer.late` and changes nothing.

**Decision requests.**
Every question the Factory would put to a person goes to Factory AI first, whatever raised it: a worker's `ask` and `block`, a question of the intake review and a producer's open decision, a check's question, a scope change, a proposed Task, the new-Task cap, a failed review, a stop that needs an action and the question after a main recovery stopped.
A worker's request carries at most five choices of at most 120 characters (`too_many_choices`, `choice_too_long`).
The Observer sees the request, the Task's card, its last 50 recorded decisions and its PRD, and sorts it into one of five kinds with an answer, and for a wrong card a fix.
Its prompt tells it that retrying work that already failed, letting a Task create more Tasks, cancelling, reverting and merging are permissions, so they sort as D.
While it judges, the question is not yet a person's: the inbox does not list it and it counts in no number.
The Factory's mode (`observer_mode`) then decides where the answer comes from:

| Kind | Meaning | 직접 (`manual`) | 함께 (`assist`) | 맡김 (`autonomous`) |
| --- | --- | --- | --- | --- |
| A | The answer is already there | Factory AI | Factory AI | Factory AI |
| B | A technical choice | A person | Factory AI | Factory AI |
| C | A product or taste choice | A person | A person | Factory AI |
| D | A permission: cost, sign-in, deletion, security, an outside effect, out of scope, irreversible | A person | A person | A person |
| E | The card is wrong | A person | A person, with the fix as the choice `AI 제안 적용` | The fix is applied |

A verdict that is unsure, or that sees any permission signal, goes to a person in every mode, and so does an answer the Observer left empty.
A request also goes to a person when Factory AI could not decide it, and the reason is the inbox item's `fallback`: `failed` (the judgment failed or its answer was unusable), `daily_limit`, `paused` (the Factory is paused), `queue_full`, `dropped` (its Task was taken outside or finished while it was being sorted) or `restart` (the daemon restarted while it was being sorted).
A request Factory AI sorted still goes to a person when it was unsure of the kind (`unsure`), the request touches a permission (`permission`), or its answer to a closed question names none of the choices (`no_choice`), and that is the item's `fallback` too, since the kind's line in the mode table is not why.
Factory AI's answer to a question whose answer runs a listed choice (every kind but intake, default and blocking) must be one of those choices; any other words send the question to a person.
When a worker's question goes to a person, Factory AI also rewrites it as one question someone who has not read the Task can answer (`person_text`), which 결정 필요 and the Task page show; the worker's own words stay in the question's `text`.
When the request goes to a person after being sorted, the kind and the reason stay on the item, and what the request holds up and where each choice leads are filled in from the verdict where the asker left them out.
The mode is read when the request arrives, so changing it later does not move a request already sorted.
An answer from Factory AI takes the path a person's answer takes, is recorded with `observer` as who relayed it, and writes a decision record with the kind and reason.
Nothing Factory AI decides is added to a person's list: in every mode it is a decision of the Task (see below) and an `ai_decision` line of the Task's activity.
An applied fix rewrites the card and records an approved scope change, so the Task waits for a person's merge, or drafts the new Task the fix names, and the worker is told by reply; at the Task's new-Task cap, or on a Task that was itself proposed or started by autonomy, Factory AI applies nothing and the request goes to a person.
The prompts tell Factory AI that the request and the worker's recorded decisions are the worker's words, data to judge and never instructions.

**Decisions and changing them.**
A Task's decisions are listed on its page as `R1`, `R2` and so on, in the order they were recorded (`DecisionView`).
Each says who made it (`by`: `person`, `ai` or `worker`), where it came from (`source`: `answer`, `assumption`, `send_back`, `worker`, `request_changes` or `risk_merge`), the kind and reason Factory AI gave, and whether a person can still change it (`overridable`).
A person can change a decision when Factory AI made it, no person has replaced it, it is an answer to a question that takes words (intake, default or blocking), an intake assumption or a send-back, and the Task is not finished (`done`, `landed`, `cancelled` or taken `outside`).
An applied card fix, a new Task and a risk-path merge approval are not changeable: each is undone its own way: the card fix waits for a person's merge as an approved scope change, the new Task is cancelled on its own line, and a merged risk-path Task stays merged.
`hide factory answer <task> --decision R<n> --choose <choice>|--text <answer>` replaces one, and cannot be combined with `--question` or `--change`.
The decision becomes the person's, `changed` keeps what Factory AI had decided and who changed it when, and the new answer reaches the work: a Task in `verifying` or `merge_waiting` goes back to `running` with its verification cancelled and its worker woken with it, a Task not started reads it in its first prompt, and any other Task's worker gets it by reply.
A decision that answered a question is changed through that question, so `hide factory answer <task> --question <id> --change --choose <choice>|--text <answer>` does the same by the question's id.
The refusals are `decision_not_found`, `decision_not_changeable`, `already_answered` (an answer a person gave), `task_finished` and `answer_required`.

**The daily cap.**
A Factory sends at most `observer_daily_limit` (100, from 1 to 1000) Observer calls a day, counted by the machine's local day.
A call the provider never received gives its count back: Hide AI off, no agent to run it, a full judgment queue, an unsupported request, or a provider that refused it before any model turn (not logged in or at its usage limit); a paused Factory sends and charges nothing.
A transient failure, a spent Hide AI budget and an unavailable provider count, since each can follow a turn: Hide AI reports a provider past its restart cap as unavailable after the turn answered.
At the cap nothing is sent and the request goes to a person with the fallback `daily_limit`.
The first refusal of the day adds one `daily_limit` line to the Factory's activity and sets `observer_capped` on the Factory's view for the rest of that day.

**A quiet worker.**
A worker whose agent rests for `no_report_minutes` after a turn with no report is woken once, through a next-prompt letter, else a resume of the same session with a note to report, and never by typing into its pane.
When the same window passes again with no report, the Observer reads one text from the worker, the first its adapter declares and has: the user's turn, the last answer, then the screen.
The Task page names that text (`diagnosed_from`).
The diagnosis answers one of three:

- **A question.** The engine raises the blocking question the worker was asking, the worker sleeps, and the question is sorted like any request.
- **Forgot `done`.** The worker is asked once to report `done`, and if it still does not, the Task stops.
- **Stopped.** The Task stops as "no report" with the diagnosis as one line under it.

A worker that reports in the meantime ends the sequence, and an agent that declares neither a next-prompt letter nor a resume is diagnosed at once.
With Hide AI off or at the daily cap there is no diagnosis and the Task stops as "no report", which the recovery schedule then works on (see [The recovery schedule](#the-recovery-schedule)).
A paused Factory does not watch its workers' rests at all; a diagnosis that answers after the pause is set aside, and resuming reads each rest from the start.
The wake and the diagnosis stay on the stopped Task for its page, and a retry, a resume or a new start clears them.

**A vanished worker.**
A worker pane that disappears without Hide closing it starts again once in the same worktree and session (`auto_restarts`), and a second disappearance stops the Task as "worker gone".
A restart that waits for its agent's usage reset spends nothing until it starts.
A pane the operator closes in Hide pauses the Task with `pause_reason` `pane_closed`, which lists it in the inbox with `resume` and `cancel`; nothing starts it again until a person resumes it.

**A risk-path merge.**
In a 맡김 Factory, a verified Task whose only gate is a risk path asks Factory AI once per attempt whether it may merge, never while main is broken.
An approval merges through the path `hide factory merge` takes, with the pre-merge check run again and the head pinned, records the approval in the operator's language with the reason as a decision (source `risk_merge`) and an `ai_decision` line of the Task's activity; a refusal or a failure leaves the Task in `merge_waiting` for a person, and so does an approval that answers after the Factory left 맡김, closed or saw main break, and an approval of an earlier verification than the one the Task waits on now.
In 직접 and 함께 a risk path always waits for a person.

**Pausing a Factory.**
`hide factory pause --factory` stops the Factory's starts, judgments and auto merges, and asks each running worker to sleep, which it does when its current turn ends, while a worker whose agent cannot sleep keeps working and the answer names its Task; a request that arrives meanwhile goes to a person.
A Pi or omp worker sleeps by closing its pane and wakes in a fresh one the same way as any other, and the resume binds it there; a Grok, OpenCode or Cursor worker keeps working through the pause.
A worker that reports `done` in that last turn is verified as usual, but its checks wait for the resume rather than failing.
A Task verified while paused that a person must merge, by its mode, an open question or a gate such as a failed check or a risk path, goes through its usual merge checks to `merge_waiting`, where `hide factory merge` takes it; one that would merge on its own waits for the resume, and nothing toward its merge is read meanwhile.
A Task sent back to its worker while paused, by a conflict with main, a failed check or a person, goes back to `running` with its worker asleep where its agent can sleep, and the worker hears why on resume, as it hears every answer given during the pause.
A Factory AI verdict asked before the pause still lands: an answer reaches the sleeping worker on resume, while a risk-path approval merges nothing and is asked again on resume, and a diagnosis is set aside.
`hide factory resume --factory` wakes each sleeping worker with what was answered meanwhile, and sends a running Task's worker that could not sleep what was held for it, reviews the cards that arrived, runs the checks that waited, and asks again about a verified Task held only by a risk path unless main is broken.

**The Factory AI and the workers.**
`factory_ai` chooses the agent, and `factory_ai_model` and `factory_ai_effort` its model and effort, that run every judgment of this Factory; unset, the Factory uses the agent Settings › Hide AI chose.
A stored choice naming an agent Hide AI no longer knows is never replaced by the app's choice: the judgment fails as having no provider, its count is given back, and the request goes to a person.
A choice is checked when it is set: the agent Hide AI knows, a well-formed model, an effort that agent declares, and the agent ready to answer (`factory_ai_unavailable` with the reason otherwise).
`workers` is the list of worker candidates described under [The worker lifecycle](#the-worker-lifecycle); the intake review picks one with a reason when there is more than one, and `hide factory worker <task> <n>` pins one, or `auto` returns the choice to the review.
A worker that already runs keeps its candidate, and the pin applies from its next new start.

## Verification

A Factory chooses one verification, or none.
`ci` reads the required check runs of the Task's pull request and runs nothing itself.
`verify` runs a bundle of shell commands in the Task's worktree, all of which must exit 0.
With none, auto merge is unavailable, and a Task goes from `done` straight to `merge_waiting`.

The Factory never reruns a verification: one failure is a failure, and the worker fixes and reports `done` again.
`n/3` counts those failures per Task, the limit is 3, and the third failure stops the Task as "verification failed three times" with the failing check and the worker's last report.
A failure wakes the worker with the check name, the log path or CI link, and the count.
A check's send-back counts like a failed verification.
Environment failures and merge conflicts do not count.
`retry` resets the count.

**CI.** The check runs of the worktree's head commit are read with `gh api --paginate`, keeping only each run's name, status, conclusion and link, one line per run.
A whole run is about 3.5 KB and a node keeps 64 KiB of a command's output, so the full answer of a commit with twenty runs would be cut; a line is about 180 bytes.
A commit whose read decided nothing is read again after 30 seconds.
Every named check must have a completed run, and only named checks decide; a commit with no run of a named check yet is pending, never passed.
`--ci` with no names takes the default branch's required checks from its protection, and `init` and `config ci=` refuse a Factory that would name no check (`ci_checks_required`).
A completed run passes on `success` or `neutral`, decides nothing on `skipped`, `cancelled` or `stale` (still pending), and fails on any other conclusion with the run's link.
A GitHub error that carries an environment signal is the environment's.
Any other read error, a line that is not a check run among them, is an unread answer: it decides nothing and counts as no failure, the run stays running and is read again, and the third unread answer of a run adds a `note` line to the Task's activity naming what the read answered, in the operator's language, so a read that fails the same way each time is never shown as verifying in silence.
A verify bundle's poll the node answered in a shape the Factory cannot read is unread the same way.

**Verify bundle.** Bundles run one at a time on the machine, in a queue of at most 256, each command through the shell with its output in the run's log.
The cap, `verify_timeout_minutes` (60 by default), applies to the whole bundle from its first command, and the command running when it passes fails the run.
A command that exits 137, prints "no space left on device" in its last 64 KiB, or dies with no exit code after printing "killed" is an environment failure; every other non-zero exit is the Task's.
Each run belongs to the engine, which ends its whole process tree when the Task is cancelled or the core stops.

**Before a merge.** When the verification passed and no question holds the Task, the Factory checks the merge in seconds, for a verify or local Factory, and never reruns CI.
`git merge-tree` against the current default branch names conflicting files; a conflict sends the worker back to `running` with a rebase message and does not count as a failure.
An optional `quick_check` command then runs on the Task worktree; its failure counts as a verification failure.
Paths matching `risk_paths` add a gate.
A verify Factory then runs its bundle once more on the Task with the latest default branch merged in, and a failure there counts as a verification failure.

## Merge

A Task's merge mode is its own field, else the Factory's.
In `auto` mode a verified Task that meets no gate is merged by the Factory.
In `manual` mode it goes to `merge_waiting` and a person merges.
Merge mode is always `manual` for a Factory with no verification.

A gate sends an `auto` Task to `merge_waiting` and appears in the Task page and inbox.

| Gate | Set when |
| --- | --- |
| `review_directly` | The card asked for direct review. |
| `approved_scope_change` | A person approved a scope change on the Task. |
| `breaking_change` | The worker reported `--breaking`, or a check flagged a breaking or public-contract change. |
| `no_verification` | The Factory has no verification. |
| `risk_path` | A changed path matches `risk_paths` (empty by default). |
| `manual_mode` | The Task's mode is `manual`. |
| `open_question` | A question other than a default or scope-change question is open. |
| `check_failed` | A judgment could not run. |
| `autonomy_diff` | An autonomy Task changed more than 200 lines. |
| `dirty_main` | A local merge finds the main checkout uncommitted or off the default branch. |
| `merge_refused` | GitHub or git refused the merge for a reason that is not the environment's, such as a branch policy; the reason is in the Task's events and in the answer to a person's `merge`. |

A review by an agent alone never merges.
A person's `merge` runs merge-tree and the quick check again on the latest main, then the same merge at once; a conflict sends the worker to rebase and a failed quick check counts as a verification failure.
A refused merge is tried once and then waits for a person, never once per tick; only an environment signal, or a merge GitHub answered before naming its commit, is asked again, at the GitHub back-off for a rate-limit, server or network signal (a minute for any other signal) and every 30 seconds for an unnamed commit, which is only read and never merged again: a pull request that merged lands whatever gate appeared since, because nothing can undo it, and one that did not merge goes through every check again before another merge; a commit still unnamed after 10 minutes waits for a person as a refused merge.
While a merge's commit is unnamed, a red run on a main head the Factory has not recorded is read as pending, not as an outside push.
`request-changes --comment` returns the Task to `running`, clears its gates and sends the worker the comment.

A Factory does not stack Tasks: a Task starts after every Task it depends on has merged ([When a Task may start](#when-a-task-may-start)), and each merges into main on its own.
People and their agents stack by [CONTRIBUTING.md, Starting work beside open pull requests](../CONTRIBUTING.md#starting-work-beside-open-pull-requests): of two pull requests that conflict in code, the one green later goes on top of the other, only the bottom layer merges main, and a stack merges from the bottom; overlapping Tasks that join the top of a stack when they finish, rather than main, would follow the same rules (issue #645).

**GitHub.** The merge is `gh pr merge` with the Factory's `merge_method` and `--match-head-commit` set to the commit the Task-stage verification passed on, so a push after it makes GitHub refuse the merge; a Factory without verification pins the head a person reviewed.
A verified Factory with no passed commit refuses to merge, a pull request already merged returns its merge commit, and a merge GitHub answered before naming its commit is read again rather than recorded empty.
`init` picks the first merge method the repository allows, in the order merge, squash, rebase, and `config` changes it.
**Local.** The Factory merges the verified commit, not the branch's current tip, into the project's primary checkout with `git merge --no-ff`, only while that checkout is on the default branch with no uncommitted tracked changes; a merge that fails is aborted, so the checkout is never left mid-merge.
Otherwise the Task stays in `merge_waiting` with `dirty_main` until it is clean.

A merge records its commit, and the Task becomes `landed`.
Main verification is the chosen verification on that commit: the push run's check runs for `ci`, or the bundle run in `<project>.worktrees/factory-main`, a detached worktree the Factory creates beside the project's worktrees, for `verify`.
A green result, or no main verification, makes the Task `done` and removes its worker, pane and worktree.

A red result breaks main and stops auto merge; manual merges still run.
While main is broken an `auto` Task that passed verification waits in `verifying` without merge-tree or quick-check reads, unless a person's gate sends it to `merge_waiting`; main is read again every 30 seconds.
If the broken commit is among the Factory's own merges since the last green, the Factory finds the first failing one, asking again for runs that were skipped or cancelled, and reverts that merge alone.
On GitHub the revert is a pull request from `factory/revert-<task id>`, which the Factory force-pushes, checks and merges with `--merge`; locally it is a revert commit made in `factory-main` and fast-forwarded into the primary checkout, only while that checkout is on the default branch and clean, as for a Task merge.
A revert that conflicts is aborted in `factory-main`, so that worktree stays usable for main verification and the next revert.
A revert whose verification passes is merged on its own, and the original Task becomes `relanding`: it runs again in the same worktree and session on the latest main, with a new pull request.
A broken main adds a `main_broken` line to the Factory's activity with the failed run's link, and `by_factory` says whether one of the Factory's own merges broke it.
When the failing commit is not the Factory's, nothing is reverted: the Factory drafts a fix Task for a person to confirm and stops auto merge.
Auto merge resumes when the head of main verifies green again.
A recovery that cannot name one merge, cannot make or merge the revert, or finds the revert red does not repeat itself: it stays stopped and puts the failed run's link, the candidate merges and these choices in the inbox: retry the revert, revert a named merge, or resume auto merge after a person fixed main.

## Outside work

The person wins outside the Factory, and the Factory follows.
Each open Factory reads GitHub every `outside_read_minutes` (2 by default): up to 200 issues labelled `factory` and 100 pull requests, or, locally, its local issues.
A failed read backs off 1, 2, 5, 15 and then 30 minutes on a rate-limit, server or network signal; three failures in a row mark the Factory's read stale.
A held issue missing from that list is looked up by number, at most 20 per read and continuing where the last read stopped; one that cannot be read now keeps its Task, and a deleted one counts as its label removed.

| What the Factory sees | What it does |
| --- | --- |
| A pull request the Factory did not open closes a Task's issue, and it is open | A pull request one of the Factory's own Tasks opened never counts, whatever its worker wrote; the report and the decisions a worker records sit in code blocks in its pull request body, so a "Fixes #N" there stays text. GitHub can still close another Task's issue from a closing keyword a worker put in a commit message or in a card it proposed, when that merge lands; the other Task is then cancelled and kept for revive. A pull request from a fork counts only once it is merged, since anyone can open one. Otherwise the Task becomes `outside`. A running worker is stopped, its worktree stays for the keep period, and the Task's activity gets an `outside` line (`closing_pr`) with the pull request's link while `revive` stays available. |
| That pull request merges | The Task is `done` with an `outside` line (`pr_merged`), and a dependent Task's predecessor counts as merged. A stopped worker's worktree still waits out its keep period. |
| The issue closes with no pull request, or the `factory` label is removed | The Task is cancelled with an `outside` line (`issue_closed`), kept for the keep period. |
| A person edits the issue body | A Task before its start goes back to `drafting` and is reviewed again. A running Task gets a scope-change question. The body is compared with the one the Factory wrote, ignoring line endings and surrounding space, so the first edit counts. |
| A person labels an issue the Factory does not hold | A new Task is drafted from the issue and reviewed, its checkboxes becoming the criteria, and it starts once nothing is left open, with no card confirmation. The body is not edited. |
| A finished Task's issue reopens | An `outside` line (`issue_reopened`) on the Task's activity only. |
| A push to main the Factory did not make | Main verification runs; while its checks are still running the head is read again every 30 seconds, and a red result takes the outside-push path above. |

A pull request or issue the Factory does not hold is ignored.
Cancelling a Task closes its open Factory pull request, and reviving reopens it.

## Questions, discoveries and autonomy

A question belongs to a Task card and is answered with `hide factory answer <task> [--question <id>] --choose suggestion|default|<choice>` or `--text`.
Every question has a suggestion, and a worker question has a deadline and may have a default action.
Its kind decides what an answer does.

| Kind | Raised by | An answer |
| --- | --- | --- |
| `intake` | The review, a check, a producer's open decision | Clears it; the Task is Ready when nothing else is open. |
| `split` | The review | `split` splits the Task, `proceed` clears it. |
| `default` | A worker `ask`, a drift or check finding | The default keeps going; a different answer wakes the worker. After the deadline the default is the answer. |
| `blocking` | A worker `block` | Wakes the worker through a reply; the Task waits for a slot and resumes its session. No deadline decides it. |
| `scope_change` | A worker, a re-add, an edited issue | `approve` records approval and adds a gate, and a re-added card and PRD become the Task's, sent to the worker in the reply; `reject` keeps the scope. After the deadline it is rejected. A newer re-add replaces a pending one. |
| `new_task_cap` | The Task reached the new-Task limit | `continue` lets it create more; any other choice leaves it stopped for a person to retry or cancel. |
| `proposed_task` | A prerequisite proposal outside autonomy | `approve` drafts the Task, and the proposer depends on it and gives its slot back until it lands. |
| `action` | A stop, a failed review, a main break | The listed choice runs: `retry`, `retry-review`, `enable-ai`, `start-as-is`, `cancel`, `resume-auto`, `retry-revert`, `revert <task>`. |
| `confirm_card` | The fix Task drafted after an outside push broke main | `confirm` lets the Task become Ready, `cancel` cancels it. A labelled issue's card needs no confirmation. |

Every question goes to Factory AI first, which answers it in a person's place where the Factory's mode allows and otherwise hands it to a person (see [Factory AI](#factory-ai-the-observer)).
A question a judgment or the engine raises is written for someone who has not read the Task, in the operator's language and without internal ids, command names or file paths.
It says what it holds up (`stopped`), gives each choice with what choosing it leads to (`outcomes`), names the recommended choice (`suggestion`) and the default with its deadline when it has one, and may carry evidence (`evidence`): links, log paths and the worker's report.
A request whose asker did not write what it holds up or where its choices lead gets them from Factory AI's verdict when it hands the request to a person.
A question with a default lets the worker continue, and the Task waits only at the end of verification for the answer or the deadline.
A blocking question releases the slot and stays open however long it waits; the inbox shows how many days.
A worker may report five classes of discovery with `hide factory propose --class`:

| Class | Effect |
| --- | --- |
| `in-scope` | A recorded fix inside the card. |
| `decision` | A recorded decision the agent made, reversible and inside the criteria. |
| `scope-change` | A scope-change question; the worker continues inside the current scope. |
| `prerequisite` | A new Task proposal. |
| `unrelated` | A follow-up candidate (see [Follow-up candidates](#follow-up-candidates)); nothing is added to a person's list. |

`--reclassify <discovery>` may only move a discovery toward a person, in the order `in-scope`, `decision`, `prerequisite`, `scope-change` and `unrelated` (the last two being equal); the reverse answers `reclassify_away_from_person`.
A prerequisite needs a card.
A Task that was itself proposed or started by autonomy may not propose a Task (`proposal_depth_exceeded`), and each Task may propose `new_task_limit` Tasks (3 by default) before it stops with the "new-Task cap" question.
A proposal that names an enabled `--autonomy` scope drafts the Task now and makes the proposer depend on it and sleep as `blocked`; any other proposal is a question that a person approves before the Task is drafted, and approving a prerequisite makes the proposer wait on it the same way.
The worker's claim is not the fit: the intake review reads the scope's description and must answer that the card fits it, or the Task loses its autonomy and stays `drafting` with a question a person answers before it runs.

Autonomy scopes are the kinds of Task a worker may start without a person.
The presets are `flaky_test`, `dependency_patch`, `lint_format` and `docs_links`, all off, and a person may add scopes by name.
An autonomy Task is reviewed like any other, may not propose Tasks, and gates to a person when it changes more than 200 lines.

### Follow-up candidates

A worker's `unrelated` discovery is a follow-up candidate.
The Task keeps it with the state `open`, its activity gets a `follow_up` line, and the Factory's view (`follow_ups`) and the Task page list it; no question or to-do is raised for it.
`hide factory follow-up <task> <discovery> issue|factory|discard` settles it, and each choice sets the state, records the issue or Task it became and adds a `follow_up` line to the Task's activity:

- `issue` creates an issue without the `factory` label, titled with the finding's first line, whose body is the finding, a link back to the Task and a hidden marker (state `issue`).
- `factory` creates an issue with the label, so the Factory starts it as a Task at once; the Task is made when the issue is, so the outside read does not make a second, and a local Factory adds the Task directly (state `factory`).
- `discard` drops it (state `discarded`).

A settled candidate is refused with `follow_up_settled`, and a name that matches none with `follow_up_not_found`.
An issue that cannot be made leaves its reason on the candidate (`failure`), answers `follow_up_failed`, and may be pressed again; the marker makes the second press find the issue a first press made, so one candidate never makes two.
While the Factory's GitHub access is blocked an issue is refused with `github_blocked`; `discard` still works.

## Environment

An environment problem is not a Task's failure and is never counted against it.

- **Before a start.** Starts hold while free disk at the first waiting Task's project is under `disk_floor_gb` (20 by default), or while macOS reports critical memory pressure. Warn pressure starts normally, and other systems report normal. The held Tasks show the reason and are checked again after a minute or when a worker stops; without a hold the machine is read on a tick where a start can happen (a free slot and a runtime not at its usage limit) and otherwise once a minute, so a backlog waiting on full slots is not read every tick.
- **At a failure.** Only a structured signal makes a failure the environment's: no space left on the device, exit 137, a GitHub 401, 403, 429 or 5xx answer, a network error, or a Herdr connection error. The adapters read the GitHub and git signals from the command's exit code and its stderr text. Everything else is the Task's. A runtime's usage limit comes from the core's provider usage rows (the toolbar's Weekly Usage reader): a main row or bucket at 100 percent with a reset ahead parks that runtime until the reset, new starts of an unpinned Task use the other runtime, and a worker that ends its turn without a report while its runtime is limited waits for a slot instead of stopping. The rows refresh every 5 minutes only while a window shows them, so a daemon with no window open learns of a limit late.
- **Code handles each signal first.** Disk full holds starts and, in every Factory that keeps `remove_finished_worktrees` on, removes at once the worktrees of finished Tasks and cancelled Tasks past their keep period. Out-of-memory and Herdr connection errors send the Task back to `waiting`. A rate-limit, server or network error backs the read off as above. A 401 or 403 blocks the Factory's GitHub steps as one state with one to-do for a person (see [GitHub sign-in and permission](#github-sign-in-and-permission)), and the Factory does not perform the sign-in itself.
- **Cascade.** Three different Tasks failing the same check or command within 30 minutes are read as the environment: each failure is taken back, the Tasks go to `waiting`, and new starts halt for 30 minutes.
- **If it does not clear.** A hold still there 30 minutes after it appeared goes through the recovery schedule below.

### The recovery schedule

The Factory works through the holds it meets before it asks a person.
A hold is one condition that keeps work back: the machine holding new starts (`disk_floor`, `disk_full` or `memory_critical`), a cascade that halted new starts, outside reads that failed three times in a row for a reason other than GitHub's sign-in or a permission, or a Task stopped for a reason a restart can clear (a refused worker start, a repeated environment failure, no report or a stall).
A hold keeps its clock in the Factory's record, so a restart does not reset it, and ends when its condition is gone.
At 30, 90 and 150 minutes it runs one action of the closed list below, and at 180 minutes it becomes a person's.
`factory_env_diagnosis` picks the action from the hold's facts, what was tried and the actions that are on and fit the hold; when it cannot answer, the first action that fits the hold and has not been tried runs; an action outside the hold's own list runs only when a diagnosis picks it.
Each step has five minutes to work before the next, so a schedule that fell behind (a machine that slept) runs its late steps five minutes apart, and a hold none of whose actions is on is a person's at once rather than at 180 minutes.
A worker restart runs once per Task, and after it the schedule picks only among the other actions; with none left it does nothing and waits for the 180-minute mark.
A full disk is not scheduled: it runs `remove_finished_worktrees` at once, with a line of the Factory's activity when it removed something.
Each run is a `recovery` line of the Task's activity for a Task's hold and of the Factory's otherwise, first running and then settled as `improved` when the hold clears, `partial` when it is still there five minutes later, or `unchanged` at once when the action had nothing to act on.
A cleanup line also lists the worktrees it removed and the disk it freed.
A Task whose stop the schedule is working on is not a person's yet: its card is `recovering` and 결정 필요 does not list it.
At 180 minutes it appears as a stopped item carrying what was tried (`attempts`) and, when a diagnosis gave one, its cause as evidence, and `retry` starts it again.
A hold of the Factory that reaches 180 minutes becomes one `hold` to-do, and `hide factory resolve hold:<name>` (`start-disk_floor`, `start-disk_full`, `start-memory_critical`, `halt` or `reads`) starts its schedule over from that moment.
A diagnosis that names a command outside the list gives a `command` to-do when the command is one line of at most 400 bytes made only of printable ASCII, the space, and letters and digits of any script (anything else is refused and logged, never cut, since the copy button writes it as it is): the cause in one sentence, the command to copy, what running it does, and a button, `hide factory resolve C<n>`, that marks it done.
The same command is not added twice while one is open, and a Factory keeps at most 20 open.
The diagnosis is told that logging in, deleting outside the Factory and installing tools are only ever a command for a person.

The closed recovery list is the only set of actions the Factory runs without a person, and all five are on by default, for a new Factory and, by the store move, for an existing one.
`config --set recovery=<action>=off` turns one off, and the watch picks from the same list.
Each action touches only the diagnosed Factory's Tasks, except the start hold and the runtime switch, which belong to the machine.

| Action | Does |
| --- | --- |
| `remove_finished_worktrees` | Removes the worktree of the Factory's finished Tasks and of its cancelled Tasks past their keep period. A worktree with leftovers is kept. |
| `restart_worker` | A Task stopped by a refused start, a repeated environment failure, no report or a stall goes back to `waiting`, and its worker starts again in the same worktree and session. |
| `sleep_wake_worker` | A running worker waiting on input is asked to sleep and woken in the same session with a note to carry on. Herdr does not put an agent that waits for a person to sleep, so for such an agent the note waits for its next prompt. |
| `switch_runtime` | New starts of unpinned Tasks leave the Factory's default runtime for an hour, while the other runtime is not limited; the runtime is the machine's, so this holds for every Factory. |
| `retry_reads_and_reconnect` | Clears the Factory's read back-off, the machine's start hold and the Factory's cascade halt. |

A Task that alone repeats an environment failure three times is the Task's: it stops as "same environment failure repeated", which the schedule then works on.

### GitHub sign-in and permission

A 401 from GitHub (a lost sign-in) or a 403 (a missing permission) blocks that Factory's GitHub steps as one state, however many Tasks it stopped.
The block (`github_block`) keeps whether it was a permission (`forbidden`), the scope a 403 named, the step that was refused and since when.
While it lasts the Factory asks GitHub nothing it would be refused: outside reads, pushes, pull requests and the creation of an issue wait, and each Task whose step waits shows `permission_wait`.
The Factory has one `github` to-do in 결정 필요 with the command to run, `gh auth login` or `gh auth refresh -s <scope>` when a 403 named the scope, and a button, `hide factory resolve github`.
The button asks GitHub for the signed-in account and a read again, and answers `github_still_blocked` while GitHub still refuses; the Factory also checks a lost sign-in by itself every 5 minutes.
A missing permission is not checked on its own, because the check only reads and a read passes while the refused write would still be refused; pressing the button lets the refused step try again, and a 403 there raises the to-do again.
The scope in `gh auth refresh -s <scope>` is taken only when it has a scope's shape (lowercase words joined by `_` or `:`); otherwise the command is `gh auth login`.
When a check passes, the block and every `permission_wait` clear and the stopped steps continue where they were.
The Factory never runs the sign-in command itself.

## The watch

The watch reads a Factory's board for what a person cannot already see in 결정 필요, such as work that stopped moving, a chain held by one wait, or a pattern of failures.
It runs when a Task finishes, when main breaks, when a Task reaches its new-Task limit, and every `watch_interval_minutes` (30 by default, at least 5) for a Factory that has Tasks.
It asks `factory_watch` with the Factory's board summary.
A warning names one action of the closed recovery list or none, and it is never a person's item.
Every warning is a `watch` line of the Factory's activity, with the Task it names when it names one.
An action that is on runs, on the Task it names or on the Factory, and counts against `watch_daily_limit` (5) per Factory per UTC day; a warning past the limit is logged as capped and recorded without its action, and so is a warning about a Task that moved after the board was read or a Task the Factory does not hold (`watch.unresolved`), which never widens to the whole Factory.
An action that ran adds a `recovery` line with its outcome and, for a cleanup, the worktrees it removed and the disk it freed.
A watch that is slow or fails changes no Task.

## Configuration

`hide factory config [--project <path>]` prints the Factory's settings and the machine's worker limit, and `--set <key>=<value>` changes them for the next decision.
`config` is the stored record, which keeps durations in milliseconds and the disk floor in bytes; `settable` prints each of those under the key and in the unit `--set` takes (`disk_floor_gb`, `stall_minutes` and the like).
Only an operator may set values.
A value a key does not take answers `config_invalid` naming the key, and a key `--set` does not take answers `config_invalid` with the key and every key it takes (`detail.keys`).
A Factory's `merge_mode` cannot be `auto` while it has no verification (`auto_needs_verification`).

| Key | Value | Default |
| --- | --- | --- |
| `merge_mode` | `auto` or `manual` | `manual` until `init` chose a verification, then `auto` |
| `merge_method` | `merge`, `squash` or `rebase` | The first the repository allows (merge, squash, rebase) |
| `verify` | Commands joined by `&&&`; empty clears | none |
| `ci` | Comma-separated check names | none |
| `no_verification` | Any value | none |
| `quick_check` | A shell command; empty clears | none |
| `max_workers` | 1 to 64, machine-wide | 5 |
| `question_deadline_hours` | Hours | 24 |
| `stall_minutes` | Minutes, at least 1: the quiet window of each worker's watch | 30 |
| `no_report_minutes` | Minutes, at least 1, after a turn ends without a report | 2 |
| `watch_interval_minutes` | Minutes, at least 5 | 30 |
| `watch_daily_limit` | Count | 5 |
| `outside_read_minutes` | Minutes, at least 1 | 2 |
| `cancel_keep_days` | Days | 7 |
| `done_fold_days` | Days | 3 |
| `archive_fold_days` | Days before a finished Task leaves the list | 90 |
| `new_task_limit` | Count | 3 |
| `verify_failure_limit` | Count, at least 1 | 3 |
| `autonomy_diff_limit` | Changed lines an autonomous Task may merge alone | 200 |
| `verify_timeout_minutes` | Minutes per bundle run, at least 1 | 60 |
| `disk_floor_gb` | Gigabytes | 20 |
| `default_runtime` | An agent Factory can start and this machine has; replaces the first worker candidate with that agent on its defaults | Claude Code when `init` finds it on the path, else Codex when it finds that, else `claude` |
| `workers` | A JSON list of 1 to 5 `{agent, model, effort, description}`, each description at most 200 characters (`worker_description_too_long`), the first the default; `default_runtime` follows it | The default runtime on its CLI's defaults |
| `observer_mode` | `manual`, `assist` or `autonomous` (직접, 함께, 맡김) | `assist` |
| `observer_daily_limit` | 1 to 1000 Factory AI calls a local day | 100 |
| `factory_ai` | An agent id, or `default` for the Hide AI choice | `default` |
| `factory_ai_model`, `factory_ai_effort` | A model, or an effort the agent declares; `default` clears; refused with `factory_ai_required` before `factory_ai` | none |
| `harness` | `<name>:<instructions>`; empty clears | none |
| `autonomy` | `<scope>=on` or `off` | all off |
| `recovery` | `<action>=on` or `off` | all on |
| `worker_args` | `<agent>=<arguments>`, split on spaces; empty clears | none |
| `risk_paths` | Comma-separated: `dir/**`, `*.ext`, or an exact path or folder | none |
| `prd_in_issue` | `on` or `off` | off |
| `macos_notifications` | `on` or `off` | off |

`max_workers` is the machine's, not the Factory's, and any Factory's `config` sets the same value.
`worker_args` holds the whole argument list for one agent, such as a permission mode the operator chose, and an empty list removes it.
A worker candidate names an agent whose adapter declares a start (`agent_not_startable`) and that this machine has (`agent_not_installed`), and a model or effort its start can take; a number outside its range answers `out_of_range`.
A Factory stored before candidates reads its `default_runtime` as the one default candidate, and nothing is migrated.
`harness` places the named instruction in each worker's first prompt; gates never trust a harness's own claims, and a harness's own checks belong in `verify`.

## The `hide factory` command

```
hide factory init <project> [--ci [<check>...]] [--verify <command>]... [--no-verification] [--merge auto|manual] [--confirm]
hide factory add [--task <id>|<issue>] --title <t> --goal <g> --criterion <c>... [--out-of-scope <s>]... [--open <decision>]... [--after <task>]... [--external <ref>]... [--prd <path>] [--review-directly] [--priority <n>] [--merge auto|manual] [--runtime <agent>] [--worker <n>] [--project <path>]
hide factory status [--project <path>]
hide factory show <task>
hide factory inbox
hide factory answer <task> [--question <id>] [--choose suggestion|default|<choice>] [--text <answer>] [--change]
hide factory answer <task> --decision <R<n>> [--choose <choice>] [--text <answer>]
hide factory ask --question <text> --suggestion <text> --default <action> [--choice <text>]... [--deadline-hours <n>]
hide factory block --question <text> --suggestion <text> [--choice <text>]... [--deadline-hours <n>]
hide factory propose --class in-scope|decision|scope-change|prerequisite|unrelated --text <text> [--title <t> --goal <g> --criterion <c>...] [--autonomy <scope>] [--reclassify <discovery>]
hide factory done --result <one line> [--changed <text>]... [--verified <text>]... [--unverified <text>]... [--breaking]
hide factory decide --text <decision>
hide factory config [--project <path>] [--set <key>=<value>]...
hide factory priority <task> <n>
hide factory dep add|remove <task> --on <task>
hide factory pause|resume|retry|merge|cancel|revive <task>
hide factory pause|resume --factory [--project <path>]
hide factory worker <task> <n>|auto
hide factory follow-up <task> <discovery> issue|factory|discard
hide factory resolve github|C<n>|start:<task>|hold:<name> [--project <path>]
hide factory request-changes <task> --comment <text>
hide factory check --at intake|after-done|periodic --instruction <text> [--project <path>]
hide factory close [--project <path>]
Add --json to print the answer as JSON.
```

| Command | Does |
| --- | --- |
| `init` | Without `--confirm`, shows the source, the verification candidates the code detected (the branch's required checks and verify commands read from `Cargo.toml`, `package.json`, `Makefile` or `pyproject.toml`), the merge mode and, on GitHub, the login `gh api user` names, the repository, and everything the Factory reads and writes there, and writes nothing. With `--confirm` and a verification choice, creates the Factory, records that approval (account, repository, time) in its history as `github.approved`, and, on GitHub, creates the `factory` label. A closed Factory is reopened with `--confirm`. A project that already has an open Factory answers with it. |
| `add` | Creates a Task or updates the named one. A positional issue reads its title and body for missing fields. `--prd` copies the file into the store. |
| `status`, `show`, `inbox` | Read the board, one Task page, and the inbox. |
| `answer` | Answers a question and records who relayed it. `--change` replaces an answer Factory AI gave (see [Factory AI](#factory-ai-the-observer)). `--decision R<n>` changes a decision Factory AI made, by its number on the Task page, and tells the worker; it cannot be combined with `--question` or `--change`, and a decision a person or the worker made, an applied fix, an unknown number and a finished Task are refused (`decision_not_changeable`, `decision_not_found`, `task_finished`). |
| `ask`, `block`, `propose`, `done`, `decide` | A worker's reports on its own Task. `ask` and `block` take up to five `--choice`. `done` takes the report in four parts: a one-line `--result`, and any number of `--changed`, `--verified` and `--unverified`; without `--result` it is refused with `result_required`, and the retired `--summary` with `summary_replaced`, both naming the four parts. |
| `config`, `check` | Reads and sets settings, and adds a natural-language check. A check cannot be removed once added. |
| `priority`, `dep`, `pause`, `resume`, `retry`, `merge`, `request-changes`, `cancel`, `revive`, `close` | A person's actions. `close` needs no Task in an active lifecycle state (anything except drafting, waiting, done or cancelled). `pause --factory` and `resume --factory` pause and resume a whole Factory. |
| `worker` | Pins the Task's worker candidate by its number from 1, or `auto` for the review's pick (`worker_out_of_range`). `add --worker <n>` pins at add. |
| `follow-up` | Settles a follow-up candidate, a worker's unrelated finding named by its discovery id `D<n>`: `issue`, `factory` or `discard` (see [Follow-up candidates](#follow-up-candidates)). |
| `resolve` | The single button of a to-do, named as the item's `resolve` field names it: `github` checks the sign-in again and lets the GitHub steps continue (`github_still_blocked` while it is still refused), `C<n>` marks a command the operator ran as done, `start:<task>` asks the start of a worker whose pane a person looked at again, and `hold:<name>` starts a held recovery's schedule over. A name of none of these shapes is refused before it reaches the engine (the CLI prints its usage, the screen's message is an invalid request), and a well-formed name with no open to-do behind it answers `item_not_found`. |
| `help` | Prints the commands above. |

Without `--project`, `add`, `config`, `check`, `close` and a Factory's `pause` and `resume` use the only open Factory, and answer `factory_ambiguous` when there are several.
A project path is made absolute by the CLI.
`--json` prints the engine's answer as one JSON object, and without it the answer is printed for a person.
Every answer carries `ok`.
A refusal prints `refused: <reason>`, the current state and allowed actions or the invalid fields when it has them, and `next: <action>`, and exits non-zero; its JSON is `{"ok": false, "reason", "next_action", "detail"}`.
`add` answers `{result, task, questions}`, `init` a preview (its `github` is `{account, repo, reads, writes}` or `null` for a local project), `created` or `existing` object, `show` `{task: <TaskDetail>}`, `inbox` `{count, items}`, `status` a `FactorySummary`, and `config` `{config, machine}`.
Other actions answer `{message, task}` with the Task's id, display id and new state.
Common refusal reasons are `role_not_allowed`, `task_not_found`, `task_ambiguous`, `factory_not_found`, `factory_ambiguous`, `factory_closed`, `card_invalid`, `action_not_allowed_in_state`, `config_invalid`, `auto_needs_verification`, `github_login_required`, `github_permission_missing`, `main_dirty`, `revive_expired`, `already_answered`, `task_finished`, `answer_required`, `decision_not_found`, `decision_not_changeable`, `result_required`, `summary_replaced`, `follow_up_not_found`, `follow_up_settled`, `follow_up_failed`, `github_blocked`, `github_still_blocked`, `item_not_found`, `too_many_choices`, `choice_too_long`, `worker_out_of_range`, `factory_ai_required` and `factory_ai_unavailable`.

## The read model

`FactorySummary` and `TaskDetail` are the contract for any screen of the Factory, and `status`, `inbox` and `show --json` print them as they are.
Every number in them is derived in `hide-factory/src/summary.rs` from the stored Tasks, and a screen computes nothing.
The movement board replaces the former `drafting`/`waiting`/`running` column and flow names with `before`/`moving`/`stuck`; lifecycle state names stay unchanged.
Persisted Tasks accept a missing card `summary` without migration.
The engine's sentences in `state_label`, `waiting_for`, `result`, `remaining`, `gates` and `stop` have a code beside them, so a screen can say them in any language; the codes are listed under [Codes](#codes) and pinned by `every_summary_code_is_pinned` in `summary.rs`.
An inbox item's `text` carries only its `kind`, and a question's own text, a split's piece count, a stop's detail and `TaskDetail.verification` have no code yet.

**`FactorySummary`**

| Field | Meaning |
| --- | --- |
| `my_turn` | The one person-facing number: every 결정 필요 item across all Factories. |
| `factories` | One `FactoryView` per Factory. |
| `inbox` | Every `InboxItem`, in the inbox order below. |

**`FactoryView`**

| Field | Meaning |
| --- | --- |
| `id`, `project`, `project_name` | The Factory's id, canonical path and short name. |
| `source` | `github` or `local`. |
| `verification` | `ci`, `verify` or `none`. |
| `closed` | Whether the Factory is closed. |
| `flow` | `before`, `moving`, `stuck` and `done_today` counts; `done_today` counts by the machine's local day, read through the clock port's UTC offset. |
| `my_turn` | This Factory's 결정 필요 items. |
| `paused` | A person paused the whole Factory. |
| `observer_mode` | `manual`, `assist` or `autonomous` (직접, 함께, 맡김). |
| `observer_today`, `observer_limit` | Factory AI calls counted today and the daily cap. |
| `observer_capped` | Today's calls reached the cap, so the rest of the day goes to a person. |
| `github_block` | The GitHub sign-in or permission block (`forbidden`, `scope`, `stage`, `since`), or none. |
| `follow_ups` | The open follow-up candidates of all its Tasks, newest first, at most 50. |
| `follow_ups_open` | How many follow-up candidates are open, the ones past the list included. |
| `activity` | The Factory's latest 50 lines of activity, oldest first. |
| `metrics` | The seven-day numbers described under `Metrics` below. |
| `factory_ai` | The Factory AI's agent, model and effort, or none for the Hide AI choice. |
| `workers` | The worker candidates, the first the default. |
| `macos_notifications` | Whether the desktop app shows this Factory's macOS notifications. |
| `columns` | The four board columns, each with its Task cards in order. |
| `cancelled` | Cancelled Tasks, off the board. |
| `graph` | Nodes, reduced edges and unrelated Tasks. |
| `dependencies` | Every edge as (predecessor, Task); the graph's edges are a reduction of these. |
| `outside_read_at` | When outside work was last read. |
| `stale` | Three outside reads failed in a row. |
| `main_broken` | Auto merge is stopped by a broken main. |
| `auto_merge_available` | The Factory has a verification. |
| `merge_mode` | `auto` or `manual`, `manual` when there is no verification. |

**`CardView`**

| Field | Meaning |
| --- | --- |
| `task`, `display_id` | The Task id, and the issue number once Ready. |
| `ai_decisions` | How many of Factory AI's decisions stand on the Task. |
| `permission_wait` | A GitHub step of the Task waits for the Factory's access to come back. |
| `recovering` | The recovery schedule is working on the Task's stop. |
| `column` | `before`, `moving`, `stuck`, `done` or none. |
| `title` | The card title. |
| `summary` | A single-line description, capped at 60 Unicode characters, from the optional persisted card summary or the first goal sentence without template headings or a repeated title. |
| `issue`, `issue_url` | The real issue reference and its GitHub URL, absent before an issue exists; local references have no URL. |
| `pr` | The current PR, including an outside PR closing the issue. |
| `worker_runtime` | The worker's actual runtime, absent before a worker exists. |
| `worker_label` | The worker's agent as a person reads it, its accessible name. |
| `pause_reason` | For a paused Task, `person` or `pane_closed`. |
| `resume_at` | For a waiting Task, its worker runtime's engine usage-hold deadline, only while it is in the future. |
| `waiting_group` | `person` for a stuck Task needing a person or paused; `other` for another stuck Task; otherwise absent. |
| `stage` | 0 waiting, 1 work, 2 verification, 3 merge, 4 complete. |
| `track` | The cell the 라인 row and the Task page's stage track light: the stage, except that a Task past intake that has not run yet sits at 1, work. |
| `state`, `state_label` | The state id and its label, with the stop reason for a stopped Task. |
| `needs_person` | The Task is blocked, stopped, `merge_waiting`, paused by a closed pane, or has an open question a person answers; a Task blocked only on requests Factory AI is still sorting is not yet, and neither is a stop the recovery schedule is still working on. |
| `waiting_for` | For a waiting Task, the predecessors by display id, the environment hold, or `slot`; for a blocked one, `answer` or `predecessor`. |
| `waiting_code` | The same as a `WaitingFor` code. |
| `waiting_on` | For `predecessors` on a waiting Task, the display ids of the predecessors it waits on, for a waiting or blocked Task. |
| `env_hold` | For `environment`, the `EnvHold` code. |
| `stop` | For a stopped Task, the `StopReason` code behind `state_label`. |
| `priority` | The Task's priority. |
| `since` | When it entered its state. |
| `unread` | A completion nobody has looked at: an operator's `show` of the Task clears it. |
| `folded` | A completion older than 3 days, or a purged cancelled Task. |
| `archived` | A finished Task older than 90 days. |
| `failures` | Verification failures counted against it. |
| `external` | External waits. |
| `revive_until` | When a cancelled Task can no longer be revived. |
| `worker_pane` | The worker's pane while the Task is not finished. |

**`InboxItem`**

| Field | Meaning |
| --- | --- |
| `group` | `answer`, `merge`, `stopped` or `todo`. |
| `kind` | The question kind, `merge`, `stopped`, or `paused` for a Task whose worker pane the operator closed; for a to-do, `github`, `command`, `start` or `hold`. |
| `rank` | The order key: 0 blocking question, 1 other answers, 2 merge, 3 stopped and action questions, 4 to-dos. |
| `factory`, `task`, `display_id`, `title`, `project` | Where it belongs. `task` and `display_id` are none for a to-do of the Factory (`github`, `command`, `hold`). |
| `question` | The question id, when the item is a question. |
| `text` | The question or the to-do in one sentence: the asker's words for a question (Factory AI's rewrite of a worker's question when it gave one), the stop's detail for a stop, the cause for a command or hold to-do. |
| `stopped` | What the item holds up, as the asker wrote it. |
| `holding` | The same as a `Holding` code, always set. |
| `outcomes` | Each choice with what choosing it leads to, where the asker wrote it. |
| `fallback` | Why Factory AI did not decide the item: `failed`, `daily_limit`, `paused`, `queue_full`, `dropped`, `restart`, `unsure`, `permission` or `no_choice`; none when its kind is a person's in the Factory's mode. |
| `evidence` | What the item unfolds: links, log paths, the worker's result line, the pane of a start to-do, the diagnosis's cause. |
| `resolve` | For a to-do, the name `hide factory resolve` takes: `github`, `C<n>`, `start:<task>` or `hold:<name>`. |
| `command`, `impact` | For a `github` or `command` to-do, the command to copy, and for a `command` to-do what running it does. |
| `suggestion` | The preselected answer. |
| `result` | What sending the preselected answer does. |
| `result_code` | The same as a `ResultCode`. |
| `unblocks` | For a blocking question, a merge or a `github` to-do, the display ids of the Tasks this frees. |
| `gates` | For a merge item, the `Gate` codes it waits on. |
| `stop` | For an item of a stopped Task, the `StopReason` code. |
| `env_hold` | For a hold to-do about the start hold, the `EnvHold` code. |
| `attempts` | For a hold to-do or a stop the schedule gave up on, what the recovery schedule tried: when, which `RecoveryAction`, and its `RecoveryOutcome`. |
| `default_action` | What the worker does meanwhile. |
| `choices` | The listed answers. |
| `deadline`, `remaining` | The deadline, and a short phrase for it or for how many days a blocking question has waited. |
| `remaining_hours` | Whole hours left before the deadline, rounded up, 0 once it passed; none for a blocking question. |
| `waiting_since`, `waiting_days` | When it started waiting, and whole days for a blocking question. |
| `decision_kind`, `observer_reason` | The kind and reason Factory AI gave a request it sorted; for a stopped item, the diagnosis line. |

The inbox is 결정 필요, and it lists only what a person moves.
It holds each open question a person may answer (not one Factory AI is still sorting), each `merge_waiting` Task, each stopped Task that has no action question and that the recovery schedule is not working on, each Task paused by a closed worker pane, and the to-dos.
A to-do is one button for something a person does outside the Factory or confirms: a lost GitHub sign-in or missing permission (`github`, one per Factory), a command only a person can run (`command`), a worker start that never showed a session (`start`), and a hold the recovery schedule escalated (`hold`).
A closed Factory lists no to-do of its own (a GitHub block, a command, a hold), since nothing would clear it.
Items come in order of rank and, within a rank, the item waiting longest first.
That puts blocking questions first, then other answers, merge waits, stops and to-dos, so what has waited the longest to be unblocked is at the top.
Cancelling a Task answers its open questions as `cancel`, so a cancelled Task lists none and no deadline applies a default to it.

The CLI and screen use the same movement columns.
`before` holds drafting and waiting Tasks that have never run.
`moving` holds running, verifying, relanding and landed Tasks.
`stuck` holds blocked, merge-waiting, stopped, paused, outside and previously-run waiting Tasks.
`done` holds only done Tasks; cancelled Tasks stay off the board.
A worker record, a verification attempt or a report establishes prior execution.
Person waits sort before other waits, stopped cards before other cards in their group, then the existing priority and age order applies.
`flow.done_today` counts only Tasks completed on the machine's local day.

The stage is derived from the same stored lifecycle and execution history.
Drafting and never-run waiting are stage 0.
Running, blocked, paused, relanding, previously-run waiting and stops other than verification or publication refusal are stage 1.
Verifying and stops for `verify_failed` or `publish_refused` are stage 2.
Merge waiting, landed and outside are stage 3; done is stage 4.
The track is the stage with one difference: only drafting stays at 0, so a Task waiting for its first start already lights 작업.
The engine derives it, and the web shell draws the cell it is given without computing one.

The existing intake-review response supplies the optional card summary without another judgment or additional input fields.
A failed or older response falls back to the goal sentence.
A goal edit invalidates its old summary, and an observed issue-body edit refreshes it from the body already returned by that read, without another GitHub request.
Old Tasks use the fallback when read and are never rewritten just to add the field.

**`TaskDetail`**

| Field | Meaning |
| --- | --- |
| `card` | The Task's `CardView`. |
| `factory`, `project` | The Factory's id and path. |
| `goal`, `criteria`, `out_of_scope` | The card text. |
| `before`, `after` | The chain: predecessors, and the Tasks that wait on this one. |
| `attachments` | Each PRD copy with its path, hash, version and original path. |
| `pr` | The pull request: number, URL, head, whether the Factory opened it, whether it is open. |
| `verification` | `n/3`, or "검증 없음" without verification. |
| `attempts` | A list of `AttemptView`. |
| `decisions` | The `DecisionView` of each decision record, in the order recorded. |
| `questions` | Every question with its answer and who relayed it. |
| `discoveries` | The discoveries with their class; an unrelated one carries its follow-up state. |
| `checklist` | One `CriterionView` per completion criterion of the card. |
| `report` | The worker's last report in four parts (`result`, `changed`, `verified`, `unverified`, and `raw` for a report that came as a plain letter), or none. |
| `activity` | The Task's activity, oldest first, at most the last 200 lines. |
| `follow_ups` | The Task's follow-up candidates, in the order found. |
| `issue_text` | The issue as written, for the folded original; none without an issue. |
| `gates` | The reasons a merge waits for a person. |
| `gate_codes` | The same as `Gate` codes. |
| `allowed` | The actions the state allows. |
| `stop` | The stop reason's label. |
| `stop_code` | The same as a `StopReason` code. |
| `merge_sha` | The merge commit. |
| `worker_name`, `worktree`, `branch` | The worker's name, folder and branch. |
| `worker` | The `WorkerLine` of the worker that actually started. |
| `pinned_worker` | The candidate a person pinned, 1 first. |
| `ai_picked_worker`, `ai_pick_reason` | The candidate the intake review picked, 1 first, and why. |
| `auto_restarts` | Automatic restarts used since a person last started it. |
| `diagnosis` | Factory AI's one line under a no-report stop. |
| `resting_since` | When the worker's current rest began, as the core saw it. |
| `woke_at`, `diagnosed_at` | When the engine woke the resting worker, and when it asked for a diagnosis. |
| `diagnosed_from` | The `DiagnosisSource` that diagnosis read. |

**`DecisionView`**

| Field | Meaning |
| --- | --- |
| `id` | `R<n>`, the decision's place in the Task's list from 1, which `answer --decision` takes. Decisions are never removed, so the place is stable. |
| `text` | The decision, a question and its answer joined, or the assumption, the fix or the comment. |
| `by` | `person`, `ai` (Factory AI) or `worker`. |
| `recorded_by` | Who recorded it, as stored: the relaying pane, `observer` or `worker:<task>`. |
| `source` | `answer`, `assumption`, `send_back`, `worker`, `request_changes` or `risk_merge`; none on a record from before sources were kept. |
| `kind`, `reason` | The kind (`A` to `E`) and reason Factory AI gave, when it decided. |
| `at` | When it was recorded. |
| `overridable` | Whether a person can change it now: Factory AI made it, as an answer, an assumption or a send-back, and the Task is not finished. |
| `changed` | When a person replaced it: who, when, and what Factory AI had decided (`from`). |

**`CriterionView`**

| Field | Meaning |
| --- | --- |
| `text` | The completion criterion. |
| `state` | `met`, `unmet` or `unknown` as the last check judged it, none until a check did. |
| `reason` | The check's one line, when it gave one. |

**`FollowUpView`**

| Field | Meaning |
| --- | --- |
| `task`, `display_id` | The Task the finding came from. |
| `discovery`, `text` | The discovery id, `D<n>`, and the finding. |
| `state` | `open`, `issue`, `factory` or `discarded`. |
| `issue`, `issue_url` | The issue it became; local issues have no URL. |
| `became` | The Task a `factory` choice made. |
| `failure` | Why the last attempt to make its issue failed. |
| `at` | When its state last changed. |

**`Metrics`**

| Field | Meaning |
| --- | --- |
| `finished`, `person_items_tenths` | Tasks done in the last seven local days, and the average number of 결정 필요 items a person answered or pressed per finished Task, in tenths. |
| `started`, `start_median_ms` | Tasks whose first worker started in those days, and the median time from the Task's creation to that start. |
| `ai_decisions`, `overridden`, `override_percent` | Factory AI's decisions made in those days, how many of those a person has changed, and the percentage. |

A number with nothing to count is left out, which a screen shows as a dash.
A person's item is an answer to a question, a merge, a request for changes, a retry, a resumed pane the operator had closed, or a start to-do pressed; a confirmation of a to-do's command or a GitHub recheck is not counted.

**`ActivityEvent`**

A line of a Task's or a Factory's activity is `{at, task?, kind, ...}`, where `task` names the Task a Factory line is about.
The log keeps at most 500 lines for a Factory and 200 for a Task, dropping the oldest, and a screen says each line in the operator's language from its kind and facts; only the free text in a line is someone's own words.

| Kind | Facts | Written when |
| --- | --- | --- |
| `intake` | `label`, `criteria`, `assumptions` | The review completed the card. |
| `started` | `resumed` | A worker started, or started again in the same session. |
| `report` | `report` | The worker reported `done`. |
| `pull_request` | `number`, `url` | A pull request was opened or found for the Task. |
| `verification` | `number`, `ci`, `outcome` (`passed`, `failed` or `environment`), `check`, `link` | A verification attempt answered. |
| `sent_back` | `text` | A check sent the work back to the worker. |
| `recovery` | `action`, `outcome`, `removed`, `freed` | A recovery action ran, and again when it settled. |
| `follow_up` | `discovery`, `state`, `issue` | A candidate was found, or settled. |
| `ai_decision` | `text` | Factory AI decided in a person's place. |
| `outside` | `what` (`closing_pr`, `pr_merged`, `issue_closed` or `issue_reopened`), `link` | Something outside the Factory moved the Task. |
| `main_broken` | `by_factory`, `link` | Main verification failed after a merge. |
| `cleanup_kept` | `worktree`, `detail` | A finished Task's worktree was kept because it holds leftovers. |
| `watch` | `text`, `action` | The watch saw something, and the action it ran when it named one. |
| `daily_limit` | `limit` | Factory AI reached the day's cap. |
| `note` | `text` | A notice from before the log, kept with its words, or a verification whose answer could not be read three times. |

**`WorkerLine`**

| Field | Meaning |
| --- | --- |
| `agent`, `label` | The agent id and the name a person reads. |
| `model`, `effort` | The model and effort it started with, none for the CLI's defaults. |
| `picked`, `pick_reason` | The candidate's description and the review's reason, when the review picked it. |

**`AttemptView`**

| Field | Meaning |
| --- | --- |
| `number` | Its place in the Task's attempts, from 1, so a run after an environment failure or a cancelled run is the next number; `n/3` counts failures, not attempts. |
| `stage` | `task` (after `done`) or `pre_merge`. |
| `started_at` | When it started. |
| `outcome` | `passed`, `failed`, `environment`, `cancelled` (the run was ended before it answered: the Task went back to its worker, was cancelled or was taken outside, or a restart found it unfinished behind a later attempt) or `running`; only the last attempt can be running. |
| `check` | The failing check or command. |
| `link` | The CI link or the log path. |
| `log_tail` | The last 4 KiB of a local log. |

### Codes

The values below are pinned in `contracts/snapshot-wire-enums.json`, under the `factory_*` keys, and by `every_summary_code_is_pinned` in `summary.rs`.

| Code | Values |
| --- | --- |
| `TaskState` | `drafting`, `waiting`, `running`, `paused`, `blocked`, `verifying`, `merge_waiting`, `landed`, `done`, `stopped`, `relanding`, `outside`, `cancelled` |
| `Column` | `before`, `moving`, `stuck`, `done` |
| `WaitingFor` | `predecessors`, `slot`, `environment`, `answer` |
| `EnvHold` | `disk_floor` (free disk below the floor), `disk_full` (a command found no space), `memory_critical` |
| `StopReason` | `no_report`, `stalled`, `verify_failed`, `new_task_cap`, `environment_repeated`, `worker_start`, `publish_refused`, `worker_gone` |
| `Gate` | `review_directly`, `approved_scope_change`, `breaking_change`, `no_verification`, `risk_path`, `manual_mode`, `open_question`, `check_failed`, `autonomy_diff`, `dirty_main`, `merge_refused` |
| `QuestionKind` | `intake`, `split`, `default`, `blocking`, `scope_change`, `new_task_cap`, `proposed_task`, `action`, `confirm_card` |
| `QuestionOrigin` | `review`, `worker`, `check`, `engine` |
| `ResultCode` | `wake_worker`, `apply_or_merge`, `ready`, `split`, `drafting`, `new_task_cap_choice`, `run_action`, `merge`, `restart_worker`, `resume_worker`, `resolve` |
| `Holding` | `worker` (a worker sleeps until the answer), `start` (the Task's start), `merge`, `progress` (the Task stopped), `starts` (every new start of the Factory), `github` (the Factory's GitHub steps), `continues` (the work goes on with the default) |
| `DiscoveryClass` | `in_scope`, `decision`, `scope_change`, `prerequisite`, `unrelated` |
| `FollowUpState` | `open`, `issue`, `factory`, `discarded` |
| `DecisionBy` | `person`, `ai`, `worker` |
| `DecisionSource` | `answer`, `assumption`, `send_back`, `worker`, `request_changes`, `risk_merge` |
| `DecisionKind` | `A`, `B`, `C`, `D`, `E` (see [Factory AI](#factory-ai-the-observer)) |
| `CriterionState` | `met`, `unmet`, `unknown` |
| `RecoveryAction` | `remove_finished_worktrees`, `restart_worker`, `sleep_wake_worker`, `switch_runtime`, `retry_reads_and_reconnect` |
| `RecoveryOutcome` | `improved`, `partial`, `unchanged` |
| `AttemptStage` | `task`, `pre_merge` |
| `AttemptOutcome` | `passed`, `failed`, `environment`, `cancelled`, `running` |
| `PauseReason` | `person`, `pane_closed` |
| `ObserverMode` | `manual`, `assist`, `autonomous` |
| `DiagnosisSource` | `user_turn`, `last_answer`, `screen` |

## Performance boundaries

The engine runs on its own thread, `herdr-core-factory`, and the judgments run on a second, `herdr-core-factory-judge`.
Dropping the core stops the engine first, then ends the verify runs, the judgment in flight and any `git` or `gh` call under way, so nothing the Factory started outlives the daemon.
The engine takes the runtime lock only to read owned data (a delivery projection, a worker's pane state, the snapshot's agent rows) or to hand the core a request (a sleep, a wake, a kit read), and does no file, network or subprocess work while holding it.
Every `git` and `gh` call runs on the engine thread with a 120 second deadline, and a verify bundle runs in an owned process tree the engine polls, so a long run never waits on the thread.
The Factory adds two snapshot sections, `factory` and `factory_task`, which carry the summary and the open Task page only when they change (ARCHITECTURE.md, The Factory host), and a Task changes no terminal, tab or scroll path.

A machine without a Factory runs the thread's two-second wait and nothing else.
With Factories, each two-second tick does bounded work:

- It reads the delivery projection once, applies at most 16 letters, and confirms them.
- It applies finished judgments and polls each verifying Task: a CI Factory makes one `gh api` read per verifying Task per tick, and a verify Factory polls the runner with no subprocess.
- It checks each running Task's worker once and each `landed` Task's main verification once.
- It reads outside work once per `outside_read_minutes` for each Factory, with back-off, and reads the default branch head only when that read happens or main is broken.
- It walks each Factory's Tasks a few times to move them, with one external call at most per Task step.
- It reads each open, unpaused Factory's recovery holds once and runs a step only when its time has come, and asks GitHub whether a blocked Factory's access is back once every 5 minutes.
- It settles pending sleeps and wakes and republishes the open Factories as recipients.

The work is bounded: 500 activity lines per Factory, 200 per Task, 20 open command to-dos per Factory, 32 queued commands, 16 waiting judgments per Factory, one judgment in flight, 256 queued verify runs and one running, 16 held letters per woken worker, 4,096 remembered letters, 20,000 events and 5,000 Tasks per Factory, a 256 KiB kept record, a 6 KiB prompt, a 48 KiB judgment input, a 256 KiB request and a 4 MiB attachment.
Crossing a cap is a reported failure and never a larger number.
A judgment and a worker start are the only places the engine waits on a provider or on Herdr, and each retries on a timer and never on a loop.
