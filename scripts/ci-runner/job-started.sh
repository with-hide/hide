#!/bin/bash
# The Mac mini CI runner's job-started hook (docs/BUILD.md, The Mac mini CI
# runner). The runner's worker starts it once a job is assigned, after
# downloading the job's actions and before any of its steps.
#
# `pr.yml` sends a job here only for this repository's own branches, but any
# workflow may ask for a runner's labels, and a pull request from a fork runs
# the fork's copy of the workflow. So the runner itself takes only jobs of
# with-hide/hide whose code is with-hide/hide's: every pull request event is
# checked against the head repository the event file names, and a job whose
# event file cannot be read is refused. Pull requests from outside
# contributors also wait for a maintainer's approval before any workflow runs.
#
# Failing the hook is not enough: the worker would still run the job's
# `if: always()` and `if: failure()` steps and its actions' `pre` steps. So a
# refused job's worker, this hook's parent, is killed outright, as the
# candidate runner's hook does (scripts/candidate-runner/job-started.sh); the
# listener completes the job as failed and waits for the next one. Where the
# parent is not the worker, the whole runner stops instead.
set -euo pipefail
repository=with-hide/hide
runner="${HIDE_CI_RUNNER:-}"

refusal=""
if [[ "${GITHUB_REPOSITORY:-}" != "$repository" ]]; then
  refusal="a job of ${GITHUB_REPOSITORY:-an unnamed repository}"
else
  case "${GITHUB_EVENT_NAME:-}" in
    "") refusal="a job with no event" ;;
    pull_request*)
      head="$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["pull_request"]["head"]["repo"]["full_name"])' \
        "${GITHUB_EVENT_PATH:-}" 2>/dev/null || true)"
      [[ "$head" == "$repository" ]] || refusal="a pull request from ${head:-an unreadable head repository}"
      ;;
  esac
fi
if [[ -z "$refusal" && -n "$runner" ]]; then
  # The job's HOME and TMPDIR, which the completed hook emptied.
  mkdir -p "$runner/home" "$runner/tmp"
  exit 0
fi
[[ -n "$refusal" ]] || refusal="a job with no HIDE_CI_RUNNER"

refusal="refused $refusal (${GITHUB_EVENT_NAME:-no event} of ${GITHUB_WORKFLOW_REF:-an unnamed workflow})"
echo "::error::this runner takes only $repository's own code; $refusal"
logger -t hide-ci-runner "$refusal" || true
# What the job downloaded goes too, as after any job; the completed hook will
# not run once the worker is gone.
bash "$(dirname "${BASH_SOURCE[0]}")/job-completed.sh" || true
worker="$PPID"
case "$(ps -o command= -p "$worker")" in
  */Runner.Worker\ spawnclient*)
    # A moment for the worker to send the line above to the job's log.
    sleep 2
    kill -KILL "$worker"
    ;;
  *)
    # The refused job's later steps cannot be stopped alone, so the runner
    # stops taking jobs until its administrator has looked: its registration
    # is set aside, so the listener the service starts again finds none, and
    # every worker and listener of this runner is killed.
    echo "::error::the hook's parent $worker is not the runner's worker; the runner stops taking jobs until its administrator restores ${runner:-its folder}/.runner"
    logger -t hide-ci-runner "parent $worker is not Runner.Worker; stopping the runner" || true
    if [[ -n "$runner" && -f "$runner/.runner" ]]; then mv -f "$runner/.runner" "$runner/.runner.stopped"; fi
    if [[ -n "$runner" ]]; then
      # By pid from ps: pkill on macOS leaves out its own ancestors, the worker among them.
      pids="$(ps -U "$(id -u)" -o pid=,command= | grep -E "$runner/bin[^/]*/Runner\.(Worker|Listener)( |$)" | awk '{ print $1 }' || true)"
      if [[ -n "$pids" ]]; then kill -KILL $pids || true; fi
    fi
    ;;
esac
exit 1
