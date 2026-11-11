#!/usr/bin/env bash
# The web and desktop entrypoint for verification, as plain argv.
#
# Usage: verify-web.sh [install [--ignore-scripts | holder | docs] | PACKAGE ACTION [args...]]
# PACKAGE: web, desktop, mobile (test only), relay-protocol (typecheck only), site (build only). ACTION: typecheck, lint, test, build, e2e, e2e-list,
# bundle, build:phone and build:docs-embed (web only), playwright-install,
# package and electron-install (desktop only).
#
# Runs the web shell and desktop app typecheck, lint, unit tests and build.
# Playwright flows use the explicit per-package e2e actions below.
# The PRD harness runs each verify command with execvp and no
# shell, so the pnpm workspace install and the per-package steps live here
# rather than in a longer command string in `agents/config.json`:
#
#     "web": "bash scripts/verify-web.sh"
#
# Every artifact lands inside this worktree (`web/dist/`, `desktop/dist/`,
# both packages' `node_modules/`); see docs/BUILD.md.
set -euo pipefail

cd "$(dirname "$0")/.."

. scripts/toolchain-env.sh

# Explicit CI modes retain step boundaries and their own failure outcomes.
if (( $# > 0 )); then
    if [[ "$1" == install ]]; then
        shift
        case "$#:${1:-}" in
            0:|1:--ignore-scripts) exec pnpm install --frozen-lockfile "$@" ;;
            # The xterm the pane holder's screen tests draw on, and nothing
            # else: a Rust lane needs no web shell (docs/TESTING.md, The pane
            # holder).
            1:holder) exec pnpm install --frozen-lockfile --filter hide-holder-xterm ;;
            # What the user docs build needs (docs/TESTING.md, The user docs):
            # the web shell's packages, which the live parts are built from, and
            # the site's own, a pnpm root of its own (`site/pnpm-workspace.yaml`)
            # that no other lane installs.
            1:docs)
                pnpm install --frozen-lockfile --filter hide-web...
                # From inside `site/`, so pnpm reads that project's `packageManager` pin and
                # switches to it; `--dir` from here would run the machine's own pnpm against it.
                cd site
                exec pnpm install --frozen-lockfile
                ;;
            *)
                printf 'usage: %s install [--ignore-scripts | holder | docs]\n' "$0" >&2
                exit 2
                ;;
        esac
    fi
    (( $# >= 2 && $# <= 130 )) || { printf 'expected PACKAGE ACTION [args...]\n' >&2; exit 2; }
    package=$1
    action=$2
    shift 2
    case "$package" in
        web|desktop) directory=$package ;;
        mobile)
            [[ "$action" == test ]] || { printf 'mobile supports test only\n' >&2; exit 2; }
            directory=$package
            ;;
        site)
            [[ "$action" == build && $# == 0 ]] || { printf 'site supports build only\n' >&2; exit 2; }
            cd site
            exec pnpm build
            ;;
        relay-protocol)
            [[ "$action" == typecheck && $# == 0 ]] || { printf 'relay-protocol supports typecheck only\n' >&2; exit 2; }
            exec pnpm --dir web exec tsc -p ../hide-relay-protocol/tsconfig.json
            ;;
        *) printf 'unknown verification package: %s\n' "$package" >&2; exit 2 ;;
    esac
    case "$action" in
        typecheck|lint|test|build) exec pnpm --dir "$directory" "$action" "$@" ;;
        bundle|build:phone|build:docs-embed)
            [[ "$package" == web ]] || { printf '%s requires web\n' "$action" >&2; exit 2; }
            exec pnpm --dir web "$action" "$@"
            ;;
        e2e)
            if [[ "$package" == web ]]; then
                bash scripts/verify-cargo.sh build -p hide-relay-protocol --example phone_peer
                pnpm --dir web build:phone
            fi
            if [[ "$package" == desktop ]]; then
                pnpm --dir "$directory" build
            fi
            exec pnpm --dir "$directory" exec playwright test "$@"
            ;;
        e2e-list)
            # Discovery imports specs, but never launches the built app. Keep
            # preparation in e2e, which always builds desktop before execution.
            exec pnpm --dir "$directory" exec playwright test --list "$@"
            ;;
        playwright-install)
            [[ $# == 0 ]] || { printf 'invalid playwright-install arguments\n' >&2; exit 2; }
            exec pnpm --dir "$directory" exec playwright install chromium
            ;;
        electron-install)
            [[ "$package" == desktop && $# == 0 ]] || { printf 'electron-install requires desktop and no arguments\n' >&2; exit 2; }
            exec pnpm --dir desktop exec install-electron --no
            ;;
        package)
            # `compile` and `assemble` split one package in two (package.yml's
            # macOS lane): the release binaries, then the app from them. The
            # host's own build is part of the assembly.
            [[ "$package" == desktop && ( $# == 0 || ( $# == 1 && ( "$1" == compile || "$1" == assemble ) ) ) ]] \
                || { printf 'package requires desktop and nothing, compile or assemble\n' >&2; exit 2; }
            if [[ "${1:-}" == compile ]]; then
                exec pnpm --dir desktop exec node scripts/package.mjs compile
            fi
            exec pnpm --dir desktop package "$@"
            ;;
        *) printf 'unknown verification action: %s\n' "$action" >&2; exit 2 ;;
    esac
fi

pnpm install --frozen-lockfile

for package in web desktop; do
    # Web build already typechecks before bundling. Desktop's esbuild does not.
    if [[ "$package" == web ]]; then
        pnpm --dir "$package" build
        pnpm --dir "$package" build:phone
    else
        pnpm --dir "$package" typecheck
    fi
    pnpm --dir "$package" lint
    pnpm --dir "$package" test
    if [[ "$package" == desktop ]]; then
        pnpm --dir "$package" build
    fi
done
