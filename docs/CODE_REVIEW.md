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
- Two implementations of one boundary (two engines, two hosts, two systems) that each carry their own copy of a shared rule instead of calling one place.
- A fix that adds a special case for the instance in front of it (a second string match, threshold, delay or hardcoded value for a decision that already has one), above all when the same kind of bug has come back before: fix the common cause and model the state instead of adding a case.

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
- Whatever grows with use (a queue, a cache, a list, a count of processes) has a cap, and crossing it is a reported failure rather than a larger number.
- Work added to a high-frequency path says its cost per input, its fan-out and its bound (`docs/PERFORMANCE_TESTING.md`).

### 6. Tests

- A test asserts what a caller observes, with the expected answer taken from outside the implementation.
- A test that costs more to keep than it returns, or that would still pass with the fix reverted, is a finding.
- A bug fix comes with a test that fails without it and covers the kind of bug, not only the reported case, so the same bug cannot come back unseen; a fix without one is a finding.
- A change that stores a device or machine id adds a step to the release-state scenario (`scripts/release_state/scenario.py`) and regenerates the folders it applies to ([TESTING.md, Upgrading a released state folder](TESTING.md#upgrading-a-released-state-folder)); without one, the first start after an upgrade can hold the device unconnected and no test sees it, which is an ordinary finding.
- `docs/TESTING.md` owns which layer to test at and how to keep a test deterministic.

## Writing a finding

- Name the perspective, the file and line, and the failure scenario or the duplicated decision with both of its places.
- Grade it:
  - **serious**: a bug, data loss, a security or boundary violation;
  - **ordinary**: fragmentation, a structure a planned change will have to rewrite, a missing owner or cap, or a gap the next author would have to guess at;
  - **minor**: anything else worth saying.
- Serious and ordinary findings block the merge until fixed or until the person who owns the decision accepts them in writing on the pull request.
  Minor findings do not block; the author fixes them or lists them as follow-ups.
- A review round after a fix reads only what changed since the round before and says whether each earlier finding is closed.

## Work spread over several pull requests

Fragmentation is easiest to miss when each pull request is reviewed alone.
When one piece of work lands as a series, such as a mission's nodes or a stack, the last review of the series reads the combined diff of the whole series against where it started and looks for the same decision made in two of its pull requests.

## Where reviews start from

- `AGENTS.md` routes every reviewer here.
- The file list a pull request's `Review` section names (runtime mutex, snapshot wire, ownership, engine contract, failure path, high-frequency path) is where a reviewer spends the most time; the perspectives above apply to the whole diff.
- `docs/TESTING.md`, `docs/VERIFICATION.md` and `docs/PERFORMANCE_TESTING.md` own the detail behind perspectives 5 and 6.
