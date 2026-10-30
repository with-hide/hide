---
name: merge-green
description: Merge the hide repository's green pull requests as the lead, in merge-order's order, each through premerge-check under the shared lock, without asking again when the check passes, then remove the merged branch's worktree. Use when the operator says "초록이면 머지", "CI 통과한 거 머지해", "그린 PR 머지", "merge the green ones", or asks for the premerge-check procedure before a merge. A delegated session never runs this.
---

# merge-green

The rules belong to `CONTRIBUTING.md`, Merging into main and Which green pull request merges first; this skill is the order the operator asks for them in.
Only the lead merges, and only when the operator asked for merges in this session; a delegated session reports and stops.
That request covers every pull request that passes below: do not ask again before each merge.

## 1. Main is green

```sh
gh run list --branch main --event push --workflow verify --limit 3
```

When the latest completed push run failed, merge nothing and follow `CONTRIBUTING.md`, When main's push run fails; `premerge-check.py` does not look at main's own run.

## 2. The order

```sh
python3 scripts/merge-order.py
```

Take the pull requests it lists in that order and none it leaves out.
Skip one where a fix you or the operator asked for has not landed on its head yet.

## 3. The check, one at a time on this machine

```sh
lockf -k /tmp/hide-premerge.lock python3 -u scripts/premerge-check.py <number>   # macOS
flock /tmp/hide-premerge.lock python3 -u scripts/premerge-check.py <number>      # Linux
```

Always run it under that lock, which every session on the machine shares; it waits for its turn.
Unlocked checks run side by side: on 2026-10-29 three of them took a 14-core Mac to load 210, and each passed its 20-minute limit and started over.
Never start a second check beside one that is running, yours or another session's.

## 4. Merge

When the check passes, run the `gh pr merge <number> --merge --match-head-commit <head>` line it printed, as printed, without asking.
When it refuses, merge nothing for that pull request, keep its reason for the report, and go on to the next.
Run `merge-order.py` again before the next merge, because each merge changes the order.

## 5. Remove the merged worktree

After `gh pr view <number> --json state` says `MERGED`, find the worktree whose branch is the pull request's head branch:

```sh
git worktree list --porcelain
git worktree remove <path>
git branch -D <branch>
```

Use `git worktree remove` without `--force`: when it refuses for uncommitted or untracked files, leave the worktree and name it in the report.
Leave it too when it is the root checkout, your own working directory, or an agent is still running in it (`hide agent list`).

## 6. Watch main and report

Watch the first completed push run on main that contains the merges, as `CONTRIBUTING.md`, Merging into main, asks.
Report in a few lines: what merged, what was refused and why, which worktrees were removed and which were kept and why, and main's push run result.
