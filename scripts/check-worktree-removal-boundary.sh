#!/usr/bin/env bash
set -euo pipefail
# Searches use `git grep`, never `rg`: ripgrep is not on the CI runner.

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# The host's confirmed removal and owned-task retirement share the same
# worktree and branch deletion helpers on every device.
# Force is the operator's choice in the delete confirmation and nothing else's:
# `--force` only when they ticked Discard, `-D` only when they were told the
# branch is unmerged and ticked it, or explicitly discarded their task.
# Completed tasks keep their branch. The Overview cleanup never forces.
executor=hide-host/src/worktrees.rs
cleanup=herdr-core/src/worktree_cleanup.rs
fail() {
    echo "$1" >&2
    exit 1
}

# One `--force`, pushed only behind the removal helper's `force` argument, and
# that argument is the operator's accepted discard.
[ "$(git grep -cF -- '"--force"' "$executor" | cut -d: -f2)" = 1 ] ||
    fail "git worktree remove --force must appear once, behind remove_worktree's force argument"
git grep -qF -- 'arguments.push("--force");' "$executor" ||
    fail "--force must be added only by remove_worktree when force is set"
# The confirmed removal goes through `remove_worktree_stopping`, which keeps why
# Git stopped for the dialog; `remove_worktree` wraps the same helper.
git grep -qF -- 'remove_worktree_stopping(root, target, request.discard_changes)' "$executor" ||
    fail "the confirmed removal must force only as far as the operator accepted a discard"
git grep -qF -- 'remove_worktree(root, checkout, discard)' "$executor" ||
    fail "task retirement must force only an explicitly discarded task"

# One `-D` in the shared helper, chosen only by an accepted force policy.
[ "$(git grep -cF -- '"-D"' "$executor" | cut -d: -f2)" = 1 ] ||
    fail "git branch -D must appear once, behind the shared deletion helper"
git grep -qF -- 'let flag = if force { "-D" } else { "-d" };' "$executor" ||
    fail "git branch -D must be chosen only by the accepted force policy"
git grep -qF -- 'delete_branch(root, branch, request.force_delete_branch)' "$executor" ||
    fail "confirmed branch deletion must use the operator's force_delete_branch"
git grep -qF -- 'delete_branch(root, branch, discard)' "$executor" ||
    fail "task branch deletion must use the explicit discard policy"
git grep -qF -- '&["branch", flag, "--", branch]' "$executor" ||
    fail "branch deletion must use the chosen flag"

# The Overview cleanup removes without force and never deletes with -D.
if git grep -qE -- '"branch", "-D"|"--force"|"-f"' "$cleanup"; then
    fail "the Overview cleanup must never force-delete a branch or a worktree"
fi
if git grep -nE -- 'remove_worktree\(' "$cleanup" | grep -vqF -- ', false)'; then
    fail "the Overview cleanup must call remove_worktree without force"
fi

echo "Worktree removal forces only an accepted discard; the Overview cleanup never forces"
