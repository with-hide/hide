#!/usr/bin/env bash
# The web and desktop entrypoint for verification, as plain argv.
#
# Usage: verify-web.sh [install [--ignore-scripts] | PACKAGE ACTION [args...]]
# PACKAGE: web, desktop. ACTION: typecheck, lint, test, build, e2e, e2e-list,
# playwright-install, package and electron-install (desktop only).
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
        if (( $# > 1 )) || { (( $# == 1 )) && [[ "$1" != --ignore-scripts ]]; }; then
            printf 'usage: %s install [--ignore-scripts]\n' "$0" >&2
            exit 2
        fi
        exec pnpm install --frozen-lockfile "$@"
    fi
    (( $# >= 2 && $# <= 130 )) || { printf 'expected PACKAGE ACTION [args...]\n' >&2; exit 2; }
    package=$1
    action=$2
    shift 2
    case "$package" in
        web|desktop) directory=$package ;;
        *) printf 'unknown verification package: %s\n' "$package" >&2; exit 2 ;;
    esac
    case "$action" in
        typecheck|lint|test|build) exec pnpm --dir "$directory" "$action" "$@" ;;
        e2e)
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
    else
        pnpm --dir "$package" typecheck
    fi
    pnpm --dir "$package" lint
    pnpm --dir "$package" test
    if [[ "$package" == desktop ]]; then
        pnpm --dir "$package" build
    fi
done
