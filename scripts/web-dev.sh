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
#                 checkout and your own files are not touched; the agents step
#                 then reads every agent as not signed in, since each pane and
#                 hook uses that home.
#   --no-build    Reuse target/release and web/dist as they are.
#   --no-browser  Print the address instead of opening a browser.
#
# What it does, in order (docs/INSTALL.md, Try hide in a browser):
#   1. builds the web shell and the release daemon (`scripts/verify-cargo.sh
#      release`, which embeds web/dist into hided), and fetches the pinned
#      Herdr (`scripts/fetch-herdr-runtime.sh`);
#   2. places the build as a build Hide placed: `<home>/.hide/host-helper/
#      <16 hex>/{hided,hide,hide-agent-hooks}` with `current` pointing at it,
#      and the pinned Herdr at `<home>/.hide/host-helper/herdr/current/herdr`.
#      The kit installs only from a build in that shape (hide-host/src/kit.rs,
#      `placed_build`); a hided run from target/ installs nothing;
#   3. starts `herdr server` when none answers on the socket, since `hide
#      connect` does not start one (the desktop app and the login item do);
#   4. runs `hide connect` from the placed build, which starts the daemon,
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

# The pinned Herdr exists for these two (contracts/herdr-bundle.json).
case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)
        system=macos
        platform=macos-aarch64
        ;;
    Linux/x86_64)
        system=linux
        platform=linux-x86_64
        ;;
    Darwin/* | Linux/*)
        echo "error: no pinned Herdr for $(uname -s) $(uname -m); hide runs on Apple Silicon Macs and Linux x64 (docs/INSTALL.md, Requirements)" >&2
        exit 2
        ;;
    *)
        echo "error: web-dev.sh runs on macOS and Linux; on Windows install the package (docs/INSTALL.md)" >&2
        exit 2
        ;;
esac

checkout=$(cd "$(dirname "$0")/.." && pwd)
cd "$checkout"

for program in pnpm cargo zsh jq; do
    command -v "$program" > /dev/null || {
        echo "error: $program is not on PATH; docs/INSTALL.md, Requirements, lists what a source build needs" >&2
        exit 2
    }
done

# 1. Build, with your own home and caches (the toolchain, the Herdr cache).
if ((build)); then
    [[ -d web/node_modules ]] || pnpm install --frozen-lockfile
    pnpm --dir web build
    bash scripts/verify-cargo.sh release
fi
for program in hided hide hide-agent-hooks; do
    [[ -x "target/release/$program" ]] || {
        echo "error: target/release/$program is missing; run without --no-build" >&2
        exit 1
    }
done
herdr=$(zsh scripts/fetch-herdr-runtime.sh --platform "$platform")

# A Herdr pane carries the socket, identity and binary of the Herdr that
# opened it, and a shell inside hide carries the daemon's variables; a build
# started with them would attach to that Herdr and that daemon instead of
# its own. The desktop app is launched the same way (AGENTS.md, The
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
for program in hided hide hide-agent-hooks; do
    install -m 755 "target/release/$program" "$place/$program"
done
ln -sfn "$version" "$root/current"

herdr_version=$("$herdr" --version | awk '{print $NF}')
mkdir -p "$root/herdr/$herdr_version"
install -m 755 "$herdr" "$root/herdr/$herdr_version/herdr"
ln -sfn "$herdr_version" "$root/herdr/current"
herdr=$root/herdr/current/herdr

# 3. The Herdr server. An isolated home gets its own socket under /tmp, not
# under the checkout, because a Unix socket path holds about a hundred
# bytes and a worktree's path can use most of them; your own home uses
# Herdr's default socket, so a Herdr you already run is reused as it is.
# The server outlives this script and the daemon; `herdr server stop` with
# the same HERDR_SOCKET_PATH ends it.
if ((into_home)); then
    socket=""
else
    socket_dir=/tmp/hide-web-dev-$(id -u)-$(printf '%s' "$checkout" | digest_of | cut -c1-8)
    mkdir -p "$socket_dir"
    chmod 700 "$socket_dir"
    socket=$socket_dir/herdr.sock
    export HERDR_SOCKET_PATH=$socket
fi
export HOME=$home

herdr_running() {
    [[ "$("$herdr" status server --json 2> /dev/null | jq -r '.running')" == true ]]
}
if ! herdr_running; then
    mkdir -p "$home/.hide/web-dev"
    nohup "$herdr" server > "$home/.hide/web-dev/herdr-server.log" 2>&1 &
    server_pid=$!
    for _ in $(seq 1 50); do
        herdr_running && break
        sleep 0.2
    done
    herdr_running || {
        kill "$server_pid" 2> /dev/null || true
        echo "error: herdr server did not answer in 10 s; see $home/.hide/web-dev/herdr-server.log" >&2
        exit 1
    }
fi

# 4. The daemon and the address. `hide connect` attaches to the daemon the
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
