#!/usr/bin/env bash
# One packaged-app test run, retaining its first outcome and artifacts.
set -euo pipefail
cd "$(dirname "$0")/.."

suite=${1:?expected suite}
shift
case "$suite" in window|update|macos) ;; *) exit 2 ;; esac
output="$PWD/agents/runs/package-e2e/$suite"
mkdir -p "$output/screenshots"
export HIDE_E2E_SCREENSHOT_DIR="$output/screenshots"
# A test that looks at Herdr itself is tagged `@herdr`; no package carries Herdr.
package_test=(bash scripts/verify-web.sh desktop e2e "$@" --grep-invert @herdr)
if [[ "${RUNNER_OS:-}" == Linux ]]; then package_test=(xvfb-run -a "${package_test[@]}"); fi
# Never attribute a preceding run's report to a build that failed before Playwright.
rm -f desktop/e2e-report.json
status=0
"${package_test[@]}" --output "$output/test-results" || status=$?
if [[ -f desktop/e2e-report.json ]]; then mv desktop/e2e-report.json "$output/report.json"; fi
exit "$status"
