# Code review

This document says what every review of a change in this repository looks at, how a finding is written, and which findings block a merge.
It applies to anyone who reviews: a person, a mission's reviewer agent, a reviewer started by hand, and the author reading their own diff before opening a pull request.
`CONTRIBUTING.md` owns the gates a pull request must pass; this document owns the judgment a gate cannot make.

## Why this exists

Most of the bugs this repository has shipped did not come from a wrong line.
They came from the same decision written in two places that later drifted apart, from a parallel copy of code that already existed, and from structure added for a future that never came.
A review that reads only whether the diff does what its title says misses all three.
So every review covers the perspectives below, and names the perspective of each finding.

## The perspectives

### 1. Correctness

- A bug with a concrete failure scenario: which input or state, and what goes wrong.
- A race, an ordering that can flip, or a lock held across blocking work; nothing blocks under `Mutex<Runtime>` (`docs/ARCHITECTURE.md`).
- A failure swallowed by a default, an empty value or a silent skip; a failure is raised where it happens and reaches someone who can act on it, with the detail in the diagnostic log.
- A retry of the same intent that repeats an external effect instead of converging on the first result.

### 2. Fragmentation

- The same fact or decision computed in two or more places, or the same file read by two separate copies of its parser.
  One decision has one owner; every caller asks that owner.
