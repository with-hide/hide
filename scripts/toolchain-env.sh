#!/usr/bin/env bash
# Points cargo at the machine's installed Rust toolchain, whatever HOME it runs
# under.
#
# Source this before the first cargo call, do not execute it:
#
#     . "$(dirname "$0")/toolchain-env.sh"
#     cargo test --manifest-path herdr-core/Cargo.toml
#
# rustup resolves its toolchain from RUSTUP_HOME, which defaults to
# `$HOME/.rustup`. A verification runner gets its own HOME, so that default
# names an empty directory, and rustup does not fail there: it downloads and
# installs the whole toolchain into it, saying so only in a warning.
#
#     $ env HOME="$(mktemp -d)" cargo --version
#     info: syncing channel updates for stable-aarch64-apple-darwin
#     info: downloading 6 components
#     warn: the missing active toolchain `stable-aarch64-apple-darwin` has been
#           auto-installed
#     $ echo $?
#     0
#
# That exit 0 is what made two earlier, since-removed test scripts' workarounds
# dead code. Each recovered RUSTUP_HOME only inside `if ! cargo --version`, and
# the shim never takes that branch; it pays for a download instead. Neither
# script was wrong about the cause, and neither one ran.
#
# The cost was measured rather than guessed. Every run directory under
# `agents/runs/` had grown its own private copy: 1.3 GB of `.rustup` plus
# 128 MB of `.cargo` per run, 9.1 GB across eleven of them, duplicating a
# `~/.rustup` that was already installed on the machine.
#
# The real toolchain is found from the shim rather than from a literal path,
# because `cargo` on PATH is that shim and rustup installs its home beside the
# shim's own. An explicit RUSTUP_HOME or CARGO_HOME still wins, so a caller
# that means to use a different toolchain says so.
#
# See AGENTS.md, "Build Output Belongs To Its Worktree".

hide_toolchain_shim="$(command -v cargo || true)"
if [[ -n "$hide_toolchain_shim" ]]; then
    # The shim lives at <cargo home>/bin/cargo, and rustup installs its own
    # home as a sibling of that cargo home.
    hide_toolchain_cargo_home="$(cd "$(dirname "$hide_toolchain_shim")/.." && pwd)"
    hide_toolchain_rustup_home="$(dirname "$hide_toolchain_cargo_home")/.rustup"
    if [[ -d "$hide_toolchain_rustup_home" ]]; then
        export CARGO_HOME="${CARGO_HOME:-$hide_toolchain_cargo_home}"
        export RUSTUP_HOME="${RUSTUP_HOME:-$hide_toolchain_rustup_home}"
        # hide-vt's prepared build inputs (the pinned Zig and libghostty-vt
        # source) are the same kind of machine download as the toolchain, so
        # a runner HOME reads them from the same home instead of downloading
        # its own copy. Windows keeps them under %LOCALAPPDATA%, which no
        # runner HOME moves. docs/BUILD.md, "The terminal parser and Zig".
        case "$(uname -s)" in
            MINGW*|MSYS*|CYGWIN*) ;;
            *) export HIDE_BUILD_CACHE="${HIDE_BUILD_CACHE:-$(dirname "$hide_toolchain_cargo_home")/.cache/hide-build}" ;;
        esac
    fi
fi
unset hide_toolchain_shim hide_toolchain_cargo_home hide_toolchain_rustup_home
