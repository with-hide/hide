#!/bin/bash
# The release candidate runner's job-started hook (docs/BUILD.md, The
# candidate's Mac build). The runner starts it once a job is assigned and
# before any of the job's steps runs; a nonzero exit fails the job there.
#
# Any workflow of a repository may ask for a runner's labels, and the Team
# plan's runner groups can be limited to repositories but not to workflows, so
# the runner refuses every job but the release candidate's own, dispatched on
# main. The values come from the job GitHub assigned: a hook's environment is
# the runner's own plus the job's `github` context, and a workflow's `env:`
# never reaches it. A pull request, a fork or a branch push names its own
# event, ref and workflow ref, never these.
set -u
allowed="with-hide/hide/.github/workflows/release-candidate.yml@refs/heads/main"
if [[ "${GITHUB_REPOSITORY:-}" == with-hide/hide && "${GITHUB_EVENT_NAME:-}" == workflow_dispatch
  && "${GITHUB_REF:-}" == refs/heads/main && "${GITHUB_WORKFLOW_REF:-}" == "$allowed" ]]; then
  exit 0
fi
echo "::error::this runner builds only $allowed dispatched on main; refused ${GITHUB_EVENT_NAME:-an unnamed event} of ${GITHUB_WORKFLOW_REF:-an unnamed workflow} in ${GITHUB_REPOSITORY:-an unnamed repository}"
exit 1
