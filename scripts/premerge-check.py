#!/usr/bin/env python3
"""Answer whether a pull request may merge into main as main is now.

CONTRIBUTING.md, "Merging into main", owns the policy; this is its check.

A pull request's `verify` ran on its merge with main as main was when the run
started, and main moves on after that. Updating a green branch only so that
`verify` runs again is what slowed busy days: every merge sent every other
green pull request back through a full run and the runner queue. What two
changes that each passed break together without a conflict is mostly a
compile, type or baseline failure (on 2026-10-05 one pull request removed an
export another one imported, and both had passed), and those checks finish in
about two minutes on the merge result itself. A behavior only the two changes
together break is left to main's push run, which plans every lane.

Usage: python3 scripts/premerge-check.py <pull request number>

Exit status: 0 the pull request may merge now, and the merge command is
printed; 1 it may not, and the output says what to do instead; 2 the check
could not decide, and the output says why.
"""
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from pull_requests import BASE, MERGE_IDENTITY, ROOT, Undecided, git, gh_json, latest_verify, verify_runs
from pull_requests import merge_tree as merged_tree

# The checks where two changes that each passed can fail together without a
# conflict, and that finish in minutes: the type and compile checks, the
# baselines one change can grow while another deletes, and the structural
# checks of `pr.yml`'s policy lane. The unit and end-to-end suites stay with
# main's push run, because they take the runner time this check exists to
# save. Each runs in the merge result's own worktree, from the folder named.
CHECKS = (
    ("node modules", ".", ["bash", "scripts/verify-web.sh", "install"]),
    ("web typecheck", ".", ["bash", "scripts/verify-web.sh", "web", "typecheck"]),
    ("desktop typecheck", ".", ["bash", "scripts/verify-web.sh", "desktop", "typecheck"]),
    ("web e2e size budget", "web", ["node", "scripts/check-e2e-test-size.mjs", "e2e"]),
    ("desktop e2e size budget", "desktop", ["node", "../web/scripts/check-e2e-test-size.mjs", "e2e"]),
    ("rust fmt and clippy", ".", ["bash", "scripts/verify-cargo.sh", "lint"]),
    ("harness ignore anchor", ".", ["bash", "scripts/check-harness-ignore-anchor.sh"]),
    ("capability readers off lock", ".", ["bash", "scripts/check-capability-readers-off-lock.sh"]),
    ("no workstation identity", ".", ["bash", "scripts/check-no-workstation-identity.sh"]),
    ("worktree removal boundary", ".", ["bash", "scripts/check-worktree-removal-boundary.sh"]),
    ("herdr pin single source", ".", ["zsh", "scripts/check-herdr-pin-single-source.sh"]),
    ("core touches no machine", ".", ["python3", "scripts/check-core-touches-no-machine.py"]),
    ("ai prompt language", ".", ["python3", "scripts/check-ai-prompt-language.py"]),
)

# A check that runs this long has stalled; the answer is undecided, not a pass.
CHECK_TIMEOUT_SECONDS = 20 * 60
# Lines of a failed check's output shown; the rest is the command's to rerun.
FAILURE_TAIL_LINES = 60

class Refused(Exception):
    """The pull request may not merge now; the message says what to do instead."""


def open_pull(number):
    """An open, ready pull request into main: its head commit and the issues it closes."""
    pull = gh_json("pr", "view", number, "--json", "state,isDraft,baseRefName,headRefOid,closingIssuesReferences")
    if pull["state"] != "OPEN":
        raise Refused(f"#{number} is {pull['state'].lower()}, not open.")
    if pull["isDraft"]:
        raise Refused(f"#{number} is a draft; its lanes run once it is marked ready for review.")
    if pull["baseRefName"] != BASE:
        raise Refused(f"#{number} merges into {pull['baseRefName']}, not {BASE}.")
    return pull["headRefOid"], pull["closingIssuesReferences"]


def issue_note(number, closing):
    """A reminder, never a refusal: Hide and GitHub relate a pull request to an
    issue only through a closing keyword, and some pull requests finish none."""
    if closing:
        return None
    return (f"note: #{number} closes no issue. If it finishes one, put `Closes #<issue>` on the body's Closes line "
            "before merging (a body edit reruns `verify` as `policy` alone, so the merge waits for that run); with no issue, merge as it is.")


def require_verify_passed(runs):
    """Raise unless the latest `verify` run on the head passed; return its link."""
    latest = latest_verify(runs)
    if latest is None:
        raise Refused("No `verify` run exists for the head yet; push it, or wait for the run to start.")
    if latest["status"] != "completed":
        raise Refused(f"`verify` is still running on the head: {latest['html_url']}")
    if latest["conclusion"] != "success":
        raise Refused(f"`verify` ended {latest['conclusion']} on the head: push a fix, or rerun a cancelled run. {latest['html_url']}")
    return latest["html_url"]


