#!/bin/zsh
# Fails when the pinned Herdr version or digest is restated anywhere outside
# contracts/herdr-bundle.json. The drift this prevents already happened
# once: 002c2f4 changed one site and dcebff5 had to chase the other three.
set -euo pipefail

export LC_ALL=en_US.UTF-8
export LANG=en_US.UTF-8

script_dir=${0:A:h}
project_root=${script_dir:h}
manifest=$project_root/contracts/herdr-bundle.json

[[ -f "$manifest" ]] || {
  print -u2 -- "error: pinned Herdr runtime manifest is missing: $manifest"
  exit 1
}

repo=$(jq -er '.repo | strings | select(length > 0)' "$manifest")
version=$(jq -er '.version' "$manifest")
sha256=$(jq -er '.sha256' "$manifest")
linux_sha256=$(jq -er '.linux_x86_64.sha256' "$manifest")
windows_sha256=$(jq -er '.windows_x86_64.sha256' "$manifest")

# What still fetches or starts the pinned Herdr: the fetch scripts and the
# test fixtures that look at Herdr itself; the app, its packages and the CI
# lanes carry none (PRD engine-bundle-4-herdr-removal D-2).
derived_sources=(
  scripts/fetch-herdr-runtime.sh
  scripts/fetch-herdr-runtime.ps1
  web/e2e/herdr-fixture.ts
  desktop/e2e/fixture.ts
  scripts/web-shell-measure/isolated-env.sh
)

# The version matches as a whole token: `0.9.1` is not restated by `0.9.143`
# (another tool's version) or `10.9.1`, but is by `v0.9.1`, `0.9.1.` and `0.9.1-rc1`.
version_pattern="(^|[^0-9.])$(print -rn -- "$version" | sed 's/[][\.*^$+?(){}|/]/\\&/g')(\$|[^0-9.]|\.(\$|[^0-9]))"

failed=0
for relative in $derived_sources; do
  # Not `path`: in zsh that name is tied to $PATH and assigning it wipes the
  # command search path for the rest of the script.
  source_path=$project_root/$relative
  [[ -f "$source_path" ]] || {
    print -u2 -- "error: derived source is missing: $relative"
    failed=1
    continue
  }
  if grep -Eq -- "$version_pattern" "$source_path"; then
    print -u2 -- "error: $relative restates the pinned version $version; read it from the manifest"
    failed=1
  fi
  if grep -Fq -- "$repo" "$source_path"; then
    print -u2 -- "error: $relative restates the pinned repository; read it from the manifest"
    failed=1
  fi
  if grep -Fq -- "$sha256" "$source_path" || grep -Fq -- "$linux_sha256" "$source_path" || grep -Fq -- "$windows_sha256" "$source_path"; then
    print -u2 -- "error: $relative restates the pinned digest; read it from the manifest"
    failed=1
  fi
done

(( failed == 0 )) || exit 1

jq -n \
  --arg version "$version" \
  --arg manifest "$manifest" \
  --argjson checked "${#derived_sources[@]}" \
  '{status: "pass", pin: $version, manifest: $manifest, derived_sources_checked: $checked}'
