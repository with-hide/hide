#!/bin/zsh
# Moves the Herdr pin to a release tag: downloads the asset, takes the version
# the binary reports as the pinned version, writes the API schema that same
# binary reports as the contract, then updates the manifest and every document
# that quotes the pin. Nothing is written unless the asset verified.
# Re-running it for the tag already pinned converges and writes nothing.
set -euo pipefail

export LC_ALL=en_US.UTF-8
export LC_CTYPE=en_US.UTF-8
export LANG=en_US.UTF-8

usage() {
  print -u2 -- "usage: $0 <release-tag> [--repo OWNER/NAME] [--dry-run] [--keep-binary PATH]"
  print -u2 -- "       release-tag is the GitHub release tag, e.g. v0.8.3 or"
  print -u2 -- "       preview-2026-08-31-b1ff4582e968; a bare 0.8.3 means v0.8.3"
  print -u2 -- "       --keep-binary saves the verified asset so a caller need not"
  print -u2 -- "       download and re-verify it"
}

target_tag=""
target_repo=""
dry_run=false
keep_binary=""
while (( $# > 0 )); do
  case "$1" in
    --repo)
      (( $# >= 2 )) || { usage; exit 2; }
      [[ -n "$2" ]] || { usage; exit 2; }
      target_repo=$2
      shift 2
      ;;
    --dry-run)
      dry_run=true
      shift
      ;;
    --keep-binary)
      (( $# >= 2 )) || { usage; exit 2; }
      keep_binary=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      usage
      exit 2
      ;;
    *)
      [[ -z "$target_tag" ]] || { usage; exit 2; }
      target_tag=$1
      shift
      ;;
  esac
done

[[ -n "$target_tag" ]] || { usage; exit 2; }
if [[ "$target_tag" =~ '^[0-9]+(\.[0-9]+)+$' ]]; then
  target_tag=v$target_tag
fi
[[ "$target_tag" =~ '^[A-Za-z0-9][A-Za-z0-9._-]*$' ]] || {
  print -u2 -- "error: release tag has characters a GitHub tag cannot carry: $target_tag"
  exit 2
}

script_dir=${0:A:h}
project_root=${script_dir:h}
manifest=$project_root/contracts/herdr-bundle.json
contract=$project_root/contracts/herdr-api.schema.json
[[ -f "$manifest" ]] || {
  print -u2 -- "error: pinned Herdr runtime manifest is missing: $manifest"
  exit 1
}

current_repo=$(jq -er '.repo | strings | select(test("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$"))' "$manifest")
[[ -n "$target_repo" ]] || target_repo=$current_repo
[[ "$target_repo" =~ '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' ]] || { print -u2 -- "error: invalid release repository: $target_repo"; exit 2; }
current_tag=$(jq -er '.tag | strings | select(test("^[A-Za-z0-9][A-Za-z0-9._-]*$"))' "$manifest")
current_version=$(jq -er '.version' "$manifest")
current_sha256=$(jq -er '.sha256' "$manifest")
current_linux_sha256=$(jq -er '.linux_x86_64.sha256' "$manifest")
current_windows_sha256=$(jq -er '.windows_x86_64.sha256' "$manifest")
target_url="https://github.com/${target_repo}/releases/download/${target_tag}/herdr-macos-aarch64"
target_linux_url="https://github.com/${target_repo}/releases/download/${target_tag}/herdr-linux-x86_64"
target_windows_url="https://github.com/${target_repo}/releases/download/${target_tag}/herdr-windows-x86_64.zip"

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/herdr-bump.XXXXXX")
cleanup() {
  rm -rf -- "$temporary_root"
}
trap cleanup EXIT
downloaded=$temporary_root/herdr

/usr/bin/curl --fail --location --silent --show-error "$target_url" --output "$downloaded" || {
  print -u2 -- "error: release asset could not be downloaded: $target_url"
  exit 1
}
chmod 755 "$downloaded"

# The binary names its own version; a stable tag must agree with it, and a
# preview tag carries a build id the version string repeats. An asset whose
# report contradicts its tag is a mislabelled upload, not a bump.
target_version=$("$downloaded" --version | /usr/bin/awk '{print $NF}')
[[ -n "$target_version" ]] || {
  print -u2 -- "error: downloaded binary reports no version"
  exit 1
}
case "$target_tag" in
  v*)
    [[ "$target_version" == "${target_tag#v}" ]] || {
      print -u2 -- "error: downloaded binary reports $target_version, not ${target_tag#v}"
      exit 1
    }
    ;;
  *)
    [[ "$target_version" == *"${target_tag##*-}"* ]] || {
      print -u2 -- "error: downloaded binary reports $target_version, which does not name the build in $target_tag"
      exit 1
    }
    ;;
