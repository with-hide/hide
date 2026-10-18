# Agent workflow UI contract

This document maps the retained Agent workflow components in `design/hide-ui.lib.pen` to their code owners and behavioral contracts.
Product screens and historical candidates are not library content.
The former Final, R2, R3, handoff candidates, and the Overview family-only comparison are historical and must not be reconstructed.
The current contract is the approved v6 Sessions panel and v9 pane header, mapped to [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md).

## Product decisions

- The sidebar is one list of operator sessions and visible orphans; delegated children live in the direct-child badge popover unless core escalation makes them the operator's turn.
- The Sessions tool groups core-derived work into My Turn, Review/Merge, Progress, Resting and Today Resolved.
- Resolve hides a session from the sidebar and phone while retaining its pane, conversation and graph presence.
- Overview opens Agents and retains Tasks/Projects globally, or Issues/PRs/Conversations for a project.
- The pane header distinguishes the pane shown by Hide from the terminal that owns keyboard focus.
- Explorer decorates its existing file tree from the normalized Changes projection and does not own another Git reader.
- Provider artwork, identity, status, ownership, work and lineage come from the same canonical projection on every surface.

## Library component map

| Board | ID |
| --- | --- |
| Component / Pane header and focus | `mR198` |
| Component / Narrow and non-agent headers | `b6Nt3Q` |
| Component / Shared agent identity | `ZjPJ6` |
| Component / Hierarchy widths | `INj5G` |
| Component / Workspace disclosure | `g7trZv` |
| Component / Tree interaction states | `m0gZNn` |
| Component / Browser file diff toolbars | `p0Do2` |
| Component / Search and Recent Sessions identity | `dexoX` |
| Component / Explorer Git states | `BMEZi` |
| Component / Child navigation outcomes | `ZEwQt` |

The deleted `Review / UI Handoff / 00 Start here` and `90 Shared dependencies` boards were navigation aids, not product masters.
Their adopted reusable content lives in the library; product compositions are reviewed in task-local scratch and implemented in code.

## Master ownership

| Master | Code owner | Contract |
| --- | --- | --- |
| Agent identity `HXWFK` | `web/src/agentRow.ts`, `web/src/components/sidebar-agent-row.tsx`, `web/src/components/agent-row.tsx` | One provider artwork, title, canonical state, ownership, and instrumentation identity across live lists, search, relationship, and Overview. |
| Tree row `n1zn1O`, lineage segment `N6BTh4` | `herdr-core/src/sidebar.rs`, `web/src/sidebar.tsx`, `web/src/lineage.ts` | Disclosure and selection remain separate, and lineage columns remain aligned through multiline rows. |
| Workspace row `pPtY6` | `herdr-core/src/sidebar.rs`, `web/src/sidebar.tsx`, `web/src/projects.ts` | Workspace membership and agent delegation stay distinct, and expanding a Workspace changes no pane, tab, or read state. |
| Focused pane `ZvLjg`, icon header `Z3BnL` | `web/src/PaneView.tsx`, `web/src/PaneHeaderBand.tsx`, `web/src/PaneRelations.tsx` | One quiet identity row, direct-child badge and one overlaid state band preserve shown wash, focus outline, zoom and overflow. |
| Direct-child badge `PaMPI` | `web/src/PaneRelations.tsx`, `web/src/components/agent-children-popover.tsx` | Status counts open the shared direct-child list; All opens Overview Agents. |
| Browser chrome `KUcQU`, address toolbar `eMlZD`, document toolbar `hxdu7` | `web/src/BrowserDisplay.tsx` (browser), `web/src/Editor.tsx`, `web/src/viewers/FileViewer.tsx` (document) | Browser, file, and diff surfaces keep their own controls and never inherit agent-only actions. |
| Agent identity `HXWFK` in the direct-child popover | `web/src/PaneRelations.tsx`, `herdr-core/src/runtime.rs` | A child click or Enter navigates; unavailable targets retain their reason and All opens Overview Agents. |
| Relationship action `p7Vim` | `web/src/PaneRelations.tsx`, `web/src/lineage.ts`, `herdr-core/src/runtime.rs` | One request ID carries Pending, Target unavailable, refusal and timeout outcomes across the popover, Return control, raised-child band and retained canvas. |
| Explorer Git row `mSu8p`, panel `Wo6qx` | `web/src/ExplorerTree.tsx`, `web/src/explorer.ts`, `herdr-core/src/changes.rs` | A fixed status slot renders M, A, U, R, conflict, folder-changed, and clean states without changing file-tree interaction. |

## Session scope and lineage

The sidebar and Sessions tool use the core's final ownership, escalation and resolve values.
A delegated child stays in its parent's badge while that parent can handle the work.
Closing the parent pane or ending its agent makes a remaining child a visible orphan.
The Sessions tool follows the front checkout's project; Home includes every project on the selected device.
The checkout chip narrows that scope, and only the front checkout receives the left selection bar.
Numeric shortcuts follow the sidebar's drawn order and deduplicate repeated sessions.

## Pane header and focus

