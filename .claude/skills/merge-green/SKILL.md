---
name: merge-green
description: Merge the hide repository's green pull requests as the lead, by adding them to the merge queue in merge-order's order without asking again, then remove each merged branch's worktree. Use when the operator says "초록이면 머지", "CI 통과한 거 머지해", "그린 PR 머지", "merge the green ones", or asks how a green pull request gets into main. A delegated session never runs this.
---

# merge-green

The rules belong to `CONTRIBUTING.md`, Merging into main and Which green pull request merges first; this skill is the order the operator asks for them in.
Only the lead merges, and only when the operator asked for merges in this session; a delegated session reports and stops.
That request covers every pull request below: do not ask again before each one.

## 1. Main is green

```sh
gh run list --branch main --event push --workflow verify --limit 3
```

When the latest completed push run failed, add nothing to the queue and follow `CONTRIBUTING.md`, When main's push run fails; the queue does not look at main's own run.

## 2. The order

```sh
python3 scripts/merge-order.py
```

Take the pull requests it lists in that order and none it leaves out.
Skip one where a fix you or the operator asked for has not landed on its head yet.

## 3. Add each to the merge queue

```sh
gh pr view <number> --json headRefOid --jq .headRefOid
gh pr merge <number> --match-head-commit <head>
```

Give no merge method: the queue merges with a merge commit.
GitHub refuses the call when the head moved after you read it or its `verify` has not passed; keep the reason for the report and go on to the next.
Add them one after another without waiting: the queue checks each on main with the ones ahead of it, up to five at once, and merges the ones that pass.
Run `merge-order.py` again before adding a pull request whose stack layer below has just merged.

## 4. Wait for the queue

```sh
gh pr view <number> --json state,mergedAt
```

A pull request whose group check fails leaves the queue; its page names the failed `verify` run.
Keep that run for the report and do not add it again: its author merges main into the branch and fixes it, as `CONTRIBUTING.md`, Merging into main, says.

## 5. Remove the merged worktree

After `gh pr view <number> --json state` says `MERGED`, find the worktree whose branch is the pull request's head branch, and remove it through Hide, which closes its workspace and panes before Git removes the folder:

```sh
git worktree list --porcelain
hide workspace remove <path> --delete-branch
```

Never use a raw `git worktree remove` for it: Git deletes the folder and leaves the checkout's workspace open, which comes back after a restart pointing at a directory that is gone.
Run it without `--force`: when it refuses (`worktree_remove_needs_force` for uncommitted or untracked files, `worktree_remove_branch_unmerged` for a branch with commits the base lacks, `worktree_remove_agent_working` for an agent still at work there), leave the worktree and name it and the refusal in the report.
It refuses the root checkout and your own working directory by itself.

## 6. Watch main and report

Watch the first completed push run on main that contains the merges, as `CONTRIBUTING.md`, Merging into main, asks.
Report in a few lines: what merged, what left the queue and why, which worktrees were removed and which were kept and why, and main's push run result.
