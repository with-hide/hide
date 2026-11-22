# hide wait
Leave "carry on once this is done" to Hide and rest: Hide checks the target again on its own, and when it is met, fails or passes its deadline you get one Hide notice that tells you which, and wakes you if you were asleep. A Conductor waits inside its plan with a waiting node (`kind: wait`, `hide mission help`); every other agent waits with this command.

```
hide wait pr <number> [--ci] [--for <30m|2h|3d>]
hide wait branch <name> [--for <30m|2h|3d>]
hide wait mission <id> [--node <key>] [--for <30m|2h|3d>]
hide wait list        hide wait cancel <wait-id>        hide wait help
```

- `pr <number>` waits until the pull request merges (met) or closes without a merge (failed). With `--ci` it waits until every check on its newest commit has ended: all passed is met, any failed is failed and the notice names the checks that failed; a new push waits for the new commit's checks, and a merge before then is met.
- `branch <name>` waits until the branch's own commits are in the project's default branch; a branch just cut from it waits for its first commit of its own.
- `mission <id>` waits until that mission closes as `done` (met) or `cancelled` (failed); one closed as `merge_wait` is still waited on. With `--node <key>` it waits until that node is done (a code node counts once its pull request is ready, not when it reaches main) or is dropped (failed).
- Run it from a checkout of the project the pull request or branch is in. A mission or node must be on this machine; your own mission, or one that waits on it, is refused (`wait_self`).
- Every wait has a deadline: 7 days unless `--for` gives one between 1h and 30d. When it passes, the wait ends as expired.
- Asking for the same target again answers with the same wait (`"existing": true`) and makes no new one. You hold at most 8 open waits and the machine 256 (`wait_agent_full`, `wait_full`); `hide wait cancel <id>` lets one go.
- A project whose code host cannot read a pull request by its number (GitLab today) refuses `pr` and `--ci` (`wait_unsupported`); wait on its branch or a mission instead.
- While you wait and rest off the operator's screen with no letter unread, Hide puts you to sleep, as after a report; the operator sees the wait under your row. The notice wakes you with your conversation; read its first line and carry on. The wait ends with its notice, so wait again for the next thing.
- When you end (not sleep), your waits end with you and nobody is told.
