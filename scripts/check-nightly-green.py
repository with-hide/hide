#!/usr/bin/env python3
"""Whether a release candidate may start: the newest scheduled nightly on
main did not fail, and no `nightly-failure` or `agent-canary` issue is open
but the agent CLI support record, which stays open for good
(CONTRIBUTING.md, Releases; docs/TESTING.md, Nightly failures).

Usage:
  check-nightly-green.py [--repo OWNER/REPO] [--allow-red-nightly REASON]

It reads GitHub through `gh api` with the caller's token and exits 1 naming
each reason a candidate may not start. `--allow-red-nightly` with a reason
lets an urgent release through: the reasons are printed as warnings beside
it, and it exits 0. An empty reason is no reason.
"""

import argparse
import json
import subprocess
import sys

REPOSITORY = "with-hide/hide"
WORKFLOW = "nightly.yml"
LABELS = ("nightly-failure", "agent-canary")
# The always-open record issue `scripts/agent-canary-record.py` keeps under
# the `agent-canary` label; its title is the record's key there too.
RECORD_TITLE = "Agent CLI support record"


def gh(path):
    done = subprocess.run(["gh", "api", path], capture_output=True, text=True, encoding="utf-8", timeout=60)
    if done.returncode:
        raise RuntimeError(f"gh api {path} exited {done.returncode}: {done.stderr.strip()[:300]}")
    return json.loads(done.stdout)


def refusals(repo, call=gh):
    """Each reason a candidate may not start, as one line."""
    found = []
    runs = call(f"repos/{repo}/actions/workflows/{WORKFLOW}/runs?branch=main&event=schedule&status=completed"
                "&per_page=1")["workflow_runs"]
    if runs and runs[0]["conclusion"] == "failure":
        run = runs[0]
        found.append(f"the newest scheduled nightly on main failed: {run['html_url']} "
                     f"({run['head_sha'][:12]}, {run['created_at']})")
    for label in LABELS:
        issues = call(f"repos/{repo}/issues?labels={label}&state=open&per_page=100")
        if len(issues) == 100:
            raise RuntimeError(f"100 or more open {label} issues; read them before a release")
        for issue in issues:
            if "pull_request" in issue or (label == "agent-canary" and issue["title"] == RECORD_TITLE):
                continue
            found.append(f"open {label} issue #{issue['number']}: {issue['title']} ({issue['html_url']})")
    return found


def main(argv=None, call=gh):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=REPOSITORY)
    parser.add_argument("--allow-red-nightly", default="", help="why this release goes out with a red nightly")
    args = parser.parse_args(argv)
    reason = args.allow_red_nightly.strip()
    try:
        found = refusals(args.repo, call)
    except (RuntimeError, KeyError, ValueError) as error:
        print(f"::error title=nightly gate::could not read the nightly's state: {error}")
        return 1
    if not found:
        print("The newest scheduled nightly on main did not fail, and no nightly-failure or agent-canary issue is open.")
        return 0
    if reason:
        print(f"::warning title=nightly gate::starting with a red nightly: {reason}")
        for line in found:
            print(f"::warning title=nightly gate::{line}")
        return 0
    for line in found:
        print(f"::error title=nightly gate::{line}")
    print("A release candidate starts once these are fixed, or with allow_red_nightly naming why it cannot wait.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
