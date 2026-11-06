#!/usr/bin/env bash
# The capabilities that shell out - the changes view's Git reader, the port
# reader, the project panel's worktree, GitHub, and disk readers, the
# Background AI provider probe, and the Weekly Usage reader's `claude -p
# /usage` child - never run under the runtime mutex or on a per-event path.
# All but one are driven by the session-sync coordinator thread. The changes
# reader is driven by its own pump (`ChangesPump` in `changes.rs`), which the
# core starts once: History reads each checkout through that checkout's host,
# so a device's History must not wait on this machine's session sync.
#
# Both properties are structural, so they are asserted structurally: the
# runtime module holds the mutex, so it must fork nothing; and the readers'
# entry points must be reachable only from their one driver.
set -euo pipefail

cd "$(dirname "$0")/.."

readers=(changes ports worktrees github github_poll disk ai usage)

# These readers reach the network, walk a whole tree, or start a provider CLI,
# so they must also move the blocking part off the coordinator thread itself.
# `gh pr list` takes seconds, `du` over a build tree takes longer, and the AI
# probe starts a `codex app-server` child and runs `claude auth status`, and
# the usage reader runs `claude -p /usage` for seconds; run inline, any of
# them would be that much added latency on every pane event.
worker_readers=(changes worktrees github github_poll disk ai usage)

# 1. The module that holds the mutex, its runtime submodules, and the file
#    module it calls synchronously while holding it, execute no subprocess at
#    all. The test module at the end of each file is exempt: its fixtures build
#    git repositories in a temporary directory with no runtime and no mutex.
runtime_sources=(hide-core/src/runtime.rs)
for source in hide-core/src/runtime/*.rs; do
    [[ -f "$source" ]] || continue
    # This file is the parent `#[cfg(test)] mod tests` body. Its fixtures
    # intentionally create temporary repositories and are not runtime code.
    [[ "$source" == hide-core/src/runtime/tests.rs ]] && continue
    runtime_sources+=("$source")
done
for source in "${runtime_sources[@]}" hide-core/src/files.rs; do
    boundary="$(awk '$0 == "#[cfg(test)]" {print NR; exit}' "$source")"
    forks="$(awk -v boundary="${boundary:-0}" \
        'boundary > 0 && NR >= boundary { exit } /Command::new/ { print FILENAME ":" NR ": " $0 }' \
        "$source")"
    if [[ -n "$forks" ]]; then
        printf '%s runs a subprocess while the runtime mutex is held:\n%s\n' "$source" "$forks" >&2
        exit 1
    fi
done

for reader in "${readers[@]}"; do
    module="hide-core/src/${reader}.rs"
    if [[ ! -f "$module" ]]; then
        printf '%s is missing; the reader it asserts no longer exists\n' "$module" >&2
        exit 1
    fi

    # 2. Only the reader's one driver drives it. A call from anywhere else
    #    would put a `git` fork back on an event or a snapshot pull. The
    #    changes reader's driver is the pump in its own module, so nothing
    #    outside that module may name it.
    # Inline test fixtures do not drive production readers. Apply the same
    # test-module boundary as the subprocess check above; a test name alone
    # must not be interpreted as a runtime call site.
    callers="$(
        while IFS= read -r source; do
            [[ "$source" == "$module" ]] && continue
            [[ "$source" == hide-core/src/runtime/tests.rs ]] && continue
            [[ "$source" == hide-core/src/runtime/tests/* ]] && continue
            awk -v reader="$reader" '
                /^#\[cfg\(test\)\]/ { exit }
                $0 ~ reader "::.*Reader|" reader "_reader" { found = 1 }
                END { if (found) print FILENAME }
            ' "$source"
        done < <(find hide-core/src -name '*.rs' -type f | sort)
    )"
    driver="hide-core/src/session_sync/coordinator.rs"
    [[ "$reader" == changes ]] && driver=""
    if [[ "$callers" != "$driver" ]]; then
        printf 'the %s reader is driven from outside %s:\n%s\n' \
            "$reader" "${driver:-its own pump}" "$callers" >&2
        exit 1
    fi

    # 3. The reader answers a request rather than reading on every wake, so it
    #    cannot become a per-tick fork.
    if ! grep -q 'fn read_if_due' "$module"; then
        printf '%s no longer gates its reads behind a refresh window\n' "$module" >&2
        exit 1
    fi
done

for reader in "${worker_readers[@]}"; do
    module="hide-core/src/${reader}.rs"

    # 4. The slow readers hand their subprocess to a worker thread rather than
    #    running it on the thread that applies every engine event.
    if ! grep -q 'BackgroundRead' "$module"; then
        printf '%s no longer runs its subprocess on a worker thread\n' "$module" >&2
        exit 1
    fi
done

# 5. The changes pump is started by the core, once, and nowhere else.
pump_starts="$(grep -rl 'ChangesPump::spawn' hide-core/src --include='*.rs' | sort | tr '\n' ' ')"
if [[ "$pump_starts" != "hide-core/src/handle.rs " ]]; then
    printf 'the changes pump is started from %s, not only by the core in handle.rs\n' "${pump_starts:-nowhere}" >&2
    exit 1
fi

# 6. The coordinator reads each request under the lock and releases it before
#    the reader runs.
for request in read_worktrees_request read_github_request read_github_poll_request read_disk_request read_ai_request; do
    # The binding may destructure - one lock acquisition can answer for more
    # than the request - so what is asserted is that the request is read out of
    # a `let Some(...)` before the reader runs, not the exact binding shape.
    if ! grep -qE "let Some\([^=]*request[^=]*\) = ${request}\(" \
        hide-core/src/session_sync/coordinator.rs; then
        printf 'the coordinator no longer reads %s before running its subprocess\n' "$request" >&2
        exit 1
    fi
done

printf 'capability readers run off the runtime mutex, off every per-event path, and off the coordinator thread\n'