The identity row is 28pt: child-only parent Return, status mark, provider, title, real session PR/CI, Zoom, Menu and Close.
The second-row terminal overlay is only the operator's turn or the pane's children: a parent's children button (`자식 N`, after the operator-raised count) keeps its right end, and without an ask the rest of the row names each child with its row mark, a child with news first.
Shell panes carry only the shell mark, title and pane actions.
The title truncates before retained facts and actions; the parent name becomes an icon first at narrow widths.
Not connected, instrumentation uncertainty and the zoomed `+N` remain in this row.
The old child-chip row and relationship sheet are removed.

One ask band overlays the terminal without changing its viewport or PTY size, and stays until the ask is answered, dimmed once read.
Priority is own Approval/Answer/Blocked, raised child, then own Stopped; the pane's own ask leads with its verb and a raised child's with that child's name.
Fix, Review, Merge and Result have no band: the identity row's PR mark and status mark say them.
Working uses only a thin blue line, and CI wait, Waiting, Idle and Shell have no band.
Why a terminal is not live (device away, restoration failure, sleep, ended, unavailable, reconnecting) is said in the pane body, never on the second row; nonzero exit and Unavailable use red.
The band carries the core reason and the same elapsed-time reference as Sessions.
Only a raised child exposes Open; connection and sleep actions stay in the body.
The removed band's covered terminal rows reappear without resizing.

Click, Enter or Space on the child badge opens the shared direct-child popover.
Up/Down selects a child; Enter or click opens its Workspace, tab and pane.
Escape returns focus to the badge, and All opens Overview Agents.
Open and parent Return remain pending until the core publishes the outcome for that exact request ID and target.
The same pending intent blocks duplicate execution.
An unavailable target is refused before dispatch, while a core refusal, timeout, retirement, or remote-control failure ends only the matching request.
Both paths keep pane geometry and tab topology intact, display a scoped reason, and expose Retry only when the core outcome permits it.
Retry starts a new request after the failed one has settled.
The result remains visible in the retained canvas when navigation removes the source popover or header.
The shell does not treat `lastError`, an inactive historical layout, or optimistic remote navigation as success or failure, and it emits no rollback focus event or UI-owned timeout.

The inline child-navigation outcome sheet `ZEwQt` and its state references are retained in the committed library.
The sidebar's inactive checkout/project states reuse the adopted row masters and retain the existing Project row as a maintained component.

The header wash marks the pane currently shown by Hide.
The outer primary hairline marks the terminal that owns keyboard focus.
Moving keyboard focus into Overview retains the shown wash and removes the terminal outline.
Unread weight is not reused to mean parent, child, delegated, or selected.

Zoom or Restore stays at the right edge in every state.
Fork, ports, sibling access, and other secondary capabilities live in the existing overflow menu.
Close remains separate and preserves the core-owned consequence check.
At narrow widths the parent name falls back to its icon before current identity or actions are lost.

## Explorer Git decorations

The Explorer's file tree uses Seti rows and reserves one trailing Git status slot.
The filename truncates before that slot and the row's file open, disclosure, rename, drag, keyboard, and context-menu owners remain unchanged.
The badge is informative and intercepts no click.
The tooltip and accessibility label contain the full relative path and status name.

| Mark | Meaning |
| --- | --- |
| M | Modified |
| A | Added |
| U | Untracked |
| R | Renamed, with previous and current relative paths retained by the projection |
| ! | An actual Git conflict |
| `●` | A folder with at least one changed descendant |
| Empty slot | Confirmed clean or no decoration available |

Deleted paths remain in Changes and do not become fake Explorer rows.
An unsaved editor buffer remains editor state and is not relabeled as Git M.
Folder aggregation derives from the complete changed set, including deleted descendants that the current tree cannot enumerate.
The highest-risk descendant controls the folder's semantic help while the visible folder mark remains `●`.

`ChangedFileStatus` carries Modified, Added, Deleted, Untracked, Renamed, and Conflict.
`ChangedFileSnapshot.previous_relative_path` carries the source side of a rename while `relative_path` remains the destination that can be opened.
The Changes reader requests Git status while Explorer is visible or a Changes/diff surface needs it.
It refreshes at the existing two-second bound, runs outside the runtime mutex, and performs no row, hover, or scroll subprocess.
Changing the focused checkout replaces the root-scoped decoration set, so one Workspace never paints another's status.
Git loading or failure appears above the file tree and never erases usable file navigation.

## Verification contract

Static acceptance runs `node scripts/gen-pen.mjs`, `node scripts/check-pen.mjs`, and `node scripts/check-design-contract.mjs`.
Code acceptance runs the required Rust and web commands from the implementation worktree (`bash scripts/verify-cargo.sh test`, `bash scripts/verify-web.sh`).
Manual acceptance in the desktop app uses exactly one identified candidate Hide app and one isolated Herdr server.
Screenshots must compare the approved v6/v9 baselines with candidate Sessions and pane-header surfaces in Light and Dark, including long Korean titles, narrow widths, child navigation, shown/focus distinction and terminal geometry.
Idle and driven observations must record their load and must not turn one bounded run into a universal performance claim.

This contract applies Design principles 2, 4, 7, 9, and 12 by matching the operator workflow, rendering derived ownership and Git state, separating every availability state, and checking Korean and English at real widths.
It applies Engineering principles 4, 5, 7, and 13 by keeping failures caller-visible, extending the existing projection and readers, and fixing rename, conflict, lineage, and root-isolation classes rather than individual screenshots.
