#!/usr/bin/env bash
# One packaged-app test run, retaining each engine's first outcome and artifacts.
set -euo pipefail
cd "$(dirname "$0")/.."

suite=${1:?expected suite}
engine=${2:?expected engine}
shift 2
case "$suite" in window|update|macos) ;; *) exit 2 ;; esac
case "$engine" in herdr|own) ;; *) exit 2 ;; esac
output="$PWD/agents/runs/package-e2e/$suite/$engine"
mkdir -p "$output/screenshots"
export HIDE_E2E_ENGINE="$engine" HIDE_E2E_SCREENSHOT_DIR="$output/screenshots"
options=()
if [[ "$engine" == own ]]; then options=(--grep-invert @herdr); fi
prefix=()
if [[ "${RUNNER_OS:-}" == Linux ]]; then prefix=(xvfb-run -a); fi
# Never attribute a preceding run's report to a build that failed before Playwright.
rm -f desktop/e2e-report.json
status=0
"${prefix[@]}" bash scripts/verify-web.sh desktop e2e "$@" "${options[@]}" --output "$output/test-results" || status=$?
if [[ -f desktop/e2e-report.json ]]; then mv desktop/e2e-report.json "$output/report.json"; fi
exit "$status"
