#!/usr/bin/env bash
# Runs hide in a browser from this checkout, with no hide.app installed.
#
# Usage: scripts/web-dev.sh [--into-home] [--no-build] [--no-browser]
#
#   --into-home   Install into your own home (~/.hide, ~/.local/bin/hide and
#                 each agent's hook file), the way a device install does.
#                 Refused where the desktop app is installed, because the app
#                 and this build would take turns rewriting the same hooks.
#                 Without it everything lands under .hide-web-dev/home in this
#                 checkout, the daemon's state under /tmp/hide-web-dev-<uid>-
#                 <hash>, and your own files are not touched; the agents step
#                 then reads every agent as not signed in, since each pane and
#                 hook uses that home.
#   --no-build    Reuse target/release and web/dist as they are.
#   --no-browser  Print the address instead of opening a browser.
#
# What it does, in order (docs/INSTALL.md, Try hide in a browser):
#   1. builds the web shell and the release daemon (`scripts/verify-cargo.sh
#      release`, which embeds web/dist into hided and builds the pane holder);
#   2. places the build as a build Hide placed: `<home>/.hide/host-helper/
#      <16 hex>/{hided,hide,hide-agent-hooks,hide-holder}` with `current`
#      pointing at it. The kit installs only from a build in that shape
#      (hide-host/src/kit.rs, `placed_build`); a hided run from target/
#      installs nothing, and the daemon starts each pane's holder from the
#      `hide-holder` beside it;
#   3. runs `hide connect` from the placed build, which starts the daemon,
#      and opens the address it answers.
#
# Windows is not covered: the daemon there runs from a runtime copy that
# `hide runtime prepare` makes (hide-kit/src/runtime.rs), and this script has
# no PowerShell form yet; docs/INSTALL.md, What each system supports, records
# the gap.
set -euo pipefail

usage() {
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
}

into_home=0
build=1
browser=1
while (($# > 0)); do
    case "$1" in
        --into-home) into_home=1 ;;
        --no-build) build=0 ;;
        --no-browser) browser=0 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

case "$(uname -s)" in
    Darwin) system=macos ;;
    Linux) system=linux ;;
    *)
        echo "error: web-dev.sh runs on macOS and Linux; on Windows install the package (docs/INSTALL.md)" >&2
        exit 2
        ;;
esac

checkout=$(cd "$(dirname "$0")/.." && pwd)
cd "$checkout"

for program in pnpm cargo jq; do
    command -v "$program" > /dev/null || {
        echo "error: $program is not on PATH; docs/INSTALL.md, Requirements, lists what a source build needs" >&2
        exit 2
    }
done

# 1. Build, with your own home and caches (the toolchain, the terminal parser's inputs).
if ((build)); then
    [[ -d web/node_modules ]] || pnpm install --frozen-lockfile
    pnpm --dir web build
    bash scripts/verify-cargo.sh release
fi
for program in hided hide hide-agent-hooks hide-holder; do
    [[ -x "target/release/$program" ]] || {
        echo "error: target/release/$program is missing; run without --no-build" >&2
        exit 1
    }
done

# A shell inside hide carries the daemon's variables, and a Herdr pane the
# socket, identity and binary of the Herdr that opened it; a build started
# with them would attach to that daemon instead of its own. The desktop app is launched the same way (AGENTS.md, The
# installed app is the operator's). The XDG folders go with them below, once
# the home is chosen: an isolated home with your XDG_STATE_HOME would still
# be your state folder.
for name in $(compgen -e); do
    case "$name" in HERDR_* | HIDE_*) unset "$name" ;; esac
done

# 2. Choose the home and place the build.
if ((into_home)); then
    home=$HOME
    installed=""
    case "$system" in
        macos) [[ -d /Applications/hide.app ]] && installed=/Applications/hide.app ;;
        linux) [[ -d /opt/hide ]] && installed=/opt/hide ;;
    esac
    if [[ -n "$installed" ]]; then
        echo "error: $installed is installed; it and this build would each rewrite the agent hooks and ~/.local/bin/hide. Open the app instead, or run without --into-home." >&2
        exit 1
    fi
else
    home=$checkout/.hide-web-dev/home
    mkdir -p "$home"
    chmod 700 "$checkout/.hide-web-dev" "$home"
    unset XDG_STATE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME
fi

# The SHA-256 of a file, or of stdin with no argument.
if [[ "$system" == macos ]]; then
    digest_of() { /usr/bin/shasum -a 256 "$@" | /usr/bin/awk '{print $1}'; }
else
    digest_of() { sha256sum "$@" | awk '{print $1}'; }
fi

root=$home/.hide/host-helper
version=$(digest_of target/release/hided | cut -c1-16)
place=$root/$version
mkdir -p "$place"
chmod 700 "$home/.hide" "$root" "$place"
for program in hided hide hide-agent-hooks hide-holder; do
    install -m 755 "target/release/$program" "$place/$program"
done
ln -sfn "$version" "$root/current"
export HOME=$home
# Each pane's holder listens on a Unix socket in the state folder, and a
# socket path holds about a hundred bytes, which a worktree's path can use
# most of; an isolated home keeps its state folder under /tmp instead.
if ! ((into_home)); then
    state=/tmp/hide-web-dev-$(id -u)-$(printf '%s' "$checkout" | digest_of | cut -c1-8)
    mkdir -p "$state"
    chmod 700 "$state"
    export HIDE_STATE_DIR=$state
fi

# 3. The daemon and the address. `hide connect` attaches to the daemon the
# state folder names when it is this build, starts one when there is none,
# and refuses one of another build: the daemon of the build before an edit,
# alive for ten idle minutes after its tab closed. That one is stopped and
# the connection made again.
connect() {
    answer=$("$place/hide" connect) || true
    [[ "$(printf '%s' "$answer" | jq -r '.ok // false')" == true ]]
}
if ! connect; then
    if [[ "$(printf '%s' "$answer" | jq -r '.reason // empty')" == other_build ]]; then
        "$place/hide" stop
        connect || true
    fi
fi
url=$(printf '%s' "$answer" | jq -r '.url // empty')
if [[ -z "$url" ]]; then
    echo "error: hide connect did not answer with an address: $answer" >&2
    exit 1
fi
echo "$url"
if ((browser)); then
    case "$system" in
        macos) /usr/bin/open "$url" ;;
        linux) xdg-open "$url" > /dev/null 2>&1 || echo "open the address in a browser" >&2 ;;
    esac
fi