- A new function that is most of an existing one; extend the existing one for the callers that exist now instead.
- A parallel copy of a component, helper, table or event the project already has: the reviewer searches for a sibling (`rg`) before approving new code that does the same job, and names the one to reuse.
- Two implementations of one boundary (two engines, two hosts, two systems) that each carry their own copy of a shared rule instead of calling one place.
- A fix that adds a special case for the instance in front of it (a second string match, threshold, delay or hardcoded value for a decision that already has one), above all when the same kind of bug has come back before: fix the common cause and model the state instead of adding a case.
- A branch that sends a device or remote case down a weaker path than this machine's (an opt-out flag, a skipped check, a missing limit) is a finding unless `docs/INSTALL.md`, What each system supports, names the gap in its cell and a device-lane test holds it; a node link that could skip its silence limit kept a device's panes from coming back (with-hide/hide#2882).

### 3. Simplicity

- An extension point, setting, indirection or trait with one implementation that nothing needs today.
- A general mechanism built for a case that has not arrived.
- A change that makes a path more complex than the thing it replaces or wraps.
  When two designs meet the requirement, the simpler one wins unless the review can name the requirement the other one meets.

### 4. Module boundaries

- One part does one job; a component that both decides and transports, or both renders and queries, is split.
- Code outside a boundary that knows what is behind it (for the pane engine, `docs/ARCHITECTURE.md` says where that boundary is).
- New code that a planned removal will have to rewrite: a foundation laid on something already scheduled to go.
  Transition paths a removal needs are fine; a new feature built on the path being removed is not.

### 5. Ownership and limits

- Whatever the change starts (a process, a thread, a timer, a watcher, a file, a connection) has an owner and ends on every exit path, and when that owner ends.
- A state that waits on another's event (a lock, an answer, a peer's next byte) says what ends it and logs once when it passes its bound; a wait nothing ends is a finding.
- Whatever grows with use (a queue, a cache, a list, a count of processes) has a cap, and it never grows past that cap.
  At the cap, a record that keeps accumulating (agent registrations, the session archive, closed missions) makes room by dropping its oldest finished entries, with a diagnostic, rather than refusing the new one; new work is refused, as a reported failure, only when nothing finished can be dropped ([delivery.md](delivery.md), [ARCHITECTURE.md: Session archive](ARCHITECTURE.md#session-archive)).
- A cap added or changed says in its doc comment or the pull request what it counts and what each path does when it is full ([Reviewing a cap](#reviewing-a-cap)).
- Work added to a high-frequency path says its cost per input, its fan-out and its bound (`docs/PERFORMANCE_TESTING.md`).

### 6. Tests

- A test asserts what a caller observes, with the expected answer taken from outside the implementation.
- A test that costs more to keep than it returns, or that would still pass with the fix reverted, is a finding.
- A mission plan's verification row that rests on a weaker check than its claim needs (a unit test for a flow a user performs, an isolated-app row missing on a node that changes what a user runs) is an ordinary finding; `docs/VERIFICATION.md`, Pick the check by the claim, names the check each claim needs.
- A bug fix comes with a test that fails without it and covers the kind of bug, not only the reported case, so the same bug cannot come back unseen; a fix without one is a finding.
- A change that stores a device or machine id adds a step to the release-state scenario (`scripts/release_state/scenario.py`) and regenerates the folders it applies to ([TESTING.md, Upgrading a released state folder](TESTING.md#upgrading-a-released-state-folder)); without one, the first start after an upgrade can hold the device unconnected and no test sees it, which is an ordinary finding.
- An end-to-end test names the user flow it protects and why a cheaper layer cannot observe it; a spec without both is an ordinary finding.
- A new or changed test comes with its repeat result in the pull request's folded `Tests` block (changed tests times runs, linked to the repeat job); a test with none is an ordinary finding, because a single green run does not show a test is stable.
- A new fixed wait, `sleep` or `timeout:` override names the state it waits for, as [Wait for state, not time](TESTING.md#wait-for-state-not-time) requires; one without it is an ordinary finding.
- `docs/TESTING.md` owns which layer to test at, how to keep a test deterministic and the checklist for a test change ([Reviewing a pull request that adds or changes a test](TESTING.md#reviewing-a-pull-request-that-adds-or-changes-a-test)).

### 7. Every agent, system and host

`AGENTS.md`, Every Agent And Feature On Every System, owns the rule; a review checks the change against it.

- Logic outside an agent's definition file (`hide-agent-adapter/agents/*.toml` and what reads it) that names one agent (`claude`, `codex`, `gemini`...): adding another agent must need a definition and no new branch.
- A macOS, Windows or Linux difference written inline instead of once in its layer (`hide-platform`, `desktop/src/main/`), or a feature that works on this machine's pane and not on a pane of a connected device.
- A gap on a system or a device that has no cell in `docs/INSTALL.md`, What each system supports, with its reason and what would close it.
- `gh`, `glab`, GitHub or GitLab words in core logic outside the host interface: GitLab and a later Bitbucket should need an adapter and nothing else.

### 8. The command line

`hided/AGENTS.md` owns the rule; a review checks the change against it.

- A second name for a thing that has one, or a word used with a second meaning (a topic's name as a new topic's option, an option that takes another value type elsewhere).
- A command under a topic other than the thing it changes, a verb with no line in the registry, or an option that does a verb's job.
- A rename that drops the old spelling, removes or renames an entry, an argument or an envelope key of the contract, or keeps a second copy of the retired words outside the one table (`hide-shell/src/retired.rs`).
- A command word changed in the parser and left in what teaches it (the guide, a skill stub, a `next_action`, the site notes).
- A command only a program calls that its topic's `--help` prints with no reason, or one kept with no caller.

### Reviewing a cap

A reviewer asks these four questions of every cap the change adds or changes; a failed answer is an ordinary finding.

1. What does it count?
   What use adds (checkouts the operator opened, session files), not what use passes (every checkout published, every entry walked).
2. What is in use, and what is finished?
   What is in use is never dropped, and a record ages by when it was last used, not when it was made.
3. What does each path do when it is full?
   A write drops the oldest finished entry and refuses only when nothing finished can go.
   A read, list or walk answers its newest part and says it was cut, in the answer itself; an `Err` or a refusal of the whole read at a cap is a finding while anything finished could go.
   A loop that accepts, spawns or opens ends only when its owner ends; a failure such as a full file table fails that one unit and is reported.
4. Which test fills it?
   It puts one in-use and some finished entries past the cap, and asserts the in-use entry survives, new work is taken and the read says it was cut.

## Writing a finding

- Name the perspective, the file and line, and the failure scenario or the duplicated decision with both of its places.
- Grade it:
  - **serious**: a bug, data loss, a security or boundary violation;
  - **ordinary**: fragmentation, a structure a planned change will have to rewrite, a missing owner or cap, or a gap the next author would have to guess at;
  - **minor**: anything else worth saying.
- Serious and ordinary findings block the merge until fixed or until the person who owns the decision accepts them in writing on the pull request.
  Minor findings do not block; the author fixes them or lists them as follow-ups.
- In a mission, review rounds stop at the limit its verification strength allows (`hide mission help`, Verification strength): when that round fails, Hide opens no further round, and the Conductor, not the operator, owns the decision in three steps.
  1. Only ordinary or minor findings remain: the worker fixes the serious ones in the same pull request, which then merges, and the rest become a follow-up node of the same mission, never an issue alone; the line is skipped naming that node.
     The Conductor may ask the checker for one re-look of the serious fixes first, and its pass passes the line.
  2. A serious finding comes back in the same area: the node goes to a fresh worker of a different model, with a handover note and a list of what not to try again, and its checkers count a fresh set of rounds; a fresh checker alone does not.
  3. The fresh worker reaches the limit too: the Conductor asks the operator.
  Serious findings still block until fixed in the node.
- A review round after a fix reads only what changed since the round before and says whether each earlier finding is closed; it raises a new finding only when it is serious.
- In a mission, a commit that only merges the destination into the branch is not a round: its CI proves the merge and the review that passed before it stands, unless resolving a conflict changed the node's own code (`hide mission help`, While it runs).
- The first round gives every finding at once, each with its grade, so the author fixes them in one pass rather than one round per finding.
- The author fixes a finding together with every other place the same kind of problem appears, not only the line it was found on.
- In a mission, the worker confirms its plan for each serious finding with the checker that raised it before fixing it; the checker answers only OK or what to change, and the worker goes on with its plan when no answer comes in 10 minutes, or with the changes the checker named, without asking again. Ordinary and minor findings are fixed at once.

## Work spread over several pull requests

Fragmentation is easiest to miss when each pull request is reviewed alone.
When one piece of work lands as a series, such as a mission's nodes or a stack, the last review of the series reads the combined diff of the whole series against where it started and looks for the same decision made in two of its pull requests.

## Where reviews start from

- `AGENTS.md` routes every reviewer here.
- The file list in a pull request's folded `Risky files` block (runtime mutex, snapshot wire, ownership, engine contract, failure path, high-frequency path) is where a reviewer spends the most time; the perspectives above apply to the whole diff.
  `contracts/peer-link.json` belongs in that block whenever it changes: a machine of another build reads it, and a field or message an older reader misreads cannot be taken back once two machines are paired ([contracts/README.md](../contracts/README.md#peer-link)).
- The folded `Tests` block of a pull request that adds or changes a test (layer, what the test catches, repeat result, waits) is where a reviewer starts perspective 6; the reviewer checks each line against the diff rather than taking it as written.
- `docs/TESTING.md`, `docs/VERIFICATION.md` and `docs/PERFORMANCE_TESTING.md` own the detail behind perspectives 5 and 6.
