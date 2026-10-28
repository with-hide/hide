#!/bin/bash
# The release candidate runner's job-started hook (docs/BUILD.md, The
# candidate's Mac build). The runner's worker starts it once a job is
# assigned, after downloading the job's actions and before any of its steps.
#
# Any workflow of a repository may ask for a runner's labels, and the Team
# plan's runner groups can be limited to repositories but not to workflows, so
# the runner refuses every job but the release candidate's own, dispatched on
# main. The values come from the job GitHub assigned: a hook's environment is
# the runner's own plus the job's `github` context, and a workflow's `env:`
# never reaches it. A pull request, a fork or a branch push names its own
# event, ref and workflow ref, never these.
#
# Failing the hook is not enough: the worker would still run the job's
# `if: always()` and `if: failure()` steps and its actions' `pre` steps, and a
# cancel runs `always()` steps too. So a refused job's worker, this hook's
# parent, is killed outright; the listener then completes the job as failed
# and waits for the next one.
set -euo pipefail
allowed="with-hide/hide/.github/workflows/release-candidate.yml@refs/heads/main"
if [[ "${GITHUB_REPOSITORY:-}" == with-hide/hide && "${GITHUB_EVENT_NAME:-}" == workflow_dispatch
  && "${GITHUB_REF:-}" == refs/heads/main && "${GITHUB_WORKFLOW_REF:-}" == "$allowed" ]]; then
  exit 0
fi

refusal="refused ${GITHUB_EVENT_NAME:-an unnamed event} of ${GITHUB_WORKFLOW_REF:-an unnamed workflow} in ${GITHUB_REPOSITORY:-an unnamed repository}"
echo "::error::this runner builds only $allowed dispatched on main; $refusal"
logger -t hide-candidate-runner "$refusal" || true
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
    echo "::error::the hook's parent $worker is not the runner's worker, so the refused job could not be stopped"
    logger -t hide-candidate-runner "could not stop the refused job: parent $worker is not Runner.Worker" || true
    ;;
esac
exit 1
