"""What git and GitHub say about a pull request, for `merge-order.py`.

`merge-order.py` says which green pull request goes into the merge queue first
and where new work starts; it reads a pull request's `verify` verdict and its
merge with main from here.
"""
import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = "main"

# A merge commit made only so a merge result can be checked out or merged
# again; it is never pushed, and a reserved domain keeps it from naming anyone.
MERGE_IDENTITY = {
    "GIT_AUTHOR_NAME": "merge order",
    "GIT_AUTHOR_EMAIL": "merge-order@example.invalid",
    "GIT_COMMITTER_NAME": "merge order",
    "GIT_COMMITTER_EMAIL": "merge-order@example.invalid",
}


class Undecided(Exception):
    """git or GitHub could not answer; the message names the command and why."""


def git(*args, cwd=ROOT, check=True, env=None):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True,
                            env=None if env is None else {**os.environ, **env})
    if check and result.returncode != 0:
        raise Undecided(f"`git {' '.join(args)}` failed: {result.stderr.strip()}")
    return result


def gh_json(*args):
    result = subprocess.run(["gh", *args], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise Undecided(f"`gh {' '.join(args)}` failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def merge_tree(ours, theirs, repo=ROOT):
    """The tree of the two commits merged in memory, and the paths that conflict."""
    result = git("merge-tree", "--write-tree", "--name-only", "--no-messages", ours, theirs,
                 cwd=repo, check=False)
    lines = [line for line in result.stdout.splitlines() if line]
    if result.returncode not in (0, 1):
        raise Undecided(f"`git merge-tree` failed: {result.stderr.strip()}")
    return lines[0], lines[1:]


def verify_runs(head):
    """Every workflow run a pull request's push started on the head commit."""
    return gh_json("api", f"repos/{{owner}}/{{repo}}/actions/runs?head_sha={head}&event=pull_request&per_page=100",
                   "--jq", ".workflow_runs")


def latest_verify(runs):
    """The latest `verify` run among a head's runs, which alone decides; None before one starts."""
    runs = [run for run in runs if run["name"] == "verify"]
    return max(runs, key=lambda run: (run["created_at"], run["id"])) if runs else None
