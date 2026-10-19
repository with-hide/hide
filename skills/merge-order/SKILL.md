---
name: merge-order
description: Decide where work starts beside the open pull requests of the hide repository, whether a pull request should stack on another, and which green pull request merges first, by running `python3 scripts/merge-order.py` and acting on its answer. Use when the user invokes "/merge-order", or says "머지 순서", "어느 브랜치에서 시작", "스택 쌓아", "어떤 PR부터 머지", "stack this on", "merge order", "which PR merges first". Not for whether one pull request may merge now; that is `scripts/premerge-check.py`.
---

# merge-order

The rules belong to `CONTRIBUTING.md`, "Starting work beside open pull requests" and "Merging into main"; read them there when a case is not below.
`scripts/merge-order.py` computes them from git and GitHub as they are now and only reads.
This skill says when to run it and what to do with the answer.

## Before the work has code

Decide without the script.
Start on an open pull request's branch only when the work needs what it adds (an API, state, schema or contract), and open the pull request with `--base <that branch>`.
Work that only might overlap one starts from main, like work that does not.

## After the first push

From the branch's worktree:

```sh
python3 scripts/merge-order.py <number>          # a pull request
python3 scripts/merge-order.py --branch <name>   # a branch with no pull request yet
```

Act on the `Do:` line, and only on it:

| `Do:` says | Do |
| --- | --- |
| Stay as you are | Nothing now. A documents-only conflict it names is resolved once, by merging main right before the merge. |
| Stack on #A: `gh pr edit …; git fetch … && git merge origin/<branch>` | Run it, resolve the conflict once, push. Merge; never rebase. |
| #A is risky: do not stack | Stay. After #A merges, merge main once. |
| the stack already has 3 layers | Stay. Run the script again after that stack's bottom merges. |
| neither is green yet | Stay. Run it again when either turns green. |
| conflicts with main | On the bottom layer, merge main. On an upper layer, merge only the layer below. |

In a stack, an upper layer merges only the layer directly below it, never main; the bottom layer alone merges main.

## Merging (the lead only)

Only the reviewer merges; a delegated session never does.

```sh
python3 scripts/merge-order.py
```

It lists the green pull requests that may merge now, the one that breaks the fewest open pull requests first, with each stack's upper layers under the layer they sit on.
Take them in that order and use the installed `ship` skill for each approved merge.
Its merge command must run `python3 scripts/premerge-check.py <number>` as the final premerge check and pin the merge to the checked head, as `CONTRIBUTING.md` requires.

- Rows that break as many: the more important work first; the script then puts the stack with more green layers first, then the one green longer.
- A stack merges from the bottom up, as far as its layers are green; GitHub moves the next layer's base to main when the one below merges.
- Run `merge-order.py` again after each merge: the counts change.

## When it cannot answer

`undecided:` and exit status 2 mean git or GitHub could not answer (`gh` not logged in, a fetch that failed, a head that moved while it was read).
Fix that and run it again; never guess an order instead.
Add `--json` when another tool reads the answer.