esac
target_sha256=$(/usr/bin/shasum -a 256 "$downloaded" | /usr/bin/awk '{print $1}')

# The Linux asset is the same release for the Linux CI lanes. This machine
# cannot run it, so its version is not asked; the tag and the digest pin it.
downloaded_linux=$temporary_root/herdr-linux
/usr/bin/curl --fail --location --silent --show-error "$target_linux_url" --output "$downloaded_linux" || {
  print -u2 -- "error: release asset could not be downloaded: $target_linux_url"
  exit 1
}
target_linux_sha256=$(/usr/bin/shasum -a 256 "$downloaded_linux" | /usr/bin/awk '{print $1}')

# The Windows asset is a zip holding herdr.exe and its ConPTY runtime. It runs
# nowhere this script does, so as with Linux the tag and the digest pin it, and
# the Windows lane of CI is what runs it and checks the version it reports.
downloaded_windows=$temporary_root/herdr-windows.zip
/usr/bin/curl --fail --location --silent --show-error "$target_windows_url" --output "$downloaded_windows" || {
  print -u2 -- "error: release asset could not be downloaded: $target_windows_url"
  exit 1
}
target_windows_sha256=$(/usr/bin/shasum -a 256 "$downloaded_windows" | /usr/bin/awk '{print $1}')

