#!/usr/bin/env bash
# The cargo entrypoint for verification, as plain argv.
#
# Usage: verify-cargo.sh test [args...] | lint | release | cli
# CI-scoped lanes: test-scoped | check | build | clippy | metadata [cargo arguments...]
#                  nextest [cargo nextest run arguments...] (the CI Rust lane; .config/nextest.toml)
#                  nextest-list [cargo nextest list arguments...] (the tests `nextest` would run, which the repeat lane counts)
#                  fmt-check (cargo fmt --all --check alone)
#                  hakari (workspace-hack is current; scripts/install-hakari.sh)
#
# The PRD harness runs each verify command with execvp and no shell, so an
# `ENV=value cargo ...` binding fails with ENOENT at verify time rather than at
# configuration time, and a sealed run cannot be amended to fix it. A script is
# the only place an environment decision can live, which is why this file exists
# rather than a longer command string in `agents/config.json`:
#
#     "test": "bash scripts/verify-cargo.sh test"
#     "lint": "bash scripts/verify-cargo.sh lint"
#
# Every build lands in the worktree's own `target/`, whatever CARGO_TARGET_DIR
# the caller carries: the desktop packager reads the release binaries from
# `target/release/` at that fixed path, and a build directory shared between
# worktrees reads the other checkout's artifacts as fresh. `git worktree
# remove` takes the cache with the work. See docs/BUILD.md.
set -euo pipefail

cd "$(dirname "$0")/.."

# System helpers must start even when the caller names an unavailable locale.
# macOS Perl can panic before acquiring fixture locks under LANG=C.UTF-8.
export LC_ALL=C LANG=C

. scripts/toolchain-env.sh

export CARGO_TARGET_DIR="$PWD/target"

# A Herdr pane carries the socket and identity of the Herdr that opened it,
# and a test daemon that reads them from its environment follows the
# operator's live Herdr and labels its agents from the operator's
# conversations. No test may reach that server. Retirement fixtures must also ignore an inherited legacy
# coordination-home override so no private kit pass follows operator state.
for name in $(compgen -e); do
    case "$name" in HERDR_*|HCOORD_*) unset "$name" ;; esac
done

# A Hide pane carries the same kind of handle to the operator's Hide:
# HIDE_STATE_DIR and HIDE_CAP_REF name the operator's state folder and
# credential, HIDE_PANE_ID and HIDE_INSTALL(_CLI) name the pane and the
# installation that started it, and the daemon's own settings (HIDE_PORT,
# HIDE_KEEP_ALIVE and the rest) ride along. A test that reads one sees the
# operator's running daemon and refuses or writes there: 104 hide-kit tests
# failed that way in a pane (with-hide/hide-private#650), and three
# hide-agent-hooks tests wrote under ~/.hide/agent-hooks (#627).
# Every HIDE_* variable is therefore dropped, except the inputs a build or a
# test is meant to be given by its caller: HIDE_TEST_* and HIDE_E2E_* (a
# program or a prerequisite a test asks for, docs/TESTING.md), HIDE_BUILD_*
# and HIDE_VT_* (build.rs inputs, set by toolchain-env.sh too), the release
# identity, HOLDER_CONPTY, and the opt-in live probes. A new input of that
# kind takes a HIDE_TEST_ name, or is added to this list in the change that
# reads it; a variable the daemon or a pane sets is never added.
for name in $(compgen -e); do
    case "$name" in
        HIDE_TEST_*|HIDE_E2E_*|HIDE_BUILD_*|HIDE_VT_*) ;;
        HIDE_HOLDER_CONPTY|HIDE_VERSION|HIDE_COMMIT|HIDE_POSTHOG_PROJECT_TOKEN) ;;
        HIDE_AGY_*|HIDE_AI_LIVE_EVIDENCE|HIDE_CODEX_BINARY|HIDE_SUGGESTIONS_REPLAY_*|HIDE_SNAPSHOT_OUT) ;;
        HIDE_*) unset "$name" ;;
    esac
done

# A test that runs git reads none of the account's git settings. The account's
# commit.gpgsign with an SSH signer made a fixture commit fail whenever the
# signing agent did not answer, under load (with-hide/hide-private#620, #333),
# and CI's runners carry no such setting, so a test that passes only because
# of one passes nowhere else. /dev/null is the empty config on every system
# (Git for Windows reads it as `nul`); a fixture still names its own
# user.name and user.email.
isolate_git_config() {
    export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
}