def merge_tree(main, head, repo):
    """The tree of main merged with head; a conflict refuses the merge."""
    tree, conflicts = merged_tree(main, head, repo)
    if conflicts:
        raise Refused("The branch conflicts with main in " + ", ".join(conflicts) +
                      ". Merge main into the branch, resolve it, push, and wait for `verify`.")
    return tree


def run_check(name, directory, command, out):
    started = time.monotonic()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("HERDR_", "HIDE_", "HCOORD_"))}
    process = subprocess.Popen(command, cwd=directory, env=environment, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               start_new_session=True)
    try:
        output, _ = process.communicate(timeout=CHECK_TIMEOUT_SECONDS)
    except BaseException as error:
        # The check owns a process group (cargo's rustc, pnpm's node); end all of it.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        if isinstance(error, subprocess.TimeoutExpired):
            raise Undecided(f"{name} ran past {CHECK_TIMEOUT_SECONDS // 60} minutes: `{' '.join(command)}`")
        raise
    seconds = round(time.monotonic() - started)
    if process.returncode != 0:
        out.write("".join(output.splitlines(keepends=True)[-FAILURE_TAIL_LINES:]))
        raise Refused(f"{name} fails on the merge with main (`{' '.join(command)}`, {seconds}s). "
                      "Merge main into the branch and fix it there; `verify` runs again on the push.")
    out.write(f"ok  {name} ({seconds}s)\n")


def remove_worktree(worktree, repo):
    result = git("worktree", "remove", "--force", str(worktree), cwd=repo, check=False)
    if result.returncode != 0:
        return f"could not remove {worktree}: {result.stderr.strip()}; remove it with `git worktree remove --force {worktree}`"
    return None


def check_merge(main, head, checks, worktrees, label, repo=ROOT, out=sys.stdout):
    """Raise Refused unless `checks` pass on main merged with head."""
    if git("merge-base", "--is-ancestor", main, head, cwd=repo, check=False).returncode == 0:
        out.write("The branch contains main, so `verify` already ran on this merge.\n")
        return
    tree = merge_tree(main, head, repo)
    commit = git("commit-tree", tree, "-p", main, "-p", head, "-m", f"premerge check of {label}",
                 cwd=repo, env=MERGE_IDENTITY).stdout.strip()
    worktree = Path(worktrees) / f"premerge-{label}-{commit[:12]}"
    if worktree.exists():
        raise Undecided(f"{worktree} exists: a check of this merge is running, or one was cut short. "
                        f"Remove it with `git worktree remove --force {worktree}` and run again.")
    Path(worktrees).mkdir(parents=True, exist_ok=True)
    git("worktree", "add", "--quiet", "--detach", str(worktree), commit, cwd=repo)
    try:
        for name, directory, command in checks:
            run_check(name, worktree / directory, command, out)
    except BaseException:
        # The check's own failure is the answer; a failed removal is reported beside it.
        problem = remove_worktree(worktree, repo)
        if problem:
            print(problem, file=sys.stderr)
        raise
    problem = remove_worktree(worktree, repo)
    if problem:
        raise Undecided(problem)


def worktree_folder():
    """`<checkout>.worktrees` beside the primary checkout, where every worktree of it lives."""
    common = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir").stdout.strip())
    checkout = common.parent
    return checkout.parent / f"{checkout.name}.worktrees"


def main(argv):
    if len(argv) != 2 or not argv[1].isdigit():
        print(f"usage: {argv[0]} <pull request number>", file=sys.stderr)
        return 2
    number = argv[1]
    # A terminated check still ends its process group and removes its worktree.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(2))
    try:
        head, closing = open_pull(number)
        verify = require_verify_passed(verify_runs(head))
        print(f"`verify` passed on {head[:12]}: {verify}")
        git("fetch", "--quiet", "--no-tags", "origin", BASE, f"pull/{number}/head")
        if git("cat-file", "-e", f"{head}^{{commit}}", check=False).returncode != 0:
            raise Undecided(f"#{number}'s head {head[:12]} was not fetched; it moved while being read, so run again.")
        main_head = git("rev-parse", f"origin/{BASE}").stdout.strip()
        print(f"Checking #{number} merged with {BASE} at {main_head[:12]}.")
        check_merge(main_head, head, CHECKS, worktree_folder(), f"pr{number}")
    except Refused as refusal:
        print(f"refused: {refusal}")
        return 1
    except Undecided as reason:
        print(f"undecided: {reason}", file=sys.stderr)
        return 2
    note = issue_note(number, closing)
    if note:
        print(note)
    print(f"#{number} may merge now: gh pr merge {number} --merge --match-head-commit {head}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