# Device platforms travel with the Mac app. Keep their verified bytes in the
# same manifest as the host assets, including when extending the current pin.
extra_pins=$temporary_root/device-pins.json
print -r -- '{}' > "$extra_pins"
platforms_added=false
for device_platform in macos-x86_64 linux-aarch64; do
  device_key=${device_platform//-/_}
  device_url="https://github.com/${target_repo}/releases/download/${target_tag}/herdr-${device_platform}"
  device_file=$temporary_root/herdr-$device_platform
  /usr/bin/curl --fail --location --silent --show-error "$device_url" --output "$device_file" || {
    print -u2 -- "error: required device release asset could not be downloaded: $device_url"
    exit 1
  }
  device_sha=$(/usr/bin/shasum -a 256 "$device_file" | /usr/bin/awk '{print $1}')
  previous_sha=$(jq -r --arg key "$device_key" '.[$key].sha256 // empty' "$manifest")
  if [[ "$current_repo" == "$target_repo" && "$current_tag" == "$target_tag" ]]; then
    if [[ -z "$previous_sha" ]]; then
      platforms_added=true
    elif [[ "$previous_sha" != "$device_sha" ]]; then
      print -u2 -- "error: $target_tag is already pinned but its $device_platform asset digest changed"
      exit 1
    fi
  fi
  jq --arg key "$device_key" --arg platform "$device_platform" --arg url "$device_url" --arg sha "$device_sha" \
    '.[$key] = {platform: $platform, source_url: $url, sha256: $sha}' "$extra_pins" > "$extra_pins.next"
  mv "$extra_pins.next" "$extra_pins"
done

# The contract is whatever this exact binary answers; nothing is fetched from
# a second place that could disagree with the asset that ships.
target_contract=$temporary_root/herdr-api.schema.json
"$downloaded" api schema --json | jq . > "$target_contract" || {
  print -u2 -- "error: downloaded binary did not answer 'api schema --json'"
  exit 1
}
contract_protocol=$(jq -er '.protocol' "$target_contract")

outcome=""
if [[ "$current_repo" == "$target_repo" && "$current_tag" == "$target_tag" ]]; then
  if [[ "$current_sha256" != "$target_sha256" || "$current_linux_sha256" != "$target_linux_sha256" ]]; then
    print -u2 -- "error: $target_tag is already pinned but its asset digest changed"
    print -u2 -- "pinned_sha256=$current_sha256 downloaded_sha256=$target_sha256"
    print -u2 -- "pinned_linux_sha256=$current_linux_sha256 downloaded_linux_sha256=$target_linux_sha256"
    print -u2 -- "the upstream release was replaced; confirm the new asset before repinning"
    exit 1
  fi
  if [[ "$current_windows_sha256" != "$target_windows_sha256" ]]; then
    print -u2 -- "error: $target_tag is already pinned but its Windows asset digest changed"
    print -u2 -- "pinned_windows_sha256=$current_windows_sha256 downloaded_windows_sha256=$target_windows_sha256"
    print -u2 -- "the upstream release was replaced; confirm the new asset before repinning"
    exit 1
  fi
  if cmp -s <(jq -S . "$contract") <(jq -S . "$target_contract"); then
    outcome=unchanged
    [[ "$platforms_added" != true ]] || outcome=platforms_added
  elif [[ "$dry_run" == true ]]; then
    print -u2 -- "error: $target_tag is already pinned but contracts/herdr-api.schema.json is not what its binary reports"
    print -u2 -- "the contract drifted from the pin; rerun without --dry-run to regenerate it"
    exit 1
  else
    outcome=contract_regenerated
  fi
elif [[ "$dry_run" == true ]]; then
  outcome=planned
else
  outcome=bumped
fi

# Update pin tokens and the complete provenance block together so changing
# repositories cannot leave a stale source or modification notice.
derived_documents=(
  README.md
  docs/INSTALL.md
  contracts/README.md
  AGENTS.md
  desktop/resources/THIRD_PARTY_NOTICES/herdr-APACHE-2.0.txt
)

replacements_json=$temporary_root/replacements.json
print -r -- '[]' > "$replacements_json"
[[ "$outcome" == unchanged || "$outcome" == contract_regenerated || "$outcome" == platforms_added ]] || for relative in $derived_documents; do
  document=$project_root/$relative
  [[ -f "$document" ]] || {
    print -u2 -- "error: document quoting the pin is missing: $relative"
    exit 1
  }
  tag_hits=$(grep -Fc -- "releases/tag/$current_tag" "$document" || true)
  version_hits=$(grep -Fc -- "$current_version" "$document" || true)
  digest_hits=$(grep -Fc -- "$current_sha256" "$document" || true)
  if (( tag_hits == 0 && version_hits == 0 && digest_hits == 0 )); then
    print -u2 -- "error: $relative quotes neither the pinned tag, version nor digest"
    print -u2 -- "the document moved away from the pin; update the script's document list"
    exit 1
  fi
  if [[ "$dry_run" == false ]]; then
    /usr/bin/python3 - "$document" "$current_repo" "$target_repo" "$current_tag" "$target_tag" "$current_version" "$target_version" "$current_sha256" "$target_sha256" <<'PYDOC'
import sys
from pathlib import Path
p = Path(sys.argv[1])
old_repo, repo, old_tag, tag, old_version, version, old_digest, digest = sys.argv[2:]
text = p.read_text()
for prefix in ("releases/tag/", "releases/download/"):
    text = text.replace(f"https://github.com/{old_repo}/{prefix}{old_tag}",
                        f"https://github.com/{repo}/{prefix}{tag}")
text = text.replace(old_version, version).replace(old_digest, digest)
notice = p.suffix == ".txt"
begin, end = ("BEGIN HERDR PROVENANCE", "END HERDR PROVENANCE") if notice else ("<!-- herdr-provenance:start -->", "<!-- herdr-provenance:end -->")
assert text.count(begin) == text.count(end) == 1, f"missing or ambiguous provenance block: {p}"
if repo == "herdrdev/herdr":
    if notice:
        prose = f"This application distributes the upstream Herdr release {tag} from {repo}.\nThe bundled binary's code is not modified by hide; on macOS a release replaces only its ad-hoc signature with hide's Developer ID one, which notarization requires.\nhide monitors upstream stable through herdr-update.yml; updates must pass contract and runtime checks."
    else:
        prose = f"hide distributes the [upstream Herdr release {tag}](https://github.com/{repo}/releases/tag/{tag}).\nThe bundled binary's code is not modified by hide; a macOS release replaces only its ad-hoc signature with hide's Developer ID one, which notarization requires.\nThe daily `herdr-update.yml` workflow proposes upstream stable releases with `--repo herdrdev/herdr`; updates must pass contract and runtime checks."
else:
    commit = tag.rsplit("-", 1)[-1] if tag.startswith("preview-") else None
    if notice:
        source = f"built from commit {commit} (tag {tag})." if commit else f"released as {tag}."
        prose = f"This is a modified Herdr preview from {repo},\n{source}\nIt supplies host scope, event sequences, and agent lineage not available in upstream stable.\nhide monitors upstream stable through herdr-update.yml and returns when contract and runtime checks pass."
    else:
        source = f", built from commit `{commit}`" if commit else ""
        prose = f"hide distributes a modified Herdr preview from the [{repo} fork](https://github.com/{repo}/releases/tag/{tag}){source}.\nThis fork supplies host-scoped snapshots, ordered event sequences, and agent lineage that the upstream stable release does not yet expose.\nThe daily `herdr-update.yml` workflow continues to propose upstream stable releases with `--repo herdrdev/herdr`; return to upstream when the contract field tests and runtime checks pass."
start = text.index(begin) + len(begin)
finish = text.index(end, start)
text = text[:start] + "\n" + prose + "\n" + text[finish:]

p.write_text(text)
PYDOC
  fi
  jq --arg document "$relative" \
     --argjson tag_lines "$tag_hits" \
     --argjson version_lines "$version_hits" \
     --argjson digest_lines "$digest_hits" \
     '. + [{document: $document, tag_lines: $tag_lines, version_lines: $version_lines, digest_lines: $digest_lines}]' \
     "$replacements_json" > "$replacements_json.next"
  mv "$replacements_json.next" "$replacements_json"
done

if [[ "$outcome" == bumped || ( "$outcome" == platforms_added && "$dry_run" == false ) ]]; then
  jq --arg repo "$target_repo" --arg tag "$target_tag" \
     --arg version "$target_version" \
     --arg source_url "$target_url" \
     --arg sha256 "$target_sha256" \
     --arg linux_url "$target_linux_url" \
     --arg linux_sha256 "$target_linux_sha256" \
     --arg windows_url "$target_windows_url" \
     --arg windows_sha256 "$target_windows_sha256" \
     --argjson device_pins "$(cat "$extra_pins")" \
     '.repo = $repo | .tag = $tag | .version = $version | .source_url = $source_url | .sha256 = $sha256
      | .linux_x86_64 = {platform: "linux-x86_64", source_url: $linux_url, sha256: $linux_sha256}
      | .windows_x86_64 = {platform: "windows-x86_64", source_url: $windows_url, sha256: $windows_sha256}
      | . + $device_pins
      | .verification = "This file is the single source of the pin for host and device platforms. scripts/fetch-herdr-runtime.sh and .ps1 verify the bytes against it; native lanes execute the matching platform. desktop/scripts/package.mjs carries the host asset and, on Mac, each Unix device asset. contracts/herdr-api.schema.json is the schema the pinned host binary reports. Change the pin with scripts/bump-herdr.sh <release-tag>."' \
     "$manifest" > "$manifest.next"
  mv "$manifest.next" "$manifest"
fi
if [[ "$outcome" == bumped || "$outcome" == contract_regenerated ]]; then
  install -m 644 "$target_contract" "$contract"
fi

# The verified binary is handed over last, so a failure anywhere above leaves
# nothing behind at the caller's path.
if [[ -n "$keep_binary" ]]; then
  install -m 755 "$downloaded" "$keep_binary"
fi

jq -n \
  --arg outcome "$outcome" \
  --arg from "$current_tag" \
  --arg tag "$target_tag" \
  --arg version "$target_version" \
  --arg sha256 "$target_sha256" \
  --arg source_url "$target_url" \
  --arg linux_sha256 "$target_linux_sha256" \
  --arg linux_url "$target_linux_url" \
  --arg windows_sha256 "$target_windows_sha256" \
  --arg windows_url "$target_windows_url" \
  --argjson protocol "$contract_protocol" \
  --argjson documents "$(cat "$replacements_json")" \
  '{status: "pass", outcome: $outcome, from: $from, tag: $tag, version: $version,
    sha256: $sha256, source_url: $source_url,
    linux_x86_64: {sha256: $linux_sha256, source_url: $linux_url},
    windows_x86_64: {sha256: $windows_sha256, source_url: $windows_url},
    protocol: $protocol, documents: $documents}'