# Each job's log names the toolchain that builds, the one rust-toolchain.toml
# pins: a list of installed toolchains does not say which one ran. Once per
# call, after the arguments are accepted, and on stderr, because `metadata`
# hands its stdout to scripts/ci-plan.py. See docs/BUILD.md.
report_toolchain() {
    rustc --version >&2
    cargo --version >&2
}

# hide-vt builds libghostty-vt from inputs scripts/prepare-vt-inputs.py
# downloads and checks once per machine; its build itself downloads nothing.
# Every mode that can build hide-vt prepares them first, which does nothing
# when they are ready, and stops with the reason when it cannot, rather than
# leave the build to fail later. docs/BUILD.md, "The terminal parser and Zig".
prepare_vt_inputs() {
    python3 scripts/prepare-vt-inputs.py --quiet >/dev/null || {
        printf 'The hide-vt build inputs could not be prepared (the reason is above). Run `python3 scripts/prepare-vt-inputs.py` once on this machine where ziglang.org, github.com, codeberg.org and deps.files.ghostty.org are reachable, then run this command again.\n' >&2
        exit 1
    }
}

# Whether scoped cargo arguments select hide-vt: the whole workspace, the
# crate by name, or the pane holder, the one crate that links it.
selects_vt() {
    local argument previous=
    for argument in "$@"; do
        case "$argument" in
            --workspace|--all|-phide-vt|--package=hide-vt|-phide-holder|--package=hide-holder) return 0 ;;
        esac
        case "$previous:$argument" in
            -p:hide-vt|--package:hide-vt|-p:hide-holder|--package:hide-holder) return 0 ;;
        esac
        previous=$argument
    done
    return 1
}

# Scoped modes cannot move the checkout or its artifacts through cargo flags.
# Keep the legacy modes unchanged for sealed verification commands.
case "${1:-}" in
    test-scoped|check|build|clippy|metadata|nextest|nextest-list)
        mode=$1
        shift
        (( $# <= 128 )) || { printf 'too many cargo arguments\n' >&2; exit 2; }
        for argument in "$@"; do
            case "$argument" in
                --target-dir|--target-dir=*|--manifest-path|--manifest-path=*|--config|--config=*)
                    printf 'cargo argument may not redirect verification: %s\n' "$argument" >&2
                    exit 2
                    ;;
            esac
        done
        report_toolchain
        if [[ "$mode" != metadata ]] && selects_vt "$@"; then prepare_vt_inputs; fi
        case "$mode" in test-scoped|nextest) isolate_git_config ;; esac
        if [[ "$mode" == nextest ]]; then
            status=0
            cargo nextest run --locked "$@" || status=$?
            # A lane runs nextest several times and every run writes the same
            # JUnit path; keep each report under its own name for
            # scripts/ci-flaky-report.py, which reads them all.
            report=target/nextest/ci/junit.xml
            [[ ! -f "$report" ]] || mv "$report" "target/nextest/ci/junit.$$.$RANDOM.xml"
            exit "$status"
        fi
        [[ "$mode" != nextest-list ]] || exec cargo nextest list --locked "$@"
        [[ "$mode" != test-scoped ]] || mode=test
        exec cargo "$mode" --locked "$@"
        ;;
    test)
        shift
        report_toolchain
        prepare_vt_inputs
        isolate_git_config
        exec cargo test --locked --workspace "$@"
        ;;
    fmt-check)
        report_toolchain
        exec cargo fmt --all --check
        ;;
    hakari)
        # workspace-hack holds the features the manifests ask for, and every
        # member that should depends on it; neither command writes a file.
        # docs/BUILD.md, "One feature set per dependency".
        report_toolchain
        cargo hakari generate --diff
        exec cargo hakari manage-deps --dry-run
        ;;
    lint)
        report_toolchain
        prepare_vt_inputs
        cargo fmt --all --check
        exec cargo clippy --locked --workspace --all-targets -- -D warnings
        ;;
    release)
        # The binaries the packaged app ships; release hided embeds web/dist,
        # so `pnpm --dir web build` runs first (desktop/scripts/package.mjs).
        # The pane holder links hide-vt, whose Zig inputs are prepared first.
        report_toolchain
        prepare_vt_inputs
        exec cargo build --release --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks -p hide-holder --bin hide-holder
        ;;
    cli)
        # Every pane runs in a pane holder, which links hide-vt.
        report_toolchain
        prepare_vt_inputs
        exec cargo build --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks -p hide-holder --bin hide-holder
        ;;
    *)
        printf 'usage: %s test [args...]|lint|fmt-check|hakari|release|cli|test-scoped|check|build|clippy|metadata|nextest|nextest-list [args...]\n' "$0" >&2
        exit 2
        ;;
esac
