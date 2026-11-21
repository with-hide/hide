#!/usr/bin/env bash
# Checks the releases file built into the app (`web/src/releases.json`)
# before a release candidate is built from it (PRD whats-new B29,
# docs/UI_BEHAVIOR.md, What's new and Update history): the file reads whole,
# names only drawings and practice chapters this commit has, gives every line
# in every interface language, and its newest entry is the version being
# released. The release skill runs it on the release commit; a failure names
# what is wrong and stops the release.
set -euo pipefail
version="${1:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  printf 'usage: scripts/check-releases-entry.sh <version>, a release version such as 1.2.11 (a preview release has no entry)\n' >&2
  exit 2
fi
cd "$(dirname "$0")/.."
HIDE_RELEASE_ENTRY="$version" bash scripts/verify-web.sh web test src/releasesFile.test.ts
