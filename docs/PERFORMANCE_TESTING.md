# Performance and rendering verification

Read this before investigating terminal lag, flicker, selection, scrolling, resize, tab switching, CPU, memory, or snapshot/attach performance.
This is a repeatable verification procedure, not a record that any particular build passed.
Keep raw evidence and the run verdict under `agents/runs/<slug>/`; never commit traces, recordings, profiles, transcripts, or screenshots.
Use [CONTRIBUTING.md](../CONTRIBUTING.md) for delivery gates and [dev-runtime.md](dev-runtime.md) for bundle identity.

This guide is kept as pieces under [docs/performance/](performance/), one file per subject, each naming the code it covers.
Read this table, then the pieces whose Code paths cover what you change.
The headings after the table keep every section name and anchor this file had, so a reference such as `docs/PERFORMANCE_TESTING.md, <section>` still lands here and names its piece.

| Piece | Code |
| --- | --- |
| [Measurement procedure](performance/procedure.md) | `scripts/web-shell-measure/`, `web/e2e/hided-fixture.ts`, `desktop/e2e/fixture.ts` |
| [Audit coverage and regression ownership](performance/audit-coverage.md) | `hide-core/src/handle.rs`, `scripts/check-capability-readers-off-lock.sh`, `scripts/check-shell-structure.py`, `web/src/store.test.ts`, `web/e2e/terminal-webgl.spec.ts` |
| [Resident tick inventory](performance/resident-ticks.md) | `hided/src/watch.rs`, `hided/src/login_item.rs`, `hided/src/mobile/mod.rs`, `hided/src/hook_request.rs`, `hide-core/src/fetch.rs`, `hide-host/src/git_watch.rs`, `hide-platform/src/watch.rs` |
| [Resident work and hook path costs](performance/resident-cost-review.md) | `hide-relay/`, `hide-agent-hooks/`, `hided/src/hook_request.rs` |
| [Web shell and browser gateway measurement](performance/web-shell.md) | `scripts/web-shell-measure/`, `scripts/holder-perf.py`, `hide-node/src/terminal/echo_timing.rs`, `hide-holder/` |
| [Projects and Overview costs](performance/projects-and-overview.md) | `hide-core/src/disk.rs`, `hide-core/src/runtime/disk_sweep.rs`, `hide-host/src/disk.rs`, `hide-host/src/cleanup.rs`, `web/e2e/projects-sidebar.spec.ts` |
| [Project Memory, Overview reads and terminal path costs](performance/project-memory.md) | `hide-host/src/memory.rs`, `hide-session/`, `web/src/overviewTasks.ts`, `web/src/terminalLinks.ts`, `web/e2e/pane-move.spec.ts` |

## Verification layers and current CI coverage

Moved to [Measurement procedure](performance/procedure.md#verification-layers-and-current-ci-coverage).

### Maintenance and review policy

Moved to [Measurement procedure](performance/procedure.md#maintenance-and-review-policy).

## Whole-app audit coverage

Moved to [Audit coverage and regression ownership](performance/audit-coverage.md#whole-app-audit-coverage).

### Invariant and regression ownership

Moved to [Audit coverage and regression ownership](performance/audit-coverage.md#invariant-and-regression-ownership).

## Resident ticks, timers and watchers

Moved to [Resident tick inventory](performance/resident-ticks.md#resident-ticks-timers-and-watchers).

## The opener guard hook on a shell call

Moved to [Resident work and hook path costs](performance/resident-cost-review.md#the-opener-guard-hook-on-a-shell-call).

## Letters at a tool event

Moved to [Resident work and hook path costs](performance/resident-cost-review.md#letters-at-a-tool-event).

## The guardrails on a pre-tool call

Moved to [Resident work and hook path costs](performance/resident-cost-review.md#the-guardrails-on-a-pre-tool-call).

## Resident work cost review

Moved to [Resident work and hook path costs](performance/resident-cost-review.md#resident-work-cost-review).

## 1. Define the claim before running anything

Moved to [Measurement procedure](performance/procedure.md#1-define-the-claim-before-running-anything).

## 2. Identify the build and protect the operator

Moved to [Measurement procedure](performance/procedure.md#2-identify-the-build-and-protect-the-operator).

## 3. Isolate runtime state before making fixtures

Moved to [Measurement procedure](performance/procedure.md#3-isolate-runtime-state-before-making-fixtures).

## 4. Reproduce with real interactions

Moved to [Measurement procedure](performance/procedure.md#4-reproduce-with-real-interactions).

## 5. Measure the boundary actually under discussion

Moved to [Measurement procedure](performance/procedure.md#5-measure-the-boundary-actually-under-discussion).

### CPU, lock contention, and idle work

Moved to [Measurement procedure](performance/procedure.md#cpu-lock-contention-and-idle-work).

### Memory and long-uptime degradation

Moved to [Measurement procedure](performance/procedure.md#memory-and-long-uptime-degradation).

## 6. Preserve the architecture while fixing the cause

Moved to [Measurement procedure](performance/procedure.md#6-preserve-the-architecture-while-fixing-the-cause).

## 7. Regression gates, cleanup, and verdict

Moved to [Measurement procedure](performance/procedure.md#7-regression-gates-cleanup-and-verdict).

## Web shell measurement

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#web-shell-measurement).

### Pane topology latency

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#pane-topology-latency).

### Restore after a reboot

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#restore-after-a-reboot).

### Judging a change

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#judging-a-change).

### The nightly performance lane

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#the-nightly-performance-lane).

### Screen key echo, lost keys and the terminal path's cost

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#screen-key-echo-lost-keys-and-the-terminal-paths-cost).

### The pane holder's path

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#the-pane-holders-path).

## Scoped browser gateway discovery

Moved to [Web shell and browser gateway measurement](performance/web-shell.md#scoped-browser-gateway-discovery).

## Projects and Overview cost contract

Moved to [Projects and Overview costs](performance/projects-and-overview.md#projects-and-overview-cost-contract).

### Project worktrees, disk and cleanup

Moved to [Projects and Overview costs](performance/projects-and-overview.md#project-worktrees-disk-and-cleanup).

## Project Memory cost contract

Moved to [Project Memory, Overview reads and terminal path costs](performance/project-memory.md#project-memory-cost-contract).

### Overview Work, backlog, History and Usage reads

Moved to [Project Memory, Overview reads and terminal path costs](performance/project-memory.md#overview-work-backlog-history-and-usage-reads).

### Pane movement cost contract

Moved to [Project Memory, Overview reads and terminal path costs](performance/project-memory.md#pane-movement-cost-contract).

### The doorbell's lines in a terminal

Moved to [Project Memory, Overview reads and terminal path costs](performance/project-memory.md#the-doorbells-lines-in-a-terminal).

### Terminal path links

Moved to [Project Memory, Overview reads and terminal path costs](performance/project-memory.md#terminal-path-links).
