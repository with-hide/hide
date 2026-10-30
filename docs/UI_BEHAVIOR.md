# UI behavior

This document owns what Hide's UI does: the rules for how each screen and control behaves, independent of any platform's exact pixel values.
It replaces the behavioral content of the retired `DESIGN.md`.
Public component authority lives in `web/src/components/ui` and `web/src/components`, described in [DESIGN_WORKFLOW.md](DESIGN_WORKFLOW.md); numeric authority lives in `design/tokens.json`.
Each rule below names the code that owns it: a web owner in `web/src/` and, where the behavior is core-decided, a core owner in `herdr-core/src/`.
Interface wording is written here as the shipped Korean copy where that is what the screen carried when the rule was written (`요청`, `정리`, `맡기기`); every language, English included, reads its own text from the catalogs in `web/src/i18n/resources/`, and [LOCALIZATION.md](LOCALIZATION.md) owns that boundary.

## Web Workspace

The web shell's Workspace screen follows the approved S6 proposal, its View areas follow the approved boards of PRD S7, and its body follows PRD right-lane: Agent Views, one lane on the right over or beside them, and a tool rail, every control sitting once, on the container it changes.
Web owner: `web/src/WorkspaceScreen.tsx`, `web/src/workspace.ts`, `web/src/LaneDivider.tsx`, `web/src/TreeButton.tsx`, `web/src/laneCalls.ts`, `web/src/ViewAreas.tsx`, `web/src/Tools.tsx`, `web/src/viewLayout.ts`, `web/src/viewDrag.ts`, `web/src/viewFocus.ts`.

The Workspace's path and icons sit in the center of the window's top row (see [The window's top row](#the-windows-top-row)); there is no toolbar row under it.
The path is `project / checkout`, where the project opens that project's panel on the Overview's Projects (PRD sidebar-task-lens B41), led by the device's colored band when the Workspace is not this Mac's; a tab in Home reads `~/hide`, since Home is no project.
After the path, a Git checkout shows a small neutral FileDiff and the whole checkout's uncommitted tracked/untracked status-record count; zero is hidden, and a rename can count as two records because rename folding is disabled.
Clicking the count opens Changes idempotently and leaves terminal keyboard ownership in place; its tooltip explains the count, while a conflict, loading, unavailable or stale read has a distinct small mark and never claims a clean tree.
Upstream differences use ↑/↓ counts against the tracking branch, with zero counts drawn as a quiet compare mark and no-upstream/gone distinct; the counts compare with the remote as Hide last fetched it (PRD git-freshness D-01, D-02).
Hide fetches every registered repository with an `origin` in the background on the machine that holds it, a connected device's with that device's credentials: the one in front at most once a minute and again when the window comes forward after a minute, every other one at most every five minutes, one fetch per machine at a time, and none while no window is attached.
The fetch is `git fetch --prune --no-tags --no-recurse-submodules origin` without `FETCH_HEAD` and with automatic gc and maintenance off, so it moves only remote-tracking refs and never a branch, the index, a file or a submodule: of `origin`'s configured refspecs only those that write under `refs/remotes/origin/` are passed, with `--refmap=` so no other one applies, and a repository whose refspecs write only elsewhere (a mirror into its own branches) is not fetched at all, and it never opens a terminal prompt, askpass, keychain or Credential Manager window; Git before 2.29 cannot leave `FETCH_HEAD` alone, so there the fetch writes it.
A ref lock left by the operator's own Git skips that turn; a lock still held on the third fetch in a row, or two refs that cannot both exist, is a failure like any other.
The detail opens even at 0/0 and names the tracking ref and when the remote was last checked (`N분 전 확인`), or `아직 확인 안 함` before Hide's first successful fetch.
A failed fetch keeps the last counts dimmed and says since when they are old; a credential or network failure waits for the window to come forward, any other failure retries after 1, 2, 5 and then 15 minutes, and the failure goes to the diagnostic log once.
With the background fetch turned off in Project settings the project is not fetched and the detail shows the last `FETCH_HEAD` time, the operator's own fetch.
`herdr-core/src/runtime/fetch.rs` owns the schedule, `herdr-core/src/fetch.rs` runs it, and `hide-host/src/worktrees.rs` (`fetch_remote`) is the node's fetch.
Only the ↑ arrow uses the existing success green and only the ↓ arrow uses warning orange; both counts and the file count remain neutral, including when the branch is ahead and behind at once.
While the branch has commits to receive and none to send, the detail ends with `받기`, which moves the branch and its files to the tracking branch as Hide last fetched it, on the machine that holds the checkout: fast-forward only, with no fetch, merge commit, rebase or push, and only from the HEAD the detail showed.
While it runs the button reads `받는 중` and stays until the Git read shows the new HEAD (once the node has moved it, a failed Git read ends the wait too); then the counts are what changed, and no message says it worked.
When uncommitted, untracked or ignored files would be overwritten, nothing moves and the detail says `커밋 안 한 파일 N개가 겹쳐서 못 받았어요` with `다시 받기`; a checkout partway through a merge, rebase, cherry-pick or revert says `진행 중인 머지나 리베이스를 먼저 끝내야 받을 수 있어요`; any other failure says `받지 못했어요` with the same button, and its reason goes to the diagnostic log.
When the branch also has commits to send, the detail has no button and says `양쪽에 커밋이 있어 직접 합쳐야 해요`, because choosing between a merge and a rebase is the operator's.
A failing fetch leaves the button, which then takes the tracking branch as last received.
`herdr-core/src/runtime/checkout_git.rs` owns the run, and `hide-host/src/branch_merge.rs` (`catch_up`) is the node's fast-forward.
At the right, the current HEAD's subject and relative age open a compact history popover directly below that trigger, with up to five newest commits from the captured HEAD; no commit is invented for an unborn branch.
Above them, dimmed and marked `들어올 커밋`, sit up to five of the tracking branch's commits not yet in HEAD, with `+N개 더` when more are behind; while the fetch fails they are the ones last received and the list says since when.
A local commit no remote has is marked `푸시하지 않음` when `origin` is on GitHub.
When Git cannot say in time which commits a remote holds, or the upstream was pruned during the read, the header keeps its other facts, lists no incoming commit, and no commit opens a page; the reason goes to the diagnostic log once.
A commit GitHub can show, one some remote already has or an incoming one, opens its GitHub commit page in the Browser View from its subject; a commit only this checkout has does not.
Each compact item has its subject on the first line and short SHA and relative time on the second, with the exact local time available on the timestamp.
Its associated CI result sits at the right of that second line; commits with no CI omit the result.
The adjacent CI mark describes only that exact SHA, using GitHub's aggregate statusCheckRollup even on main without a PR, and distinguishes passing, pending, failed, none, unknown and unavailable.
The header's Git read, each CI result and the remote check share one observed shape (PRD git-freshness D-09, `herdr-core/src/observed.rs`): the last value read stays, with when it was read and whether it is fresh, stale or behind a failed read.
A CI read that fails, or ages past 90 seconds, keeps each commit's last result dimmed, not openable, with `GitHub 읽기 실패 · N분 전 기준` or `N분 전 기준` in its hint (B22); a commit GitHub never answered for reads unavailable.
A Git read that fails keeps the ↑/↓ counts dimmed with `Git 읽기 실패 · N분 전 기준` instead of hiding them.
Its trigger opens the same history below itself, with initial focus on the content so no CI hint covers the first item; Tab reaches the CI links and their read-time hints.
Each CI result opens only the provider-returned URL validated against that repository and exact commit in the existing Browser View.
An outside click closes the history and keeps the clicked destination's focus; Escape closes it and returns focus to its trigger.
CI describes committed changes only, and a provider error or unknown aggregate never becomes success.
Changing device, checkout, connection or HEAD fences old results; the front checkout's file status refreshes independently of the Changes lane and stale data is labelled.
On narrow widths the relative age disappears before the subject, then the subject contracts to the commit icon, preserving the path, count and trailing tools; details remain keyboard reachable and Escape returns to their triggers.
Compact commit and CI buttons keep their own icon and text footprint even when the open sidebar leaves less space than the window width; they never overflow into Overview or Open server.
When the shell disconnects, retained upstream claims disappear until the connection is live again, and a conflict count's accessible name identifies the total changed records before the conflicted subset.
Home and ordinary folders carry no Git header.
Web owners: `WorkspaceTitleBar` in `web/src/WorkspaceScreen.tsx` and `web/src/CheckoutGitHeader.tsx`; core owner: `herdr-core/src/runtime/checkout_git.rs`, read by `herdr-core/src/checkout_git.rs` on the existing Changes pump.
Its right end holds two tool icons, in this order: Overview and `Open server`; the lane's tools are on the rail (see The right lane).
`Open server` (a globe): one known listener opens directly as a page among the lane's documents, and multiple listeners open a compact keyboard picker.
Its accessible name carries how many listeners are known (`Open server, 2 running`), so a screen reader tells a globe that opens a page at once from one that opens the picker; while the count is unknown (a remote Workspace, no connection, a discovery that is still reading or failed) the name is `Open server`, and the tooltip always says `Open server`.
A Workspace of the machine the window runs on counts as local when that machine's core runs on another machine: the core reads that machine's listeners through its link, and its server opens in that window.
The picker shows each real bind address and port, preserves IPv4/IPv6 scope, and supports arrows, Home, End, Enter and Escape.
An empty, loading, disconnected, failed or stale catalog gives small feedback in the same popover; a remote Workspace explicitly has no local discovery.
Discovery never starts or stops a server, and an opened page retains the Browser View’s own connection failure and recovery controls.
Hovering or focusing an icon shows its name in a tooltip; `Open server` has no chord.
A right-click or the menu key on the path offers Show or Hide right lane and Copy Workspace path.
An empty Agent group or draft offers an agent start and a separate `Start with terminal` action.
An area showing a delegated child still draws that tab's canvas even though the tab stays off the ordinary strip, so choosing the child from the sidebar opens its pane rather than an empty start surface.

### The window's top row

Web owner: `web/src/TitleRow.tsx`, `web/src/windowChrome.ts`, `WorkspaceTitle` in `web/src/WorkspaceScreen.tsx`; desktop owner: `desktop/src/main/windowChrome.ts`, `desktop/src/main/host.ts`, `desktop/static/status.html` (PRD sidebar-titlebar-refresh D-15 to D-18).

The macOS desktop app has no system title bar and no `hide` title: the window's first row, 32 high in the sidebar's fill over a hairline, is where macOS's title bar and the Workspace toolbar were, and the traffic lights sit in it, centered on its height.
It opens with the sidebar's switch, the panel icon, which shows or hides the left sidebar as its chord does and names the chord in its tooltip (`사이드바 숨기기 ⌘B`, `사이드바 보기 ⌘B`); it stands at the same place whether the sidebar shows or not, so a hidden sidebar is one press away and a second press puts it back; like the chord, a click leaves the keyboard where it was, in a terminal above all, and Tab still reaches the switch.
Over the rail and the sidebar, after the switch, it names the device in front, `This Mac` or the device's name, as the sidebar's top line did, with the `This Mac ⌄` menu while the rail is hidden; its right edge is the sidebar's edge, so the divider runs from the window's top.
Over the center it carries the Workspace's path and icons while a Workspace is in front, and nothing on any other screen (Overview, a device that is not connected), whose own header stays under it as before.
The switch's glyph starts a step after the traffic lights' space, where the device name used to; with the sidebar hidden the path follows the switch, and full screen, where macOS removes the lights, gives the space back so the row starts at its own inset.
Dragging any empty part of the row moves the window and a double-click does what macOS's "Double-click a window's title bar" setting says; the switch, the path, the icons and the device menu take a press instead of a drag.
The desktop page's connecting and failure states keep a drag strip of the same height, so no screen draws under the traffic lights.
Window-wide notices (the connection badge, the notice line, the agent close notice, draft recovery and Workspace notices) run the window's width under the row.
Windows, Linux and a browser tab keep their own window frame and draw the same row with no traffic-light space.

### A part that cannot be drawn

Web owner: `web/src/RenderBoundary.tsx`, `web/src/App.tsx`.

A part of the window that fails to draw takes no other part with it: the top row, each window-wide notice, the sidebar, the screen in the center, each overlay and the browser displays' bridge fail on their own, and the page keeps its connection to the core.
The sidebar, at its own width, and the center screen show one quiet line in their place, `이 영역을 표시하지 못했어요. 다음에 바뀔 때 다시 표시해요.`; the top row keeps an empty strip of its height the window is dragged from, and a notice, an overlay or the bridge that fails is left out.
A part is drawn again on the next change to the core's snapshot or to the page's own state, a new diagnostic not counted, and nothing listens for that change while every part draws.
The failure is a diagnostic naming the part and the error, once per error until the part draws again; there is no banner.
Only when the window's frame itself cannot draw does the line take the whole window, under the drag strip.

### The right lane

The Workspace body, under the top row, is Agent Views (the Agent area), one right lane, and the tool rail, 48px wide, at the window's right edge.
The lane has two columns (PRD lane-two-columns B1): the viewing column on the agents' side, which holds the checkout's documents (the View areas) or the 보기 slot, and the tool column beside the rail, which holds one tool; both, or one alone, are drawn, and with neither the lane is closed.
The rail's top is the viewing column's: the 보기 button, while the slot holds something, with the mark of what it holds (a pull request as its PR mark in its state's icon and colour, a record or a mission as a page, a project's worktrees as a Git folder, its cost as coins), then the documents button (a file icon) with a badge counting the open views, shown or folded, also given as its accessible description; with no document the button is off, still answering a hover or the keyboard with its hint `No open document` (B2).
Below a short rule the rail lists the tools, Project, Memory, Explorer and Changes from the top, the Project's tools before the checkout's, each an icon alone (a briefcase, a brain, an open folder, a file diff) whose name, scope and chord are its hint on hover (`Project · herdr-ide`, `Memory · herdr-ide`, `Explorer · settings-ui  ⇧⌘E`, `Changes · settings-ui  ⇧⌘G`) and whose name is its accessible name (B3; PRD lane-two-columns B25).
There is no Work tool, no Files tool and no PRs tool: the Sessions list is the sidebar's 태스크 list, the pull requests are the sidebar's PR list, and what Files held is split between the documents and the Explorer (D-03).
Memory is a tool of its own beside the Project tool, not a tab inside it, so what waits on the operator now and what Memory asks to be tidied each have their own icon and number (operator decision 2026-10-23, replacing PRD lane-two-columns D-20).
A second short rule between Memory and Explorer marks where the Project's tools end and the checkout's begin (PRD work-tab-pr-lens D-37); it is derived from each tool's scope, so a tool of the other scope moves it.
A rail button is drawn pressed, with a primary bar at the rail's edge, while its column shows it: a tool icon while the tool column shows that tool, the documents or 보기 button while the viewing column draws it.
Pressing a tool icon opens the tool column on that tool, or swaps the tool it shows; pressing the pressed one closes the tool column, and the viewing column stays (B14).
A closed lane that a tool icon, an open, a reveal or the 보기 slot opens shows that one column alone, the other closed, whatever the lane held when ⌘E closed it; only ⌘E brings back both columns (operator decision 2026-10-23, replacing the mockup's reopen of the other column).
Pressing the documents or 보기 button brings it to the front of the viewing column; pressed while it is in front, it folds the viewing column, and pressing it again brings the same content back (B11).
The Project icon is a briefcase and the Explorer icon an open folder, so neither repeats the Overview's dashboard mark or the other's folder.
The Project icon carries a warning-coloured number counting its project's 할 일, no number at 0, its hint ending in it, such as `할 일 2`; worktrees to remove, memory to move into documents and bots' pull requests are never in it (B38).
The Hide AI icon carries the same warning-coloured number, the sum of its 제안 and Memory tabs' numbers (see Hide AI: the Hide AI tool), none at 0 or while Hide AI is off, its hint ending in `제안 3 · Memory 1`.
At the rail's bottom, just above its `…`, the pin is an icon of its own, in the same place whether the lane is up or closed and whichever column it shows: an empty pin while unpinned, a filled pin drawn pressed while pinned, and its hint says what a press does (B15).
The rail's `…` menu holds Expand, which moves both columns together, and with the lane closed the `…` is off (B15).
A Workspace seen for the first time shows Agent Views alone, its lane closed.
The Explorer's and Changes' header names its tool, then its scope behind one small icon, a branch for the checkout, in the scope text's colour, and ends with `✕`, which closes the tool column (B14); the icon is decoration.
The scopes read `Explorer · <checkout>` and `Changes · <checkout>`; a new checkout in front updates them.
The Project tool has a head of its own (see The Project tool).
The viewing column has no header of its own: the documents draw their View areas with their tabs, and the 보기 slot its head (see The 보기 slot).

Unpinned, the lane opens over Agent Views at their right edge, set apart by a shadow and a left border in both themes; the terminals under it keep their grid and layout, are not dimmed, and keep their input and output (B20).
An unpinned lane stays up until the operator takes it down: a click on a terminal pane, choosing an agent or a tab, another Workspace in front and a restart all leave it as it was.
⌘E closes the lane whole, and Escape inside an unpinned lane or on its rail closes it the same way and hands the keyboard to the focused pane, so a tool, an open or a reveal afterwards opens its own column alone and only ⌘E brings both back (operator decision 2026-10-23); Escape first closes a menu, popover or search open in the lane, and Escape in a terminal is the program's.
Pinned (`Pin · keep it beside the agents`), the lane docks beside Agent Views, which narrow by its width once; unpinned (`Unpin · back over the agents`), Agent Views take their width back and the lane is over them again.
The pin is the lane's, whatever its columns show, and a pinned lane closed and opened again comes back docked.
A pinned lane docks in any window where its columns' minimums fit beside 480px of Agent Views, the viewing column giving up its width first and the tool column after, each taking its stored width back as the window widens; in a window too narrow for even that the two columns stack, the one brought to the front last drawn and the other behind it, and a body too narrow for even one column draws the lane over the agents as if unpinned, the rail's pin still filled and saying `Pin · too narrow, drawn over` (B19).
Expand gives the lane the whole body between the sidebar and the rail, the tool column keeping its width and the viewing column taking the rest; Expand again or Escape returns to the pin, widths and terminal layout as they were, no terminal resizes meanwhile, and Agent Views under an expanded lane take no pointer or keyboard (B15).
While the keyboard is in the lane, a document, a page, the 보기 slot or the tool, the Zoom pane chord (⇧⌘↩, Alt+Shift+Enter on Windows and Linux) is Expand and Expand again, so one key fills the window with whatever the operator works in; anywhere else it zooms the focused pane.
The tool column's `✕`, the pressed tool icon and the tool's chord close the tool column alone, closing no view; the documents' `✕`, drawn once at the right end of the tab bar of the area at the viewing column's top-right corner as a tool's head draws its own, closes the viewing column the same way, every view kept on the documents button (operator decision 2026-10-24); the viewing column also closes with its last document or the slot's `✕` when no documents stood under the slot; and a lane whose last column closes is closed, the keyboard going to the focused pane (B12, B13, B14).
⌘E (`Toggle right lane`) closes the lane while it shows and otherwise brings it back with the columns it had, so a lane closed whole by ⌘E or the path menu's Hide right lane returns both columns, the 보기 slot included when it stood in the viewing column; a lane whose columns were closed one by one has nothing to bring back and opens on the documents when the checkout has some, else on the Explorer (B15).
The columns are kept with the Workspace, so ⌘E after an app restart brings back the same, and the path menu's Show right lane does the same as ⌘E.
Documents coming back take the keyboard into their active View area, the editor of the document there or the page, as an open does, so typing, ⌘S and ⌘W act on what came back rather than on the terminal it was pressed from; a tool, or the 보기 slot, leaves the keyboard where it was (B16).

The documents are the checkout's View areas, with their tabs, splits, previews and browser displays, each page inside its area's bounds (see View areas).
The active document's header starts with the checkout's name, then the path in ` › ` steps, cutting a long path at its start so the file name stays; document and page toolbars keep their path or address and controls.
The Explorer tool is the file tree as it was, its rows, menu, drags and file management (see The Explorer); it has nothing of its own to turn off, so an Explorer open with no document is the tool column alone, with no disabled button and no `tree only` notice (B4, B5).
Opening a file, a diff or a page from the Explorer, Changes, ⌘P, a terminal link, Open in Browser or Open server shows the documents in the viewing column with the opened tab in front, over the agents or docked as the pin says, and leaves the tool column as it is (B6, B7); the preview, Keep open and Open to the side rules are unchanged.
A diff opened from Changes, a commit or a changed file, opens in the viewing column while the Changes list stays in the tool column (B7).
Select in File Tree, a reveal, or a terminal link that names a folder turns the tool column to the Explorer with the row unfolded and selected, and leaves the viewing column as it is (B8).
A pull request's card, a session's record, a mission's map, or a project's worktree table or cost opens in the 보기 slot, in front of the documents in the viewing column, and leaves the tool column as it is; the documents stay on the rail's documents button, and `←` steps back through the items opened before (B9).
A changed file pressed on a card opens its diff among the documents in the viewing column, the card staying on the rail's 보기 button, which brings it back (B10).
Closing a document tab closes that view alone, the unsaved-draft protection as it was; closing the last one closes the viewing column, the lane staying while it has a tool column and closing otherwise, and the 보기 slot never comes forward on its own (B12).
⇧⌘E (`Toggle Explorer tool`) and ⇧⌘G (`Toggle Changes tool`) press that tool's rail icon: the tool column opens on it, swaps to it, or closes when it already shows it (PRD lane-two-columns D-08; sidebar-task-lens D-19, D-29); `Toggle Project tool` and `Toggle Hide AI tool` do the same for the Project and Hide AI icons and have no chord until one is bound.
⇧⌘L (`Open Project Memory`) opens the Hide AI tool on its Memory tab, and ⇧⌘R (`Open PR list`) turns the sidebar to its PR list, the whole device's, showing the sidebar when it is hidden; neither closes what it opened when pressed again (B25).
`Toggle documents` presses the documents button and has no chord until one is bound.
On Windows and Linux these chords are Alt+Shift+E, L, R and G; in a browser tab Chrome keeps ⇧⌘R for reload, so the PR list is ⌥⇧R there, and ⌥⌘1-4 select a device tile in the desktop app (⌥⌘1-9, see the device rail) and do nothing in a browser tab.
A chord the operator had bound to `Toggle Memory tool` or `Toggle PRs tool` runs `Open Project Memory` or `Open PR list` instead, unless that command has a chord of its own (B25).
A chord the operator had bound to the retired `Toggle Work tool`, `Toggle Files tool` or `Toggle File Tree` is dropped without an error, and the rest of their set stays.
The desktop app's View menu names the lane commands, and a chord bound to any of them in Settings, Shortcuts keeps doing the same.
Each sends the value the lane should end at, so the same command arriving twice lands where it did once.
A right-click or the menu key on the Workspace's path offers Show or Hide right lane with the same effect as ⌘E.
A file read opens the documents when its document is ready, in the Agent tab that requested it.
Choosing another Agent tab while it reads leaves the checkout's Views and selection as they are, and closing a requesting tab drops only that tab's pending open.
When several tabs request the same file during its read, they share the read and document buffer while each keeps its own display request.
A buffer opened through another path before the read answers retains its draft and save state, and the pending GUI displays still open in their requesting tabs.
Closing that buffer's last current display does not cancel another tab's pending GUI open.
Choosing an agent or a tab from the sidebar, the palette, a tab cycle or the tab strip, or a terminal taking the keyboard, leaves the lane where it is, pinned or not; the lane keeps its columns and pin, and the documents keep showing the checkout's Views (see File Views per checkout).
A new checkout in front changes the documents and the Explorer to that checkout's; the 보기 slot, the window's, stays, and its rail button opens that checkout's lane when it is closed (B22).

⌘⇧P shows and focuses the sidebar.
`Toggle device rail` has no default chord either and is bound the same way; it shows or hides the device rail.

The lane has a divider at its left edge, docked or over the agents, and one between its two columns (B17).
A divider shows its grip on hover, keyboard focus and while dragging; a drag moves a guide with the pointer and lands once on release, and a focused divider moves one 32px step per arrow key, each a separator that reports its width.
Fast arrow presses retain every step before the preceding width has been confirmed, and reloading restores the final width.
The column divider trades width between the viewing column and the tool column, so the lane and Agent Views keep theirs; the lane's divider changes the width of the column at the lane's edge, against Agent Views when docked, and over them otherwise, where it may reach the body's left edge.
The lane keeps four widths (D-09): the documents start at 520px and are never narrower than 360px, the 보기 slot the same, the Explorer starts at 240px and is never narrower than 180px, and the Project tool, Memory and Changes share one width that starts at 400px and is never narrower than 360px; Agent Views keep 480px beside a docked lane.
The widths are the Workspace's and the same over the agents and docked, so a width dragged over the agents is the width the pin docks at, and documents widened to read a page leave the 보기 slot and the tools as they were.
A web page opened into the lane widens the documents, when they are narrower than 900px, to 900px, so a site lays out its desktop screen: an open from the Explorer, a terminal link, `hide browser open` from the operator's pane, History, Recent Panels or a page's own new tab that puts a page in front of its area (PRD browser-width-zoom B1-B5; lane-two-columns B18).
It widens over the agents and docked alike only as far as Agent Views keeps 480px beside both columns, so a narrow window gives the page less than 900px, and it never narrows wider documents.
The widened width is stored as a divider landing is, so a narrower width the operator drags afterwards stays through a file tab and back; choosing a page tab already open, a restart, an agent's open behind the page in front and an expanded lane widen nothing, and a width the core refuses leaves the page at the width it opened at, with only the log saying so.
The pin turning on or off, a docked lane opening or closing, a column of a docked lane opening or closing, a docked divider landing, and a docked viewing column turning between the documents and the 보기 slot when their widths differ resize the agents' terminals once (B21).
A lane over the agents opening, going away, coming back, expanding or closing, its divider landing, a drag in progress, and anything inside the lane, an open, a close, a tab change, a split or a drag of a view, resize nothing.
Changing Workspace or lane geometry during a divider drag cancels its guide without saving a width, and releasing the old pointer cannot change the new Workspace.
The lane's columns, pin and four widths are stored per Workspace and survive a restart: a pinned lane comes back docked, an unpinned one over the agents, and neither expanded (B17).
A Workspace stored by a build with the one-tool lane comes back closed at the default widths, its open documents, splits and comments kept and counted on the documents button (B23, D-10).

Agent Views is always live: clicking a pane or a tab focuses it and typing goes to it, and chords such as ⌘F, ⌘T and ⌥W act where the keyboard is, a View area, the lane's tool or a pane.
A lane going out of sight with the keyboard inside it, or a viewing column that folds or closes while the keyboard is in its documents, hands the keyboard to the focused pane.
That handback retires the hidden region as a command target in the same drawing, even when the terminal input is already the shell's active element and no new focus event fires; input the operator moved to another live column or a popup keeps its owner.
Tab never walks into a folded or stacked-behind column: it is inert and holds no Escape layer or focus return, and a native page in it is hidden before it goes inert.

An Agent tab shows the focused pane's status mark, then title, using the sidebar row's title and mark rules; the agent's logo stands between them only while the agent logo view mode shows it ([Agent logo view mode](#agent-logo-view-mode)); a tab with no agent keeps the neutral `>_` in both modes.
A custom Herdr label wins; an empty, numeric, or `Tab N` label instead follows the focused pane's chosen agent title (manual name, Hide AI goal, automatic name, then provider, as [Task identity](status-model.md#task-identity) defines), foreground process name, then `Tab N` using Herdr's stable number; a Herdr workspace label is never a tab or agent name.
A plain terminal tab shows the terminal icon and that name; terminal titles are never used to guess a process.
Local and remote process names are read only for focused panes of attached tabs, outside the runtime lock, on focus or agent-state changes and a 30-second recheck.
The foreground process is the process-group leader, or the last returned process when the leader is absent; its name is the basename of `argv0`, falling back to `name` only when `argv0` is empty, and to `Tab N` when both are empty.
The tooltip and accessible name carry the kind, full name and agent state; widths, scrolling, truncation and close behavior stay the same.
Every Agent tab reuses the same name and Rename field in its area menu described below.
Rename opens an inline field with the displayed name selected: Enter saves to that host's Herdr, an empty name restores automatic naming, and Escape or blur cancels editing.
While saving, the committed name stays unchanged; a refusal or timeout keeps the entered text with “이름을 저장하지 못했어요 · 다시 시도” below it, Enter retries, and Escape returns to the prior name.
Failure details go to diagnostics, with no banner; a successful name survives reconnect while Herdr keeps the tab.
Copy name copies the displayed name.
A View tab carries the file-type mark, and a diff tab the comparison mark, so a kind is never told by color alone.
Its title is cut at the tail to fit, and its tooltip and accessible name carry the kind, the full path, and `Preview` or `Unavailable` while the view is one, so a long path stays readable.
A dirty view shows a warning-colored mark after its title.
Closing a view is always called Close view, distinct from moving a file to the Trash and from closing a pane or tab.
The close chord, ⌘W in the desktop app and ⌥W in a browser, closes the smallest unit that holds the keyboard: the focused View area's active display, or the focused terminal pane.
Before that, the close chord and the close-pane chord (⇧⌘W, ⌥⇧W in a browser) close what covers the screen, as a window's close does: the innermost menu, popover, dialog or sheet (Settings, Shortcuts, the Overview over a Workspace, the Start panel), a close's confirmation, which stays unanswered as with Escape, or the Overview screen, which gives way to the Workspace in front.
Windows and Linux press the same chords through the system rule, Ctrl+Shift+W and Alt+Shift+W in the desktop app, and plain Ctrl+W stays the terminal's.
Nothing under a cover closes, since the operator cannot see it; a filter, a field being added, a drag and a pane's find bar answer Escape but cover nothing, so the chords pass them.
The page records its last focused region as View area, pane, the lane's tool, the 보기 slot, or none from focus events, including native browser-page focus; the chord does not walk the active DOM element or choose a region from panel size.
When the desktop app hands the keyboard back from a native page to deliver a menu command, the focus the shell's last element regains is not a move while that command runs, so ⌘W and ⌘T pressed in a page act on that page's View area; once the command has run, the keyboard's owner is the shell element holding it, so after any other command from a page the next chord acts where the operator now types.
While the viewing column draws the 보기 slot, the chord closes the slot as its `✕` does when the keyboard is in the slot or nowhere (the sidebar row or the rail button that opened it); a pane or a View that holds the keyboard keeps the chord, and the tool column closes nothing (PRD lane-two-columns B13, B16).
Otherwise, with the lane's tool or nothing focused, nothing closes and a diagnostic states why (PRD lane-two-columns B16).
Tabs close only from their own close control or menu; closing a tab's only pane still removes its tab through Herdr.
An unavailable or retired keyboard target never falls through to closing a different pane or a whole tab.
When a connected pane selects, splits, moves, or closes a View through `hide view`, the same View layout rules apply to that pane's Workspace even while another Workspace is in front.
These commands do not move the keyboard target; a close that would lose the last View of an unsaved document reports the refusal and keeps the draft.
`hide file open` and `hide diff open` put the calling pane's file or changed-file diff into its own Workspace and never select that Workspace or another tab.
Each of them, and `hide browser open` for a page, opens its View as a tab of the active View area, or brings the same file, diff or address forward where it already is; an agent never splits the View areas, which only the operator does (PRD unseen-agent-views D-24).
An open with `--beside` opens nothing and is refused with the next action to run it again without `--beside`.
What an agent opens is unseen until the operator looks at it, and waits in Needs You as `볼 것` (To look at) on its row ([status-model.md](status-model.md#a-view-to-look-at)).
Only an agent in the pane the operator is in, the selected pane of the front Workspace, changes what the operator sees: its open or `hide view select` brings the View to the front and shows the documents as a click does, the tool column staying as it was.
Another pane of that Workspace, of the same tab or another, adds its View to the checkout's list and leaves the selection, the active area, the keyboard, the lane and the tab on screen as they were, and the documents button's badge counts the new view (see File Views per checkout).
A Workspace not on screen is not what the operator sees: there, an agent in its active tab brings its View to that Workspace's front, so a page it opens loads and can be driven, and an agent in another tab adds its View behind.
A shell bound only to the checkout, which no pane vouches for (a pane Hide has not connected yet, or a command run with its pane reference removed), counts as another pane on the Workspace on screen and as the active tab on one that is not.
The desktop window's browser gateway, creating a page for a CDP client, brings its page forward, since a page loads only in front.
No option brings a View forward from another pane; the operator does that.
A browser page's own new target, bound to its area, does not open the lane.
`hide view split`, `move` and `close` apply to the checkout's list, from any tab, including a tab that is not on screen.

### Agent areas

Each checkout has an ungrouped collection and up to 32 named groups, one level deep.
Existing tabs, named or unnamed, stay ungrouped; a tab name never creates a group.
The group selector, actual tabs and New tab share the existing tab-bar row, with no Add agent button and no additional group header above the canvas, on local and connected devices.
Each group owns its Agent area tree, tab order and selection; selecting a group restores that tree, while Files keeps the checkout's own View tree and per-tab bookmarks.
The selector offers Create group, Rename group and Ungroup, and the tab menu can create a group around a tab, move it to another group or remove it from its group.
A move puts the whole tab at the end of the destination's active area and selects it; its panes, processes, conversations and bookmarks stay with it.
An emptied non-root area collapses, but an empty group's root stays.
Group names are trimmed, contain 1 to 80 Unicode characters with no line breaks or control characters, and must be distinct by exact name within the checkout; groups stay in creation order.
Ungroup moves the group's tabs in order to the end of the ungrouped active area and closes no session.
Undo restores the last ungrouping while its session-local receipt remains valid, until the next group mutation or the checkout's saved state is unloaded; Undo is shown only while it can act and does not survive restart.

New tab opens one terminal immediately in the chosen area's bar, in the selected group, without an instruction field or provider picker.
The button, keyboard shortcut and checkout menu use this same behavior on this machine and connected devices.
Previously saved drafts remain available to finish or close.
An empty group creates its first draft when a start is requested.
Empty groups and grouped or ungrouped drafts show the instruction field, the shared provider/model picker and Start, with a terminal icon and secondary `Start with terminal` button always visible beside that start flow.
Terminal start needs neither an instruction nor an agent selection.
The first start fills that draft's position with the live tab.
Existing starts that add a pane to a live tab retain their progress, failure and recovery form; New tab remains the tab bar's creation entry point.
Starting in an enlarged pane returns that tab to its split layout and keeps its existing terminals running.
While a start is being confirmed its temporary terminal occupies the existing reservation and does not appear as another tab.
The internal creation label is never shown as a pane or tab name and is not copied into a reopened terminal.
The picker uses the same supported kinds, model validation and remembered defaults as the [Start panel](#start-panel), with no effort choice.
Pending starts keep the submitted inputs, prevent another submission, and show their progress or actionable failure where the start was requested.
An uncertain result offers Check status for the original intent, never a fresh start merely because an answer was lost; a definite failure can retry that intent.
A failed status check shows its reason immediately and offers another check of the same intent while the original effect remains uncertain.
If a definitely failed start's tab has since moved, the form keeps its inputs and submits a new intent at the tab's current location.
Unsent input follows the same draft or live tab through moves, ungrouping and Undo.
The text survives navigation and failure for this page's lifetime, as the start panel's text does, but is not saved across an app restart.
After restart the saved intent can still be checked without recovering or resending its prompt.
If the original terminal's identity cannot be proven, the check keeps the result uncertain and never starts a replacement process.
A disconnected device disables the start controls and sends no start; Enter during IME composition does not submit.

Tabs have no numbered shortcut or hold keycap; ⌃⇥ steps through the recent tabs (see Keyboard shortcuts per system).

At most 64 normal Agent tabs are placed across all the checkout's groups together, counting drafts, live tabs and pending admissions; each group has up to six areas and three split levels.
New tab, Reopen that needs a tab, and protected replacement close count pending admissions and refuse before any external effect when full; the existing one-line notice asks the operator to close a tab.
After an ambiguous creation reply, Hide checks the request marker once without resending the mutation.
An unconfirmed creation retains its request-specific place and reports that uncertainty in the same notice; elapsed time alone never frees the place.
External tabs beyond the cap remain in Herdr topology, with their waiting count in that same notice, and enter the active area in authoritative order when a slot opens.
A waiting tab cannot take Hide's keyboard or active-tab selection.
This boundary is an Observer-approved, user-vetoable implementation assumption from the Agent groups contract.

Agent Views draws the selected group's area tree, separate from the documents' View tree.
Each area keeps its own tab bar and New tab button, and draws its active draft or live pane canvas; dividers separate areas.
Only the area holding the keyboard carries the user-accent selected-tab underline, across Agent Views and the lane.
Clicking a tab or pane activates its area and sends the keyboard to that pane.
All shown tabs of the selected group stay attached and awake, while only its active area's live tab receives read and sleep-visit updates.
Hidden groups add no shown-tab attachments, polling or PTY restart; the existing recent-tab attachment policy still applies.
Pane headers, child chips, relationship controls and pane splits remain inside each canvas; find belongs to the focused pane.
⌘F on a full-screen Claude Code or Codex pane opens that agent's own search over its whole conversation in the pane, with the agent's own keys and count (Claude Code: type, Enter, `n`/`N`; Codex: type, Enter, Ctrl+P), and no find bar appears; any other pane, and an agent drawing inline, gets the find bar.

An Agent area's tabs share its bar the way a browser's tabs do (the Pen library's `Component / Adaptive Work Tab`), shrinking in three continuous stages so the bar is used to its end and the selected tab keeps its title longest.
While an equal share still holds the title minimum, each tab asks for the preferred width and all shrink alike with their titles.
Below that the selected tab keeps the title minimum with its title and close control, and the other tabs split the rest alike down to the icon identity width.
An unselected tab narrower than the title minimum draws its marks and a truncated title with narrow padding and no close control, and below the icon identity plus one control it draws its marks alone, centred.
Once every other tab is a mark, the selected tab gives up width from the title minimum, keeping its close control and a truncated title while it holds the icon identity plus two controls, and below that it becomes its marks with its close control; less than one control's width can then stay empty at the bar's end.
The strip scrolls only when even that selected mark and the other marks overflow.
The same width and tab count always draw the same strip whichever way the window was resized, a resize never paints an overflowing frame, and opening, closing or selecting a tab shares the bar again at once.
A tab being renamed keeps the preferred width at every density, and each tab's tooltip and accessible name still give its agent, full title, state and agent pane count.
Two or more agent panes in the same tab add a neutral pane icon and count beside the title; shell panes and an agent's internal subagents do not count.
One agent pane shows no count beside the title.
The count folds away while renaming, at compact and mark density, or when the title slot holds less than two small controls, leaving the title and close or shortcut control their room.
The full count remains in the tooltip and accessible name at every density.
Agent and View areas share the drag, divider and narrow-window controls described below.
A drag keeps its original tab in place and changes no terminal size until a valid drop.
Dropping on a tab bar reorders or moves the tab; dropping on a content edge highlights the new half with Split left/right/up/down and creates another area on release.
Moving the last tab out collapses its area, as does closing it or its disappearance from Herdr.
Closing a tab, or its only pane, takes the tab out of its area the moment the close is approved, without waiting for Herdr: the area shows the tab it showed before it, or collapses when the tab was its last, and when the closed tab held the keyboard the keyboard goes to what is now shown in its place (the area's next tab, or the neighbouring area's), never to the next tab in Herdr's order.
When that close leaves the checkout in front with no tab at all, the operator is done there: in the same frame the screen goes back to the most recent checkout of this machine they were in before it that still holds a tab and is not being removed, as choosing it from the sidebar would, and a reload opens there.
With no such checkout, the screen stays and the area shows its empty state; a tab off the strip (a delegated child's) still keeps the checkout, and a close Hide makes on its own (an agent's sleep) never moves the screen.
Nor does a wake Hide makes on its own: a letter that wakes a sleeping agent, or a finished mission that puts its workers to sleep, leaves the tab on screen, the keyboard focus and the open windows as they are, and an agent whose sleep closed its pane opens its new tab behind the one on screen.
A close Herdr refuses, or one that fails before it is sent, puts the tab back where it stood, and the operator and the keyboard with it, to that checkout too, when nothing has moved since; a close whose result is unknown shows the tab again until the status check settles it.
A pane closing inside a tab that keeps other panes leaves the canvas the moment the close is approved, and its neighbours take its space as Herdr will lay them out; its sidebar row stays until Herdr confirms the close, and a close Herdr refuses or cannot answer draws the pane back where it stood.
A split, a zoom or unzoom and a pane resize Hide asked for are drawn at once, a split as soon as Herdr names the new pane, without waiting for Herdr's layout; a new live tab replaces its draft in the same area when Herdr answers its creation.
In the same frame a pane's rectangle changes, the terminal inside it takes the rows and columns that fit the new rectangle and its program redraws, without waiting for Herdr: a pane's neighbour widens the moment the pane leaves, a zoomed pane fills the tab the moment it is zoomed, and a split pane's terminal shrinks to its half as soon as Herdr names the new pane.
When Herdr's layout differs from the drawn one, Herdr's is drawn and each terminal in it takes that rectangle's grid, one more reflow; when it is the same, nothing moves again.
Changes to one tab's panes are made one at a time in the order asked, with up to eight waiting; a ninth waiting change is not made, and one Herdr refuses drops the changes waiting behind it and draws the layout Herdr confirmed.
Changes in different tabs never wait for each other.
A sole empty area shows the group's start surface.
Agent tabs cannot enter the lane, and a sole tab cannot split its own area.
Invalid size, area or depth limits show the forbidden cursor without an overlay; Escape, outside release and a vanished target leave the layout unchanged.
Each area scrolls its selected tab into view; when Agent Views are too narrow, an area switcher shows one area without changing the saved tree.

New tab, Split right/left/up/down, available directional Move commands, Rename…, Copy name and Close tab… form each tab menu.
A disabled split explains its reason, and opening the menu changes no selection.
Next and previous Agent area focus and Agent area grow and shrink are chordless commands of the shortcut registry (Settings › Shortcuts › Area commands, and the desktop Pane menu); they have no key until the operator binds one, and one that cannot run now does nothing.
An area's New tab adds at its end.
The new tab chord, ⌘T in the desktop app and ⌥T in a browser, opens where the keyboard is: with a View area focused it is that area's New tab, with an Agent pane focused it adds an agent tab at the end of the area showing that pane, and anywhere else (the sidebar, the Overview, the lane's tools, the Agent controls) it adds one to the Agent active area.
Agent Views and the documents keep separate active areas, so the chord follows the recorded keyboard owner, never whichever active area changed last.
Externally created ordinary tabs append to the ungrouped active area without changing the selected group or shown tab; Herdr's own reorder never changes Hide's area order.
Delegated-only tabs remain absent from every bar and occupy the active canvas when chosen from the sidebar.
Choosing a normal tab or agent elsewhere, including the sidebar, search and recent history, selects its owning group and area before showing its tab and pane.

Group membership, drafts, trees, ratios and active selections return after restart.
Only authoritative topology removes missing live tabs; an unresolved catalog or connection does not erase saved groups or drafts.
Reopen closed tab uses the former group, area and position when they survive, otherwise the ungrouped fallback.
An older build may discard Agent layout state; returning starts with all current tabs in one area and keeps the View layout.
A Workspace of an SSH device the core dials has the same named-group and draft controls but keeps its existing single-area restriction within each group.
A machine that dials in to the core, which is what the machine the core moved off becomes, keeps its Agent areas as this machine does: its tree, splits and active area are the same before a move, after it and after the move back, its Herdr shows the tab of the area in use, and the panes of every area's tab are live.

Closing a primary Herdr workspace's last tab or last pane while a linked worktree remains first creates a shell at the checkout root in the same area and position.
That tab stays in its area, marked closing, until Herdr confirms the close, so the area is never empty while the shell is made.
The existing close guards run first, and linked workspaces remain untouched.
If shell creation fails, nothing closes; if closing is refused, the shell remains.
Retry close reuses that intent's shell, while Dismiss ends the failed intent; detailed Herdr failures are diagnostic-only.

A tab the operator named is a place they keep, like a channel: its name outlives the agents in it.
A tab is named when the operator gave it its name through Rename, which Hide records for that tab; clearing the name in Rename makes it unnamed again.
A name `hide agent spawn` gave, or one given before Hide recorded Rename, still shows on the tab and its sidebar line, but the tab behaves as unnamed until the operator renames it.
Closing a named tab's only pane while that pane holds an agent (its close button, ⌘W or ⇧⌘W on it, its pane menu, or a subtree close aimed at it) first splits a shell at the tab's checkout into the same tab, then closes the agent pane.
The tab stays in its place and holds the keyboard while the shell is made, and the keyboard moves to the shell once it is there; Reopen closed tab brings the agent back beside the shell.
The failure rules above hold: if the shell cannot be made, nothing closes; if the close is refused, the shell stays, and Retry close closes beside that same shell, making another only when the operator closed it meanwhile; a shell Herdr made whose answer was lost, or that arrived after the close timed out, counts as that same shell.
Closing that shell while it is the tab's only pane, or Close tab… at any time, closes the tab and its name as before.
An unnamed tab, a delegated child's tab, a pane whose terminal Herdr could not restore and a sleep's own close behave as any other close.
Reopen closed tab and a wake give a tab back the name it had, named again only when the operator had named it, so a tab never named comes back unnamed rather than named after the agent it showed.
An agent that quits on its own leaves its shell in the pane, as Herdr starts agents inside the pane's shell, so its named tab stays without Hide doing anything.
A provider whose sleep closes its pane (Pi, OpenCode and omp; [ARCHITECTURE.md](ARCHITECTURE.md#agent-sleep)) still takes a named tab whose only pane it is while it sleeps, and its wake opens a new tab under the same name.

### View areas

The lane's documents, in its viewing column, hold one or more View areas, each with its own tab bar above its own view, split left/right or up/down as often as the limits allow.
A split divides one area in two along one axis, and either half can split again along either axis, so every arrangement of side-by-side and stacked areas is a tree of halves.
Each layout keeps its own active area for opening and layout actions.
The recorded keyboard owner alone gives one Agent or View area the user-accent selected-tab underline.
Every other area's tab bar uses the card background, and its selected tab keeps a foreground title and secondary background without the underline.
The keyboard View area has a 2px accent content border outside any native page slot; the other areas reserve the same transparent border.
The keyboard Agent pane has a 2px accent perimeter and a lightly accent-tinted header, in single, split and zoomed presentations.
Adjacent terminal panes share a 1 CSS px `--border` hairline, vertical or horizontal, using the leading edge of the second side so nested splits never double it.
The same hairline marks connected-device panes and their retained layout while disconnected, using each frame's internal leading edges.
It overlays content without taking terminal space or pointer input, stays below the focus perimeter, and leaves the resize grab and guide unchanged.
Single and zoomed panes have no internal hairlines; unzoom restores them.
This reuses the `Component / Agent column` library's second-pane border and the existing `--size-hairline` token across Neutral and Hide, light and dark.
Menus, search, tools and an inactive app show no canvas input emphasis; returning input restores it without changing selection or geometry.
A View's address and path controls retain that View as the target of Close and New tab while suppressing its canvas emphasis.
The content stays readable and unfiltered; focus adds no content blur, opacity reduction, capture loop or geometry change.
The same rule applies at every lane width, in the narrow single-area presentation and in both themes.
A View area's tabs use the same continuous shrinking strip as Agent tabs, with the same preferred, title-minimum and icon-identity widths.
The selected view keeps its title and close control longest; other views first give up their close controls, then their titles, and the selected view gives up its title only when no more width can be taken from the others.
At marks density a file keeps its existing file-type mark, a diff its comparison mark and a page its globe; the full identity remains in the tooltip and accessible name.
A saving spinner or unsaved-draft dot stays visible over a file's mark at that density, and the tooltip names saving, unsaved and tab-only draft state.
Only when even the minimum marks and selected close control outrun the area's width does its own strip scroll, with the selected view kept in sight on selection and resize.
Clicking a tab or a view, or moving focus to another area with a focus-area command, makes that area active.
The divider between two areas turns accent-colored on hover and keyboard focus, drags with a guide line, and lands once on release, so a drag never resizes a document or a terminal on every pointer move.
A focused divider moves with the arrow keys along its axis, one step and one change per press.
No area gets narrower or shorter than its minimum, a divider stops where either neighbour would, and each side of a split keeps between 15 and 85 percent of it.
An area whose last view leaves disappears, and its neighbour takes the space.
When the last view closes, the viewing column closes; the lane stays while it has a tool column and closes otherwise (PRD lane-two-columns B12; see The right lane).

### File Views per checkout

The checkout owns one Files View list, its split and its active area, and each View remembers the sessions that opened it and whether the operator did (PRD session-files D-10, D-26).
The Files strip shows only the Views the session whose pane has the keyboard opened, and the operator's own, which show in every session; in a split Agent tab the pane with the keyboard decides (D-13).
The operator's Views are those opened from the Explorer, ⌘K, the file tree, a new tab's address field or an address typed into a page, by a caller Hide could not bind to a pane, and every View that was open before the update without an opener mark (D-29).
A pane with no agent session, a plain shell, shows only the operator's Views, and a path clicked or opened there is the operator's (D-28).
A session is its terminal or its native session, so a session found again in another pane keeps its Views, and an agent that exits to its shell or sleeps keeps them too (D-27).
The same rule holds for a file, a diff, a page, an image and a PDF (D-30); there is no folded list of other sessions' Views, which show when their session has the keyboard (D-15).
Switching Agent tab or pane, through the strip, sidebar, search, cycling or a confirmed local or device switch, switches the strip and the shown View together: each session shows again the View it last showed, the first time its first View, and with none of its own the operator's; the previous View does not stay behind as a dimmed tab (D-12).
Plain shells, and agents that have opened nothing in the checkout, share the operator's strip, so moving the keyboard among them leaves the View in front where it is.
A View named from elsewhere, a pane's `N views` row, Recent Panels or a session record's gallery tile, that the strip does not show brings the session that opened it to the keyboard first, so it shows in that session's strip.
An area that holds none of the strip's Views is not drawn, and its neighbour takes the space; a dragged tab lands among the Views the strip draws.
The lane, its tool, pin, tree and widths, and document editing and saving remain shared by the Workspace.
A session's Views close when Hide closes its pane, its tab or its subtree, or archives it (`세션 보관`), and only then; a View another session also opened, and the operator's, stay (D-14, D-27).
Before such a close, a View of a document with unsaved work that only that session's Views show asks once for the whole close: `Save and close` saves each and then closes, `Close without saving` drops the text, and `Cancel` closes nothing; a document whose text this page does not hold stays open as the operator's View, as does one a close that did not ask finds unsaved.
A pane that closes outside Hide, such as a shell's `exit`, closes no Views; they leave the strip with their session and the cap below closes them first.
Closing the lane preserves its documents, unsaved text and save state.
Closing the selected input View chooses its area's most recently visited surviving View, then the right neighbour and then the left when none was visited, and the keyboard follows the confirmed result.
Closing a background View, including the selected View of another area, leaves input where it was.
A View context menu borrows the current input without selecting its target, and its automatic focus return does not cancel a pending close follow.
An emptied area collapses and input follows its surviving neighbour.
The last View returns input to the existing tree if it is on, else closes the lane and returns to the existing Agent pane; with no pane there is no invented input target.
A last document reference waits for save protection; failure or conflict leaves it open, and a newer input destination wins over a delayed close completion.

From the pane the operator is in, a file, diff or browser open or a select brings the View to the front, its area becomes the active one and the lane is called to Files.
From another pane of the Workspace on screen, of the same tab or another, the View is added at the end of the area it lands in, in that pane's session's strip; the selection, the active area, the keyboard, the lane and the tab on screen stay as they were, and the Files icon's badge counts the Views the strip on screen draws.
On a Workspace not on screen, the agent in its active tab brings the View to that Workspace's front and an agent in another tab adds it behind; neither moves the screen or calls the lane.
A shell bound only to the checkout counts as another pane there; the browser gateway's new page comes to the front of its area, where it loads.
The operator brings it forward from the strip; selecting the caller's tab does not.
A GUI action, such as a click on a terminal link, acts for the operator as before.
A file read remains bound to the original tab and native session: a closed, moved or replaced caller is refused rather than falling back to the operator's tab.

Matching checkout and path documents use one shared buffer, and the same address opened from two tabs is one page.
Dirty-document close guards count every reference.
A page added behind the front may stay pending until it first appears, and `--wait` proves only a real load.
The desktop hides every retained page absent from the current placements.

The list, split and selection are saved with the Workspace and survive restart and core moves.
Obsolete `tab_views` bookmarks are ignored without erasing the saved `view_layout`, lane, documents or comments.
A first start after the update shows an empty list: Views saved per tab by the earlier build are not read, while the lane's tool, pin, tree, widths, comments and the Agent layout are kept.
Authoritative topology drops a closed tab's pending opens and keeps the Views and documents; only Hide's own close closes a session's Views.
An unanswered connected-device session or unresolved catalog preserves saved state.
Local and connected-device callers follow the same rules; the phone keeps its existing flow.
No additional badge, notice or setting is introduced.
The checkout keeps at most 32 Views in its six View areas and depth three.
An open that would add the 33rd View first closes the View looked at longest ago in the checkout, the way closing its last View closes a file (Reopen Closed brings it back); a View in front of any area and a document with unsaved work never close.
Only when nothing can close is the open refused, with the View notice, and existing content stays as it was.
Web owner: the shell draws the snapshot projection, the strip already filtered, and asks the unsaved question in `web/src/Overlays.tsx` (`ConfirmViewsClose`) from the projection's `session_drafts`.
Core owner: `herdr-core/src/runtime/{view_sessions,view_callers,view_areas,workspace_control,workspace_view}.rs` and `herdr-core/src/workspace_views.rs`.
Regression owners are `runtime/tests/view_bookmarks.rs`, `runtime/view_sessions.rs`, `web/e2e/{view-bookmarks,right-lane}.spec.ts` and `desktop/e2e/browser.spec.ts`.

### Opening, preview, and Open to the side

A single click on an Explorer file or a History row opens it in the active area's preview view (italic title), and the next single click replaces that preview in place, so browsing leaves one tab per area rather than a trail.
Each area has at most one preview, and a click never touches another area or a pinned view.
A double-click on the row or the tab, Keep open, or the first edit pins the preview where it is, including a double-click that lands while the file's first read is still running; because the first edit pins, a document that is dirty, saving, or whose save failed is never a preview in any area that shows it.
Opening a file that is already shown moves to its view instead of adding a tab, choosing the one used last when several views show it.
The keyboard goes with every open from the Explorer (a click, a double-click or Enter), a History row, ⌘P, Open to the side or a terminal link into the view that shows it once the core places it, so ⌘W, ⌘S and typing act on that file rather than on the pane that printed the link; a page the operator opens (a terminal link's URL or HTML file, the Explorer's Open in Browser, Open server, or a row's, a card's or ⌘K's link) takes it the same way once the core names the display it opened, so ⌘W closes that page, while a View area's New tab gives it to its address field and a page's own new tab leaves it with that page; a folder a link reveals opens nothing to type in, so the keyboard stays in the pane; the tree's arrow keys only move its selection, so the keyboard stays in the tree until something opens.
Opening a file while the lane is closed or on another tool opens it on Files first.
Diffs are placed by the same rules.

Open to the side (from the Explorer's file menu, a History row's menu, or ⌘↵ on a ⌘P result) is the only way to show one file twice.
It puts a pinned second view in the area beside the active one, trying right, then left, then below, then above, or in a new area on the right when there is only one area; when that area already shows the file, its view is focused instead, and with no view open at all it opens in the empty area.
From the only area, Open to the side is a split, so it is offered only where Split right would be, and otherwise stays listed, disabled, with its reason.
Both views show one document: an edit in either appears in the other at once, and each keeps its own scroll position, cursor, and selection.
Closing one of them leaves the document, its text, and its unsaved state in the other; closing the last one goes through the same save and conflict protection every document close has.
Dragging a tab moves its view and never copies it.

### Dragging a view

A dragged View tab lifts a floating copy that follows the pointer, while the tab keeps its place and nothing on screen resizes.
Over a tab bar, an insertion line marks where the tab will land: in its own bar the drop reorders, and in another area's bar it moves the view there with no split and no copy; when that area already shows the same document, the moved view lands at the line and that area's view of the document gives way, so no area holds one document twice.
Over the left, right, top, or bottom edge of an area's content, the half of that area the drop would create is highlighted with a short label such as `Split right`, and the drop creates that area and moves the view into it.
Only one destination is highlighted at a time, and moving to another edge or bar replaces it.
Where the view cannot go - it is the only view of the area whose edge it is over, the area cannot be halved at its minimum size, the Workspace is at its area or depth limit, or the target is not a View area - no overlay appears and the pointer shows the forbidden cursor.
Escape, a release outside a valid target or outside the window, and a target that disappeared or became ineligible before the release all keep the original order and layout; only a valid drop changes the layout, once.
Nothing is resized, reattached, or saved while a drag is in progress, and the drag itself is never stored.
A drag never moves a view to another Workspace, never turns an Agent tab into a view, and never splits a terminal.

### The View tab menu

A right-click on a View tab, or the menu key while the tab has focus, opens its menu with these items in order: New tab, Keep open, Split, Move, Copy path, Select in File Tree, the OS file manager's reveal, Close view.
Split and Move open submenus ordered Right, Left, Up, Down; Move appears only when at least one neighbouring area exists.
Keep open appears only on a preview view.
Select in File Tree shows the Explorer with the file's row unfolded and selected and opens nothing; the reveal follows [the reveal rule](#explorer-file-management), and a page has neither.
Both are disabled with the reason while the file is unavailable or cannot be read yet.
A Move item appears only toward an area that exists in that direction, and moves the view into it without a split.
A Split direction the Workspace cannot make stays listed, disabled, with its reason under it; when every direction has the same refusal, the Split trigger shows that reason once and stays disabled.
The labels say where the view goes, never "Move to Group".
Close view closes the view and never the file on disk; the menu has no file deletion and never closes a pane or a tab.
Opening the menu moves no focus and changes nothing, and Escape closes it.
Next and previous View area focus and View area grow and shrink are chordless registry commands like the Agent area ones, and ⌘↵ on a ⌘P result opens that file to the side, so splits, moves, closes and resizes can all be done from the keyboard once the operator binds the area commands.

### View states

A view whose file is being read shows `Opening…`.
A restored view whose device or root is not ready says what it waits for (such as `Waiting for <device> to connect`) and reads its file by itself once that is ready.
A view whose file cannot be read shows why, with Close view and Retry, and its tab title reads as struck through; either action acts on that view alone and leaves the active area where it was.
Each state belongs to its view alone, so one missing file never blanks another view or area.
After a restart the app reopens the last Workspace as it was left: its areas and their sizes, each area's tabs in order with its preview, pinned views, and active view, the active area, the lane's tool, pin, tree and its widths, a pinned lane docked and an unpinned one over the agents; unsaved text returns from the browser's drafts, and Herdr's current tabs and panes are used as they are.
A first run, or a last Workspace that no longer exists, starts on the Overview.
Herdr connecting before the initial path catalog and saved View arrive does not mean the last Workspace is gone; the existing starting screen waits for that saved choice to settle.
The unanswered-connection fallback does not settle a connected session's pending saved Workspace; a settled connection failure still opens the Overview.
A layout file that cannot be read is kept aside and the app starts on the Overview, where the operator picks a Workspace and continues with a new layout.
A Workspace stored by an earlier build, with the one-tool lane, with the File Views and Tools columns, or with the side panel or layout before them, restarts once with its lane closed at the default widths and its views, tabs and areas as they were (PRD lane-two-columns B23, D-10).

### Browser displays

A View area's new tab offers only what can become a display, never a tool or a pane.
The View strip’s +, its New tab menu item, and the new tab chord while the area holds the keyboard open a distinct browser display in that area with no address and the address field focused.
Its body shows Open with File (⌘P), opening the file palette, and Diff only while the checkout has changes, opening a palette of changed files.
Picking either replaces that empty tab in place; entering a URL navigates the same tab, and closing an untouched tab creates no recovery entry.
The new-tab page offers no tools or panes; ⌘P remains the file palette everywhere.

A web page is a view like a file: it opens in the active area, has a tab, splits, moves, and closes like one, and comes back after a restart at the address it last showed.
A page with an address carries a globe mark and the page's title, else its host, else a local file's name; its tooltip and accessible name carry `Page`, the title, and the full address.
It opens from `hide browser open` in a connected Herdr pane or a shell inside a registered checkout, from clicking or pressing Enter on an HTML file in the Explorer or its Open in Browser menu item, from a page that opens a tab (beside that page, which stays in view: in the area next to it, else in a new area to its right), and from the address field.
A sign-in popup a page opens is a small window of its own above hide, so the sign-in can report back to the page and close; ⌘W closes it.
A link that would open another app (Slack, Zoom) asks first with the app's name, and opens in that app only on Open.
On a connected SSH device, the native page uses that device's localhost or consented checkout resources; a route failure appears in the page's existing failure state.
Opening an address the Workspace already shows moves to that view and loads it again instead of adding a second one.
The view's own toolbar holds Back, Forward, Reload (Stop while the page loads), the address, which shows a web address without its scheme until it is focused, the device size button and Comment; focusing the address selects the whole address, Return loads what was typed, and Escape puts the page's address back.
A page zoomed off 100% shows its zoom as a small badge at the address field's right end, and nothing at 100% (PRD browser-width-zoom B6-B9); the badge opens − · zoom · + · `Reset` (`100%로`), which stays open while the operator steps through Chrome's steps from 25% to 500%, disables − at 25% and + at 500%, leaves the keyboard in the shell, and goes with the badge once the page is back at 100%, as it does on ⌘0.
Pages of one host share their zoom, so every other page of that host shows the same badge, in any area or Workspace.
The device size button lays the page out at a phone's (390), a tablet's (768) or a desktop's (1280) width instead of its area's (B10-B15): Fit, the default, is its icon alone, and a chosen size shows its width, with the scale the page is drawn at when the area is narrower (`1280 · 39%`).
A narrower size draws the page at its own width centered at the top of the area, the sides left empty; a wider one fits the area's width, scaled down; the page's own viewport is the chosen width at any zoom, a phone or tablet size lays out as one does (its meta viewport applies), and the browser identity stays the same.
While a size is chosen the zoom badge is hidden and ⌘= ⌘- ⌘0 on the page zoom nothing, neither the page nor a terminal's text; Fit brings the page's zoom and its badge back.
The size is the display's own: it stays through a tab change, a move to another area and the page's own navigation, and a reload of the window or a restart returns it to Fit.
A sign-in popup, an image or a PDF, and a page in a plain browser tab have no zoom badge or device size button.
A page that cannot load says so in its place with the address and the reason, and Reload tries again; nothing else on screen changes.
While the palette, a menu or a dialog covers a page, the page is shown as a still picture of itself from its first frame, so the overlay draws over it and the page is never seen on top; a page with no picture yet is blank for a moment until one arrives, and an older picture turns to the page as it is now within a few frames; the page comes back live when the overlay closes.
A tooltip never turns a page into a picture; a tooltip, such as a tab's or a page toolbar button's, opens on the side of its trigger where the whole of it shows clear of every page, and keeps its usual side when no side is clear.
While the shell drags something (a tab, a divider, a lane divider, a tree item), every page is shown as its still, so the guide or preview draws over it and a drop lands in the shell rather than the page; the pages come back live at release, and a drag inside a page is the page's own.
In a plain browser tab the view keeps its address on the toolbar row, level with a document header beside it, and below it reads `Pages open in the Hide desktop app.`; a web address offers Open in browser, and nothing else is drawn in its place.
[BROWSER_DISPLAYS.md](BROWSER_DISPLAYS.md) owns which addresses a page may hold, the `file:` boundary, and the page's lifetime.

### View openers and comments

Each area's selected View shows its actual opener's 14px provider mark at the right of the address or path toolbar, while tabs retain their type mark and title without repeated opener logos: a page, a file, a diff, an image or a PDF opened by `hide file open`, `hide diff open` or `hide browser open` from the agent's pane, or by the operator clicking a path or an address in that agent's terminal (PRD view-opener-feedback D-02, D-04, D-24).
A View the operator opened from the Explorer, ⌘K, the file tree or a new tab's address field, and one a caller Hide could not bind to a pane opened, wears no mark.
The mark follows the agent that last opened the View; a link inside the page and an agent's `hide browser` command keep it, and an address the operator types takes it off.
Two sessions that open the same View both keep it in their strips, and the mark is the one that opened it last (PRD session-files D-26).
Its hint, above the toolbar mark, says the agent's name, `Opened this · <product>` and `Click to go to its pane`, and the mark's accessible name says the same; the tab's tooltip also identifies its opener, including on background tabs.
Opening, waiting and unavailable Views retain a path toolbar with the same opener, and a missing or unknown opener creates no invented identity.
Pressing it brings the agent's pane forward with the keyboard, leaving the View selected; an agent whose pane closed is found again by its session in another pane, and one found nowhere draws its mark dimmed with `(closed)` in the hint and goes nowhere.
A pane whose agent opened Views that are open shows `N views` after its PR mark, a list of each View's kind mark, name, address or path and unsent comment count, counted once however many tabs show it; a row brings that View forward.
A View the operator has not looked at since the agent opened it ends its row with the `볼 것` mark, an eye in the warning badge, whose hover says `볼 것` (PRD unseen-agent-views B10).

Comments: a page's toolbar Comment button turns Comment mode on, with `Click an element or drag an area · Esc` over the page, an outline on the element under the pointer, and the page's clicks taken as comment targets until Comment or Esc turns it off.
The button shows on only once the page is held for Comment; until then the page still takes its own clicks, and a second press takes the turn-on back.
A local HTML page (a `file:` page inside a checkout) takes Comment exactly as a web page does: the outline, a click on an element, a drag over an area, the pins and the note.
Comment that cannot start, or that ends on its own, says why in the button's hint, in words the operator can act on (reopen the page, reconnect the window, answer the dialog in the page), and the button shows its warning colour until the next press; the operator's own turn-off says nothing.
A file or a diff takes a comment from the `+` on the line under the pointer or the cursor (beside the line number, on the line's first row, however many rows the line wraps to or tall a Markdown Live heading makes it), a drag down the gutter or the line numbers for a range, or ⇧⌘. (Ctrl+Shift+. on Windows and Linux) on selected text; a range of removed diff rows is on the old side, any other on the new.
An image or a PDF takes one from an area dragged while its toolbar's Comment is on, which is disabled, with the reason in its hint, while the file cannot be read; a PDF comment names its page, and an area stays on the same content at any zoom.
A picked area, rows or element stays marked for as long as its note is open (an area keeps the outline a saved region comment has, on a page over its still), and Cancel or Esc takes the mark with the note.
A note opens under its last line (beside an area), with `⌘↩ to add`, Add and Cancel; Esc cancels, an empty note cannot be added, and a View with 50 comments shows `50 / 50` and why in the note's place.
On a page the note opens beside the picked element or area over the page's still, and a pin opens its comment there; a press beside either closes it, and the page takes the keyboard again.
A comment shows under its line, or as a numbered pin and outline while Comment is on, with its number in its recipient's letter, edit and delete; a line comment's card is as wide as the visible text and stays beside the gutters when the lines scroll sideways.
A deleted comment leaves `Comment deleted · Undo` on the View's line for a moment, and Undo brings it back at the same place and number; there is no confirmation.
A comment whose lines changed moves with them, or says `Changed` and quotes what it was when no line with its text is left; a changed image or PDF marks its area comments `Changed`.

A tab with unsent comments carries their count, orange while some came back undelivered, and one line under the View, never over the page, says `N comments · M views` with `Send N → <logo> <agent> ▾`; the line is not drawn while the View has none.
Send N sends every unsent comment for that agent, from every View, as one letter from the operator ([delivery.md](delivery.md#the-operators-view-feedback)); ▾ offers `This view only`, the other agents in the checkout with their status marks (and logos only while the agent logo view mode shows them), and for a closed agent `Reopen and send`, which starts its session again in a new pane of the checkout and sends.
A View the operator opened opens its Send as `Send N comments to…`, a choice of agent.
After Send the comments leave the View and the line says `Sent N to <agent> · delivered when it rests` with Go to pane for a moment, or that it is delivered when its device is back; a Send that could not go keeps the comments and says why with Retry, which sends again to the same agent and the same Views that Send asked for (`This view only` stays one View), and a letter that expired brings its comments back marked `Not delivered` with Send again.

Web owners: `web/src/ViewComments.tsx`, `web/src/viewFeedback.ts`, `web/src/editor/comments.ts`, `web/src/editor/useLineComments.tsx`; core owner: `herdr-core/src/runtime/view_feedback.rs`; test owners: `web/src/viewFeedback.test.ts`, `web/e2e/view-feedback.spec.ts`; original visual boards are maintained separately.

### Narrow windows

Whether a pinned lane docks depends on the Workspace body's width, not the window's, so hiding the sidebar with ⌘B can dock it again.
A pinned lane docks while its columns' minimums and Agent Views' 480px fit side by side with the dividers, the viewing column narrowing first and the tool column after; when both columns cannot stand side by side they stack, the one the rail last brought forward drawn and the other behind it, and in a body too narrow for even one column the lane opens over Agent Views with its pin still filled; widening the body docks it again (PRD lane-two-columns B19).
The lane opens at any width: in a body narrower than the lane it takes the whole body, giving up the viewing column's width down to its minimum first and the tool column's after, and a body narrower than one column's minimum is still the lane's whole width, never the sidebar's.
When the View areas cannot all have their minimum, only the active area shows, with an area switcher to the others.
Widening the window brings back what the Workspace stores, the lane's columns, pin, widths and the area sizes, because none of these narrow arrangements is stored, and another Workspace's lane never changes while one is narrow.

### Library masters

PRD right-lane B25 drew the lane, rail and pin parts of its approved v3 boards in `Component / Workspace columns`, and the lane states in `Screen / Workspace`; those boards are maintained outside the repository, so PRD lane-two-columns (D-12) draws its two-column states in the gallery's `RightLaneScene` instead: closed with the pin off and on, over the agents, pinned, the Explorer beside the documents and alone, expanded, PRs, Memory, Changes and the narrow stacked lane.
Delivery reviews actual captures in both themes through the public [design workflow](DESIGN_WORKFLOW.md).
The production gallery and View components provide the public visual reference; original boards are maintained separately.
Their sheets draw every state as refs: the View tab sheet draws preview, pinned, hover, active-in-the-active-area, active-in-another-area, dirty, unavailable, diff, a long title, and the floating drag copy; the placement sheet draws a reorder, a move into another area, a right and a down split, and an ineligible target; the tab menu sheet draws a preview's menu and a pinned view's menu with a disabled Split and its reason; the area states sheet draws each view's opening, waiting, and unavailable states.
The browser display's toolbar and its loading, load failed, and plain browser tab states are on `Component / Browser file diff toolbars`.

### Agent panes and the Agents explorer

A pane header's first row is the pane: its state on the left, what can be done to it on the right, and nothing in it changes place when a control shows or hides; the turn row under it is the conversation.
On the left stand the move grip, the ancestor path, the status mark, the agent's logo and title, then the fork mark, the PR mark, the plan chip, the views chip, the Answer tag and the letters-waiting pill; on the right stand `@`, zoom, the overflow control and close, after the limited-integration icon.
The grip, `@` and zoom are drawn on the pane that holds the keyboard, on any other pane while its header is pointed at or focused, and always on a screen with no hover, keeping their place while hidden, so the hidden places sit together left of the overflow control; an `@` whose list is open stays drawn; a zoomed pane's zoom chip is always drawn, since it names the panes it hides.
`개요에서 보기` is the pane menu's first item among the agent's own actions, with no shortcut and no header button; it opens the Overview on Work with the pane's session chosen, as a click on it there would (PRD pane-open-in-overview B11; the header button of B1-B3 was folded into the menu by operator decision).
An agent pane's `@` lists the other agents on the same device, found by name, task or project as typed; choosing one types its reference, as Copy name and ID writes it, into this pane's prompt with no Enter and gives this pane's terminal the keyboard, while Escape closes the list back to `@`; the pane's own agent and a device's agents elsewhere are not offered, because the id would name nothing there, and a shell pane has no `@`.
Dragging the grip, the title or unused space in the first header row moves that pane after the pointer crosses the ordinary drag threshold; a short title click still opens its record.
While the drag runs, a chip with the pane's name follows the pointer, and a card or hint a hover opens (the plan chip, a hint) stays closed until the drag ends.
Header buttons, relationship rows and terminal text keep their existing actions.
A pane drops on another pane's left, right, top or bottom edge in the selected group's Agent areas, on a tab title to join beside its selected pane, or on that area's New tab button to separate into a tab.
A centre, self, Files, other checkout, device or group is not a destination.
Hovering a tab shows an inert preview without selecting or reading it, waking its session, moving the keyboard, or changing a terminal's grid; Escape, blur, outside release and a changed source or target cancel it.
The pane menu offers the same destination tabs, panes, edges and New tab action without a new shortcut.
A sole pane dropped on its own area's New tab button keeps its tab; in another area it moves that existing tab.
A successful move focuses the destination and removes an empty source tab, including a named tab, collapsing an empty non-root area without opening a replacement shell.
The operation keeps the running terminal, conversation and responsibility relationships; a manually placed delegated pane stays where it was put.
Related tabs cannot start another geometry operation while the move is pending, while other tabs and terminal input remain available.
A definitive failure offers Retry locally; an uncertain result offers a status check and exposes the confirmed live placement before another move can be tried.

An agent pane's head carries no task chip, no checklist popover and no chord for one (PRD session-record-goal-flow D-01, B1); the PR, letter and branch marks in the same place stay, and the session record holds the whole record.
A turn the agent's provider refused for a usage limit or an API error ends as the label's `한도`, and the second row and the turn card draw it as `막힘` with its line as the cause (B9); only the session record tells the two apart.
A plain shell keeps its terminal mark.
A pane's terminal stands 8px in from the pane's sides and 4px from its top and bottom, so its text never touches the pane's border, and its grid is the one that fits that inset box.
On an agent pane whose session Hide knows, the status mark, the logo and the title are one button: under the pointer or focus they light together and open nothing, since a card under the head kept rising in the way of the terminal (the turn card opens from the request on the second row, and the record shows the whole title), and a press shows that session's record in the lane's 보기 slot ([The 보기 slot](#the-보기-slot)), opening the lane when it is closed, while the keyboard and the input cursor stay in the pane's terminal (PRD overview-device-tasks B22, sidebar-task-lens B15); the Overview's `레인에서 보기` lands the same way.
A press while the lane already draws that record turns the lane back to its tool, and the fork mark and the PR mark keep their own presses outside the button (B23).
Whether the lane draws that record is read from the slot's front item and the lane's tool, both of which the core keeps.
A plain shell, and an agent pane whose session is not known yet, keeps the plain head with nothing to press; the head becomes the button in place once the session is known, on a device's pane as on this Mac's (B24).
Over a narrow window the record shows in the lane floating over the panes (`web/src/PaneHeaderBand.tsx`, `PaneRecordButton`).
The provider logo is drawn on a pane's own header, and in the heads and places that are about the agent or its provider itself: the start picker, Settings, the footer's usage chips and the Usage pages, the session record's head, the tree popover's head, the last-words popover's head, the phone's detail head, the Hide AI tool's ⚙ chip, the first-run tiles, and the selected View's toolbar ([View openers and comments](#view-openers-and-comments)).
The sidebar's 프로젝트 and 태스크 rows and the Agent tabs draw it only by the operator's choice or while ⌥ is held ([Agent logo view mode](#agent-logo-view-mode)); ⌘K and the recent-pane list and the phone's list lead with the status mark alone in both modes (PRD agent-mark-quiet); a session record's children, the PR card's session rows and the child tree popover's rows draw the logo after the status mark in both modes (PRD record-card-readability D-06, D-09).
A pane header draws exactly one logo, the pane's own agent, for each of the seven supported agents: Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity, the same logo Settings and the first-run choice draw.
A row without a logo names its agent by product name (`Claude Code`, `Codex`, the adapter's label, never the raw kind) in its hover hint and its accessible name, and the pane header's logo and title say it the same way.
Only a plain shell and an agent Hide does not support draw the neutral `>_`.
A logo drawn dark on a transparent ground (Grok) sits on the light plate so it stays visible on the dark theme; the others are drawn as published ([BRAND.md](BRAND.md#third-party-agent-marks)).
A child pane's ancestor path names each ancestor from the root to its parent, `›` between them, and each name opens that pane; a root has none.
The names drop before current identity or actions when the pane narrows, leaving `›` with its accessible name.
The PR mark is the one the 태스크 row of the same agent draws, from the same list the core lays on the session (docs/status-model.md, One list per session): its own PRs and every descendant's, a descendant that ended included (the `#number` when that is one PR, each state's count when several), pressing which lists each PR with who holds it, so a child's pull request is seen from the pane that delegated it; under the pointer or focus it opens below the header what the 프로젝트 row's mark shows, the PR card for one PR and the list for several.
The fork mark, a small fork icon after the title, is drawn only on a pane Fork agent made; its tooltip and accessible name read `Forked from <source name>` while the source still runs the conversation it was forked from, and the mark is then a button, reachable by keyboard, that opens the source; once the source closed or moved on to another conversation it reads `Forked from another pane` and opens nothing (`web/src/PaneHeaderBand.tsx`, `PaneForkMark`; the fact is the core's, `PaneForkSnapshot.forked_from`).
A delegated child and a handed-off root have none, and neither does a fork made while forks were children (PRD fork-operator-root B20, B21).
A device's pane header wears the mark too, as its rows do, although a device's pane offers no fork.
The letters-waiting pill follows the title and the fork mark while letters wait on the pane for a possible draft, as the core decides ([delivery.md](delivery.md#delivering-a-held-letter-now)): the envelope, the count from two letters, and `지금 전달` (`Deliver now`) on the popover fill with a border edge, which stands apart from both a focused and an unfocused header in Light and Dark.
The whole pill is one button, reachable by keyboard and pressed with Enter, and its hover and accessible name read `대기 중인 편지 N통 지금 전달`; a pane narrower than the pane breakpoint drops the word and keeps the envelope and the count, and pressing it still delivers.
Pressing sends `deliver_letters_now` and nothing else: the pill goes, and the bell line appears in the terminal as the doorbell submits it through Herdr's `agent.prompt`, with no confirmation and no warning about a draft, because the pill sits right above the composer.
The line begins with 🦉 (U+1F989, no variation selector); Codex CLI on Windows may omit it, and the saved bell still identifies the letter ([delivery.md](delivery.md#recognizing-the-bell)).
No other hold of a letter reaches the screen, and nothing changes on the sender's pane (`web/src/components/letter-mark.tsx`).
The tree popover's `개요에서 보기` opens the Overview on the agent's device with the card of the task it belongs to selected and the agent's own record in the panel (PRD overview-device-tasks B59); a resting or cleanup card's fold opens with it, so the selected card is drawn.
The zoom control names the panes it hides and unzooms the tab; the input pane keeps its accent perimeter and header tint.

The second row over the terminal shows the pane's turn, `<request> → <answer>` with no word before the request (the button's accessible name still begins `요청:`), with a child-tree button at its left when direct children are known (PRD pane-last-request).
The request is the operator's last one, the label's one-line summary of it, read against the conversation (`제안대로 진행 + 애매한 상태 더 점검` for `ㅇㅇ 그렇게 해 … 점검들 좀 해줘`), in quiet text; until the label has summarized it, with summaries off, after a failed judgment or for a turn before the summary existed, it is the request's own first line, and a request of images alone reads `이미지 N`.
A letter another agent sent after it does not replace it; a delegated child with no operator request shows its sender's last request the same way.
The answer is the operator's ask, the agent's own Working progress, for a finished turn the operator has not looked at, `끝남` in the success colour and its result sentence, or, while the agent waits on working children, `자식 기다림 · 자식 N개 작업 중`, the words its sidebar row says; once the operator has looked at a finished turn the row goes.
The progress says only what the request does not: the label's line never restates the request, and a turn whose events show nothing beyond it yet has no line, so a working turn with no progress shows its request alone, with no arrow and no `작업 중`, since the status mark and the working line already say it works.
The request shows only with an ask, while the agent works or waits on working children, or for a finished turn not yet looked at: a pane that waits on anything else, rests or finished and was read has no row (only the children button when it has live children).
A quiet turn (progress, a result, a children wait or a request alone) sits on the head's own surface under a hairline, and an ask keeps the raised surface with its warning edge.
How many lines the row takes is the View menu's choice ([Pane turn row mode](#pane-turn-row-mode)): one line by default, the request cut at two fifths of the row when an answer follows and each part cut where it overflows, an ask included, so no turn grows over the terminal on a narrow pane.
The children button stays at the first row's left, the request is never folded into an icon, and the rows overlay the terminal without changing its grid.
The request is a button the keyboard reaches: a press or focus on it opens the turn card under it, and a second press, Esc or a press outside closes it; the pointer passing over opens nothing, because a card rising on hover kept covering the terminal.
With the card open, Tab from the request or the title moves the keyboard into the card to `기록 열기`, the card stays open while focus is in it, Shift+Tab before its start goes back to the trigger, Tab past its end and Esc in the card close it and return focus to the trigger, which keeps it closed until the pointer or focus comes anew; only Tab moves focus in, so the pointer never takes the terminal's keyboard, and a card the pointer only shows is no Escape layer, so Esc reaches the terminal.
The card reads the session as one conversation top to bottom on a rail: up to two earlier turns first, oldest first and dimmed, each ringed in how it ended (the warning colour for a question or a block, the success colour for done); then who sent the request (`나` or the sender) over a bubble with the request's summary and its own first two lines, and who sent a later request under it; then the agent's half, the answer's verb as a pill in its state's colour (warning while it asks, the working colour while it works, success when finished) with the label's words for the turn, never the agent's own words, which the record keeps and which could still be the previous turn's while a new one starts; and `기록 열기`, which shows the session record in the lane; a part with nothing to say is not drawn (`web/src/PaneTurnCard.tsx`).
The second row is absent when there is no turn to show and no child button; a pane whose terminal is not live has no ask, request or progress on it, because its body says why.
The core publishes the ask (`herdr-core/src/agent_state/header.rs`, `Header.band`): the pane's own Approval, Answer or unread Blocked first, the lead raised descendant next, then the pane's own read Blocked and Stopped, which is work left.
A pull request's verb (Fix, Review, Merge) is the header PR mark's colour and a finished turn is the status mark's dot to read, so neither draws a row; the PR mark carries no review word either.
An ask stays until it is answered: once the operator has had the pane focused it dims to `--opacity-read-status`, as its row's request line does, and a question the operator read but did not answer keeps its row (`Band.read`).
The pane's own ask starts with its verb, `승인`, `질문` or `막힘` in the warning colour, then what is asked and the stable wait; an Approval names no command, since the core reads none, and shows the ask's own default words.
Work left starts with `일 남음`, the word the row's line uses, in the foreground colour on the head's own surface with no warning rail, because the warning colour is kept for what the operator answers; with the turn row off its mark on the head has a plain border, and the turn card draws it as a quiet answer, not an ask.
A raised descendant's ask starts with its verb, then the originating agent's name and `(자식)` for a direct child or `(하위 세션)` for a deeper descendant, what to do, the parent that has not received a letter, the elapsed time and Open.
The hover gives the originating agent's product, path, checkout and wait when known; a draft still names its originating child while Open goes to the parent holding the input draft.
Open goes to the raised descendant, or for a draft to the parent whose input holds it, and is disabled with the connection reason beside it while that pane's device is not connected.
An ask is quiet text on the secondary fill with a warning rail; the request text gets three shares of the flexible space, and the asking agent and an unreceived-letter explanation get one each, so they do not consume the whole reason at narrow widths or enlarged text.
Without an ask, the row shows the current pane agent's own nonblank progress only while its state is live Working, and a finished turn's result sentence only until the operator looks; idle and disconnected states cannot retain either sentence.
The label worker retires the previous turn's sentence as soon as it proves a new human turn, even when the agent stays Working, before the new turn's analysis arrives.
The row does not substitute a child's line or the pane title for missing progress; a wait on children names only how many work.
The children button stays at the row's left whenever the pane has live direct children, with an ask or progress on the row or neither: a tree icon, the status marks of the first three direct children that have one, in tree order, with `+N` for the others, and a chevron open the existing tree popover ([status-model.md](status-model.md#the-descendant-mark-and-the-tree-popover)); a child of unknown state draws no mark and counts in `+N`, and with a device away or no child to mark the face keeps the number.
The row never lists individual child names; child names, lines and navigation remain in the popover.
Its tooltip and accessible name spell out the live direct child total and every state count; an ordinary child's question, approval or error is its parent's to answer, so it is named only there, and a dot on a name is a reported completion, not verified delivery of a task.
The button opens with the raised branches expanded whenever its core raised count is positive; `외 N건` remains only when the children button is unavailable.
If the parent or a known descendant's device is not connected, the button keeps the last topology total and the reason in its tooltip and accessible name; the connection reason is also visible beside any progress until the connection returns.
With no live children there is no children button, and no empty separator.
Working keeps a thin blue line along the top and adds its row when the core has a valid sentence or a request to show; CI wait, waiting and idle have neither.
The row and the blue line overlay the terminal and change neither its viewport nor its PTY grid.
Pending and failed moves appear at the initiating popover, ancestor step or Open with the existing retry rules, plus the Agent-area notice when the source pane is no longer visible or the move started from a child's name.

Why the terminal is not live is said where the terminal would be, never on the second row (`Header.notice`, drawn by `web/src/PaneNotice.tsx`): the core publishes at most one notice, the device being away first, then terminal restoration failure, sleep, and the stream's own state.
Sleep, restoration failure and a local stream that can be attached again draw bodies of their own, described below; every other notice covers the terminal with its words on the card fill: `연결 안 됨 · <device>`, `다시 연결하는 중…`, `헬퍼가 허용되지 않음 · 설정 > 기기에서 허용하세요`, `사용할 수 없음`, `종료됨`, or a failed exit in red with the actual code.
A start or a close that passes by itself is a small caption over whatever the terminal shows, and a wheel another client holds is a corner caption, because that terminal is live.
Connection and sleep recovery controls stay in the body, and limited integration stays a header icon.
Herdr's explicit restoration failure is a terminal fact, independent of an agent's activity or its saved sleep record.
Its pane body says the terminal could not be restored, names its folder and offers Close pane; raw details remain in diagnostics.
Visiting this pane preserves a saved conversation and its title without waking it, and neither Wake nor attach Retry is offered as terminal recovery.
A missing folder alone never establishes restoration failure.
An explicit close rereads the failed terminal's incarnation and failure before sending the close, and keeps the pane until Herdr confirms removal.
A whole-tab close requires the same pane membership; a nonfailed peer must be quiet or the same terminal included in the existing close confirmation.
A newly active or unknown peer requires a fresh close decision.
Close only this leaves external descendants running; the existing subtree choice owns whether they are included.
The fresh read and close share the admitted deadline, so a slow status read cannot send a close after its permission expires.
Other active or unknown panes and descendants keep their close guards.
A failed terminal has no recoverable live layout, so a close containing one does not enter the ordinary undo stack.
Closing the last failed terminal preserves the project registration without trying to start a replacement in the failed folder.

The pane menu (from its overflow control or a right-click on the header) groups the parent and other siblings under Related sessions, offers Copy name and ID, then Close terminal, which asks about the agents it spawned as Closing an agent that spawned others says; opening it moves no focus and marks nothing read.
The pane's own children are not in it: its tree button opens them.
The menu names the pane as its header does and is drawn from the pane as it is now, not as it was when the menu opened: a session's label arrives after its first turn, and the heading already open then reads the new label and offers Fork session and Sleep agent as the core now says.
Fork session groups the new-tab and right-side destinations without changing which sessions can fork.
A right-click in the terminal focuses that pane, like a click, and opens a longer menu: Copy (only over a selection), Paste, Select all, and Find; then Split right, Split down, and Zoom terminal or Exit zoom (disabled on a tab's only pane); then the pane menu's items; an item with a chord that does the same shows it, ⌘C and ⌘V included.
The Zoom pane chord on a tab's only pane does nothing either, on this machine and on a device: the core does not send it and logs `pane.zoom.single_pane` (`remote.control.zoom_single_pane` for a device).
Copy name and ID, here and in an agent row's menu, copies the pane's reference as one line, `@<name> (pane <id> · <task>)`: the name Herdr knows the agent by (its kind when it has none, a shell pane's title), the id the pane's own Herdr knows it by (`w9J:p52`), the one `herdr pane read` and `hide request send` take on that device, without the `remote:` scope Hide gives a device's pane, and the agent's task label, cut to one short line.
The reference never holds a line break, so pasting it into a prompt never submits it; an agent that reads it sends its letter to that pane id.
An agent row in the sidebar, and an agent's mark on a pull request row, can be dragged onto a terminal: the drag shows the reference under the pointer, and the drop types it with a space after it, never Enter, and focuses that terminal, so the operator only adds what to say.
A drop on the agent's own pane or on a pane of another device is refused, because a pane id names a pane only on the machine that owns it.
The right-click leaves the drag selection as it was, so Copy copies what the operator selected; the program in the pane never hears it.
A wheel over the pane moves a drag selection with the text it covers: the pane has no scrollback of its own, so the highlight is placed again on the rows that text moved to, drawn only for the part still on screen.
Clipping the highlight preserves the original selection ends and Copy text; reversing the scroll while part remains visible restores the complete highlight.
Once none of it is on screen the highlight goes, but Copy (⌘C, the menu, Ctrl+C where a selection makes it copy) still takes the text that was selected, until the next click, drag, select all or key; Ctrl+C interrupts again only after that.

A pane whose agent sleeps (PRD agent-sleep) shows its state in place of the terminal, which stays hidden until the agent is back because the shell under it is not what the operator was talking to: Sleeping with the last progress line and Wake agent; Waking… with how old the resumed conversation is; or `Couldn’t resume this conversation` with the core's plain reason, Retry, and Start new session.
Its second row carries no ask while it sleeps, and typed input to the pane goes nowhere, from the first moment its end begins until the agent is back: keys, a dropped file's path, the find keys and a phone's reply alike ([The phone app](#the-phone-app)).
Opening the pane's tab by a committed move (a row, a tab, a checkout, a relation) wakes it in the same pane with its conversation; a Recent Panels preview does not, and neither does a click inside the tab already on screen.
Wake agent, Retry and Start new session take a mouse click as they take Enter: everything a pane draws over its terminal (these bodies, the reconnect caption, the attachment refusal, the header band) lies above every layer the terminal's renderer draws, so a click there never reaches the terminal (`web/src/terminals.ts` gives each terminal its own stacking context).
The pane menu offers Sleep agent on an agent pane that is awake on this machine, or on the machine the window runs on when that machine's core runs on another machine, disabled with the core's reason when the agent is working, waiting for the operator, of another kind, or has no conversation Herdr reported; a device's pane offers none.
Archive session (`세션 보관`, PRD session-archive) stands right under Sleep agent, or first in that group when the agent cannot sleep, on a root agent's pane; a delegated child's pane draws it disabled (`맨 위 세션과 함께 보관돼요`), as does an agent whose conversation Hide has not seen and a pane of a device that is not connected, each with its reason; a shell and Hide AI's own agents offer none ([The sidebar's 태스크 list](#the-sidebars-태스크-list) says what it does).
Fork agent and Fork agent to the side follow it in the same group (PRD fork-operator-root): both are offered on a local agent pane whose conversation Herdr reported (`fork.available`) and send `fork_pane` with the placement and the operator as its origin, which starts that conversation again as a new session the operator owns; on an agent that has a fork command but has not reported its conversation yet both are drawn disabled with the core's reason, and on a shell, an agent with no fork command or a device's pane there is neither.
Fork agent opens the fork in a new tab immediately right of the source's tab in its Agent area, or at the end when the source's tab has closed meanwhile; Fork agent to the side splits the source's tab and puts the fork to the right of the source.
A start command being accepted does not release held input: the fork must have its expected agent running in that pane and be ready, or use a kind that does not report readiness, as ordinary delivery permits.
A start that fails closes the fork and discards its held input rather than typing it into the shell.
Failure to confirm input readiness preserves the successful fork pane and discards held keys; the existing input-loss notice appears only if keys were actually held.
Either moves the keyboard to the fork as soon as Herdr has made its pane, as a new tab does, with the fork's tab or split drawn from that moment rather than from Herdr's later layout, unless the operator chose another view after the press or the Agent area has no room to show the tab, where the fork then waits; what is typed before the fork's agent has started reaches that agent, never the shell that starts it, with the node retaining it until the fork ends, up to 64 KiB and 30 seconds per holder.
Crossing either bound discards that queue and shows a dismissible notice asking the operator to type again once the pane is ready; later keys remain refused until readiness or the input deadline, at most 115 seconds from the fork worker starting, releases the guard, and late readiness never replays discarded keys.
This rule covers NodeLink keys and phone input; a phone waits for the same node holder before its reply or quick key is written.
A second press while a fork of the same pane is starting, either item, is refused with `pane.fork_already_running`, and a fork whose agent does not start closes the pane it made and shows the same failure notice for both.
On such a window's own machine, bringing a tab forward wakes what sleeps there once that machine's Herdr has it in front.
Web owner: `web/src/PaneView.tsx` (`SleepBody`), `web/src/sleep.ts`, `web/src/PaneRelations.tsx`.

An implemented provider whose sleep policy closes its pane retains a separate sleeping conversation row in its checkout's expanded rows.
The row keeps its confirmed title, names its provider by product name in its hint and accessible name, uses the existing moon and row tokens, and offers Wake explicitly; it has no pane-opening action, lineage badge, input target or numbered pane shortcut.
Saving and closing retain the original live row until authoritative close confirmation, while a sleeping or waking row follows the core's published phase.
An unknown close or wake offers Check status, disables that action while a check is pending, and never retries an external effect automatically.
The current Claude Code and Codex keep-pane behavior above is unchanged.
Web owners: `web/src/components/sleeping-session-row.tsx`, `web/src/sleeping-session.ts` and `web/src/sidebar.tsx`.

The sidebar is one list of operator-owned roots under projects and checkouts, with Needs You (`내 차례`) and Unread (`읽을 것`) raised above it.
A root whose descendant is raised enters Needs You itself, at any depth, and the raised descendant never appears there on its own row; its second line is the verb and what to do, with no name, count, PR or path.
A root whose agent opened a View the operator has not looked at is in Needs You with the `볼 것` mark, an eye in the warning badge, and the line `볼 것 · <newest View>`, ending with ` · 외 N` for the others so a long name never cuts the count; pressing it brings its pane and that View forward, the lane showing the documents, and the mark goes once the operator looks at every such View, or each is closed (PRD unseen-agent-views B1-B3, B9, B13).
A question, approval or block on the same row keeps its mark and line, which ends with `볼 것 N`, and the row stays in Needs You after it is answered while a View is still to look at (B11).
`볼 것` counts with the operator's turns on badges and the rail and sends no push, notice or sound (B12).
What to do is the core's `detail`: the label's reply, else the first line of a native question waiting now in the agent's own words (up to 40 characters, `…` when cut), else the label's line, else the verb's own words (`답을 기다림`); a native question therefore reads as itself with Hide AI off and while the label still describes the turn's work ([status-model.md](status-model.md#the-second-line)).
An idle or done parent still owns its children, and a closed pane or ended parent agent makes them orphan roots.
A parent waiting on a working descendant stays Working; a descendant that asks waits on its parent, so the parent keeps its own state, unread because the ask is news to it ([status-model.md](status-model.md#a-quiet-row-that-waits)).
A root with direct children has a chevron at its left and starts folded; the opened roots persist across launches and device reconnects.
Folded, it wears its descendant mark, the turn mark and N for raised descendants, and nothing when its descendants are working, asking it, idle or done: a parent waiting on a working child already shows its ring and that child's line, and a child's ask that is not raised already turns the parent unread (PRD session-record-redesign D-04).
The core still computes the working mark; no surface draws it.
Opened, its children follow most urgent first, joined to it by rails, and a child with children has its own chevron; grandchildren are one step further in, and a grandchild with children opens the tree popover from its chevron.
Past five siblings the rest wait behind `N개 더`, which opens them in place.
A tree row is one line with no branch line; a child on another device names that device on a second line, dimmed with `연결 안 됨` and not openable while it is not connected, and a child working in another worktree is not drawn again under that worktree.
A worktree whose unresolved agents are all delegated children has no checkout line, no purpose line and no fold, because the children are reached from the root that owns them.
An agent counts as a root when the core marks it so (`agent.state.root`): an orphan whose parent ended is a root, so its worktree keeps its line.
The primary, the focused checkout, a dirty or unpushed worktree and an Inactive or Cleanup one keep the line or fold they had.
A row carries the PR mark (Purpose, pinning, and PR chrome) the 태스크 row of the same agent draws: a root draws its session's list, its own and every descendant's PRs, each once by URL, whether its children are folded, behind `N개 더`, in another checkout, on another device or ended, and a child or grandchild row shows only the ones its own session holds or made (PRD project-tree-pr-family).
The mark is one PR's state and `#number`, or each state's count for several, dimmed while GitHub cannot be read; its hover shows the PR card for one PR, and for several lists each PR worst first with its state, its title and, on a root, the sessions that hold it, any of which opens, with the last read time while GitHub cannot be read.
An opened root therefore repeats a child's PR that the child's own row shows too: the root's is the gathering, the child's is its own.
A row whose pane has letters waiting on a possible draft carries a muted envelope just before its time, with no number, the quietest mark on the row; its accessible name says `Letter waiting`, and clicking the row goes to the pane as always, where the pill delivers them.
A tree row that asks draws its verb and what on a second line until answered, and an AI question leaves that line once read; a blocked tree row draws its cause there instead, read or not.
A raised row outside the tree keeps its request line until answered, dimmed after reading an AI question; an unread result line stays bright until read.
Quiet detail lives in the tooltip, and no progress value is invented.
Hover, focus, selection and the open popover change fill and ring without adding lines or moving adjacent rows.
A row is at least 28 high, or 44 with its second line, and grows with text scale rather than spilling.
While a close names a pane, its row retains closing feedback.
⌥1 to ⌥9 select the first nine visible root occurrences top to bottom, counting duplicate raised/tree rows only once; tree children carry no number.
Web owners: `web/src/PaneView.tsx`, `web/src/PaneHeaderBand.tsx`, `web/src/PaneMention.tsx` (the header `@`), `web/src/PaneRelations.tsx`, `web/src/components/sidebar-agent-row.tsx`, `web/src/components/agent-tree.tsx` (the verb, ask line, descendant mark, rails and chevron the three surfaces share), `web/src/components/pr-mark.tsx` and `web/src/prMark.ts` (the PR mark every surface draws), `web/src/components/letter-mark.tsx` (the letters-waiting pill and envelope), `web/src/components/agent-tree-popover.tsx`, and `web/src/sidebarTree.ts`.

### Work Hide AI does for a home

Hide AI starts agents for the operator by itself (today a suggestion's `문서 고치기` or `이슈로 남기기` in the Hide AI tool's 제안 tab, a ready branch's `PR 올리기` or `다시 맞추기`, and a Memory batch's `N개 문서에 반영`), and the work belongs to a screen, its home, not to the operator's agent lists (PRD hide-ai-work-home).
The 제안 tab's row being applied is the home of that work: it shows the progress, the branch or the pull request, the failure and the agent link, and it is where the work is read and acted on (PRD hide-ai-suggestions B11).
A fix's agent commits on its branch and stops; its pane stays open until the operator decides the branch, so `에이전트 ↗` still reaches it, and a merge, a discard, `PR 올리기` or `다시 맞추기` ends it (PRD hide-ai-branch-first-apply; the PRD's technical structure closed the pane when the branch was ready, and this was changed so the operator can still ask the agent about its branch).
The fact that Hide AI started an agent comes only from the coordination record of the spawn (`docs/delivery.md`), never from a branch or worktree name: a checkout the operator made by hand on a branch called `hide-memory-m9` is an ordinary checkout.
The work is named in words, `제안 반영하기 · s12` for a suggestion and `Memory 반영하기 · m72` for a promotion that was under way, and never by an icon of its own, because Hide AI will start other work, each kind with its own home.
`docs/status-model.md`, Work Hide AI does for a home, owns the ownership value, the group rule and the turn table; this section owns what the screen draws.

While the agent works, finishes or rests, the work is in no list: not the sidebar's tree or Working, not the 태스크 list's 작업 중, 끝남 or 쉬는 중, not the phone's list, not Overview's requests and graph, and it moves no badge, rail count, checkout count, desktop notification or push.
The agent is still found by ⌘K, the recent panel, the tab strip and `hide agent list`, and its pane header keeps the owl; only the lists that enumerate the operator's work leave it out.
The sidebar tree has no `Hide AI` head, and a checkout Hide AI made is drawn as an ordinary checkout only while it is the front checkout or an agent the operator started works in it; the Hide AI agent's own row is never in a tree.
The same rule applies inside the empty-worktree and Inactive folds, so a fold's count is of the checkouts it draws; the cleanup fold lists every cleanup candidate, because reviewing them is its job, and that is where a worktree kept after its pull request settled is found.

The operator is called out in two cases, and both are one row in the sidebar's Needs You and the 태스크 list's Needs You with the owl beside the status mark, the task name and a place line naming the task where the branch would stand (`herdr-ide › Memory 반영하기 · m72`).
The agent asks: a question, an approval or a block, with the same read rules as an operator's row, so an AI-inferred question leaves once read and a block or native question stays until answered.
The home has a turn: the agent committed on its branch and stopped, so the second line is `바뀐 것 보기` until the operator decides it (PRD hide-ai-branch-first-apply B3); the attempt's pull request is open, ready, passing or unchecked and nobody asked for changes, so the second line is `PR #924 리뷰` and the row stays after it is read until the pull request is merged, closed, made a draft, or gets a requested change or a failed check; or checks failed and the agent is no longer working, so the second line is `CI 실패` until the agent works again.
A draft, running checks and a requested change are the author's turn and draw nothing.
A row has one line, and its second line is the agent's ask or its block's cause, then `CI 실패`, then `PR #n 리뷰`; the core decides the turn and the screen only words it.
The row's accessible name carries `Hide AI`, the task and the turn (`Hide AI · Memory 반영하기 · m72 · PR #924 리뷰`).
An attempt with no live agent (a failure with no pull request, an open pull request after the operator closed the pane) never has a row; the 제안 tab alone shows it.

Choosing the row, by click, by its digit or from its desktop notification, does not go to the pane: it opens the lane on the Hide AI tool's 제안 tab of the row's Project with that row open, and the `에이전트 ↗` link there opens the pane.
A Hide AI agent that asks the operator something is the exception: its answer goes into the session, so choosing its row opens the pane as any agent's row does, and the 제안 tab opens only for a decision the screen takes, such as a ready branch.
A row whose Project the core has not yet named opens the pane as any row does.
The phone's Needs You row opens the agent's detail.
The digits that number agent rows follow the drawn order, so a Hide AI row is numbered with the Needs You rows above the tree and no checkout of its own takes a digit.

When GitHub reports the attempt's pull request merged or closed, the row leaves Needs You, the agent ends and its pane closes, and the worktree is removed when its tree is clean and no other agent or running program is in it; the branch is always kept, and a pull request reopened later does not revive the attempt.
A worktree that is dirty or in use stays and is named in the diagnostic log, and it appears in the disk cleanup review; nothing is removed while GitHub's answer is missing, and an attempt already merged or closed before an upgrade is retired the same way on the next start.
A kind of Hide AI task this build does not know, or a record with no task, shows its agent as an operator's row naming its task (`Hide AI` alone for none) and leaves the rest of the ledger alone.
A feature that makes Hide AI start a new kind of agent names the kind's home in code, and chooses what its turn and cleanup are, before it has a row.
A connected device's agents follow the same rules, and they hold across a restart of hided because the ledger is persisted.
Web owners: `web/src/hideAiOrigin.ts` (the words, the place line, the turn line, whether a tree draws a checkout, where a row opens), `web/src/AgentMark.tsx` (the owl, the badge and the pane header's mark), `web/src/AgentRowMark.tsx` (the row's mark by view mode), `web/src/sidebar.tsx` and `web/src/components/sidebar-agent-row.tsx` (the rows), `web/src/TaskLens.tsx` (the 태스크 row) and `openListedAgent` in `web/src/actions.ts`; core owners: `herdr-core/src/coordination/started.rs`, `herdr-core/src/agent_state/axes.rs` and `herdr-core/src/runtime/memory_promotion.rs`.

### Agent logo view mode

The View menu, Help and Settings stand together, in that order: stacked at the foot of the device rail, their hints and menus opening to the right, or, while the rail is hidden, at the sidebar footer's right edge after the usage chips, opening upwards.
The View menu is an eye button whose hint and accessible name are `보기` (`View`).
It opens beside its button with the label `에이전트 로고`, two choices, `행 왼쪽에 표시` and `숨기기`, with a check on the current one, a separator, and a muted line that is not a choice: `⌥ 누르고 있으면 잠깐 보기` (`Alt` on Windows and Linux).
Arrow keys move through the choices, Enter chooses, and Escape or a click outside closes the menu without changing anything and returns focus to the button.
Choosing the checked choice only closes the menu.
While the daemon is not connected both choices are disabled and the check stays on the last value received.
A browser has no Agents hold, so its menu has no key line and no peek.
The choice is a core preference, `ui_state.agent_logo_mode` (`shown` or `hidden`, `hidden` when never chosen), changed only by `agent_logo_mode_set`; `ui_state_update` never changes it, the check always reads the core's value, every window of one daemon changes at once, and it survives a restart.
A mode the core refuses leaves the check and the screens as they were and records a diagnostic, with no alert; a change the core accepts but cannot save is drawn as chosen until the next start and records `ui_state.save_failed`, as the theme does.
Hidden is the default and draws the screens as before.
Shown draws the agent's 16-pixel logo after the status mark and before the title in the sidebar's agent rows (a sleeping row after its moon), in 태스크 rows and child rows, and in the Agent tabs; a row's second and branch lines indent past it in the 태스크 list, and a kind with no logo draws the neutral `>_`.
A row Hide AI started wears the owl as a badge on the logo.
Search, overlays, Overview, a Project's conversation list, the graph, Task boards and the phone never follow the mode, and the pane header, the usage chips and the Usage pages draw their logos in both modes.
While hidden, holding the Agents key (⌥, or Alt) draws the same logos for as long as the number keycaps show: the peek is the existing hold state, with its 0.15 second delay, its end on any other key, on a window that loses focus and while a layer is open, and it adds no key listener.
Shown, the same hold shows only the keycaps.
Changing the mode or peeking never changes a row's height, the scroll position or the selection.
Web owners: `web/src/components/view-menu.tsx`, `web/src/agentLogoMode.ts` (the one rule), `web/src/AgentRowMark.tsx`, `web/src/TabBar.tsx`, `web/src/TaskLens.tsx`, `web/src/components/sidebar-agent-row.tsx` and `web/src/components/sleeping-session-row.tsx`; core owner: `herdr-core/src/runtime/events.rs`.

### Pane turn row mode

The View menu's second section, after the agent logo choices and their key line and a separator, is labeled `요청·진행 줄` (`Request and progress line`) and offers `한 줄` (`One line`, marked `기본`, `Default`), `두 줄` (`Two lines`) and `끔` (`Off`), with a check on the current one; the keyboard, Escape, a repeated choice and a disconnected daemon behave as for the logo choices.
It shapes the second row of every agent pane ([Agent panes and the Agents explorer](#agent-panes-and-the-agents-explorer)) in every window of one daemon at once:

- One line keeps every turn on one line: the request at most two fifths of the row when an answer follows, the answer in the rest, each cut where it overflows, an ask as much as a quiet turn; the whole request stays on the turn card the request opens.
- Two lines put the request (after the children button) on its own line and the arrow and answer on the line under it, each cut, so a turn takes at most two lines; a turn with only a request or only an answer takes one, and a parent with no turn to show keeps its children button alone on one.
- Off draws no request, progress, result or ask on the row; a pane with live children keeps the children button alone on it, whose warning border and tree carry a raised descendant's ask, and Working keeps its thin line.
  The pane's own approval, question, block or stop rides the head after the title as a small warning-edged mark with its verb and wait (`web/src/PaneHeaderBand.tsx`, `PaneAskMark`), named by its verb and what is asked, which opens the same turn card as the request does and dims once read as the row's ask does; an ask is never hidden by the mode.

The choice is a core preference, `ui_state.pane_turn_row_mode` (`one_line`, `two_lines` or `off`, `one_line` when never chosen), changed only by `pane_turn_row_mode_set`; `ui_state_update` never changes it, an unknown stored value opens one line with the `ui_state.pane_turn_row_mode_unknown` diagnostic, and it survives a restart.
The mode never changes the terminal's viewport or its PTY grid, because the row overlays the terminal.
Web owners: `web/src/components/view-menu.tsx` and `web/src/PaneHeaderBand.tsx`; core owner: `herdr-core/src/runtime/events.rs`.

### Subagent count

Hide draws no count of the subagents an agent runs inside its own session, on any surface: not on a sidebar row, in a pane header, in the 태스크 list, in an Overview line, in ⌘K and not in Settings (operator decision on issue #810).
A subagent has no pane of its own, so it is not a delegated child: it has no row to open and the operator can neither enter it nor act on it, so the number is not a state they would act on (design principle 13).
The child marks on screen mean only the agents Hide started, which have their own panes and tree rows: the descendant mark `! N` in the tree popover, the tree button with its `자식 에이전트 N` tooltip, and the child popover.
A subagent count beside them would give "child" two meanings on one row.
The count reaches the core and is used off the screen.
Hide's hook keeps a `working` and a `done` count per pane and reports them as the Herdr pane tokens `hide_sub_working` and `hide_sub_done`; the core reads them in `PaneHookTokens`, projects them as `PaneChildren.subagents` beside `instrumented`, which no web code reads, and sums `working` for the pet's badge (docs/status-model.md, The subagent badge; the desktop app draws no pet today).
docs/agent-hooks.md owns the hook.
`working` is how many subagents run now, and `done` is how many this session has finished; a new session in the pane starts both again from zero, so `done` is a running tally of the session.
Each count is separately knowable, and a count the core cannot read is `None`, never zero, because a zero would claim the agent works alone; an agent whose adapter declares no count has none to read.
Claude Code, Codex, Grok, OpenCode and omp report a count, and Pi (it runs no subagents) declares none; the declaration is `subagent_counts` in `hide-agent-adapter/src/declarations.rs`, and no agent reports a blocked subagent.
Settings > Agents lists no subagent count either: the kit's feature table has no `subagents` entry.

### Closing an agent that spawned others

Closing a pane or a tab asks about the agents spawned from it only when one of them runs outside what closes (PRD close-agent-subtree).
Those are the live descendants of every agent pane that closes, on this machine or a connected device, asleep or not, less the panes that close anyway; a device that is not connected contributes none.
With none, the close is the ordinary one: a quiet pane closes at once, a working or attention pane asks the Stop-work confirmation, and an unknown status shows the status notice.
A pane whose agent Herdr is still starting (`시작 중`) never shows the status notice: it closes at once, or asks the Stop-work confirmation when its start carried the operator's first prompt, which the close would otherwise drop ([status-model.md](status-model.md#close-protection-is-separate-from-read-state)).
A target whose own status is unknown still shows only the status notice.
The Stop-work confirmation has one line under its title and lists every pane that closes, named as the sidebar names it, with its status mark; a working, asking or unknown pane adds its status word, and a quiet one is dimmed with its state in the mark's tooltip and its accessible name.
While it is open the list is live: a pane that settles stays listed and dims, one that starts working or asking brightens, and its title, line, buttons and focus stay as they opened.
While a listed pane's status is unknown, `Stop work and close` is disabled, `Keep open` takes the keyboard if the button held it, and a `Check status` in the confirmation reads the status again without closing it.
`Stop work and close` sends the close the confirmation shows at the press.
Otherwise one sheet opens in place of the Stop-work confirmation, titled `이 에이전트와 자식 에이전트 N개를 닫을까요?` (a tab: `이 탭과 자식 에이전트 N개를 닫을까요?`), N counting only the descendants outside.
It has no sentence under the title, and lists the target first and then its descendants in tree order, indented by depth, each with the sidebar's status mark and name and a device chip when it runs on another device than the target; the list scrolls inside the sheet when it is long.
Closing a tab lists every agent in the tab as a target, each followed by its own descendants, so a working agent beside the parent is shown before it closes.
A line above the list counts, with the sidebar's marks and neutral text, the descendants waiting for an answer to a question, approval or block (`답 대기`), working (`작업 중`), holding an unread result their row marks with a dot (`확인 안 한 결과`; a delegated child's finished turn is its parent's news, so it reads quiet here as on its row, status-marks B24) and unreadable (`상태 모름`), leaving out a kind with none; those rows are bright and add their status word in neutral text, and a quiet one (idle, read, asleep) is dimmed with no word, its state in its mark's tooltip.
A list longer than the ten rows it shows whole folds its quiet descendants that have nothing below them needing the operator: when two or more do, the list shows the targets, the rows that need the operator and the quiet ancestors on their path, then one line `끝난 자식 N명` (en `N finished children`) with a chevron, which opens the folded rows below it in tree order and closes them again.
Each row is focusable, and its accessible name is its name, its device when it differs, and its status word.
`이것만 닫기`'s tooltip and accessible description say what it leaves: the children keep running and come up into the operator's own list.
Colour in both sheets is the status marks' and the one destructive button's (`모두 닫기`, `Stop work and close`); `취소`, `Keep open` and `이것만 닫기` are neutral.
Its buttons are `취소`, `이것만 닫기` and `모두 닫기`, and `모두 닫기` holds the keyboard when it opens, so Enter closes the whole subtree; Escape or `취소` sends nothing and gives focus back.
While a descendant's status is unknown, `모두 닫기` is disabled, `취소` holds the keyboard, and a `상태 확인` in the sheet reads the status again without closing it; while a target's own status is unknown, `이것만 닫기` is disabled too.
`이것만 닫기` closes the target alone as the ordinary close, with no second question; its direct children become the operator's roots and their own children stay under them.
While the sheet is open its list is live, as in the removal dialogs: a descendant that appears shows up, one that goes drops out, and the title's N and the count line follow.
`모두 닫기` sends one event naming the target and exactly the descendants the sheet shows at the press: one that appears after the press is not closed, and one already gone counts as closed.
The core checks each listed pane again as it arrives; if one now needs a status check, an earlier close of one is still unresolved, more than 64 are listed, four closes with children already run, or every close slot is taken, nothing closes and the one-line notice says which and what to do.
A tree of any listed size is otherwise admitted: its panes take the close slots it holds a few at a time instead of all at once.
A pane whose earlier close was refused is simply closed again, since asking again is the retry.
If every descendant goes while the sheet is open, it turns in place into the target's Stop-work confirmation, a quiet target dimmed, whose `Stop work and close` closes the target alone; a descendant that appears under an open Stop-work confirmation turns it into this sheet the same way.
Either one closes by itself only when its target pane or tab is gone or its device disconnects; otherwise it waits for the operator's answer and runs nothing on its own.
A sheet that turns into the other leaves the keyboard on the sheet itself, never on a close button an Enter meant for the old one would press.
The core closes the deepest descendants first, each pane only after every descendant below it is gone, and the target last, so no descendant ever surfaces as a root on the way; each row reads `closing…` until it goes, and the target's row counts the panes closed so far over every pane the close takes, the target included (`닫는 중 12/34…`).
A descendant whose close is refused, times out, or loses its device keeps itself and its ancestors, the target included, open while the other branches finish; the close failure notice shows and the detail goes to the diagnostic log.
Closing the same target again lists only what is left.
The local panes and tabs closed this way are one Reopen closed tab entry, named after the target: one ⇧⌘T brings the target back first and then every descendant closed with it, each under the parent it was closed under, grandchildren included, and the keyboard stays on the target.
Each agent comes back as itself: its conversation resumes, and its letters, the watch on it and its parent relationship continue in the new pane; a descendant left open is drawn under its reopened parent again.
A member that fails to come back stays in the entry together with every member under it, which is not started, so no child reopens without its parent; the other branches still come back, the notice says how many did not, and the next ⇧⌘T retries what is left.
While the tree is still closing, ⇧⌘T waits with `The latest close is still being confirmed.` rather than reopening an older item.
A device's close leaves no entry, as before.
Every entry to a pane or tab close uses this: ⌘W or ⌥W, ⌘⇧W or ⌥⇧W, the desktop menu's commands, a tab's ×, the tab menu's `Close tab…`, the pane header's ×, the pane menu's `Close pane`, and the agent row menu's `Close tab…`; closing a View, the phone app and the Overview are unchanged.
Web owner: `web/src/close.ts` (which sheet, which panes and descendants, and their states), `web/src/Overlays.tsx` (both sheets), `web/src/components/subtree-list.tsx` (the list both the sheet and the removal dialogs draw); core owner: `herdr-core/src/runtime/tree_close.rs`.

### Terminal links

A terminal pane links what its program prints, whichever program it is: an http(s) URL, a path, and a link the program marked itself (OSC 8).
A plain-text path is a link only where it names something on this Mac, so a word that merely looks like a path draws nothing; the check runs when the pointer reaches a row, never per frame.
A relative path is looked for under the pane's folder and then under its checkout's root, `~/` names the home folder, and a location written after it (`:12`, `:12:5`, `#L12`, `(12,5)`) is where the file opens.
A link may be written against prose with no space: brackets and quotes of any script, sentence punctuation, and Korean, Japanese or Chinese words before or after it.
So `열어뒀어요(http://127.0.0.1:8391).`, `캡처는("/tmp/a.png")입니다`, `「docs/a.md」를`, `“https://example.com”`, `[label](src/a.ts)`, `{"path":"/tmp/a.png"}` and `--file=/tmp/a.png` each link only the address in them.
These places are found by the kind of character, a Unicode class, not from a list of marks: a link starts after an opening bracket or quote or an `=`, ends before a closing one, stops at sentence punctuation such as `,`, `。` or `、`, and a run of Hangul, kana or Han letters is cut once, where its script meets the link.
A run that touches `/`, `.`, `-` or `_` belongs to the name, so `docs/회의록에`, `회의록/a.md` and `한글.md` are checked only as written.
The original spelling is checked before treating punctuation, attached prose or a location as context: if `docs/a.md)에`, `docs/a.md)`, `(src/a.ts:12:5)` or `src/a.ts:12:5` is itself a file name, that complete name wins.
A drive path a Windows shell prints with backslashes, such as `C:\Users\example\AppData\Local\Temp\shot.png`, is a link like its `C:/` spelling.
Among the bounded spellings, the longest existing path wins, and of two the same length the one that keeps the token's start; a failed or budget-skipped longer check never establishes a shorter link.
A path ends at a closing bracket whatever follows it, so `docs/a.md)foo` links `docs/a.md` when only that exists, but never at an opening one, so `app/(auth)/page.tsx` never links `app/`.
In Korean prose such as `보드 보기 (docs/README.md)에 C안을 추가했습니다.`, only `docs/README.md` is underlined and clickable when that is the actual path.
A URL cannot be checked, so it is the longest spelling from its scheme whose brackets balance, which holds no quote and which does not end on sentence punctuation: in `PR #682 (https://github.com/o/r/pull/682)이 끝났습니다`, only `https://github.com/o/r/pull/682` is the link, `https://en.wikipedia.org/wiki/Mercury_(planet)` keeps its brackets, and `https://ko.wikipedia.org/wiki/서울이` links all of it.
Ranges follow the terminal's actual cells, including wide glyphs, combining sequences and wrapped rows.
A path or URL the terminal wrapped at its last column, or a TUI broke at its own margin and indented, is one link across its rows; a URL spans rows only where the terminal wrapped it, because it cannot be checked.
Under the pointer a link is underlined and the pointer becomes a hand; a click on it is the link's and never reaches the program, and a drag across it selects its text and opens nothing.
A click opens it in the front Workspace: a URL or an HTML file (`.html`, `.htm` or `.xhtml`, case-insensitive) as a browser display, another file in View at its line (an ordinary tab, not a preview), a folder revealed in the Explorer.
A file in another registered checkout opens the same way in the front Workspace and moves no checkout: the pane that printed it stays on screen, the Explorer keeps its tree and selection, and the file stays that checkout's document, so its own Workspace shows the same text and closing one view leaves the other.
A folder in another registered checkout brings that checkout forward to show it in its Explorer, as does any path while the Overview is on screen, on any machine: a checkout on a node or device comes forward once that machine's Herdr has moved there, and the file opens then; with no Workspace in front a URL goes to the default browser.
Right-clicking a URL offers Open in Hide, Open in default browser and Copy link above the terminal's ordinary editing and pane actions.
Right-clicking an existing path offers Open, Open in default application, the host's Reveal in Finder/File Explorer/file manager and Copy path; an HTML file in a registered checkout, or outside every checkout where the window opens outside files, offers Open in browser and Open source instead of Open.
Open source opens that HTML file in View at the line its link names.
These choices use the exact target under the pointer, including an OSC 8 label, and keep that target after the pointer leaves it; Copy path copies its physical path without the line suffix.
Opening or dismissing the menu moves no checkout, opens nothing and sends no click to the terminal program; a late path check cannot reopen a dismissed menu or replace a newer one.
A right-click opens only the menu, never the link, also on Windows, where the menu comes after the button is released.
⌘-click (Ctrl off macOS) hands the link to the operating system: a URL to the default browser, a path to its default application or, for a folder, a Finder window, within the limits below.
A file outside every registered checkout, such as `/tmp/shot.png`, opens on a plain click in the desktop app's own window on the core's machine as a read-only tab of the front Workspace, with an External (외부) badge beside its name (PRD outside-path-in-hide).
An image, a PDF and a video open in the viewers a checkout file uses, an HTML file as a browser display, and any other text, a script such as `.sh` or `.py` included, as read-only text at the link's line that saves nothing.
Holding the pointer on the badge shows the file's full path; there is no warning and no confirmation.
The tab splits, moves and closes like any View tab, the front checkout and the Explorer's selection stay as they were, and the file never appears in the Explorer.
A second click on the same file brings its one tab forward, reads the file again from disk and goes to the new link's line; an HTML page and its Open source are two tabs.
A text file of 16 MiB to 256 MiB shows the too-large notice a checkout text file of that size shows; a file that cannot be read opens the tab's ordinary unavailable state with a Close button.
While the Overview is on screen the pane that printed the path brings its Workspace forward and the tab opens there.
The tab lasts as long as the operator keeps it: it survives a reload of the window, and it is gone after the app quits or hided restarts, when a click opens it again.
Another window, a browser tab of the web shell, the phone, or a window on another machine, opens no outside file: a click there goes to the operating system as before, and the tab another window opened shows dimmed with its name and the hint that only the window that opened it shows it, never its contents or its path.
A program, an application bundle and an installer are only revealed; audio, a folder, a file over 256 MiB and any other binary go to the operating system on a plain click, as does a click past the 32 outside files the app holds at once.
The operating system opens only a plain document (text, source, a PDF, an image, audio or video) or a folder whose name has no extension; anything else, such as a script, a program, a spreadsheet, an application bundle or an installer, is revealed in Finder, never opened.
A program's OSC 8 link takes the same routes with no confirmation dialog: `file://` and `vscode://file/` addresses are paths, any other scheme is not opened, and a refused link goes to the diagnostic log.
A pane on an SSH device, and a page outside the desktop app, links URLs only, because its paths name files this Mac cannot check.
A path with a space in it is a link only where the program marks it (OSC 8), since plain text gives no way to tell where it ends.
Web owner: `web/src/terminalLinks.ts` (detection), `web/src/terminalLinkProvider.ts` (the check and the routes), `web/src/terminals.ts` (pointer-to-cell lookup), `web/src/PaneRelations.tsx` (target-specific menu), `web/src/actions.ts` (`openLink`, `openTerminalPath`), `web/src/editor/lineRequest.ts` (the line), `web/src/externalFiles.ts` and `web/src/ExternalDisplay.tsx` (an outside file's tab); desktop owner: `desktop/src/main/localPath.ts`, `desktop/src/main/externalFiles.ts`; core owner: `herdr-core/src/runtime/view_areas.rs` (`external_open`, `external_grants`).

## The Project tool

Web owner: `web/src/ProjectTool.tsx`, `web/src/projectStatus.ts` and `web/src/ProjectDetail.tsx` (the worktree table and the cost in the 보기 slot); core owner: `herdr-core/src/agent_state/tally/project_status.rs` (the 현황 counts and the 할 일) and `herdr-core/src/usage_ledger/project_cost.rs` (the cost).
The Project tool is the rail's first tool and is about the project of the checkout in front (PRD lane-two-columns D-14, B26).
Its head names the project, then its repository, path and machine on one dim line, then ⚙, which opens Project Settings, and `✕`, which closes the tool column; under it 현황, the tool's only page, since Hide AI is a tool of its own (see Hide AI: the Hide AI tool).

현황 is four tiles, then 할 일 (D-15).
Each tile says where it goes at its top right, `사이드바` or `보기`, and `닫기 ✕` while the viewing column shows what it opened, which a second press folds (B32).
A tile's number is a quiet bar until the core has answered, and a 0 is drawn as 0; nothing the system did not make is drawn (B33).

- 세션: the live sessions and a bar of 기다림, 일함, 대기 and 쉼; pressing it turns the sidebar to 태스크, narrowed to the project by the lists' project chip, whose `✕` lifts it (B27).
  While Hide AI's summaries are on and the project has open missed work, `놓친 일 N` in the warning colour sits in its corner, and pressing it opens the Overview's backlog for the project with its 놓친 일 group unfolded; it is never counted in 할 일 or on the rail (B28).
- PR: the open pull requests and a bar of 머지 가능, 실패, 리뷰 and 검사 중; pressing it turns the sidebar to the PR list, narrowed to the project (B29).
  A project with no GitHub remote reads `GitHub 원격 없음` alone, and after a failed read a ⚠ stands beside the number with the last read's age.
- Git: the worktrees, how far main is behind origin, and one mark per worktree, changed, not pushed or removable; pressing it opens the worktree table in the viewing column (B30).
  The table has the project's disk bar, a row for every worktree of the project, however many the tile's marks stop at, with its mark, branch, pull request and size, and the removable ones folded under `정리할 것 N` with `정리…`, which opens the disk cleanup sheet and its confirmation; the table reads the worktrees again when it opens and measures the disk while it shows.
- 비용 · 7일: the last seven days' total at API prices, a small bar per day and each agent's share; pressing it opens the cost in the viewing column, with a bar per day, the amount per model and the three sessions that spent the most, each opening its record (B31).
  Only calls whose session ran inside one of the project's checkouts on the core's own machine count, since reading another device's usage would be a new remote call, so a project on a connected device reads `이 기기에서 실행한 것만 집계해요`.
  An agent with a call Hide has no price for is named `가격 없음` apart, those calls in no amount, and when its other calls are priced the total reads `≥`.
  Before the core's first reading the tile is the quiet bar and the page a placeholder; a reading that found no priced call is `$0.00`; when no reading has succeeded, the usage store failing to open included, the tile reads `–` and the page says so, and after a later failure the page keeps the last amounts and says they are the last read.

할 일 opens with a head one size above its rows and bold, with its count; a group inside it keeps a smaller muted head, so the screen reads in three steps, part, group, row.
할 일 holds only what stops without the operator (D-16, B34-B37).
Its first group, `나를 기다리는 에이전트`, lists the questions and decisions waiting for an answer, read or not, then `다시 볼 것`, the ★ tasks, the rows of the sidebar's 태스크 Needs You and 다시 볼 것; a live session whose record still holds an open question joins the asks though its row asks nothing now, one line per pane; reading an ask keeps its row and the count, answering it takes it out, and ☆ moves it to 다시 볼 것 (needs-you-cleanup B11, B13).
Its second group, `내 PR · 내 리뷰`, lists the project's pull requests the PR list puts in Needs You, the team's review asks and bots' left out; one opened from a Memory card wears `◆ Memory에서`, and a review left on GitHub or a moved state takes it out on the next read.
A row has no button: an agent's row says `pane` and brings that pane forward with the keyboard, a pull request's says `보기` and opens its card in the viewing column (B36).
Past five rows the list shows five in that order and `+N 더`, which shows the rest the core sent; past the core's 64 rows a last line sends each kind left over to the sidebar list that holds it, narrowed to the project: `나머지 N개는 사이드바 태스크에서` for the agents' rows and `나머지 PR N개는 사이드바 PR 목록에서` for the pull requests, one link per kind there is; with nothing to do its head reads `할 일 0` over a dim `없음`.

The core derives the counts and the 할 일 only on a publish whose inputs moved, and nothing on that path reads a file or runs a process (B42; [ARCHITECTURE.md](ARCHITECTURE.md#project-status-and-cost)).

## Hide AI: the Hide AI tool

Web owners: `web/src/HideAiTool.tsx` (the head, the tabs, the one-time line, the off state), `web/src/SuggestionsTab.tsx` and `web/src/suggestionList.ts` (the 제안 tab), `web/src/HideAiSettings.tsx` (⚙), `web/src/MemoryTab.tsx` and `web/src/memoryList.ts` (the Memory tab); core owners: `herdr-core/src/model/suggestions.rs` and `herdr-core/src/runtime/suggestions.rs` (the `suggestions` section), `herdr-core/src/runtime/project_memory.rs` and `herdr-core/src/runtime/memory_promotion.rs` (Memory).
Hide AI is the rail's second tool, under the Project tool, in the place Memory had, and shows the Project of the checkout in front (PRD hide-ai-suggestions D-01, B1).
Its icon is the brain the Memory tool used, one colour like every rail icon (D-25).
Inside it are two tabs, `제안` and `Memory`; each tab carries a warning-coloured number of what the operator has to do in it, none at 0, and the rail icon carries their sum, its hint ending in `제안 3 · Memory 1`.
The core counts both: 제안's number is its new suggestions and its rows that are the operator's turn, Memory's is its document disagreements, and both are 0 while the head switch is off (B3).
The lane keeps the tool's id `memory`, so a saved lane, a chord and `Toggle Hide AI tool` (`toggle_lane_memory`) reach it as before; ⇧⌘L, a Memory card number under a conversation and Project Settings' `Open project Memory` open it on the Memory tab, and a Needs You row on 제안.
The tab shown and the 제안 row open are this window's, not the core's.

Its head reads `Hide AI · <project>` beside the brain, then the switch for all of Hide AI in this Project, ⚙ and `✕`, which closes the tool column; the agent Hide AI asks now is named in ⚙, not in the head.
The switch is on from the start; turned off, neither 제안 nor Memory reads anything, Hide AI makes no call for the Project, the rail and the tabs count nothing, and the reading owl says `Hide AI가 꺼져 있어요` over what 제안 and Memory each give once it is on, under their names (B3).
Where a tab has nothing to show yet, the same owl explains it: its state is the title and what the tab does for the operator follows in three short steps, each with its mark (`HideAiExplainer`); Memory's are `우리가 대화하며 정한 것을 기억해요`, `모든 에이전트가 같은 기억을 써요` and `같은 말을 두 번 하지 않아도 돼요`.
Turning it on gives each of the two back as it was.
The first time the tool opens with Hide AI on, two lines above the tabs say that investigating uses the operator's own subscription and what is sent; they stay while the tool stays open and are not shown again (B3, D-36).
While the Project is still being read the tool says so; a Project that is not registered, or whose Hide AI could not be read, says that in one line, the second with `다시 읽기`.

### The 제안 tab

The 제안 tab lists what Hide AI suggests fixing, the fixes under way and the ones applied (D-05 to D-07).
The core decides every group, mark, hold and count; the screen draws them and every press is one `suggestion_action` (`apply`, `dismiss`, `restore`, `retry`, `revert`) naming the row by its id (`s12` for a suggestion, `m72` for a memory on its way into the documents).
The tab has one line for its state and no more (D-44): `다음에 Hide를 열 때 첫 조사` before the first investigation, `오늘은 제안 없음` when today's run left nothing open, each of these two as the owl's title over what the tab gives the operator, `세션과 PR 리뷰에서 에이전트가 자주 막히는 곳을 찾아요`, `같은 실수가 반복되지 않게 개선 방향을 제안해요` and `고르면 고친 내용이 브랜치에 준비돼요. 합치면 에이전트가 알아서 똑똑해져요`, and `제안이 꺼져 있어요` over the same steps with `켜기` under them when ⚙ turned suggestions off; while a run goes, `조사 중` sits small in the head and the rows stay as they were, and after a run that could not investigate `조사 못 함` sits there the same way until a run investigates, nothing new having risen from it (D-35).
A failure the operator cannot act on, such as a run that hit its turn or time limit or the usage limit, goes to the diagnostic log and draws nothing.

The rows fall into `고를 것`, `반영 중` and the folded `반영됨`, each head with its count, and a row's mark says whose move it is (D-37): a green open circle for a new suggestion, a yellow dot for the operator's turn, a blue turning ring with dimmed text while Hide AI works; a row under 반영됨 has no mark.
A document fix is titled by what it changes (`Rust 검증은 verify-cargo.sh로만 돌리게 하기`), with why it is there on the line under it: the discomfort in a person's words, or, for a line the grouping wrote alone, its signal and tag (`같은 명령이 계속 실패해요: cargo test`).
Under that is the file it changes, as a path with a file icon and `새 파일` when the repository has none yet, and the words it writes there in a tinted box, so what a press does is read before it; its button names the file (`AGENTS.md 고치기`), and `놓친 것 · <cause>` shows once the row is open.
Hide AI offers a document fix only once its investigation named that file and those words, checking that the path is inside the repository and finding out whether the file exists; a line that has neither stays unoffered, and one offered before this rule shows its `고칠 것` sentence and `문서 고치기` until its next investigation.
Any other suggestion is titled by its discomfort or its signal and tag, with `놓친 것 · <cause>`, then `고칠 것`, and its buttons under it (B10, D-51).
A Memory card offered for the documents shows how often it was said (`내가 말한 세션 3개 · 에이전트가 꺼내 본 세션 2개`); opened, it shows the card's text under `기억 내용` and `넣을 문서는 에이전트가 고르고, 합치기 전에 바뀐 내용을 보여 드려요`.
A card about behavior planned but not built yet is never offered for the documents.
Pressing the row opens it in place to `근거`, each piece of evidence a line with its agent's logo, `세션` or `PR #n`, the signal and its time; a session opens its record and a pull request its card, and evidence whose session or pull request is gone stays dimmed and opens nothing.
An improvement's evidence shows its kind as a small chip before `세션` in place of the signal (`문서 없음`, `다시 찾음`, `잘못 짐작`, `도구·설정 불편`; B25), and a line the grouping wrote alone from improvements reads `같은 곳에서 계속 헤매요: <what the turn's end said>`.
Its button is `문서 고치기`, where an agent edits the repository's files in a worktree and commits them on a local branch without pushing (PRD hide-ai-branch-first-apply D-01, B1), in any git repository with or without GitHub or a remote, or, in the secondary style, `이슈로 남기기` when the fix is outside the repository or not settled yet, where Hide opens an issue from CI facts alone (B14); an issue's `고칠 것` is the core's reason, or `문서로는 못 고쳐요. CI 기록을 담아 이슈로 남겨요` when it gives none.
With GitHub not connected or not permitted `GitHub 연결 필요` stands in place of `이슈로 남기기` alone and the suggestion stays (B18); a Project that is not a git repository gets no document fix suggestion (B16).
While five branches of the Project wait for a decision, `결정을 기다리는 브랜치가 5개예요` stands in place of `문서 고치기` until one is decided (B28).
Beside every choice's button, and beside the line that stands in its place, is a quiet `넘어가기` button; there is no `⋯` (D-51).
`넘어가기` means not now: the row goes away, and new evidence of the same problem gathers on it; when as much piles up again as when it was skipped, it rises once more reading `넘어갔지만 다시 생겼어요` in the warning colour, and skipped again it does not rise (D-43, B16).
A suggestion that came back after its fix merged reads `문서로는 안 됐어요` with the last pull request, and its fix is one step stronger (B15).
In a Project whose ⚙ says `자동으로 준비해 두기` a new suggestion goes to 반영 중 by itself and wears `자동`; while an automatic branch waits for the operator's decision, or an automatic pull request waits unreviewed, the next one waits in 고를 것 with `자동으로 준비한 브랜치의 결정을 기다려요` and can still be applied by hand (B13, B15).
Hide merges only when the operator presses `main에 합치기`, never by itself and never in the automatic mode, and it never pushes.

A row being applied says what its agent does: `고치는 중` (`PR 만드는 중` after `PR 올리기`, `다시 맞추는 중` after `다시 맞추기`, `이슈 만드는 중`) with the time since it started and `에이전트 ↗` while the agent lives (B2).
While that agent asks the operator something, the row shows `에이전트가 물어요` with the question, the same line the sidebar draws, and `세션으로 가서 답하기`, which opens the pane.
When the agent committed on its branch and stopped, the row has the yellow dot and reads `바뀐 것 보기` with the branch's name, its file count and +/− lines (B3), and its buttons are `main에 합치기` (named for the Project's base), `PR 올리기` only while GitHub is connected and permitted, and a quiet `버리기` (D-03).
`바뀐 것 보기` opens the branch's card in the lane's 보기 slot: the branch, `→ main`, each changed file with its +/−, and the same three buttons in its head; a file opens its diff against the base in the same slot, read on the Project's machine, and a branch of one file opens that diff straight away (B4).
Opening the row shows the same thing inside it, above its buttons: `바뀐 파일 N` with each file and its +/−, and the chosen file's diff, the first one until another is pressed, from its first hunk and at the row's caption size since the list above names the file, so a branch can be read and decided without the 보기 slot; its evidence folds to `근거 N개` with `펼치기`.
The card and its diff never move the checkout in front, its Agent tab or the keyboard; the card reads its row from the 제안 tab the rail keeps open for the Project in front, and says `지금 열린 프로젝트의 Hide AI 줄에 이 브랜치가 없어요` once the row left it.
`main에 합치기` reads `합치는 중` while Hide merges on the Project's machine: a fast-forward when it can, else one merge commit, and nothing pushed; merged, the row folds under 반영됨, the worktree goes and the branch stays (B6).
A merge that would touch files with uncommitted changes, or write over a file git does not track, an ignored one included, at the same path or a folder's path around it or inside it, changes nothing and reads `main에 저장 안 한 변경이 있어요` with `다시 시도` (B7); one the base moved under reads `main 브랜치가 바뀌어 다시 맞춰야 해요` with `다시 맞추기`, which has an agent make the same branch again on the base (B8, B12).
A merge refused by anything else changes nothing too, and reads with `다시 시도`: `합치는 사이 main 브랜치가 바뀌었어요` when the base moved while the merge was prepared, `본 뒤에 브랜치가 바뀌었어요. 바뀐 것을 다시 확인해 주세요` when the agent committed after the operator read the branch (a merge is pinned to the commit the row showed, and an open file's diff is read again at the new commit), and `합치지 못했어요` when the machine could not merge it, with the reason in the log; a discard that could not delete reads `버리지 못했어요`.
`버리기` asks nothing: the worktree and the branch are deleted and the suggestion is skipped as `넘어가기` skips it (B10); a branch merged or deleted outside Hide, by hand or through its pull request, reads the same as Hide's own merge or discard (B13).
An agent that ended with no commit reads `바뀐 것 없이 끝났어요` with `다시 시도` and `넘어가기` (B5).
After `PR 올리기`, the row reads `PR #n 리뷰` in the warning colour with the pull request once it is open and the operator's turn; `CI 실패` in the destructive colour, or what stopped it in plain words (`PR 없이 끝났어요`, `에이전트가 시작하지 못했어요`, `셸이 시작을 마치지 못했어요`, `아직 만들어지지 않아 문서는 그대로 뒀어요`, with the core's detail, such as the shell's question or the agent's reason, on a quiet line under it), keeps the row there with `다시 시도` and `넘어가기` (B11, B12); a code this build has no words for reads `PR 없이 끝났어요`.
An open issue reads `이슈 열림` with the issue.
A merged row, however its branch reached the base, folds under `반영됨 N · 효과 있었음 M`; while the watch runs its line is the quiet `지켜보는 중 · 세션 2/5`, a row whose problem did not come back reads `효과 있었음`, and nothing else is said (B15).
A request the operator keeps repeating and a memory agents keep missing rise here too (PRD memory-self-improve B6, B12, B18), titled `같은 요청을 계속 하고 있어요: <meaning>` or `에이전트가 받은 기억을 계속 놓쳐요: m31` when the analysis left no discomfort; a project's way of working rises as any suggestion does, with a skill for a procedure as its first fix.
A personal preference's fix is one line of the Hide global instruction, which every Hide pane session is given: in place of `고칠 것` the row shows `Hide 전역 지시에 들어갈 줄` over the line in a tinted box, and its button is `Hide 전역 지시에 넣기`, which needs no GitHub and is never pressed automatically (B8).
The press goes in with the preview the row was drawn from; when the file or the line changed since, nothing is written and the row reads `파일이 바뀌어서 새 미리보기예요` with the new one, and when the line has no room it reads `자리 없음: Hide 전역 지시가 한도를 넘어요` with `파일 열기`, its button held (B10).
A line with markup, a hidden character or a secret reads `이 줄은 넣을 수 없어요: 마크업, 보이지 않는 문자나 비밀값이 있어요`, its button held.
A line the file already had reads `이미 Hide 전역 지시에 있어요` and offers only `넘어가기`; pressed when the file had it, it is adopted, and its `되돌리기` leaves the file as it is.
A write that failed reads `파일을 쓰지 못했어요` and its button becomes `다시 시도` (B29).
An applied line folds under 반영됨 reading `Hide 전역 지시에 들어 있어요` in the success colour, with `되돌리기`, which takes out only that line and brings the row back to 고를 것 (B9); a line changed by hand reads `손으로 고친 줄이라 그대로 뒀어요` with `파일 열기` and no `되돌리기`.
Said again in a session that was given the line, the row comes back to 고를 것 reading `넣은 뒤에도 또 말했어요` in the warning colour with `파일 열기`, where the operator rewords it, and `넘어가기` (B11).
An ignored memory's evidence carries a chip, `목록으로 받음` or `본문으로 받음`, saying how the session had been given it (B15).
`파일 열기` shows only in a window on the core's machine, whose file it is.
Opening a row reads its operator's words again, masked, at most 300 characters each and five a row, each a quiet quoted line under its evidence; words that cannot be read again say `근거 확인 불가`, and when none can, the row reads `근거를 확인할 수 없어 넣지 않아요` and its button is held (B7).
The tab's first line is `Hide 전역 지시` with `너무 커서 세션에 넣지 않아요` or `읽을 수 없어 세션에 넣지 않아요` and `파일 열기` while sessions are not given the file because it was edited past its limit or out of UTF-8, and nothing while they are (B5); `파일 열기` hands the file to the system's editor.

A row being applied whose agent lives is also one row in the sidebar's and the 태스크 list's Needs You while it is the operator's turn (see Work Hide AI does for a home), and choosing that row opens this tool on 제안 with the row open (B11); a memory on its way into the documents is one of these rows from its first offer, named by its memory with its number (`m72`) before the title, and so is a promotion already under way before this change (D-02, B2).
The memories one `N개 문서에 반영` sends are one row titled `정정 N개 문서에 반영`, one agent and one branch with a commit per memory, and the row follows the same branch flow; a memory the agent could not apply goes back to the Memory tab's `확인할 것` with its reason (PRD hide-ai-branch-first-apply B20).

⚙ opens a sheet under the head in three areas (D-26, B19): what 제안 and Memory share, then 제안, then Memory.
The shared area is a tinted box: `사용하는 AI` and the chip of the agent Settings › Hide AI chose (the listed agent standing in while Runs on is past the usage limit), which opens that page, then `설정 › Hide AI의 순서를 따라요. 한도에 걸리면 다음 AI로` (D-53) and the line on what is sent; when that agent cannot promise to only read (OpenCode), `이 AI로는 조사할 수 없음` stands under the chip and no investigation runs.
제안 and Memory each open with a head of their own, a mark, the name, what it does and its own switch, and their settings sit under the name.
제안 holds `제안 양` (`적게`, `보통`, `많이`, one segmented choice with what the chosen one means and how many a week under it), `반영 방식` (`확인하고 반영`, `버튼을 누르면 브랜치에 고쳐 두거나 이슈를 열어요`, or `자동으로 준비해 두기`, the same way), `읽을 곳`, and `넘어간 제안 N개`, which unfolds to each one with `되살리기` and a quiet line saying whether it can still rise once (`다시 쌓이면 한 번 올라와요`) or not (`다시 올라오지 않아요`); restoring one lets the core judge it again on what piled up.
`읽을 곳` has two switches (D-52): `세션 기록` (`실패한 명령 · 에이전트 막힘 · 내 정정 · 반복한 요청 · 안 지킨 기억 · 세션에 남은 문제 · 개선할 점`) and `GitHub` (`CI 실패 · 리뷰 지적`).
The core keeps nine signals; a switch shows on while any signal of its group that can be read is on, and one press is one `suggestion_settings` with `signal_group`, which turns the whole group (B5).
`GitHub` is off with `GitHub 저장소가 아니에요` under it when the repository is not on GitHub; with Memory off, `Memory가 꺼져 있어 내 정정과 반복한 요청은 읽지 않아요` stands under `세션 기록`, and with Hide AI or the turn-end analysis (Settings › Hide AI's agent summaries) off, `개선할 점은 턴 끝 분석이 켜져야 기록돼요` stands there the same way (B27, D-57).
Memory holds `이 프로젝트의 기억 지우기` in the destructive colour, which asks first, naming the Project and the number of memories.
With the head switch off every switch in the sheet is dimmed.

### The Memory tab

The Memory tab is whether a memory is right: its questions, the memories and the search (D-02); a memory's way into the documents, a promotion that failed and a decision nothing builds yet are rows of the 제안 tab.
User text says `기억` where it said `카드`, in all four languages (D-03); the tool and the tab keep the name Memory.

The core opens one Project's Memory per daemon; the rail, whatever the lane holds, names its Project when it shows, when the front Project changes and when its window comes forward after another window named another Project, so the Hide AI icon's number is current, and the tool reads until the core answers for its own.
Turning the switch on when this Mac's agent hooks do not carry Memory yet updates them first and turns it on once they land.
A press before Hide has first read those hooks, right after it starts, shows the switch pending, waits for that read and then runs as pressed; a read that fails refuses the press, and a press for another Project meanwhile is refused as busy.
A Project on any linked device shows the same Memory controls and cards as this machine's Projects, read from the core's store with the device's own Project identity.
The disclosure says cards stay in Hide's Memory, and the delete confirmation removes them from this Project's Memory; neither claims the screen's device owns the store.
An unavailable link or Memory reader uses the existing unreadable state; its reason is recorded in the diagnostic log.
Each compact row has a one-line title and one-line reason, truncating long text; `Compare` reads and opens the document excerpt and original operator words with their dates only when requested.
A confirmed enduring operator correction (`정정 확인됨`) is a row with a checkbox, its number and title, the document's current sentence struck through in red, the sentence that replaces it in green (each ending in an ellipsis after two lines), then `내가 한 말 · <date> · "<one line of the operator's words>"` and the document's file name (full path on hover); `Compare` still unfolds the whole excerpt and the original words.
A correction whose pair is not stored yet shows only the operator's line and `Compare`, and the pair fills in after the next recheck.
Confirmed rows are checked from the start, the most recent first and at most five; with five checked the other checkboxes cannot be checked, and with none checked the bar's `N개 문서에 반영` cannot be pressed.
That one button sends the checked numbers as one promote action, and `Memory is wrong` is in the row's `⋯` menu.
When the Project already has five branches waiting for a decision (`branches_full`), the button cannot be pressed and the bar reads `결정을 기다리는 브랜치가 5개예요`.
Unresolved authority (`문서와 다름 · 어느 쪽이 맞나요?`) shows the document's and the memory's sentence on one line each (when the pair is stored) with `Memory is right` and `Doc is right`, and cannot be checked until answered; `Memory is right` makes it a checked confirmed row.
Dates remain provenance and never decide authority.
Choosing the document marks the card Outdated; choosing the card turns the row into a document correction proposal.
When the pull request merges or closes, its agent ends and its worktree goes with the branch kept, so a closed attempt's `Agent ↗` link leaves with the pane.
When a promotion reports a card outdated, its empty, clean worktree is released after its agent has left; committed or uncommitted work stays and a later promotion continues on that same branch and PR.
An accepted outdated report returns the card to verification with its original sources intact.
An outdated card sharing a batch waits while any sibling still runs, has an open PR or follows a building branch.
`기억 N` lists the other memories, two lines each: the number and title, then the kind, how many sessions it was given to, how many times it was pulled, how many of its sources are the operator's words, and when it changed.
The first five show, the rest fold under `그 외 N`, and a card shown in full is never folded; the search above them shows every match, best first, and is the Project's, so every window shows the same query and only the one typed into sends it.
Once merged, its ordinary card dims and shows the file it went into, or the merged pull request's mark when no file was named, with `파일에 반영됨 · 전달 안 함`.
A personal preference carries `Personal`, remains available to agents and never appears as a document suggestion.
Unresolved fact disagreements remain withheld and readable in ordinary card detail, with no unsupported document correction action; rule, lesson and decision disagreements are eligible for correction.
A confirmed enduring correction says `Correction confirmed · document update pending` in the existing comparison row and remains available to agents as untrusted reference data.
It retains the original human source and document quote; unrelated document commits do not ask the same authority question again.
When the owning document incorporates it, background reconciliation turns the card into the existing document pointer without another promotion.
Turning Memory off retains cards and analysis progress; its setting explains that enabling it again catches up on unprocessed conversations, including conversations from the disabled period.
A Project at its 10,000-card limit makes room for a new card by forgetting its least recently used one, and the tab says so in one quiet line under the head, `N older memories were cleared to make room for new ones.` (`새 기억을 남기려고 오래된 기억 N개를 지웠어요.`), counting the cards forgotten that way since the tab last said so.
The line is said once: it counts as seen once it is in view on a visible page, stays while the tab is open, and the next time the tab opens it is gone until another card is cleared, when it counts only the new ones.
`hide memory undo mN` brings one of the newest 1,000 cleared cards back, and so does `되돌리기` on a card just forgotten, but not while the Project is at its limit: the undo is refused with `mN cannot come back while this Project's Memory is full; forget another card first`.
A card added by hand is never cleared this way.
Only when every card is one a person made or settled (added by hand, a confirmed correction, or a card whose promotion is under way, waiting on the operator, merged or kept) does the line become the warning `This Project's Memory is full, so new cards are not kept.`, from the same check that refuses a new card; merges and corrections of existing cards still land.
An Outdated comparison dims its card and names the document and its date, asks nothing and is excluded from injection.
Document references in card metadata and Compare headers show a single-line filename, truncated when needed, with the full path on hover and the date alongside.
An unverified card waiting to be checked against the repository again is dimmed; only the card the worker is checking shows `확인 중` in its existing status line, and a promotion keeps priority there.
An already verified card awaiting its first document comparison stays usable and keeps its suggestions while waiting, even when the checking budget is spent.
A card neither given nor said for 30 days shows `30일 안 씀` beside its kind and is still given (PRD memory-self-improve B22).
After 60 days it leaves the list for `보관 N`, a folded line under the cards that opens to each archived card's number, title, archive date and `되살리기`, which brings it back with a fresh clock.
A card shown in full has `다시 확인` beside `지우기`: it takes the card out of the hooks until it is checked again, and while it waits the button reads `다시 확인 중` (B27).
Hide AI finishes the check, so with Hide AI off (Settings › Hide AI) `다시 확인` is dimmed with `다시 확인하려면 Hide AI가 켜져 있어야 해요. 설정 › Hide AI에서 켜요` as its hint, and the core refuses it as `hide memory recheck` does (D-41).
Under its sources, `변경 기록` lists what Hide changed on the card by itself, newest first: `안 쓰는 기억으로 표시`, `보관됨`, `되살림`, the operator's words added as aliases, the memories merged into it, and each recheck with who asked and, once its sources were read again, how many were letters and how many could not be read (`근거 확인 불가`) (B34).
Each line has `되돌리기`, or says why it cannot be taken back (a later change replaced it, a merged memory changed or is gone, the check already ended), and a taken-back line stays dimmed as `되돌림`.
Nothing here alerts: a miss, an alias or an aging move is only a history line (B17).
The waiting reason or exhausted checking budget is explained once above the list, without repeating it on every row.
When no card is usable and cards await checks, the existing empty slot says why and that accepted cards return one by one.
The core supplies the checking budget's next reset timestamp, and its explanation draws that date and time in the viewer's timezone and interface language, with no UTC wording.
With Memory off, stored cards are retained but the list and its empty slot stay hidden behind the existing off disclosure, and the usable count is zero.
An older consent shows a short re-consent explanation under the existing off disclosure and points to the switch above; it never enables or injects on its own.
The core supplies the usable count, current card and reasons, and the shell only draws them; no banner, alert, toast or panel is added.
The checking mark is one line in the lane at its 360px minimum in all four languages, and the card is still listed and opens in full; an unverified card is not given to agents until the check ends (PRD memory-card-quality D-13, B6, B19).
A card on its way into the repository is never folded under `그 외 N`, and a card opened from a conversation clears a search that leaves it out.
A card opens in place: `N분 전에 바뀜 · 이전 내용` when a text replaced another (the earlier text with `되돌리기`), its body, its cited files, its sources with their runtime, time and whether they are the operator's words, and `지우기`.
A forgotten card leaves `mN 지움 · 되돌리기` at the bottom of the tab until it is undone or another card is forgotten.
The `⋯` beside `기억 N` holds `데이터 삭제`, which asks first, naming the Project and the number of memories, because it cannot be undone.
An empty Project says no cards yet and when sessions are analyzed.

## Terminal image attachment boundary

Web owner: `web/src/attachments.ts`, `web/src/PaneView.tsx`, `web/src/explorerDrag.ts`, `web/src/terminals.ts`. Core owner: `herdr-core/src/runtime/attachments.rs`; device staging `hide-host/src/attachments.rs`.

Dragging an Explorer file onto a terminal focuses that pane and inserts the original absolute path as quoted text, without submitting it.
Image files instead read through the checkout's existing bounded file-bytes stream and enter the same binary attachment flow as an external image drop, retaining their original format and filename.
One Explorer image read is admitted at a time, capped at 20 MiB, and a page reconnection, receiving pane closure, or terminal transport generation change during that read discards the result.
Folders and files from another device show the forbidden drop cursor and insert nothing; moving files inside Explorer keeps its existing behavior.
Image acceptance and display remain the receiving program's responsibility, as for other terminal attachments.

Dropping local file URLs into a visible terminal focuses that receiving pane and starts one attachment intent through the existing ordered writer.
Paste captures PNG or TIFF clipboard images at the same ingress; ordinary text and keys keep their existing terminal behavior.
The core reserves the original pane and connection generation before asynchronous image preparation, file validation, or transfer, so later focus changes cannot redirect the attachment.
Clipboard images are normalized into private PNG files off the input thread, while the core owns transfer state, held input, retry, and cancellation.
Local files keep their original paths after validation; a device's attachments are staged through that device's link and insert only the resulting device paths.
When the terminal advertises bracketed paste, each path has its own paste frame and the complete drop is enqueued once in file order; otherwise the paths are inserted as shell-quoted text with a trailing space.
Spaces and apostrophes survive; shell expansion characters are escaped, and paths containing control characters, symlinks, directories, and special files are refused with an actionable notice.
Nothing performs automatic Enter or replaces existing prompt text.
One transfer is admitted at a time, capped in file count and size; clipboard decoding is also capped by megapixels.
The original terminal's later input is held up to a fixed size, then explicitly refused rather than growing the queue or silently submitting an incomplete prompt.
Only a successful transfer to the original live generation releases the attachment followed by that held input.
A compact notice above the affected terminal shows preparation, upload, or failure and offers Retry where safe, or explicit cancellation that discards held input.
An original pane that closes or reconnects invalidates the intent; neither files nor old input are forwarded into a replacement session.
Files older than 24 hours are pruned on the next attachment attempt, not by a resident background cleaner, so successful local clipboard references remain readable after the paste.
Failed or cancelled transfers clean up only their own created files where the destination remains reachable, with deferred cleanup recorded diagnostically.
It inserts paths even when a provider cannot decode the referenced file; provider validation remains visible in its own composer.
Hide provides no thumbnail shelf, attachment membership, or synchronized removal; subsequent editing and submission remain native provider operations, and pasting a path is not proof of image acceptance.
Reintroducing a shared attachment shelf requires a supported provider contract for stable attachment identity, idempotent add/remove, native draft changes, and accepted submission events; both surfaces would have to reflect the same attachment membership, and the shelf would have to clear only after confirmed submission, preserving items on failure.

## Overview

Web owner: `web/src/Overview.tsx` (the modal and the page, the title and the device switch), `web/src/MainScreen.tsx` (the tab row with its counts and controls, the unreachable device, with `web/src/HerdrTakeover.tsx` for this machine's Herdr on another protocol, the Home with no project, the side panel), `web/src/overviewTasks.ts` (a device's tasks as the record and the backlog read them), `web/src/OverviewRecord.tsx` (a session's record beside the tabs at Home), `web/src/BacklogTab.tsx` and `web/src/overviewBacklog.ts` (백로그), `web/src/ProjectsTab.tsx` and `web/src/overviewProjects.ts` (Projects), `web/src/QuickAdd.tsx` and `web/src/quickAddChoice.ts` (the quick add), `web/src/issueMarks.tsx` (an issue's glyph and labels), `web/src/IssueDialogs.tsx` and `web/src/issueStart.ts` (Start).
Core owner: `herdr-core/src/agent_state/tally/tasks.rs` (the tasks), `herdr-core/src/runtime/backlog.rs` (the backlog's add, retry, undo and origin), `herdr-core/src/runtime/disk_sweep.rs` (the Projects tab's disk), `herdr-core/src/tasks.rs` and `herdr-core/src/local_issues.rs` (issue sources).
The Overview has five tabs in this order: Work, Projects, Backlog, History and Usage (PRD overview-redesign).
Work and the sidebar's 태스크 list share canonical agent facts; Work adds their explicit delegation and delivery relationships without replacing the sidebar.
Projects opens a project home, Backlog combines source-qualified issues, and History searches retained operator requests.
The removed copied-conversation index and its raw transcript search remain retired; History uses the existing session-work ledger instead.

### The page and the device switch

The Overview opens from the sidebar's Overview row, the top row's Overview button or ⌘⇧O, initially on the device in front on the rail.
An agent pane's `개요에서 보기` opens it instead on Work, whatever tab was last, on the device that pane runs on, with its session chosen on the map (PRD pane-open-in-overview B4).
Work, Projects, Backlog and History share the chosen device and optional canonical project scope; Usage always shows the core machine's global account usage.
It opens on Work the first time and remembers the chosen tab for the current window.
It opens as a centered modal over a mounted Workspace, as large as the window allows with the window inset (`--size-settings-sheet-window-inset`) clear on every side and at most `--size-overview-sheet-w` by `--size-overview-sheet-h`, preserving terminal geometry and output, and otherwise as a central page.
Its head is the title `Overview` and the device switch: every connected device as its rail tile and name, a device that does not answer marked `연결 안 됨`, and every other device's tooltip carrying `내 차례 N` (B2).
With This Mac alone there is no switch, since there is nothing to choose.
Choosing a device changes only what the Overview shows; the rail, the sidebar and the Workspace in front stay, and a project chip that was set clears.
The tab row shows Work, Projects, Backlog, History and Usage, with counts derived from the same scoped records each view shows.
A device that does not answer dims the live Work, Projects and Backlog tabs, and their body holds `<device>에서 응답이 없어요 · <언제부터>` and `다시 연결`; the age is the core's `unreachable_since_unix_ms`, and the diagnostic reason stays in the line's tooltip.
When this machine's Herdr server answers another protocol, Overview is not reached at all: the screen that holds Hide stands in place of the whole shell ([First run](#first-run), The screen that holds Hide).
A server on Hide's own protocol is read as healthy, so Overview adds nothing for it, and its move stays in Help > Set up Hide and Settings > About.
A move made outside the wizard whose operator's `herdr` is behind leaves the same line for that `herdr` above the tabs until its OK; with a `herdr` that keeps up, nothing stays.
History keeps searching the core's retained requests for that device and marks its disconnected state in the view.
Usage stays available because choosing a different device does not change the core machine that owns its ledger.
The project chip at the row's right filters the four device views; changing the device clears it.
Usage replaces the project choice with one global account scope line naming the actual core machine.

A backlog row or session record opens the Overview's detail surface, while a project row opens its project home.
Issue and History record details use the wide detail surface with Back; the underlying list remains mounted and inert, preserving its search, filters, page and scroll.
Other records keep the existing side panel, floating below the Overview panel breakpoint.
Escape closes the panel first and the Overview after it, background click and ⌘⇧O close the modal, and closing returns the keyboard to where it was before the Overview opened (B16).
Choosing another tab closes the panel.
The Overview replaces the search palette in one action and returns to the palette's original keyboard owner; Settings and other dialogs prevent a second Overview modal, and navigation from inside the Overview closes it as part of that action.
Native browser displays use the existing dialog still rules without reloading.
Every tab, the device switch, the project chip, a row, a fold line and every button of the panel take the keyboard with a visible focus ring and an accessible name, and a state is written as a word (`묻는 중`, `일하는 중`) as well as drawn in its colour (B64).
The Overview draws its first screen from the snapshot without waiting for any disk or GitHub read, and a closed Overview adds nothing to terminal output or key input (B63).

The Home with no project shows `Add a project to start an agent.` and Add project with its chord (⇧⌘N); a browser tab has no chord for it (Chrome keeps ⇧⌘N), so the button stands alone there.
Under it a quiet `Try it on a practice screen first` opens the [Practice window](#practice-window).
With projects and no agent, Work shows its empty state and the existing New agent route; the backlog remains available.
Before the first snapshot the shell's own connecting state shows instead.

### A device's tasks

The sidebar's 태스크 list draws a device's tasks; Overview Work reads the same canonical agents and explicit relationships.
A task is a root agent and every descendant it has on any device, with the checkouts they work in and every pull request the link record joined to one of their sessions; an issue or a memo the task started from rides it (B10).
An agent Hide AI started for a home (PRD hide-ai-work-home) is a task only while it calls the operator, the same rule the sidebar's Needs You follows.
The core derives the tasks from facts it already holds, stores none of them, and rebuilds them only when their inputs change, so a new agent, an ended one or a new pull request changes the task in place.
A fork is a root of its own and so a task of its own.
A task belongs to the device its root agent runs on.
A connected device's branches, worktrees and fetched refs follow within seconds of changing there, through that device's own Git watch, without a reconnect (PRD git-freshness B19); while the device is not connected its rows keep their last values dimmed, and a reconnect reads them afresh (B21).
The Overview reads the same tasks for the Projects tab's `최상위 태스크` counts and the order of Backlog's sections.

### Work

Web owners are `web/src/overviewGraphModel.ts`, `web/src/overviewMapLayout.ts` (the force map and its drags), `web/src/overviewGraphLayout.ts`, `web/src/overviewGraphLines.ts` (which lines are drawn, the team outline, a project's accent and glow), `web/src/OverviewGraph.tsx` (folding and what each folded session is drawn as) and `web/src/overviewGraphNodes.tsx` (PRD overview-main-map).
Work offers graph and list views over real live agents, grouped by project and explicit parentage.
Both views share All, My turn, Working and Unread results, with counts from canonical marks: `turn`, `going|waiting` and `done|unread`.
Include sleeping starts off and excludes only `asleep`; idle sessions marked `quiet`, Seen demands and sleeping raised asks still marked `turn`, and `unknown`, remain available.
The list orders My turn, Working, Unread results, unknown, idle and sleeping, retaining fifty-row pagination.
A filter dims the sessions it does not match instead of removing them, so a filter never moves a circle; an excluded ancestor of a present session stays as dimmed context, and dimmed sessions never increase match counts.
A filter with no match shows Show all sessions, distinct from no current sessions and layout failure.
A team folds into its root, and a project folds into one circle (PRD overview-main-map D-19).
A team whose every descendant sleeps opens folded; one the operator folded or unfolded keeps that choice.

What the operator arranges on the map is remembered by the core in `ui_state.overview_graph` (PRD overview-graph-remembered D-05, D-06): the filter and Include sleeping, the hidden projects, the folded projects, the teams folded or unfolded by hand, and where projects and sessions were dropped.
It comes back as it was left when Overview is closed and opened, when the tab or the project scope changes and comes back, and after the app restarts, and every window shows the same arrangement.
Each action is one event to the core: a filter, a hide, a fold, Fold all, Unfold all, one drop, Reset layout; a drag sends nothing until it is released (B16).
Places are kept for each map apart: a device's map of every project, and each project's own map (B4); they are kept divided by the interface scale, so a moved project or session keeps its place on the map when the scale changes (B13).
A record whose project or session is gone draws nothing, and a new session finds its place beside its parent or in its project as before (B12).
Each kind of record keeps at most 512, and past that the least recently changed is forgotten with a diagnostic; a stored record this build cannot read loads as nothing, the rest of the arrangement and of the UI state load, and the load leaves a diagnostic (B15).
Selection, the side column's search, the graph/list choice and the camera are not remembered.

The graph is a force map centred on the operator, the way Obsidian's graph view is drawn (PRD overview-main-map D-01, D-20): a `나` circle is fixed at the centre, and every session without a lineage parent (a root) is held loosely toward it by a faint line.
Each project gathers around its own place on an ellipse around the operator, so a project's sessions settle as one cluster apart from the others.
A delegated session is the root's circle drawn smaller: its agent's logo, its state on the ring and the corner mark, tied short to its parent by a faint line, and a grandchild sits outside its parent the same way (D-02).
Behind each project's cluster is a soft glow in the project's accent colour, and its name sits in bold, larger than a session's title, in that colour above the cluster; there is no band or box (D-17).
The accent is one of sky, violet, lime and amber, chosen by a stable hash of the project's id.
Pointing at a project's name or its row in the side column, focusing either from the keyboard, or choosing one of its sessions so its details open, makes that project's glow and name clearly stronger, and they return when the pointer leaves or the choice closes; the current pane's highlight at entry lights nothing, and reduced motion changes them without a transition (B21).
Placement is computed in the web shell and settles before the first paint, so nothing wobbles on open; every start position comes from the session's id, so the same sessions open in the same shape (D-13).
Status, labels, letters, filters, selection, hiding and folding never enter the layout key.
When a session appears, every placed session holds still while only the new one finds a place beside its parent or in its project; a session that leaves takes only itself away.
Admission keeps the map within the 400-node budget (below); a map the layout cannot place fails into the existing Retry and session list.
A root's circle shows its title and its sidebar second line (what it is doing, what it asks, what it waits for); a descendant shows only its title, plus what it asks in amber while it is the operator's turn.
Under every circle's title, the model and effort Hide proved it launched the session with sit in one small quiet line (`opus · high`), as the start chose them; a session Hide did not start, or one whose conversation has changed since, shows no line rather than a guess.
Under that, a session working in a linked worktree carries a small branch chip (the folder's name for a detached one), but only where its lineage parent works in another checkout, so a team inside one worktree names it once on its top session and a session in the main checkout has none; the project stays the repository, never the worktree.
State words and judged tasks are not drawn under circles; the accessible name reads title, state, provider and the launch choice.
The ring colour and shape of each circle encode its mark (working, waiting, my turn, unread result, quiet), with the StatusMark beside it and a dashed outline on the current pane's session.

Lines carry no words.
Parent-to-child lines are always drawn faint; choosing a session brightens its team's lines and draws one dashed outline around the team's circles and words, which goes away when the choice is cleared (D-03).
Letters and Herdr direct input between the same two sessions are one line with direction arrows: solid when any letter exists, short dots when only direct input does.
A communication line touching the chosen session, or the session under the pointer or keyboard focus, carries one small badge at its middle: an envelope with its letter count and a prompt with its direct-input count, each only when nonzero, summed over both directions; its hint says the counts per direction with both sessions' names, and every other line stays bare.
A pair in different teams is always drawn; a pair inside one team is drawn only while that team is chosen.
Every drawn root is joined to `나` by a faint line; a root whose mark is `turn` draws an amber arrow to `나` instead, and a descendant's ask reaches the operator through its root, which the core marks as the operator's turn.
A folded team that holds the operator's turn draws the amber ring on its root and the amber arrow to `나`; a folded project keeps its colour border and wears the turn's `!` badge at the top of its circle, with the same amber arrow (B26).
A reply wait is not drawn as a line, because the core does not name its recipient.
An unresolved endpoint is never joined by a similar name or pane number; only independently proven device, pane and native-session endpoints join current rows, and missing results, stale sessions and unproven recipients create no line.
Direct input never creates inbox items, watches, lineage, read marks, letter notifications or phone pushes, and the bounded 30-day observation metadata carries no command or prompt content.
Letters and direct inputs share one newest-2,000-record graph budget; no letter subject or body reaches the graph or its panel.

With nothing chosen, the side column lists each project with a search box, its session count, its nonzero My turn, In progress and Unread counts in their colours, and nothing else: no Fold or Hide words (PRD overview-graph-remembered D-02).
Pointing at a row or moving the keyboard into it shows an eye at its end, whose tooltip and accessible name are `<project> 그래프에서 숨기기`, and a touch screen, which has no pointer, always shows it; pressing it takes the project off the map.
Choosing a session of a hidden project, from Waiting on you or the session list, shows the project again and opens that session.
Choosing a project's name moves the camera to that project's sessions; a project outside the graph budget opens its project scope instead.
Hiding removes that project's sessions, name, glow, outlines and lines from the map without moving any other circle; its row stays, dimmed, keeps its counts, and shows a crossed-out eye without pointing, named `<project> 그래프에 다시 보이기` and pressed, which returns the sessions to the same places (B9).
A hidden row's name is not a button that moves the camera, because there is nothing on the map to move to; the eye of a project outside the graph budget is disabled (B11).
The search box finds projects and sessions: the project list keeps the projects whose name holds the words, and under it `세션` lists, first twenty by title, the sessions whose own title, checkout or branch holds them, each with its state, agent, project and a `그래프에서 숨김` mark for a hidden project; more than twenty says how many more, and nothing found says `일치하는 항목 없음`.
A project's name finds the project and not each of its sessions again, because its row already stands for them.
While the search has words and nothing is chosen, the map dims every session, project name and folded project it does not find, as a filter dims, and moves nothing; a session found by its project's name stays bright, and a folded team's root stays bright when it holds a found session.
Choosing a found session, or Enter in the search box, which takes the first one (else moves the camera to the first found project still on the map), shows it as a pane's `개요에서 보기` does: its hidden project shows again, its folded project and team unfold, its panel opens and the camera centres it; Escape returns to the column with the search as it was.
Hiding never affects the list view or the filter counts.
Hiding the project of the selected or highlighted session clears that selection.
Below the project list, Waiting on you lists each root whose turn it is, drawn on the map or not, with its project, title and question, and Go answer focuses the asking pane: the root when it asks, else its first asking descendant.
A root in a hidden project stays in the list with a `그래프에서 숨김` mark, because hiding a project from the map does not make its question wait less (D-21).
Folding a team leaves its root circle with a chip that counts its descendants and shows their My turn, Working and Unread marks; choosing the chip unfolds it, and the descendants come back to their places (B24).
Folding a project, from the faint fold button beside its name in the map (clear while the name is pointed at), turns its cluster into one circle in the project's place, filled faintly and bordered in the project's colour, keeping the project's glow and its emphasis, with its name at the expanded name's size and colour, its session count and nonzero counts; a line between that project and another ends on the circle, and choosing the circle unfolds the cluster to its old shape; both places share one fold state, and the button never starts a project drag (B25).
Fold all folds every project into its circle and leaves each team's own fold as it was; Unfold all opens every project and every team; both sit in the toolbar of the graph view (B26).
A project folds and unfolds only on the map: its name's fold button, its folded circle, and Fold all or Unfold all; the side column does not fold (PRD overview-graph-remembered D-03).
Choosing a session by click, Enter or Space replaces the column with its panel: title, state, project and branch, provider and proved launch model and effort, what it does now, what it asks the operator when it asks, its team (every descendant with its state; choosing one selects it), its counterparts with sent and received counts only, and `세션 열기 · 기록 보기`.
Below the Overview breakpoint the panel overlays the graph; Escape closes the panel first, and changing filter or scope clears a selection that no longer matches.
Hover and keyboard focus outline the pointed session and its counterparts and never open details.

Project names share the 400-node budget with sessions; admission prioritises the selected session's project and the protected current session, and the remaining room is shared in identity order.
An oversized project retains stable-ID representatives and a reserved access slot, where a chosen hidden member appears without moving another member; the paged list searches every matching session, and selecting one outside the graph budget opens its project scope with that session selected.
One-project scope offers All projects to return.
When the confirmed focused device, pane and native session have a graph proof at entry, that exact session is highlighted without opening details; pane reuse cannot inherit the initial selection.
Every other opening only highlights; a pane's `개요에서 보기` chooses the pane's session as a click on it would: its panel opens, its team's outline is drawn, and the camera centres it at a readable zoom; Escape closes the panel back to the project list and Waiting on you, and the next Escape closes the Overview and gives the keyboard back to that pane (PRD pane-open-in-overview B5, B6, B12).
To show it, the map shows its hidden project again and unfolds its folded project and the folded team it is in, as one change each the core keeps; the filter stays, and the chosen session is drawn even when the filter or `잠든 세션 포함` would leave it out, until its choice closes (B7, B8).
A session outside the graph budget opens its project scope with the session chosen, and a session the map does not know yet opens Work with nothing chosen and a line in the diagnostic log only (B9, B10).
The first camera, and Fit graph, put the operator at the centre of the view at the zoom that shows the whole map; when that zoom is too small to read, the first camera instead centres the current session (or the operator) at a readable zoom.
Fit graph, Current pane and Reset layout remain explicit keyboard-accessible controls.
Two things drag, neither opening details (D-18).
A project's name, or its folded circle, moves every session of the project and nothing else, and the project stays where it is put.
A session's circle follows the hand and its team is pulled along by the physics; on release it stays where it was put and the rest settles around it.
With reduced motion, nothing is pulled while a circle is held, and the map settles at once on release.
Clicking a circle opens details, and the text below it remains a selection target without becoming a drag handle; outlines, glows and lines follow the moved circles.
A released project or session is remembered where it was put, and opens there again; a project's drop carries the dropped sessions it moved with it (B3).
A map that is open never jumps to a remembered place: what it already placed stays where it is.
Reset layout forgets this map's places, places the map again from the start and restores the initial camera without dispatching pane moves; the forgotten places are never drawn again, neither before the core's answer arrives nor after reopening, and the other maps' places, hiding, folds and filter stay (B5).
Selection, pan and zoom stay local to the mounted view, and closing the view stops the map's physics and ends its listeners.

### A record beside the tabs

At Home, where no Workspace is in front to draw the lane's 보기 slot, a session's record opens in the panel instead (PRD sidebar-task-lens B17, B40); another record replaces it (B16).
The record is the same one the 보기 slot draws, in the same order and with the same reading, failed, retry and disconnected states (B17, B21); [The session record](#the-session-record) owns its parts.
The panel's head is ✕ and `Workspace 열기 · 레인에서 보기` (B18).
`Workspace 열기` closes the Overview and brings the session's pane forward; `레인에서 보기` closes the Overview, opens the checkout and shows the same record in the lane's 보기 slot.
A task on another device reads `<device> 기기로 가서 열기`, which brings that device forward, and a record of a task whose root runs on another device than the one the Overview shows carries `<device>에서 실행 중인 자식 에이전트는 <root device>의 이 태스크에 속해요` above it (B14).
A task whose checkout is gone has neither button and shows its record alone.
A record whose session ended reads `이 세션은 끝나서 기록이 없다` in the record's place, and the panel stays open (B21).
Closing the panel reads the 보기 slot's own record again when the slot holds one, so the two places never fight over the one record a window opens.

### 백로그

Backlog lists both Local issues and eligible GitHub issues for a project, with source-qualified identities so `L-N` and `#N` never collide.
The saved Issue source choice (Auto, GitHub or Local) remains the default for older entry points; it does not hide either source in Overview.
Local issues remain in `local-issues.json`, numbered per project; GitHub issues are read and written through the owning machine's `gh`.
An open Issue source menu follows the core's stored choice as it arrives, including an acknowledgement received after the menu reopened; selecting an item does not create a second, optimistic choice in the shell.
A project's source defaults to GitHub when its repository reads as a GitHub repository and to Local otherwise (a folder, a repository with no GitHub remote, or `gh` not installed); a project of a device the core dials has no source, while a project of a node that dials the core keeps its source, its issues and its Start as it had them before the core moved (PRD core-host-node-move D-29).
The core hands the web a source-neutral task (the id the source shows, URL, title, open or closed, labels, when it was made and last changed), so no view reads a GitHub shape, and a new source is one more adapter in `herdr-core/src/tasks.rs`.
A GitHub project can therefore keep Local issues beside its GitHub issues.

Above the projects, while the device has open missed work and Hide AI's summaries are on, one folded line `놓친 일 N ›` in the warning colour gathers it (PRD sidebar-task-lens D-52, B52).
It opens in place to every open missed item of the device's sessions, live or ended, newest first: the missed sentence with `백로그로 · 무시`, and under it the session's title and the item's kind, which open that session's record in the panel; the questions an agent asked the operator are not missed work and stay in that session's `나를 기다리는 것` (PRD session-record-goal-flow D-12, D-13).
With no open missed item, or with summaries off, the line is not drawn.

백로그 has one section per project, the projects with a task that needs the operator first, then the most recently active (B25).
A section identifies its project, source-qualified items and open or started counts.
Status, source, author, assignee, labels and literal title search filter before the initial fifty visible rows; Load more increases the visible bound.
The add control opens the shared quick add, whose explicit destination and repository disclosure apply here too.
Enter submits the title through that quick add: a memo stands as an `L-N` row, and an explicit GitHub destination shows a dimmed `만드는 중` row that becomes the `#N` row once the issue exists (B29).
The input locks during creation, then shows its success line and closes as described below; an empty or blank Enter does nothing, and a title stops at 256 characters.
A GitHub issue made here, by the quick add or by `백로그로` carries the title alone: no request, answer or body leaves the device.
A failed GitHub create keeps its row with one line of why (`gh가 설치되어 있지 않음`, `GitHub에 로그인 안 됨`, `GitHub 원격 없음`, `네트워크 또는 사용량 제한`), `다시 시도` and `지우기` (B30).
A failure no retry can change (an add the core refused, or a repository that moved) offers `지우기` alone; the core keeps at most eight such rows, the oldest leaving first.
`다시 시도` names the same add, so an issue the first try made is joined rather than made again, even after a restart; past eight waiting creates a new add fails at once with `만들기를 기다리는 항목이 너무 많음`.
A GitHub create is made only in the repository the project was connected to when it was asked; when Connections changes meanwhile, the create stops before the issue exists, fails with `프로젝트 저장소가 바뀜`, and its `다시 시도` makes nothing in the new repository.

Open unstarted items are the initial view; status filters can include started and closed items without changing their identity or Start eligibility.
A GitHub row is `#N`, the title, a flag when it came from missed work, its labels, its age, `↗` (GitHub) and `▷ 시작`; a memo row is `L-N`, the title, the flag, its age and `▷ 시작` (B27).
Memo flags belong to the memo's device and path/number; GitHub flags remain shared by repository/number.
When one source cannot be read, its last known rows and failure remain beside the other source's usable rows; an unreadable source is never an empty successful result.
On a device the core dials every section is the one line `이 기기에서는 <project> 프로젝트의 이슈를 읽지 않아요`, with no input and no `▷ 시작` (B36).

A row opens the item in wide detail: its title, `#N` or `L-N`, its source, when it was made, its labels, `나온 세션` for an item from missed work (the session's title, the goal flow it came from and the missed sentence, which open that session's record; PRD session-record-goal-flow B31), `▷ 시작`, `GitHub에서 열기` for a GitHub issue, and `되돌리기` while it is not started and Hide made it: every memo, and a GitHub issue made by the quick add, the backlog's input or `백로그로`; an issue someone else made never offers it (B31).
The wide detail renders the real body as sanitized Markdown with actual metadata and comments, retaining loading, stale and failed states.
Comments load fifty at a time up to two hundred, with each comment bounded to four thousand characters; truncation remains visible.
`되돌리기` deletes a memo or closes a GitHub issue and the row leaves the backlog; a missed line it came from opens again with `백로그로 · 무시`, and a close that fails leaves why and `다시 시도` in the panel with the row in place (B32).
Undoing a memo on one device preserves another device's memo, origin and missed line even when their path and number match.
The record that names a Hide-made GitHub issue keeps the newest 256 typed items, so an older issue no longer offers `되돌리기`; a refusal from the core (the issue is not one Hide made, or not the project's repository), which sent nothing anywhere, reads `되돌리지 못함` with no `다시 시도`, while Hide's record that could not be read just now (`unavailable`) says so with `다시 시도` (`undoRetryable` in `web/src/overviewBacklog.ts`).

`▷ 시작` opens the Start dialog, which turns an item into work in one step: a worktree named for the item, its base, the agent, and the agent's first prompt.
The name opens as the issue number and the title's English words (`192-hided-sigterm-handler`, `L-3-...`, or `issue-192` when the title has none), and when Settings > Hide AI > Features allows it the background AI's name replaces it once it answers, unless the operator has typed; `↺ AI 이름으로 되돌리기` brings the AI's name back.
A name that is already a branch says so, and when a worktree has it the primary button opens that worktree instead.
The first prompt is filled from the item's body, which the dialog reads when it opens, names the item, and for a GitHub issue asks for a pull request that closes it when Settings > Agents > Starting work says so; for an item that came from missed work it also carries the original request and the answer of that turn, which stay on this device and are never written to GitHub (B33).
The detail, origin, Start prompt and naming body use the selected workspace and device as well as the item key; a same-path memo or a late read from another device supplies none of their words.
Closing a panel or Start dialog releases only its owner's origin, including when the connection closes.
The operator edits the prompt before starting.
Starting creates the worktree, writes the link into it, starts the agent with the prompt as its first instruction, then brings the new pane's Workspace to the front; a folder project has no worktree, so its agent starts in the folder with the prompt.

### Adding to the backlog from anywhere

`백로그에 추가` (⇧⌘M; Alt+Shift+M on Windows and Linux; ⌥⇧M in a browser tab, where Chrome keeps ⇧⌘M) opens one input at the top center of any screen, even while a terminal holds the keyboard, and takes the keyboard; ⌘K or the find bar closes first (B37).
A second press closes it.
Above the input stand the project it will go to and why it was chosen: `고른 태스크의 프로젝트` (the project of the record, item or project the Overview's panel shows), `걸러 보는 프로젝트` (the Overview's project chip), `앞에 있는 체크아웃`, or `백로그가 있는 첫 프로젝트` when none of them keeps a backlog (B38).
Tab from the title walks the projects of that device that keep a backlog, resetting the destination to Local; Shift+Tab moves focus back to the explicit destination controls.
A project of a device the core dials has none and is never offered, and with no project to offer the input reads `백로그를 둘 프로젝트가 없음` and Enter does nothing.
Quick add defaults to Local, independently of the project's older Issue source preference.
GitHub requires an explicit destination choice, with the canonical repository and public, private or unknown visibility shown before Enter.
Only the typed title is sent to GitHub; neither the session request nor its answer or body is copied there.
Enter makes the item; the input locks as `만드는 중` until it exists, and a failure keeps the typed title with one line of why and `Enter로 다시 시도`, which names the same add, so the issue is made once (B41).
A failure no retry can change hands the input back with its reason: the title can be edited, Tab chooses another project, and Enter makes a new add.
Once made, `<project>에 이슈 #N 만듦 · 보기` or `<project> 백로그에 메모 L-N 저장 · 보기` takes the input's place for about 2.4 seconds and the input closes; `보기` opens the item in the Overview's 백로그 (B40).
Escape or a click outside closes it without making anything, and closing returns the keyboard to where it was, a terminal included, which loses no input meanwhile (B40, B42).
Pasted text with line breaks joins into one line and stops at 256 characters (B42).

### Projects

Projects is one row per project of the device (B48).
Its top line is `<device> 디스크 여유 X / 전체 Y · 프로젝트 합계 Z · 정리하면 W 확보`, the free space in the warning colour below 10 GB (`LOW_FREE_BYTES`, the cleanup sheet's own threshold), and a small `계산하는 중` while the sweep runs, the sizes in mono and the words in the body font; a device the core dials has no top line (B47).
A row reads the project's name with its GitHub mark (a Git project whose issues are not on GitHub has the folder-with-branch mark) or `폴더`, `최상위 태스크` (a colour bar and the counts that are not zero among `내 차례 N · 작업 중 N · 대기 N · 읽을 것 N`, `쉬는 중 N` when all four are zero, `태스크 없음` with no task), the backlog's `N ›`, the disk, `처리할 일`, the last activity and `열기 ↗` (B48).
The backlog's `N ›` opens 백로그 on that project, and `열기 ↗` closes the Overview and opens the project's most recent checkout.
`처리할 일` is `머지됨 N → 정리` while the project holds merged checkouts and empty otherwise; pressing it opens the disk cleanup sheet on that project's finished checkouts (B49); a project of a device the core dials says `머지됨 N` with nothing to press, since its cleanup is that device's.
Projects that rest with nothing to clean fold at the bottom into `조용한 프로젝트 N · X GB ›`, which opens in place (B50).
The disk cell is a bar and a number: the bar's length is measured against the device's largest project, its purple share is what a cleanup frees (the build cache and dependencies of finished checkouts), its tooltip lists build cache, dependencies, Git, worktrees and the rest with `정리하면 N GB`, and the number opens the disk cleanup sheet on that project (B51).
A folder project has its total alone, with the tooltip `폴더 합계`, and no sheet.
Pressing the `디스크` head orders the rows by size, the quiet ones unfolded among them, and pressing it again restores the order (B52).

While the tab is on screen the core measures the device's projects one at a time in the order the tab shows them; a measured cell keeps its last value dimmed with a small spinner beside it, and a cell never measured since the app started reads `-` until it fills (B53).
Leaving the tab starts nothing more, and a measurement stays fresh for ten minutes, so returning within them shows it at once and returning later dims it and measures again; choosing another device measures that device's projects (B54).
A project whose measurement fails or runs out of time keeps its last value (or `-`) with ⚠ and one line of why in its tooltip, and the next project's measurement goes on (B55).
A project of a device the core dials is not measured: its cell is empty and nothing spins (B56).

A row opens the project home (`web/src/OverviewProjectHome.tsx`) within Projects.
It retains the project's path, last activity, Open, New agent, actual activity, source preference, storage, cleanup and registration settings.
Beside those controls it lists the project's actual sessions and checkouts, and links to Work, Backlog and History with the same canonical project scope.
Back returns to all projects; opening a checkout closes Overview and uses the existing device-qualified Workspace route.

### History

Web owners are `web/src/OverviewHistory.tsx`, `web/src/overviewHistoryState.ts` and `web/src/RequestHighlight.tsx`; the source is the existing session-work ledger.
History initially searches original retained operator requests, including requests saved while summaries were Off or failed.
Generated titles and goal flow names are an explicit additional search scope, and a flow name match opens that session's record (PRD session-record-goal-flow B32); no raw copied-transcript index is restored.
Project scope means the immutable original project recorded for the session, never its current checkout, branch or guessed title.
Device, original project, agent, status and time filters apply before a fifty-result page is selected.
The query is literal text of at most two hundred characters; each session shows at most two matching snippets and names further matches.
The UI keeps the current page and a bounded cursor history, with loading, empty, failed, disconnected and retained-offline states kept distinct.
Typing uses one cancellable debounce and stale query or connection replies cannot replace the current results.
Selecting a saved request opens the record's Requests, unfolds it, centers its exact saved offset and highlights the literal match.
A request that retention removed says it is unavailable; the record never substitutes another request with similar text.
Back restores the list's query, filters, page and scroll, and returns keyboard focus to the selected row.

### Ways in, and the views it replaced

The breadcrumb's project name, a cross-project chip and a Recent Panels project open the Overview's Projects on that project's device with the project's panel beside it (PRD sidebar-task-lens B41).
An issue or a memo from ⌘K, a session record or a missed line's `· 백로그 #N` opens the item in 백로그 while it is not started, and once it is shows its task's row in the sidebar's 태스크 list with the root's record in the lane's 보기 slot, or in the Overview's panel at Home (PRD sidebar-task-lens B40); a pull request opens its card in the 보기 slot.
A pane head's agent tree button shows that agent's row in the sidebar's 태스크 list, unfolded and selected, with its record in the 보기 slot, and no Overview opens (B40).
A link panel's `대화 보기` and parent line open that session's record, in the 보기 slot when a Workspace is in front and otherwise in the Overview's panel (B58).
`⇧⌘A` and the `Agents overview` command are gone, and no menu or shortcut list names them (B59).
The Overview starts on Work when the window loads and keeps the tab chosen while that window runs; obsolete Home places resolve to a current view rather than an empty screen.
The old copied Conversation history and its index remain removed; retained-request History follows the session-work retention choice described in [Project sessions and Memory](ARCHITECTURE.md#project-sessions-and-memory).

## Tasks, the 보기 slot and the PR list

Web owner: `web/src/TaskLens.tsx` and `web/src/taskList.ts` (the sidebar's 태스크 list), `web/src/DetailPanel.tsx` (the lane's 보기 slot), `web/src/SessionRecord.tsx` and `web/src/sessionWork.ts` (the session record), `web/src/SidebarPrs.tsx`, `web/src/PrList.tsx`, `web/src/PrCard.tsx`, `web/src/prWork.ts`, `web/src/prCardModel.ts` and `web/src/prReason.tsx` (the sidebar's PR list and the PR card), `web/src/PrDialogs.tsx` (이슈 연결 and 맡기기), `web/src/prDelegate.ts` (맡기기's first prompt), `web/src/LinkSessions.tsx` and `web/src/linkPanel.ts` (the link record's sections); a row's agent line follows `web/src/agentRow.ts`.
The core owns the lens, the view option and the slot (`herdr-core/src/runtime/detail.rs`, `ui_state.sidebar_lens`, `task_view` and `detail`) and every task's state and order (`herdr-core/src/agent_state/tally/tasks.rs`).

### The sidebar's 태스크 list

A Conductor's task row, live or archived, carries the mission chip before its time: `미션 n/m` while the mission runs, `계획 중`, `머지 대기` or `완료` otherwise; pressing it opens the map in the 보기 slot (PRD software-factory-conductor B14, B46).
A Conductor's record in the Overview carries the same chip, so a mission whose Conductor has closed is still found there.
No other attention state is added: the Conductor row reads as any agent's does ([status-model.md](status-model.md)).


A session title restored from saved conversation history carries the core's connection state on the shared Project, Task and agent-tree rows, the Overview graph/list/preview and project home, live-agent popovers, archive confirmations, feedback recipient menus and PR list heads.
While checking, and after an unsuccessful or uncertain check, the title is only dimmed with no words beside it: every restored row checks at launch and the worker retries without a new message or a retry button, so there is nothing for the operator to do.
The hint and the accessible name carry `연결 확인 중` or `연결 미확인`.
A confirmed binding drops the dimming and those words.
The same state names and dims its PR mark without changing GitHub read-stale status, keyboard access or the existing PR card/link actions.
Accessible row names include the connection state and saved summary.
Saved questions, approvals and work state do not become the current row's status through this display restoration.
A new hook reference removes the old title and PR display before its own read completes.


Under the sidebar's head (search ⌘K, Overview ⇧⌘O), right above the list, a `프로젝트 | 태스크 | PR` switch picks the list (PRD sidebar-task-lens D-10, B1, B39; lane-two-columns B41): 프로젝트 is the projects tree as before, with its raised Needs You and Done, 태스크 the device's tasks, and PR the device's pull requests (see The PR list and the PR card).
⌘B shows and hides the sidebar, and ⇧⌘B (`Switch sidebar list`; Alt+Shift+B on Windows and Linux; in a browser tab ⌥⇧B on macOS, where Chrome keeps ⇧⌘B, and Ctrl+Alt+Shift+B on Windows and Linux, where it keeps Alt+Shift+B) turns the switch to the next list, 프로젝트, 태스크, PR and round; with the sidebar hidden it shows the sidebar on that list (B2).
A window starts on 프로젝트, and then the last list and view option chosen stay, through a restart too (B3).

The list follows the rail's front device, every project of it at once, and turns with the rail (B19).
Its first line reads the project chip, then `태스크 N`, the open tasks, with a `상태별 | 프로젝트별` option at its right (B4, B6).
The project chip is one choice the 태스크 and PR lists share, every window and the phone with them, and the core keeps through a restart (`ui_state.sidebar_project`), forgetting it when the project is removed: `모든 프로젝트 ▾` shows every project of the device, and a chosen project shows its name and `✕`, which shows every project again.
Its menu lists the device's projects, each with its `내 차례 N` in the warning colour and its open tasks and pull requests counted together, as the two lists' groups count them (a team review ask and an old pull request in 내 차례 included), a zero not drawn; `지금 보는 프로젝트` first chooses the project of the Workspace in front, and a 현황 tile's 세션 or PR sets the same choice.
While a project is chosen, its rows leave out the project's name, and the 내 차례 of the device's other projects stands after the chip as `+N` in the warning colour, which shows every project again when pressed; ⇧⌘R opens the PR list on every project.
상태별 groups the tasks as `내 차례`, `읽을 것`, `다시 볼 것`, `작업 중` and `대기` (`Needs You`, `Unread`, `Revisit`, `Working`, `Waiting`), in that order, each head with its count, 내 차례 in the warning colour, empty groups not drawn, and every row naming its project before its second line (status-marks D-22, B32; PRD revisit-group D-01).
내 차례 and 읽을 것 stand still above the other groups and the folds, as the projects tree's raised Needs You and Done do, so reading one row after another never scrolls them away; they take up to half the list's height and scroll on their own past it, the arrow keys cross both parts as one list, and with nothing under them they are the whole list; 프로젝트별 keeps one scrolling list.
다시 볼 것 holds only what the operator put off with ★, which wears a ★ in its mark's slot and, on an ask, keeps its `질문 · …` line (PRD revisit-group B6; needs-you-cleanup B13, B16); a starred task that asks anew, finishes or works stands in that group with a small ★ after its title, and nothing in 다시 볼 것 sends a notification, a push or a badge count (B3, B4, B7, B18).
내 차례 holds the questions, approvals and blocks, read or not; a block is a task whose root agent errored (the core's `demand == "error"`), not a failed CI, and its line reads `막힘 · <cause>` in the warning colour.
A question or approval the operator opened and has not answered stays in 내 차례 drawn read: a hollow `!` in its mark's slot, its title and line dimmed, its question line unchanged and a small `읽음` tag in the time's place; it counts in the head, the project's band and the project chip, but not in the app icon badge, a notification or the pet, and it is never announced again while it stays or moves to 다시 볼 것 and back (needs-you-cleanup B10-B12).
A Conductor whose mission waits on a decision, and that neither works nor asks anything of its own, stands in 내 차례 with the line `판단 · <decision>`, and a root under which a delegated descendant was raised stands there with the raised ask on its line; when several things wait on the operator under one root, the line shows the one waiting longest and ends with `외 N건` (needs-you-cleanup B1, B7, B9).
대기 holds the tasks in progress that wait on proven work (the core's task state `waiting`), 작업 중 those whose agent works.
Tasks that rest, those resolved included, fold into `쉬는 중 N ›`, and tasks whose pull requests all merged while a worktree remains into `정리할 것 N ›`, at the end; each opens in place and closes with `접기` (B5).
Entering a resting task's pane, or a descendant's pane, opens its `쉬는 중` fold in either 상태별 or 프로젝트별 and brings the selected row into view; a focused task moving into rest opens that fold too.
A manual collapse stays closed through snapshot updates while focus remains there; entering another pane in that fold or returning to it reveals it again.
Archived sessions fold last, into `보관됨 N ›`: under 정리할 것 in each project of 프로젝트별, at the very bottom in 상태별, closed until opened, newest archived first, and a project whose sessions are all archived keeps its group (PRD session-archive B14, B15).
프로젝트별 draws one group per project, the projects with a 내 차례 first, its head the project's name with its counts by state in the same order and names (`내 차례 1 · 작업 중 2`), the same order inside and its own folds, and no project name on the rows.
Where a project's rows change state, a small grey label names the band (`내 차례` in the warning colour, `읽을 것`, `다시 볼 것`, `작업 중`, `대기`); it is skipped by the arrow keys and hidden from assistive technology, and each band's list carries the state's name instead; 상태별 draws no band labels, since its heads already name the state.
Inside a group 내 차례, read asks among the rest, and 다시 볼 것 stand longest-waiting first and the others stand most recent first (B9; PRD revisit-group B13; needs-you-cleanup B10).
Inside `쉬는 중`, both views order rows by their latest actual native prompt, newest first, so the order matches the age on each row; unknown prompt times stand last and equal times retain their existing order.

A row is one task, its root agent; descendants are its children only, and a memory question, a pull request with no session, a delegated child are never rows of their own (B7).
Keep the sidebar and task list (the former Sessions list) easy to scan: the root task and its parent-child hierarchy lead, PRs stay summarized in their mark, and check details belong in the PR card rather than additional list rows.
When refining these surfaces, simplify the amount of visible detail and the relationship between PRs and delegated work without hiding the operator's next action.
Its first line is the row's own status mark (the sidebar's, status-marks D-12), the title, no count of its children or of a raised descendant, folded or not, since the raised ask is the root's own line (needs-you-cleanup B7), the PR mark, the mission chip and the elapsed time (B8, B10).
In both 프로젝트 and 태스크, a session's elapsed time measures its latest actual native prompt, including a real request letter from another agent, rather than its latest state change.
State changes and a daemon restart preserve that reference; a new session never inherits it, and an unknown prompt time leaves the slot empty until the native read succeeds.
The existing localized units and shared clock also apply to child rows; open-task ordering, unread state, waiting and escalation still use their own existing state times.
Its second line is the sidebar's line in its parts: on 내 차례 the verb and the question (`질문 · …`) or the block's cause (`막힘 · …`) in the warning colour, a waiting ring's cause word (`자식 기다림`, `CI 기다림`, `답장 기다림`, `백그라운드 기다림`, or `대기` with none proven) and its sentence or detail, a stop's word (`끝남 · `, `일 남음 · `) and the background tail, work in progress its sentence alone, a waking, failed or unknown row its state, a sleeping row none (status-marks B10, B27, B29, B31, B33); on 쉬는 중 only unfinished work has a line, and a read one draws the quiet ring; with nothing to say there is no second line, and no stage bar is drawn.
An unknown row is dimmed with no mark (status-marks B16); a row Herdr is still starting is drawn the same way and says `시작 중` instead of `상태 모름` wherever the status word stands (its line, the pane header, the Stop-work confirmation).
A row with children opens them in place under its chevron, five per parent and the rest behind `N개 더`, indented to the third level; a child in another checkout carries a branch line, a child on another device a device chip, a finished child a quiet title, and pressing a child goes to its pane, bringing its device to the front (B10, B11).
Folds are kept apart from the 프로젝트 tree's (`sessions_expanded_agent_pane_ids`).

A press or Enter goes to the task's Workspace, tab and pane; the arrows, Home and End move through group heads, folds and rows, Right and Left open and fold a row, and Space opens its record (B12).
The row of the pane with the keyboard, or its nearest drawn ancestor while folded away, carries the sidebar's selected fill and `aria-current`; the list scrolls only as far as brings a newly selected row into view, and never while the operator's scroll position holds the selected row unchanged (B13).
While ⌥ is held (Alt on Windows and Linux) each task row carries its number at its top right, as the 프로젝트 tree's roots and the Agent tabs do (PRD electron-digit-shortcuts-hints): the rows are counted top to bottom as the list draws them, a group's rows then the rows of a fold the operator opened, a task's children never, nine at most, and ⌥1 to ⌥9 open that row's task the way a press does.
The `프로젝트 | 태스크` switch decides which list ⌥n counts, so on 태스크 the numbers are this list's and on 프로젝트 the tree's, whether or not the sidebar is shown.
Under the pointer or focus the small ★ after the title is not drawn and a ☆/★ button, the record button and `⋯` take the time's place in the first line, and the title gives way for them, so they never cover the PR mark or mission chip; at rest the row keeps no room for them beyond its time (an input with no hover always draws them); `⋯` offers `기록 보기` (Space), `터미널로 이동` (↩), on a task `다시 볼 것으로 표시` or, on a starred one, `다시 볼 것에서 빼기`, on a read ask in 내 차례 also `치우기`, which puts that ask away until the agent asks something new (needs-you-cleanup B15), then Resolve (not while the agent works or waits, or an approval prompt holds it) and, on a task, `세션 보관`, offered and disabled as the pane menu's Archive session is, and last `세션 닫기`, which closes the row's pane on whichever host holds it through the same confirmation as the pane menu's close; a right-click on the row opens the same menu (B15, B18; PRD session-archive B1, D-27; PRD revisit-group B8, B9).
☆ stars the task and ★ is `다시 볼 것에서 빼기`, which takes the ★ and puts the read ask away so the task rests until something new; on an ask in 내 차례, ☆ reads it and moves it to 다시 볼 것, and a new question from that agent brings it back to 내 차례 unread and announced (needs-you-cleanup B13, B14); the button is ★ only on a task the operator starred; a root agent's pane menu offers the same item before Archive session, the 프로젝트 tree, Overview's list and its detail head draw a starred task's small ★, an archived row keeps its ★, the phone lists the same group between 읽을 것 and 작업 중 and a read ask in 내 차례 with the hollow `!`, and a ★ or 다시 볼 것에서 빼기 that could not be saved, or a ★ past 256, leaves the row as it was and says so once (PRD revisit-group B10, B11, B15, B19, B20, B22, B23).
`세션 보관` closes the task's root and every descendant pane with it and keeps their conversations: a session that is not working, or only waits on the operator, goes at once, and one with work running first asks `작업을 멈추고 보관할까요?`, listing the agents working now, with `작업 멈추고 보관` and `취소`, neither holding the keyboard (B5-B7).
The row leaves the open groups for 보관됨 once every pane closed; a save that fails closes nothing, and a close some agents refuse keeps the session as it was, with one notice, the members that did close listed in its record, and `세션 보관` again finishing the same archive; the notice says whether a close was refused, timed out or lost its device, and once that root closes some other way its archived members stand in 보관됨 as their own row (B8-B12, B24).
An archived row reads its title, its project in 상태별, its PR marks and how long ago it was archived, with no status mark, since nothing runs; `보관하는 중…`, `이어가는 중…`, `이어가지 못함`, `지우는 중…` or `상태 모름` stand on its second line while they hold, and a device that is not connected dims it (B14-B18, B13).
Pressing it opens its record; `⋯` and a right-click offer `기록 보기`, `이어가기`, `상태 확인` while Hide cannot say whether its close finished, and `보관함에서 지우기…`, which asks `<name> 세션을 보관함에서 지울까요?` with `보관함에서 지우기` and `남겨 두기`, neither holding the keyboard (B19-B21, B28, B34).
`이어가기` brings the root back in a new pane, which takes the screen as any start does, and its descendants after it, one at a time, behind it; a member that does not come back keeps its reason and its own `이어가기` in the record, and the row leaves 보관됨 once every member is back (B21-B24).
Deleting removes only Hide's record of the session; the agent's own conversation file stays, so the agent's own resume list still shows it (B28-B30).
Resolve reads the row at once and moves it to 쉬는 중 only after the core saved it; a failed save leaves the row with the current error, and input, a letter or new work brings it back.
A merged pull request rests a task on its own only once its finish was read and nothing is left to wait on; until then it stays under 읽을 것 or 대기.
The PR mark draws the session's list (docs/status-model.md, One list per session), on a root row and on each child row alike, so a pull request an ended child made stays on its parent's row and a grandchild's stays on the child row between them, as their pane headers and records show it; it shows one pull request as its state's icon and `#number`, several as each state's icon and count, worst first, a closed one's count and row dimmed; one opens its card in the 보기 slot on a press, several list each first, and ⌘-press opens GitHub (B14); under the pointer or focus it shows what the 프로젝트 row's mark shows, the PR card with the title, review and checks for one PR and each PR's state and title for several.
A stale GitHub read dims the mark, with `GitHub 읽기 실패 · N분 전 값` in its tooltip, and never draws a zero (B20).
While the first snapshot is read the list shows a small skeleton and no number; with no task it reads `진행 중인 태스크 없음` and `새 에이전트`, which opens the start panel; a device with no project reads `프로젝트를 추가하면 에이전트를 시작할 수 있어요.` with `프로젝트 추가` (⇧⌘N; no chord in a browser tab); and a device that does not answer keeps its last rows dimmed under `<device> 응답 없음`, their actions off (B20, B42).
Every control is reachable from the keyboard with a visible ring and an accessible name, and a state reads as a word as well as a colour (`묻는 중`, `일하는 중`; B21).
A hidden list draws no rows, the elapsed clock updates only its own text, and the list adds no work to a key or a terminal frame (B22).

### The 보기 slot

The lane has one 보기 slot per window, which holds a pull request's card, a session's record or a mission's map and belongs to no checkout and no Agent tab, so changing either leaves it as it was (PRD sidebar-task-lens D-50, B48).
Whatever opens a card or a record opens it there, in front of the documents in the lane's viewing column, and the Workspace in front, its Agent tab, the tool column and the keyboard stay (B14, B15, B50; PRD lane-two-columns B9); no `PR #N` or record tab appears among the documents.
Its head reads `←`, the item's mark and name (`PR #N · <project>`, `기록 · <title>` or `미션 · <title>`), `↗` (GitHub) on a card, a gear on a map that opens Settings › Conductor, and `✕` (B47); a card's mark is its PR mark, in its state's icon and colour, here and on the rail.
The head keeps the lane's surface and what the slot holds sits on the page's background, a card and a record alike, as the 보기 screens draw it.
Opening another item puts it in front, and `←` steps back through the ones opened before, at most 20, the oldest leaving first; `✕` empties the slot and the viewing column goes back to what stood under it: the documents when they showed before the slot came forward, else the column closes, the lane closing with it when there is no tool column (PRD lane-two-columns B13; operator decision 2026-10-24).
A slot that opens a closed lane, whatever the lane held when ⌘E or Escape closed it, stands alone, so its `✕` closes the lane again.
The close chord (⌘W) is the slot's `✕` while the keyboard is in the slot or nowhere, as the close chord's rule says ([The right lane](#the-right-lane)); in the tool column it closes nothing.
While the slot holds something, the rail carries a 보기 button at its top with its mark, pressed while the viewing column draws the slot; the documents brought in front, or the button pressed while the slot is in front, fold it and the button stays, so pressing it comes back to the slot (B46; PRD lane-two-columns B11).
A changed file pressed on a card opens its diff among the checkout's documents, which come in front of the slot, the slot staying on the rail (B49; lane-two-columns B10).
At Home, with no Workspace in front, a record opens in the Overview's panel beside the tabs and a PR mark opens the pull request's Workspace with its card in the slot (B17).
A pane head's agent-tree button and a started backlog item (from ⌘K, a record or a missed line's `· 백로그 #N`) show the sidebar on 태스크 with the row unfolded and selected, and open the agent's record in the slot, or in the Overview's panel at Home (B40).

### A mission's map

A Conductor's mission (PRD software-factory-conductor; PRD mission-plan-visibility; PRD mission-map-git-criteria) is drawn in the 보기 slot as one page, read from the core's mission record; the page edits nothing, and the operator changes a mission by talking to its Conductor.
Only the operator opens it: the Conductor pane's mission chip, the sidebar row's mission chip, the rail's slot button or the Overview (B14; PRD mission-pane-chip D-02); no plan opens it by itself.
The page reads top down, with no tabs, flow table, time bars or legend: the head, `내 차례` only when the operator has something to do, the map, and the node panel (mission-map-git-criteria B24).
The panel opens only with a picked node, and until then the map has the slot's whole width; it stands right of the map while the slot is at least `--size-mission-wide` (900 px) wide and drops under it in a narrower slot (B30, B31, B34).
The page reads the mission's events only while it is open: the core sends the events of the mission in the 보기 slot and of no other (D-22).

The head is one line: the mission's state (`계획 중`, a blue `진행 중`, `머지 대기` in the decision colour, a grey `✓ 끝남` or a grey `취소`), its time (`n시간째` while it runs, `n시간 걸림` once it ends), a cell per node in the plan drawn as its card is (finished work green whether or not it reached main, an outside node empty), and `n / m 끝`.
It ends with `검증 <강도>` (`검증 보통`), the mission's verification strength, read only: the Conductor changes it in its plan when the operator says so, and a mission from before strength shows none.
The head carries no PR count, no main count and no cap; the map carries those.
A finished or cancelled mission's report sits under the head as its first words without their Markdown marks (no `#`, `-` or `[ ]`), and `펼치기` opens all of it drawn as a Markdown document is (B24).
Every state colour is one token of `design/tokens.json` (`--mission-done`, `-working`, `-review`, `-hold`, `-failed`, `-waiting`, `-cancelled`), and a head cell, a card's mark and border, and an edge use the same one.
Before the first plan the page is the head and one `계획 중` line (B19).

`내 차례 n` lists only what the operator can do, each item with its node's chip, which picks that node on the map (B25): the Conductor's decision, a worker the core raised for the operator (`<node> 일꾼 막힘: <its blocked line, or the cause>`, with `터미널로 이동`), an outside pull request closed without merging, and, while the mission waits on the operator's merge, the merge or join order (`머지 순서 #1 → #2`, `합칠 순서` for direct changes) with `대화로 ›`.
With nothing to do the section is absent.
A worker blocked without the core raising it stays a line on its card.
A decision the Conductor waits on is answered where it is asked, in the decision colour's border (B25-B30): the question, the one line of why it matters when the Conductor gave one, a `<node> · 판단 대기` chip for the node held on it, how long it has waited (not counted while the mission's device is not responding, since that wait cannot be read), a button per choice with `추천` on the one the Conductor recommends, `직접 답하기…`, and `대화로 ›` at the right end, which brings the Conductor pane forward with the keyboard in its terminal.
A decision with no choices, as one from before choices, shows the question, `직접 답하기…` and `대화로 ›`.
`직접 답하기…` opens a field in the decision with the keyboard in it: Enter sends, Escape closes it and puts the keyboard back on `직접 답하기…`, what was written stays for the next open, and the field takes no more than 500 characters, showing how many are left (B27).
A pressed choice or a sent answer turns the decision at once into a green ✓ with the answer and `Conductor가 읽는 중`, nothing left to press twice, and the core types it into the Conductor's conversation as the operator's own input, headed by the question (`[판단 답] <question> → <answer>`, in the interface language); the same decision answered again goes in once, and an answer to a question the Conductor has since changed goes nowhere while the new question shows (B26).
While the Conductor works the answer waits for its turn as a letter does and the decision reads `보냄 · 차례 기다림` (B28); the Conductor clearing the decision takes it away, and a new question shows anew, the old question and its answer staying as events (B29).
While no Conductor holds the mission the choices and the field are off, `이어받기` stands where `대화로 ›` was, and an answer that had not reached the Conductor is dropped, the decision going back to before it was sent with `보내기 전 · Conductor 없음` and the events recording `답 전달 안 됨`; a Conductor that takes the mission over finds the decision still there to answer (B39).
The doorbell types only into the agents it rings (Claude Code, Codex and Grok), so a Conductor of any other agent gets its answer in the conversation: its decision keeps `대화로 ›` with the choices and the field off under `이 Conductor의 에이전트는 대화에서만 답을 받아요`.
While the mission's device does not answer, the choices and the field are off under `<device> 응답 없음 · 돌아오면 답할 수 있어요`, and an answer the core could not type goes back to before it was sent with `보내지 못함 · 다시 답할 수 있어요`; the reason goes to the log (B28).
An answer typed without Herdr confirming it may already be in the conversation, so it stays with an amber mark and `전달 확인 안 됨` and a `다시 보내기` button, which types it once more; nothing else answers the question meanwhile, and `대화로` shows whether it arrived (B26).
An open mission whose Conductor is gone with no decision shows `Conductor 없음` with `이어받기` in `내 차례`, which starts a new agent in the mission's checkout told to take the mission, and the core refuses a second Conductor while the first is there (B44).

The nodes are drawn top down by what they wait on: every node that waits on nothing starts on the top row, and the edges meet one `보고·인계` card at the bottom.
A card's state is a mark whose shape reads without its colour, and colour is kept for what wants a look (B26): a dashed empty ring and dashed border for 대기, a turning ring and a blue border for 작업 중 (still under a reduced-motion setting), a purple filled ring for 리뷰 중, a yellow `!` for 판단 기다림, a red `✕` for 실패, and a green disc with a `✓` on a faint green surface for 일 끝, whether the work reached main, waits on a merge or never goes to main (a review, a check, research); a finished card's border stays the plain one, never green, and what is left before main is only its `머지 남음` or `합치기 남음` tag.
A node outside the mission (`external`) is a dotted card with `↗` before its pull request or branch in every state, since being outside is its kind, not a state (B19); its state is drawn on top: a dotted ring and `main 반영 기다림` while it waits, `확인 못 함` when Hide could not read it, the finished green ✓ and `반영됨` once it reached main, and a red ✕, a red border and `머지 없이 닫힘` when its pull request closed unmerged; a finished one's accessible name adds `미션 밖`.
Hovering a mark says its state word, and the card's accessible name starts with the same word.
A cancelled mission's unfinished cards are dashed grey and rest, and no edge from them is lit (B35).
Every node is drawn, done ones too, so the map keeps one shape from the first plan to the last report.

A card carries its title on up to two lines, one line of now, its result, its time and its finish lines (B28).
Its bottom row starts with who works it, as a checker's row names a checker: the agent's mark and the model and effort its pane was proven launched with, else the plan's marked `(계획)`, then `머지 남음` or `합치기 남음`, its time and, at the right, `기준 n/m` (`[mark] opus·xhigh · 43초 … 기준 2/3`); a node with no worker, as an outside one, shows none, and a finished card keeps it in its muted text.
On a short row the model and effort truncate before anything else does while `(계획)` stays whole, so a planned model never reads as launched; once they cannot keep a readable width (`--size-mission-run-min`) beside it they leave the row and the mark stands alone, its hover and the card's accessible name keeping the words, as on a card from before finish lines whose old sentence takes the row.
The line of now is the worker's own plan step (`지금: <step> (n/m)`) or the Conductor's line while it works, what is left (`남음: 기준 2, CI`) when neither says, `판단 기다림` while held, the failure in red, and for a waiting node what it waits for, by number where it has one (`#1504 기다림`); finished work has no line, its result speaks.
A result is drawn by the node's kind and only when the record holds it (B22): a pull request is its PR mark, the state's icon in its colour and `#n` as the panel draws it, dimmed while Hide's GitHub read is stale, and a plain `#n` while Hide has not read its state; a direct change is `→ main <first seven digits>` once Hide found its branch's commit in main (`in_main`), or the Conductor recorded one by hand, else its branch, truncated within the card.
Done work not yet in main carries `머지 남음` (a pull request) or `합치기 남음` (a direct change) in the decision colour (B27), only from what the core read: `머지 남음` for a pull request it read open (a pull request whose state it never read, as in a mission recorded before it kept one, carries nothing), `합치기 남음` for a done direct change's branch, which the core reads while the mission runs or waits on its merges, until it finds the branch in main; a branch it could not read carries nothing; a stack shows only as a purple dashed edge, and an edge from a card at work is drawn in the working colour while the mission is open.
How long a node took and how long a decision held it come only from its events (`24분`, `판단 7분`), and a node with no event that moved it shows no time (B28).
`기준 n/m` counts the ✓ lines alone, with `⤳k` beside it for the skipped ones, though the core's `criteria_met` holds both; a mission from before finish lines shows its old one-sentence criterion instead and no count (B23).
A card a later node's defect reopened carries `↻n` in red, counting the reopenings, and a card the last replan changed carries `✎` with its reason on hover; the attempts are the panel's (B16, B36).
A node checked by other agents carries a strip under its card, one row per checker with its agent's mark, the model and effort its own pane was started with (only the launch choice Hide proved for the checker's `pane_id` and `machine`: a launch that set no effort shows none and one that set no model shows `기본 모델`; without a proven launch the plan's words with `(계획)`, never passed off as launched), and its state (`✓ 통과`, `✕ 실패`, `리뷰 중`, `리뷰 대기`), and a rule line when several decide (`3명 중 과반 통과 · 1/3 통과`); edges to later nodes leave from under the strip (B29).
A defect a later node found is a red dashed edge from that node back to the one it reopened, until the defect's line passes (B21).
The map is drawn in a box that opens at `맞춤`: the whole map inside it, never larger than its own size and never below 40 %, and it keeps fitting as a panel opens beside it or the slot resizes until the operator zooms it.
A trackpad pinch, or a wheel with ⌘ or Ctrl held (Ctrl on Windows and Linux), zooms around the pointer between 40 % and 200 %, so the card under it stays there while the map can scroll that far; a plain wheel scrolls the box.
Dragging from an empty spot moves the map under the pointer while it is larger than its box; a press that moves less than a few pixels is still a press.
`−`, the percent, `+` and `맞춤` sit at the box's lower right as the browser's zoom does: `−` and `+` step through Chrome's zoom levels and are off at 40 % and 200 %, `맞춤` fits it again and stays pressed while the map fits, and hovering the percent says the gestures.
The zoom is the page's view of the moment: nothing reaches the core, and a map opened again, or another mission's, opens at `맞춤`.
In a slot narrower than `--size-mission-wide` the box is as tall as its map, up to the slot's height, and the panel follows under it.

Pressing a card, or Enter or Space on it, picks it: that card and the cards and edges one edge away stay lit, the rest dim, and its panel opens (B30).
Pressing it again, an empty spot of the map (not a drag from it), or Escape lets it go; Escape with nothing picked is the lane's.
The panel reads top down (B31): its head (mark, title, state word · kind · the worker's model and effort as the strip reads a checker's, launched or marked `(계획)`, then how long it took, how long a decision held it, its attempt and how often a defect reopened it), now (a held node's question and choices, read only; a working node's own plan checklist; a node in review its checkers; else its card line), what it made (the pull request in its state's mark and colour with the base it stacks on, the CI line, the branch, the commit, a research conclusion, a verify verdict), its finish lines, its connections, `PR 카드` when it has a pull request, `터미널로 이동` and `기록`, and its events folded under `사건 n개`.
The finish lines are one list of ✓ ○ ⤳ ✕ (B32), each the one criterion mark the CI line also uses, whose tooltip and accessible name are the same state word (`통과`, `아직`, `넘어감`, `실패`): a node's own lines in its words, `검사 명령 통과`, `CI 통과`, and a review or verification by its checkers' names (`codex 리뷰`); a skipped line says its reason, a line passed by a rerun `재실행 1회`, one the operator decided names the decision, and a defect line names the node that found it.
Connections list what it waits on and what waits on it by node name with the number beside it, faint; pressing one picks that node (B33).
An outside node's panel has no buttons.
Titles and long branch names wrap or truncate inside the card and the panel and never overflow (B34).
When the device that runs the mission disconnects, the page keeps its last values dimmed under a note, and the head and running nodes show no time (B35, PRD mission-plan-visibility B38).
A build from before missions opens with an empty 보기 slot when the saved slot held a map; the rest of the saved screen loads as it was.

### A mission on its Conductor's pane

A Conductor's pane shows its mission on its head (PRD mission-pane-chip; PRD mission-plan-visibility), where a plan chip would be, so a mission's progress stays with the pane that runs it and follows the operator nowhere else.
The mission chip sits right after the title: the waypoints mark, the word for its state, a cell per node in the plan and `n/m` (B8).
It reads `계획 중` (dim) before the first plan and no cells, `미션` with the cells and `n/m` while the mission runs, the same with a `!` before it in the decision colour while a decision waits on an open mission (`! 미션 n/m`, needs-you-cleanup B6), a colour it keeps while its popover is open, `머지 대기`, a green check with `미션` and `m/m` once done, and a grey `취소` once cancelled.
A cell is a node in its state's colour, in the order the nodes wait on each other, and a dropped node has none; past 20 nodes each run of one state is drawn as one stretch as long as its nodes, so the chip keeps its width while `n/m` stays exact (B8).
The chip's accessible name says the mission, its state, its count and the first node at work (`미션 … · 미션 4/10 지금 <node>`), since the cells are not read out (B36).
The pane shows the mission chip instead of its session's own plan chip, whose plan stays in the session record (D-07); a pane that conducts nothing shows no mission chip, and while the core cannot read the mission record the pane shows none, as the sidebar row does (B10).
The pane's second row shows `! <decision>` in the decision colour while a decision waits, else `<first node> 외 k` while workers are at nodes, in the progress place, the count left to the children button's marks beside it; with no node at work the place is empty, and neither the Conductor's own plan nor its progress stands in (B9).
While that line names the nodes at work, the Conductor's `자식 기다림 · 자식 N개 작업 중` is not drawn beside it, since both say the same work.
A waiting decision also takes the place of the Conductor's own work left (`일 남음`), since a Conductor often stops after asking; its own question, approval or block, and a raised descendant's ask, still come first.
The pointer or the keyboard's focus on the chip opens its popover, as on the plan chip ([A plan's map](#a-plans-map)), and a press, Enter or Space on the chip opens the mission's map in the 보기 slot and closes the popover, so the map is one press away (B3).
The Show plan chord (⇧⌘U, Alt+Shift+U on Windows and Linux) pins the popover, and the chord again or Esc closes it, the keyboard staying where it was; Esc on a popover the pointer only shows goes to the terminal (B4).
The popover has the plan popover's width and no map, which is drawn only in the 보기 slot; its head ends in a dim `누르면 지도`, since the chip itself opens the page there, and there is no record footer, the pane ⋯ menu and the sidebar row opening the record (B7).
While the mission is open its head is `미션 n/m`, or the chip's state word; while a decision waits its first line is `! <decision>` with `대화로`, which brings the Conductor pane forward with the keyboard in its terminal; then a row per node at work (status mark, title, PR number and where it is: `코드`, `PR`, `리뷰`, `머지`, `검사`, `main`, `조사` or `검증`), and one line `끝남 k · 다음 j` (B10).
Once the mission ends its head is `✓ 끝남 · <걸린 시간>`, or a grey `취소 · <걸린 시간>`, then the first two lines of the Conductor's report as words, its Markdown marks (`#`, `-`, `[ ]`, `**`) left out, and at most three results by kind with `+N개` for the rest (B11).
Before the first plan the popover is one `계획 중` line, and when the mission's device does not answer it keeps its last values dimmed under the big map's note (B10).
When a mission's first plan lands while the operator looks at its Conductor pane, the keyboard pane of the visible Agent tab of the Workspace in front in a window that is shown and focused (the desktop host's report, or a browser tab's own focus), that pane's popover opens pinned once and the keyboard stays in the terminal; anywhere else, another pane, tab, Workspace, window or app, nothing opens and only the chip changes (B12, B13; D-03, D-06).
The page decides it from two consecutive frames it received, a mission going from no nodes to some or first appearing with nodes, so a load, a reconnect or a restart opens nothing, nor does a replan (B14; `web/src/missionReveal.ts`).
The sidebar row's, the 태스크 row's and the Overview record's mission chip keep their words and wear the mission's state colour: the decision colour with a `!` before `미션 n/m` while one waits, else working, review while it waits to merge, done or cancelled (B35; needs-you-cleanup B5).
While a decision waits and the Conductor neither works nor asks anything of its own, its row stands in 내 차례 with the line `판단 · <decision>`, keeps its mission chip beside it, and the decision announces once on the desktop and the phone; answering it, clearing it or closing the mission takes the row out at once (needs-you-cleanup B1-B4).

### A plan's map

A session's own plan (PRD hide-plan; PRD mission-plan-visibility D-07) is read as a checklist in the pane head chip's popover, in the session record and folded in a mission node's row, and drawn as a map (`PlanGraph`, the mission map's `DependencyGraphView` with a one-line card per step) only in the 보기 slot.
The plan chip sits where the task chip was, between the title and the PR chip, only while the pane's session has a plan and conducts no mission ([A mission on its Conductor's pane](#a-mission-on-its-conductors-pane)): the list-checks mark, the word `계획`, a cell per step and `n/m`; `검토` in the warning colour with the current step's cell in it while the plan waits for the operator's review; and a green check with `계획` and `m/m` once every step is done or dropped (B1).
A cell is a step in the checklist's order, done green, the current one blue, the rest empty, and a dropped step has none; past 20 steps each run of one state is one stretch, so the chip keeps its width while `n/m` stays exact, and its accessible name says `계획 n/m, 지금 <단계>` (B1, B36).
The pointer or the keyboard's focus on it opens the checklist, a press or ⇧⌘U (Alt+Shift+U on Windows and Linux) pins it, and a second press, ⇧⌘U or Esc closes it; it never takes the keyboard from the terminal.
The popover's head is `계획 n/m · <N분 전 갱신> · 크게 ›`, then the checklist and `맡긴 일 N · 끝남 k` for a pane with children; it has no map and no record footer, the pane ⋯ menu and the sidebar row opening the record (B3, B7).
The checklist runs in the order the steps wait on each other, a step beside an earlier one at the same depth indented once: a done step has a check and dim words, the current one a blue mark, a raised background and `지금`, a step still ahead an empty circle, and a dropped one is struck through and dimmed; every title is whole, wrapping when long, an `output` of `pr` or `file` is a small tail, and a step the last `set` changed carries the history mark with its reason on hover or keyboard focus (B3, B4).
A list taller than the popover scrolls inside it, and opening it shows the current step (B5).
A finished plan's head is `계획 끝남 m/m · <걸린 시간>`, from the plan's making to its last update, and the popover's checklist folds into one line `✓ m단계 끝` and the outputs its done steps left (B6); the session record and a mission node's row, where a finished plan is read afterwards, list every step.
`크게 ›` and the pane ⋯ menu's `계획` open the map in the 보기 slot; the lane opens if closed and the keyboard stays in the terminal, and putting a plan up never opens it by itself.
On the map the steps run top down by what they wait on, parallel ones side by side: a done step is filled, the current one carries the working colour and `지금`, one that waits on a step not finished yet is dashed, a dropped one is struck through and dimmed, an `output` of `pr` or `file` is a small tail, and a step the last `set` changed carries the history mark with its reason.
An edge from a finished step is faint and one still ahead is bright.
In the 보기 slot the head says `계획 n/m · <N분 전 갱신>` (`계획 N단계` while it waits for review), and pressing a card opens its detail: its state, its line, what it waits on and why it changed.
While the plan waits for review a band says `검토 기다림 · 대화에서 답하세요` with one button, `대화로`, which goes to the pane; there is no button that approves, and the band goes once the agent runs `hide plan approve`.
A finished plan's band says `끝남`; a plan whose session is gone draws one line saying so.
While a plan runs, the pane's second row shows the current step and its line, `<단계> · <line>`, in the progress place; while it waits for review the row is the ask band, `Plan review · N steps` (`계획 검토 · N단계` in Korean) worded by the shell from the subject the core sends, and the sidebar's second line says the same ([status-model.md](status-model.md#a-plan-waiting-for-review-is-a-question)).
The session record has a `계획` section with the count, `끝남` once finished, and the checklist, kept after the session ends; while its pane works or asks, a running plan moves into the record's `지금` block ([The session record](#the-session-record)) and the section is not drawn.
Settings › Agents lists the three skills Hide installs, `hide`, `hide-conductor` (waypoints) and `hide-plan` (list-checks), each with one line on what it is for and the help it points at; they come and go with each agent's switch.

### The session record

A 태스크 row's record button or Space, and an agent pane head's status mark, logo and title, open the record in the 보기 slot ([Pane header lineage and ownership](#pane-header-lineage-and-ownership)); pressing the title again while the slot shows that record turns the lane back to its tool.
The Overview's panel draws the same record under its own head ([A record beside the tabs](#a-record-beside-the-tabs)); the two differ only in that frame, and a window reads one record at a time (PRD overview-device-tasks D-07, B17).
The record starts with its title and agent logo, then `<device> · <agent product> · <project> · ⌂ <checkout>`; long titles and paths stay within the lane.
The title is the flow the operator asked the most in, the later one on a tie, while the pane and the sidebar keep the current flow's name; with Hide AI off it is the session's own title (PRD session-record-fact-status D-06, B9, B10).
Its native status and elapsed time share a quiet row with a `터미널 열림` or `터미널 닫힘` chip; there is no coloured action box.
An open pane offers `터미널로 이동` even after its work ends, or `답하러 가기` while asking.
While that pane works or asks, a `지금` block stands right under the status row and above `나를 기다리는 것` (PRD session-docs-and-assistant D-41), drawn alike in the lane's 보기 slot and the Overview's panel.
Its first line is the turn in the pane's own words, `<request> → <now>`: who sent the request and its line as the second row and the turn card show it (the label's summary, else the request's own first line), then the second row's progress place, an ask's words in the warning colour while it asks, else a running plan's `<단계> · <line>`, else the agent's own sentence.
The work it delegated is `함께 진행한 세션`'s to count, and Hide keeps no count of the files a turn read or changed, so the block says neither.
Under the turn comes a running plan, `계획 n/m` over its checklist with done, current and later marks, which the `계획` section then leaves out; with no plan there is no plan part.
A part Hide does not hold is not drawn, and the block is absent while the pane neither works nor asks and for a closed pane; while it asks, the question is said in the block and not again under the status row.
A closed pane offers `세션 복구하기`, using the existing native-session resume in a new pane in the registered checkout containing its original working folder on its device.
A missing conversation path, unsupported agent, disconnected device or missing checkout disables restore and states why beside the action.
A restore in flight shows `복구하는 중…` and accepts no second click; a failed start shows an inline reason with Retry, and a successful start focuses the resumed pane.
The record remains open until the resumed pane is confirmed; a timeout or an unknown launch result offers `시작 결과 확인`, which checks the original request instead of starting another pane, and Retry after a confirmed failure keeps that request's identity and destination.
Archived sessions keep their existing tree-aware `이어가기` action.
While the agent neither works nor asks, the status row reads the record's last real turn: its end word and line, with a turn its provider refused for a usage limit or an API error (`한도`) skipped and its line shown as a small badge beside the word, such as `한도 소진 · 10-22 13:00 풀림`; when every turn hit the limit the word is `막힘` with the badge (PRD session-record-goal-flow D-10, B10).
When every pull request first printed in that turn's flow, a child's among them, that had not settled by that turn's end has merged or closed since, the row reads `머지됨` (or `닫힘` when none merged) with `#n`, or `PR n개` for several, and the time since the last one settled, in place of the line; a turn that asked, was blocked or hit a limit keeps its own word (fact-status D-01, B2, B3).
A pane at work or asking is the state now, and a pane drawing a limit as a block defers to the record's word and badge.
There is no `요청한 작업` checklist, no checkbox and no tab row (D-01, B2); below the status come, in this order, the `지금` block, `나를 기다리는 것`, the goal flows, the PRs and outputs, and `함께 진행한 세션`.
`나를 기다리는 것` lists the questions and decisions the agent asked the operator in the current flow and that no later turn answered (moving on to the next flow answers the ones before it, and asking one again later in the new flow lists it again, fact-status D-03, B6), one line each with `넘기기`; a line put away keeps its place with `되돌리기` while the record stays open and turns open again if the agent asks it again after it was gone; the operator's next request, or a delegated session's parent's reply, removes the lines asked before it, the agent's still-open question staying listed, and with none open the section is absent (D-12, B11, B12).
The goal flows follow in the order the goal changed, each with its name, when it began and its request count (`지금` for the current one); the current flow is open, and a past flow is folded to its result and opens on a click (D-03, D-11, B3, B13).
The result is, newest knowledge first: `머지됨` or `닫힘` with `#n` or `PR n개` and the time, when every pull request first printed in the flow settled after its line (fact-status B1, B3); the line the label gave as the flow closed, with no word (B4); `넘어감` with the line of its last turn when that turn asked or waited and the operator went on (B5); else the line of its last `끝` turn with the `끝` word, else its last real turn's end word and line, else its last turn's, so a flow with a judged turn always has a result (B7).
A flow boundary is the label's turn-end reading first: a change in what the agent makes (a board, then an interview or a PRD, then a PR) opens a flow without any `/goal`, and the same work said more clearly renames the current flow within the turn, in every agent alike (D-09, B14, B16).
In an agent that has `/goal` (Claude Code, Codex), the operator's `/goal` confirms a new flow at that request, named from the goal, and the request line carries a `/goal` mark (B15).
A goal that names a pane or agent id is refused and the flow keeps its name; a session with no goal yet takes its first request's summary as the name (B17).
An open flow lists every request sent while it was open, in time order, judged or not, placed by its time rather than by any stored flow number (D-05, B4).
A request line is its time and the label's one-line summary; a click opens the words as sent in place, and a request with no summary yet shows its first 120 characters on one line with `…` (D-06, B5).
A reply to the agent's question begins with `↩` and the question it answers, and the summary keeps a question a question, every ask a request held, and a reference such as `1번` resolved (D-07, B7, B8).
A Hide letter, from an envelope or the `🦉` line Hide typed, is a dim line with its sender and an envelope mark, and is not counted as a request; two or more in a row fold into one dim line with their count and senders that opens to each; the same words sent again within a minute are one line with `×n`, and an attached image's path becomes `이미지 n장` (D-18, B23-B25).
When the record has dropped its oldest requests past its cap, `오래된 요청 n개는 보이지 않음` closes the flows (B30).
`PR` lists the same list the session's row draws (docs/status-model.md, One list per session): this session's pull requests, then one group per direct child, under that child's name, holding the pull requests of the child and all its descendants, an ended one's included (D-14, B18).
Open and failing pull requests show as rows with a state mark, number and title; merged and closed ones fold into one `끝난 PR n개` line that opens them all, a closed one's row dimmed, and the head reads `전부 머지됨` when every pull request merged (B19).
Every output opens the existing PR card or issue action, and a GitHub value that cannot be refreshed stays dimmed with its last-read time.
`결과물` lists the outputs among the Views this session and its descendants opened, in every checkout, the one looked at last first, then the outputs other than pull requests its turns named (a board, a document, an issue, a release, D-16), each once (PRD session-docs-and-assistant D-40); pull requests stay in `PR`, and the section is absent with none (B22).
An output is something made for a person to look at, not something the agent merely opened: every page (a preview address or an outside link), and a file whose extension names an image, a PDF, a video, a board (`.pen`, `.excalidraw`, `.tldraw`, `.drawio`) or a document written for people (Markdown, HTML, plain text, reStructuredText, AsciiDoc); source code, diffs and changed files, logs and configuration files are left out, since the pull request and Changes already show the code.
The core decides it in one table by the View's kind and the file's extension (`is_output` in `herdr-core/src/runtime/view_sessions.rs`), and an opened code file simply is not listed; this kind filter is interim until the agent names its outputs itself with `hide output add` (Desk phase 2).
A View belongs to the record when one of its openers is a session of the record's tree, found by its native session on its own machine as `hide view list --session` finds it ([File Views per checkout](#file-views-per-checkout)); a drilled child's record shows its own subtree's Views, and closing a View takes it away.
Only an item with a real picture is a thumbnail, in three columns of short, wide tiles: an image, read through the file read its viewer uses (past 8 MiB it draws its file mark instead), and a page this window already holds a capture of; no page is loaded or captured for it.
Every other item is one compact row under the thumbnails: its kind mark, its name and one short muted detail, an open document's first heading (else its first line), a page's address, or for another file, such as a PDF or a board, the folder it sits in within its checkout; a named output has a package mark and no detail.
Pressing a View's thumbnail or row brings that View to the front of its checkout's documents: in the Workspace on screen as a pane's `N views` row does, and from the Overview or another checkout by bringing that checkout forward with the View, the session that opened it taking the keyboard when its strip does not show the View; a device's View brings its Workspace forward only, and nothing opens in a popup.
A named output's row opens nothing.
The section shows the first three thumbnails and four rows, and `+N` for the rest, which shows them all; the core sends at most 64 Views and counts the rest, said once as `오래된 View n개는 보이지 않음` once all are shown.
The Views do not depend on Hide AI; with summaries Off the gallery keeps them and leaves out the named outputs.
`함께 진행한 세션` is a flat list, five initially with a way to show more.
Each row shows its state now, from its live pane, else from its last real turn (`머지됨` or `닫힘` when that turn's flow's pull requests settled after it, as on the top state, fact-status B2), the agent logo and product name, the name of the work it was given (never an envelope's words), and its PR count (B20, B26).
Pressing a child expands its flows and pull requests in place, one child at a time, while the current session stays on screen.
An explicit `기록 열기` opens that child's record as the next slot item with the existing back action; a live child also offers its terminal action.
A History hit opens the flow holding the request, with its words shown and the query marked; a request the record no longer keeps says so (B32).
With summaries Off, the record is the native or manual title, native status, the `지금` block from what the pane shows, every request in time order as sent with `AI 요약 꺼짐` said once, the pull requests, the opened Views and the child sessions, with no flow names, results, summaries, named outputs or `나를 기다리는 것` (D-20, B28).
A record written before flows keeps its stored goals as flows, its requests with no summary as their first words and its past flows' results as their end words only (B27).
The problems and follow-ups a turn found leave the record and stay in the Overview backlog's missed group ([백로그](#백로그)).
Loading, failure, retry, disconnection, archive and resume continue to use their existing runtime policies.
At narrow widths and the largest interface text, all sections and controls wrap within the lane; no session section introduces horizontal scrolling.
An archived session's record keeps every part above, and its `지금` box reads `보관됨 · <경과>` and the last line the agent wrote, with `이어가기` as its one button; while it resumes the button reads `이어가는 중…`, a member that could not resume adds its reason in the failed colour, `상태 확인` stands in its place while Hide cannot say, and a device that is not connected dims the box and holds the button (PRD session-archive B19-B24, B34).
A live root whose archive left members closed lists them under its `지금` box, each with its state and its own `이어가기` (B24).
A first record read uses a small skeleton; a failed read shows Retry, and a disconnected device retains the last read dimmed until reconnection refreshes it.
GitHub values that cannot be refreshed stay dimmed with their last-read time in the tooltip; failure is never a false zero.
Web owners: `web/src/SessionRecord.tsx`, `web/src/RecordGallery.tsx` and `web/src/galleryTiles.ts` (the gallery), `web/src/DetailPanel.tsx`, and `web/e2e/session-panel.spec.ts`.
Core owner of the record's Views: `herdr-core/src/runtime/view_sessions.rs` (`settle_record_views`), held by `runtime/tests/view_bookmarks.rs`.


### The PR list and the PR card

The sidebar's PR list replaces the lane's PRs tool (PRD lane-two-columns D-20, B41); ⇧⌘R and the switch's `PR` open it, and the sidebar keeps it through a restart.
Its first line reads the project chip the 태스크 list shares ([The sidebar's 태스크 list](#the-sidebars-태스크-list)), `PR N`, then ⚠ while the last GitHub read failed or left a question unanswered.
A lane saved on the PRs or Memory tool opens with no tool column, the rest of its lane and its View layout as they were (D-10).
A project with no GitHub remote reads `GitHub 원격 없음` alone.

The PR list lists the pull requests of the rail's device that concern the operator: their own, those their Hide sessions made or worked on, those asked of them or of their team for review, and bots'.
Every row names its project unless the project chip chose one, and a project on a connected device lists like the rest with the device's name on its rows.
The core decides the group, the reason and the order (`herdr-core/src/agent_state/work/board.rs`; the rules are `docs/status-model.md`, Pull request groups); the web draws them, and every pull request is in the first group it fits.

The groups, in order, are `Needs You`, `Agent is working`, `Blocked · no agent`, `Waiting`, `Drafts`, `Bots` and `Merging · merged` (in Korean `내 차례`, `에이전트 작업 중`, `막힘 · 맡은 에이전트 없음`, `대기`, `초안`, `봇` and `머지 중 · 머지됨`); the last four start folded, a group with no row is not drawn, and `Needs You` carries the review asks of a team as one folded line, `팀 리뷰 요청 N`, which opens to rows `리뷰 · <팀>에 요청 · N시간째`.
A pull request nothing has moved on for 7 days folds into `오래된 PR N` inside its group, each row of which shows the date of its last move once opened; the two folded lines are not counted anywhere.
`Merging · merged` puts a pull request in the merge queue first, `머지 큐 · N번째 대기`, then the merged ones newest first, for 14 days or while their worktree is here; a merged row with a worktree has `정리`, which opens the Delete worktree dialog unchanged.

A row is two lines.
The first reads the PR mark with its number, the title (ellipsized, in full in the tooltip), the agents' marks (a teammate's pull request shows its author instead) and the time; the second begins with a word for what to do, in the text colour, and the rest in the group's colour: `머지 · 검사 통과`, `리뷰 · <작성자>가 N시간째 기다림`, `답변 · <에이전트>: <질문 첫 줄>`, `확인 · <에이전트>가 N분째 움직임 없음`, `충돌 · <base> 브랜치와 충돌`, `변경 요청`, `✗ <검사 이름>`, `작업 중 · <에이전트> N분째`, `CI · 검사 N개 실행 중`, `리뷰 대기 · <리뷰어> · <기간>`, `머지 큐 · N번째 대기`, `업데이트 · <base>보다 뒤`, `리뷰어 지정 · 필수 리뷰 없음`, `초안 · N일째`, and `확인 중` while GitHub has not computed the merge state.
A blocked row ends `맡은 에이전트 없음`; the words come from GitHub's values and Hide's session facts alone, and turning Hide AI on changes none of them.
A click or Enter opens the card, ⌘-click or ⌘Enter opens GitHub, an agent's mark opens that agent's pane, and the arrow keys move between rows, group headers and folded lines.
Under the pointer or the keyboard the time's slot holds the row's buttons and nothing moves: `▷ 맡기기` on a blocked row, `정리` on a merged one with a worktree, otherwise the GitHub icon and `⋯` with 맡기기, 이슈 연결, 새 이슈 만들기 and 브랜치 이름 복사.
The list has no search field, so Escape has nothing to clear in it.

Until GitHub has answered the tool shows three skeleton rows and no zero is drawn; with nothing concerning the operator it reads `열린 PR 없음` above the folded `Merging · merged`.
Without `gh` or its sign-in the tool reads one line of why with a `Settings › GitHub` button.
When a read fails the last pull requests stay, PRs carries ⚠ whose tooltip says `GitHub 읽기 실패 · N분 전 값`, and there is no banner.
When the list was read but a question beside it was not (the signed-in account, the review requests, the merge queue; `docs/status-model.md`, Pull request groups, owns what each leaves unknown), the list stays whole and current and the same ⚠ names the question in plain words: `GitHub 읽기 일부 실패 · 못 읽음: 로그인한 계정` (several are joined by commas), and after a failed read the names follow the age (`GitHub 읽기 실패 · 6분 전 값 · 못 읽음: 머지 큐`).
There is no banner or sheet for it, the sidebar's PR marks and the card are unchanged (a mark counts states the list alone decides, so it does not dim), and the failure's own words stay in the diagnostic log (PRD work-tab-pr-lens D-25 is replaced by issue #1189 here).
A pull request that cannot be judged because of such a question sits in the plain Waiting group, never in one it may not belong to, and its reason line reads `구분 못 함 · ` followed by what could not be told (`내 PR인지 알 수 없음`, `내게 리뷰가 요청됐는지 알 수 없음`, `머지 큐에 있는지 알 수 없음`) in the muted colour of `머지 상태 확인 중`; its card keeps GitHub as the primary action and draws Merge only for a pull request known to be the operator's, and it adds nothing to the PRs icon's number.
While the tool or a card shows, every listed open pull request whose checks run, an agent's or not, is read again every 60 seconds; hover, focus and a rest publish nothing.
The tool and a card each hold that one demand, and it is released only when the last of them closes.

#### The PR card

A card opens in the lane's 보기 slot (PRD sidebar-task-lens D-43, D-50): the Workspace in front, its Agent tab and the keyboard stay where they were, and the slot's head reads `←`, `PR #N · <project>`, `↗` (GitHub) and `✕`.
A card opens from a row, a 태스크 row's PR mark, the sidebar's PR card, ⌘K, a session's PR chip, Memory's PR button, a session record's PR and Recent Panels, and `이슈 없는 PR N` opens the PR list instead; at Home the pull request's Workspace opens with the card in its lane.
A stored `PR #N` tab from an older build is dropped when the app starts, and the rest of the checkout's Views stay; a pull request on a device this core does not read opens on GitHub.

From the top the card reads (B32-B36):

- the head shows the PR mark, number and lifecycle, an explicit `GitHub에서 보기 ↗` button and `⋯`; the wrapping title follows, with the author on a teammate's PR and the mono `branch → base · 파일 N · +a −d` facts below it; Workspace opens the known checkout, and the links line shows actual linked issues and branch worktrees only; an absent issue draws no placeholder, and `이슈 연결` remains in the menu;
- a quiet CI and merge section replaces the coloured agent action box: `CI 검사 18개 중 3개 진행 중`, followed by the real nonzero counts for passed, running, waiting, failed, skipped and unknown checks; missing counts say they are not confirmed, never an invented total; `전체 검사 보기 ↗` opens this PR's checks, and review/conflict facts, failed-check logs and unresolved threads retain their existing actions;
- `타임라인 · 세션 N`, a small quiet head (D-03, D-05, B3-B6; PRD record-card-readability D-01, D-09, B16), the count left out while no session worked on it: the sessions that made the pull request or worked on its branch and the pull request's own milestones, one list oldest first, sessions by their start and milestones by their time, joined by one vertical line behind their marks, every time in the right column in mono, a row with no time leaving that place empty, and with no axis or bar; the milestones are only what happened, `PR 열림`, `검사 통과` or `검사 실패` (the latest completed run, read from GitHub's check runs, D-15), `리뷰 요청 · <누구에게>`, `승인` or `변경 요청`, `머지됨`, each a small square in its colour where a session row has its mark (green for a pass or an approval, red for a failure or changes, yellow for a request, purple for the merge), its word in normal weight and its time at the right, a check or review row whose time could not be read standing right after `PR 열림` with none; an open pull request ends on one line, `지금 · 검사 N개 실행 중` with the dashed ring of proven work it waits on while checks run, and `지금` with an empty slot otherwise (status-marks D-06, D-25; PRD record-card-readability D-12, B21); there is no `작업 → PR → 검토 → 머지` stage bar; the session rows are below;
- `바뀐 파일 N`, one folded line with +/- at the right that opens in place and stays as it was left while the app runs, to the files with files `.gitattributes` marks `linguist-generated` and the common lock files gathered into `생성 파일 N개 접음` (B11);
- `본문`, never folded (D-02, B12, B13): a row of the body's `##` headings that sticks to the top of the card while the body scrolls under it, a chip scrolling to its heading and the chip of the heading last passed pressed, then the whole body as Markdown, with the HTML GitHub draws in a body drawn too (PRD record-card-readability D-11, B19, B20): `<details>` and `<summary>` as a folded box, `<img>` and Markdown's `![]()` as the picture when its address is https on a public host, because the card loads it by itself as it opens and a body must not make the operator's machine ask this computer or its network for anything (its description where it cannot be read), `<br>`, `<p>`, `<b>`/`<strong>`, `<i>`/`<em>`, `<code>`, `<kbd>`, `<sub>`, `<sup>`, and `<a>` and Markdown's links to an http or https address as links that open in the browser; any other tag, an attribute outside that list (an `on…` handler among them) and a `javascript:` or relative address stay as the text they are, nothing runs, an opening tag past 32 open elements stays as its text, and an HTML comment is not drawn; a body with no heading has no row, an empty one reads `본문 없음`, and one that could not be read says `읽지 못함 · 다시 시도` for that part alone.

There is no AI summary, no four-step stage strip, no `만든 세션` line, no folded `연결` line and no body preview (B22).
No part of the body is lifted out as the card's own judgment.
A changed file opens as the Hide diff among the checkout's documents when the Workspace in front is a checkout of the pull request's branch and its committed changes already hold the file, and the documents come in front while the 보기 slot stays on the rail and brings the card back as it was (B49); otherwise it opens the pull request's files page on GitHub.
A part that could not be read keeps the head, the decision box and the milestones from the list, and says `읽지 못함 · 다시 시도` on that part (the folded files line says `읽지 못함`), which reads again only that part; a check or review time that could not be read leaves its row timeless, and the reason goes to the diagnostic log (B21).

The merge section's button is the case's: `머지` when the pull request can merge, in a method the repository allows (a choice when it allows several) or `머지 큐에 넣기` in a repository with a merge queue; `브랜치 업데이트` when it is behind; `GitHub에서 리뷰` and `에이전트에게 리뷰 맡기기` for a review asked of the operator; `맡기기` for a blocked pull request with no agent; `리뷰어 지정` opens the pull request on GitHub; agent terminal actions belong to the timeline's session row, and GitHub stays visible in the card's header.
Merge and `브랜치 업데이트` are the app's only GitHub writes besides a pull request's body, a new issue and the close `되돌리기` makes, and appear for the operator's own pull requests.
A Merge that cannot run is disabled with its reason beside it (`검사 13개 실행 중`, `에이전트 작업 중`, `main 브랜치와 충돌`, `필수 리뷰 없음`).
Pressing it asks once, `PR #N: <base>에 머지 · <방식> · <짧은 커밋>` with `#N 머지` and `취소` (focus starts on `취소`), and the write goes out only for the head commit the card showed and the list judged: the confirmation closes by itself when the card's head, method or queue moved, or Merge became unavailable, and the core refuses a write whose head differs from the head the list was read at, so a push between the two reads is never merged unseen.
When the pull request moved meanwhile nothing is written and the card says `PR이 바뀌었어요 · 다시 확인`, whose button reads the list and the card again and clears the notice.
Why Merge is unavailable names the real cause: checks running, checks failed, changes requested, no required review, conflict, draft, an agent working, no permission.
A merged pull request's card reads and draws like any other, with no Merge.
A merged pull request moves to `Merging · merged`; asking again for one already merged or queued writes nothing a second time.
A branch update asks the same way and takes the same head condition; a conflict turns the second line into `충돌 · <base> 브랜치와 충돌`.
A failure leaves one line beside the button that says what the operator can do (`권한 없음 · GitHub에서 확인`, `GitHub 응답 없음 · 다시 시도`), the same button tries again, and the detail goes to the diagnostic log.
`에이전트에게 리뷰 맡기기` starts an agent on the branch's checkout (a new worktree when there is none) with a review instruction; it reports to the operator and neither reviews on GitHub nor merges.

Hide AI makes no summary of a pull request; the `PR 요약` feature, its Settings › Hide AI › Features item, its usage accounting and its stored summaries are gone, and a value an older build left in the settings file is read past (PRD pr-card-and-session-record-redesign D-19, B23).

The links line and the timeline's session rows are the link record (PRD link-graph), read when the card opens:
An issue on the links line opens that item in the Overview's 백로그, or, once it is started, its task in the sidebar's 태스크 list, or GitHub for one the project does not list; a worktree opens its Workspace.
What is true now (the pull request, CI, the review, whether a worktree is still here, whether a pane is live) is the snapshot's; the links and the sessions are the link record's.
`세션 N` counts the sessions that made the pull request or worked on its branch while it lived, from the record.
A session row is the sidebar's agent row shape (D-07, B7-B9; PRD record-card-readability D-09, B17): the status mark of its live pane, or an empty slot once it ended, its caption saying 끝남 (status-marks D-06), then the agent's logo; the title (the live agent's label, else the record's title as the Sessions list shows it, else the agent's product name); the `PR 만듦` chip on the session that made the pull request, only when the card lists two or more sessions; and at the right the time, `10/16 15:36 – 20:43` for an ended session and the time since its start for a live one.
Under the title an ended row reads `끝남`, a live one its state word, and an asking one its question in the warning colour; the logo names the agent, so the line does not repeat it.
An asking row keeps `답하러 가기` (the card's only accent button) under that line; while the pointer or the keyboard is on a working row `터미널로 이동` takes the time's place, and on an idle live row `대화 보기` and `터미널로 이동`.
The row's tooltip holds what the line leaves out: the agent's product name, the request (for the session that made the pull request, the request just before its address was printed; otherwise the last request on the branch; `요청 없음` without one), `맡긴 에이전트: <부모>` on a delegated session, `이어짐 N` for sessions joined by compaction or resume, and the device's name for a session on a connected device.
While the pointer or the keyboard is on an ended row, `대화 보기` and `이어서 하기` take the time's place (B9): `대화 보기` opens that session's record (in the lane while a Workspace is in front, otherwise in the Overview's panel), and `이어서 하기` starts `claude --resume <id>` or `codex resume <id>` in a new tab of the worktree it worked in and moves the screen there; a failed start leaves one line of why and `다시 시도` on the row.
An off button stays reachable and its tooltip says why: the worktree was cleaned up, the device is not connected, another device's conversation cannot be opened here yet (PRD link-graph D-41), the agent's sessions cannot resume (D-42), or the file is gone; a row whose file is gone is dim with `대화 파일 없음` and copies the path the file had.
Six sessions or more fold the oldest ended ones past five behind `이전 세션 N개` at the top of the timeline, a live session never and a milestone never (B10); with none the timeline reads `이 PR에서 일한 세션이 없어요` with `▷ 맡기기` above the milestones.
While the record fills for the first time or the panel reads, a small spinner line turns at the top of the timeline and the rows read before stay; a record that cannot be read puts one line of why and `다시 시도` there, and the rest of the card stands.
A pull request the list does not hold (an old one a session's chip asked for) is drawn from the record; while that read runs or after it failed the card is only `#N`, × and the timeline's sessions with its spinner or its failure, and it closes only once this open's read says the record has no such pull request.

이슈 연결 opens the project source's open issues, searchable, with `새 이슈 만들기` last; closed issues and another repository's are not in it.
A GitHub issue asks once, `PR #N 본문에 "Closes #M" 문구를 써요. PR이 머지되면 GitHub에서 이슈를 닫아요.`, with `취소` first and `본문에 쓰기`; confirmed, Hide links the branch to the issue (its issue link and the Workspace token, when the branch has a worktree here) and writes the line at the end of the body after a blank line, reading the body just before, and the panel's issue line fills.
When the body already closes that issue by a closing keyword, nothing is written and the link succeeds, so a retry never adds a second line.
A failed body write keeps the link and the filled line; the dialog stays with one line of why and `본문 다시 쓰기`, which writes only the body, and `닫기`.
A Local issue is Hide's link alone: choosing it asks nothing and writes nothing to GitHub, and the dialog appears only when the link fails.
`새 이슈 만들기` opens the pull request's title and body (read when it opens) in editable fields and one confirmation, `이슈를 만들고 PR #N 본문에 "Closes #(새 번호)" 문구를 써요`, `만들고 쓰기` then makes the issue and writes the body; when the body write fails the issue stays made and linked, the dialog says `#M 이슈는 만들었지만 PR 본문은 쓰지 못했어요`, and `본문 다시 쓰기` writes only the body.
On a Local source the same form says `로컬 이슈를 만들어 이 PR에 연결해요. GitHub에는 쓰지 않아요.`, and `만들고 연결` makes a Local issue and links it, with nothing written to GitHub.

`▷ 맡기기` opens the Start dialog on the pull request's branch: the worktree is the branch, shown and fixed, the shared agent picker, and the first prompt the failed checks by name with their links and the change requests as their reviewers wrote them, which the dialog reads when it opens and the operator can edit.
With no checkout of the branch here the dialog says `이 브랜치의 워크트리를 만들고 시작합니다`, and the start fetches the branch from origin and makes a worktree that tracks it; it never makes a new branch.
A failed read of the checks and reviews leaves the prompt empty with why and `다시 읽기`, and does not hold back the start.
Started, the agent works in that checkout and the screen goes to its pane; while it works the pull request is 에이전트가 고치는 중, and once it finishes it is the operator's with `확인` until its pane is looked at.
`정리` on a merged row opens the existing Delete worktree dialog for its worktree, unchanged: the branch kept unless asked, its panes closed by `Close N panes and delete`, a running agent a warning line, and its cancel changing nothing; a row with no worktree record has no `정리`.

## Explorer file management

Web owner: `web/src/ExplorerTree.tsx`, `web/src/explorer.ts`. Core owner: `herdr-core/src/changes.rs`.

A row reads as it does in VS Code: one indent per level with a hairline under each ancestor's chevron, a chevron on a folder that turns down while it is open, the item's mark, then its name.
The mark is the one the Material Icon Theme draws for that name (`web/src/fileIcons.ts`): a whole file name such as `package.json` first, then its longest extension, then a plain file; a folder has its own mark, open or closed, and a plain folder one otherwise.
Under the light theme a mark the theme draws differently on a light background takes that variant.
The same marks stand beside a file in ⌘P, on a View tab and on a History row.
A changed file's name takes its status mark's color; a folder's name stays plain and its dot carries the state.

Clicking, double-clicking or pressing Enter on an HTML file (`.html`, `.htm` or `.xhtml`, case-insensitive) opens it directly in a Browser View through the same device-scoped file boundary as Open in Browser.
Other file kinds keep their editor preview and pin behavior.
The tree's context menu on a file row is Open to the side (and Open in Browser, then Open source for an HTML file), a separator, the reveal, a separator, Cut, Copy, Paste (only while the clipboard holds an item) and Duplicate, a separator, Copy Path and Copy Relative Path, a separator, Rename, a separator, Move to Trash.
Open source opens HTML as an ordinary editor tab, and Open to the side keeps its editor behavior.
A folder row has New File and New Folder in place of the opens.
Each item shows the chord that does the same, as the operator has it bound, at its end.
The item set the menu offers is a presentation decision a test can check directly (`explorerMenuItems` in `web/src/explorer.ts`), not something the platform decides implicitly.

The root row, the checkout's name above the tree, stands for the checkout's root.
A right-click on it, or on the blank room under the last row, opens the root's menu: New File, New Folder, a separator, the reveal, a separator, Paste (only while the clipboard holds an item), a separator, Copy Path and Copy Relative Path.
It never offers Cut, Copy, Duplicate, Rename or Move to Trash.
The room under the last row is one row tall and always there, so the root's menu can be reached however long the tree is.
While the pointer or the keyboard is on the root row it shows three icon buttons beside Refresh, VS Code's pattern: New File and New Folder, which begin a name in the root as the menu items do, and Collapse All, which folds every open folder and is disabled while none is open.
The buttons keep their place in the row whether they show or not, so the name never moves when they appear, and they show all the time where there is no hover.
Refresh and the Git mark stay where they were.

Copy Path puts the item's whole path on the clipboard, in the checkout device's own spelling, and Copy Relative Path the path below the checkout root (`.` for the root itself).
⌥⌘C and ⌥⇧⌘C (Windows and Linux: Alt+Shift+C and Alt+Shift+H) do the same on the selected row, or on the root when none is selected, while the tree holds the keyboard.
They are registry commands (`copy_path`, `copy_relative_path`) listed in the shortcut sheet as Explorer only, rebound in Settings, Shortcuts, and answered by the tree rather than the window, so a terminal keeps those keys.
A clipboard that refuses the write goes to the diagnostic log, and nothing else changes: neither command touches a file, so there is nothing to undo.
Collapse All changes only the tree's expansion, which the next press on a folder restores.

Cut, Copy, Paste and Duplicate work on one item, the selected row or the row the menu opened on.
Cut and Copy (⌘X and ⌘C, Ctrl+X and Ctrl+C on Windows and Linux) put the item in the Explorer's own clipboard, which holds one item and names the checkout and device it came from.
Copy of a file also writes its absolute path to the system clipboard, so another text field or app can paste that path.
Pasting it into a terminal in this window inserts a quoted path, or uses binary attachment ingress for an image, just as dragging that file does.
That file treatment applies only while the system text still matches this window's copied file; ordinary copied text and the explicit Copy Path and Copy Relative Path commands retain plain-text paste.
Cut and folder Copy remain internal file operations, and a terminal paste never moves or duplicates the original file.
A cut item is drawn faded until it is pasted.
Paste (⌘V, Ctrl+V) puts the item in the selected folder, in the folder of the selected file, or at the root when nothing is selected; the menu's Paste on a folder row pastes into that folder, on a file row into its folder.
The folder a paste lands in opens, and the new item is selected.
A copy is a new item and never replaces one: when the name is taken in the destination the host picks the next free `name copy.ext`, then `name copy 2.ext`, and so on (a folder `src` becomes `src copy`), so pasting a copy into its own folder and Duplicate give the same result.
Duplicate is Copy and Paste in the item's own folder, with no chord.
Pasting a cut item moves it (`path_move`) and empties the clipboard; pasting it into the folder it is in does nothing.
A copy repeats on every paste.
A folder is copied with everything under it, in one `path_copy` the checkout's host makes: it builds the copy in a hidden working folder inside the destination and renames it into place, so the copy appears whole or not at all, and a failed copy leaves nothing behind.
A link, a socket or any other file that is not a plain file or folder refuses the whole copy with the name of what it met.
One copy holds at most 5,000 items and 256 MiB; the host measures before it writes, and a copy past either is refused with nothing copied.
A paste works inside one checkout of one device: pasting an item that came from another checkout, or another device, is refused with that line, and a folder cannot be pasted into itself or below itself.
A copy's failure, and a drop's, is a wrapping line under the root row, not under a row, because it concerns a whole folder whose open rows a line there would cover; it is red like a refused name's line and holds the page's own reason or the core's.
Cut, Copy, Paste and Duplicate are not undone inside the app, as a rename and a move are not: a copy only adds an item, so its undo is Move to Trash on the new item, which is selected when the paste lands, and the Trash restores it; a cut item pasted is moved back the way a drag moves it back.

Files and folders dragged in from the operator's file manager are copied into the checkout.
They land where a drag inside the tree lands (see the drag below): a folder row takes them into that folder, a file row into the folder that holds it, and the root row or the empty room under the rows at the root, and the receiving folder is tinted the same way.
Folders come with everything under them, and a name already used in the destination becomes `name copy.ext`, never a replacement.
The window reads the bytes with the browser's file API and hands them over the connection it already has; a path from outside every registered checkout never reaches the core, and the same route serves a checkout on another device, whose helper writes the files into its own checkout.
Each file is uploaded to hided's staging, as a terminal attachment is, and one `path_import` event names the staged files; hided is the only maker of the core's `path_import`, and the core reads the staged files itself and has the checkout's host write the items under a hidden working folder, so a drop lands whole or not at all.
A drop holds at most 500 items, 20 MiB a file and 40 MiB in all; the window checks the counts and the sizes before it reads a byte, and a drop past a cap adds nothing and says so in that failure line.
An upload that hided refuses, a file that cannot be read and an entry that disappears mid-drop end the same way.
A drop is undone like a copy: it adds items, the first is selected, and Move to Trash takes them away.

The reveal is one action, `reveal_external`, on Explorer rows, History rows, View tabs and sidebar project and checkout rows (`web/src/revealExternal.ts`).
A History row's menu is Open to the side, a separator, then the reveal of the row's file.
It hands the file or folder to the desktop host, which shows it selected in its parent folder in the OS file manager (`shell.showItemInFolder`) and opens nothing.
Its label is the host's OS's name for that: `Reveal in Finder` on macOS, `Reveal in File Explorer` on Windows, `Open Containing Folder` on Linux, and `Show in File Manager` elsewhere; the host reports its OS through the preload bridge, and the shell never guesses it.
A plain browser tab has no file manager to hand anything to, so no menu lists the item there.
On another device's files the item is listed disabled with `Only for files and folders on this computer.`; a History row for a deleted file lists it disabled with `The file was deleted.`
The host refuses a path that is not absolute or no longer exists, and records the refusal or the reveal in its log without the path; nothing is shown on screen.

Delete has two entry points, the menu item and a delete shortcut while the tree holds the keyboard, and both end in the same confirmation: an alert asking to move the item to Trash, telling the operator everything in a folder goes too, and that the item can be restored from Finder, with Cancel as the default action and Move to Trash as the destructive one.
Nothing reaches the core without that alert; Cancel and Escape send nothing.
The delete shortcut is scoped to the tree so a pane command cannot be rebound onto it, and it never reaches the terminal by mistake.
The item goes to the OS Trash, never to a permanent delete; the selection moves to the next sibling, else the previous one, else the parent, decided by the tree and carried in the same event so the cursor lands in the same frame as the removal.
The event also carries the item's identity as read when the prompt opened, and the core refuses an item that was replaced at that path while the modal was open, so what leaves is what the modal named.
A failed move keeps the item and puts the reason on the row under it, the way a refused name is shown.

New File, New Folder, and Rename take the name in the row itself: a draft row is inserted at the top of the target folder, or the item's own label becomes the field.
Enter sends, Escape and any other loss of focus cancel, and an unchanged rename closes the field without asking anything.
An empty name, a name with a path separator, and a name already used in that folder are refused before the round trip; the field stays and the reason is one row directly under it.
The field draws on an elevated surface so it reads as an input among labels; nothing else about the row changes.

The filesystem change is the core's: one event carries the request, the core refuses paths outside the focused checkout and any overwrite, runs the exclusive call off the runtime mutex, and settles one operation slot.
The tree reads a finished slot to re-read only the folders it touched, keeping every loaded folder and its expansion, and a failed slot to place the reason under the row the change started from.
Changes run one at a time in the order they were asked: a change asked while another runs, such as a trash pressed right after a drop, waits and starts when the one before it settles.
At most four wait, and a fifth is refused into the diagnostic log; a failed or refused change drops the ones waiting behind it into the log too, so nothing runs past a failure the operator has not seen.
The slot shows only the running change, and the next one starts in the same step that settles the one before it, so the tree can see a change working and then the next working without the first ever finished.
The tree then skips that change's own re-read, and its folders are refreshed the way a change made outside hide is, by the folder watch and device stamping; the selection still lands on the last change's item, because the core sets it on every success.
A folder being re-read, after a settled change, a change the watcher saw, or the refresh button, keeps its rows drawn until the new listing lands; it never empties for the round trip.
A device folder whose re-read is refused keeps those rows under its could-not-be-listed reason until Retry, or until the helper is ready again; a local subfolder whose re-read is refused keeps them until the next change reads it again.
The selection moves to the new or moved item because the core sets it explicitly; expanded folders and open file tabs inside a renamed folder follow it.

A drag moves one item inside the tree.
Dropping on a folder puts the item inside it, on a file puts it beside that file, and on the root row or the empty area puts it at the root, so an open folder takes a drop anywhere over its rows.
The whole receiving folder is tinted, its row and every open row under it, in the accent tint a pane move's target uses so it reads apart from the selection; the root is tinted as the whole tree and its row.
The item's own folder, the item itself, a folder inside the item, and an item from another checkout show no tint and accept nothing.
The drag never leaves a copy that a different app (such as Finder) could read as a file.

The filter field is the Explorer's first row.
Typing narrows the tree to the files hided's file index matches, the index and ranking ⌘P uses, each under its ancestor folders with every folder open, so a match inside a folder the tree never listed is found and its place stays readable; a folder is shown because a file under it matches, and the index answers at most 80 files, as ⌘P lists.
The filtered tree opens and selects files as the tree does, but pressing a folder only selects it, and New File and Rename end the filter first.
The filter never reads or writes the expansion the core keeps: clearing the field, Escape, or switching checkout shows the tree as it was.
While the index is being built the tree says so, and a checkout whose files cannot be listed says why, in the words ⌘P uses.

Git status decorates each row with one status mark: Modified, Added, Untracked, Renamed, and Conflict render as `M`, `A`, `U`, `R`, and `!` with a semantic color and a matching status name in tooltip and accessibility help.
A folder with any changed descendant renders a dot mark; the mark describes derived folder state and never relabels the folder as a modified file.
Deleted descendants still mark an existing ancestor folder but never create a file row that no longer exists.
Clean and unavailable decoration both reserve the slot.
The checkout's Git status is a mark in the Explorer's root row, beside the refresh button, so its answer never moves a row the operator is aiming at.
While the first answer is on its way the mark is a small spinner.
When a read failed for a reason the operator can repair, such as a `git` that cannot run or a repository Git cannot read, or when the decorations are kept from an earlier read after the latest one failed, it is a warning `!` badge whose tooltip gives the reason.
A folder that is not a repository has nothing to repair, so it draws no mark and the diagnostic log records it (`changes.unavailable`).
The decoration is not a control and cannot intercept file open, disclosure, inline editing, drag, keyboard navigation, or the context menu.

Git state comes from the root-scoped History projection.
Rename keeps both previous and current relative paths, conflict remains an independent status, and folder state is derived from the complete changed set rather than only loaded outline children.
Explorer visibility reuses the History reader's bounded refresh outside the runtime mutex.
Switching Workspaces replaces the decoration root, and no per-row, hover, selection, or scroll path starts Git.

## Editor preview tab

Core owner: `herdr-core/src/runtime/editor.rs` (`place_editor_tab`, `promote_editor_tab`), `EditorTabSnapshot.preview`. The web Workspace's one-preview-per-View-area model is the same idea applied per area; see [Web Workspace](#web-workspace).

A single click on an Explorer file other than HTML, or a History row, opens it in the checkout's one preview tab (VS Code's model): the strip draws the title in italic, and the next single click replaces the tab in the same slot instead of adding one.
The core owns the preview flag and decides replacement and promotion; the shell only says what the click meant.
Promotion happens in the same slot, on four triggers: a double-click on the Explorer row, a double-click on the tab title, the first edit, and Keep Open; a drag to a new slot promotes as well.
A dirty tab is never replaced: the core promotes it where it sits and opens the new preview beside it.
Every other entry point - opening a file by path, Reopen Closed Tab, a Markdown or terminal link, a file the Explorer just created - opens an ordinary tab, and a single click on a file that already has a tab focuses it without touching the slot.
A replaced preview tab is not a close: its document, mode, and wrap state are dropped and nothing enters Recent Closed; closing the tab yourself records it as any file tab.
Editor tabs stay ephemeral, so the preview flag is never persisted.
The tooltip and the accessibility label read `name · Preview` while the tab is one and drop the suffix on promotion; the tab's colors, close button, and its Recent Panels row are the ordinary tab's.

## File document toolbar and Markdown

Web owner: `web/src/Editor.tsx`, `web/src/viewers/FileViewer.tsx`.

The central file surface uses one document toolbar, preserving the tab strip and Explorer.
The current folder and filename give context; Find uses the platform's native find bar, Wrap changes the source text container, and reveal actions target Explorer and Finder.
Unsaved drafts and the existing read-only/conflict notices remain visible in every mode.
Diff tabs retain their own viewer, with the same Wrap toggle.

The core names each open file's kind (text, markdown, image, pdf, binary), and the overlay picks the adapter from it; the toolbar is the same bar in every kind, with controls a kind cannot use taken away rather than left dead.
A PDF (recognised by its signature whatever its name) shows in a continuous, width-fitted, text-selectable, non-editable view.
Its toolbar keeps the breadcrumb and the two reveals, shows Find disabled with the reason that Find is unavailable for PDF, and hides Wrap, the Markdown mode group, and Unsaved, which a PDF can never earn.
A PDF that cannot be decoded, cannot be read, or is password-protected shows a `PDF unavailable` state with the reason under the same toolbar.
An image hides Wrap as well; a file that is not UTF-8 shows a `Preview only` state explaining the file type cannot be shown as text, and keeps Wrap disabled beside a disabled Find.
An image opens whole inside its display and never larger than its own pixels; an image that does not decode shows the `Image unavailable` state.

An image or a PDF zooms the way a browser page does, from 25% to 800% of the size it opened at.
A trackpad pinch, or a mouse wheel with Ctrl held, zooms around the pointer, so the point under it stays there while the content can still scroll that far.
While its display holds the keyboard (after a click in it), ⌘= and ⌘- step through Chrome's zoom levels around the middle of the display and ⌘0 returns to the size it opened at; the same chords size a text document's text and a terminal's, as before.
A zoomed document scrolls with the trackpad or wheel, and a zoomed image also moves under a drag, with the grab cursor while it can.
A PDF page is drawn again at the scale it is shown at and the screen's pixel density once the pinch or keys have rested, so it stays sharp; until then the page already drawn is stretched.
Only the pages in view and one view either side are drawn, ten at most, and a page that leaves gives its drawing back.
The zoom belongs to the display showing the file: nothing on screen reports it, another display of the same file keeps its own, and opening a file, or a preview moving to another file, starts at the fit.

Markdown files alone show the centered Live/Source choice, and Live is the default.
Both are editors over the same draft: Live draws the formatting in place and hides the markup on every line the caret is not on (the way Obsidian's Live Preview does); Source is the monospaced editor with its line-number ruler and Wrap toggle.
The core owns mode and wrapping per open file tab, and wrapping per open diff tab; another tab has independent choices, returning to a tab restores them, and a new tab, close/reopen, or app restart starts Live with wrapping on.
Autosave captures its file identity when scheduled so a subsequent tab selection cannot redirect the write.
Closing a file tab carries that tab's matching pending save in the same close intent, and the tab remains open with a visible error if the exact path and contents cannot be saved.
The editor retains only its latest unacknowledged draft while older core snapshots arrive, preventing a snapshot echo from moving the caret or replacing newer input.

In the Live view, the line holding the caret, and every line a selection crosses, shows its source; a fenced code block is one unit, so a caret anywhere inside it shows both fences.
Markup hides again the moment the caret leaves, with no animation.
Headings hide their hashes; bold, italic, and strikethrough hide their delimiters; inline code and fenced blocks use the editor's monospaced font; unordered markers draw as a bullet and ordered markers keep their digits, both with a hanging indent; a quote indents behind a bar and hides its `>`; a link shows its text in the accent color with the brackets and URL hidden; a `---` line draws as a rule.
Tables, footnotes and inline HTML other than `<img>` (`<br>`, `<kbd>`, `<sub>`) are not drawn: they stay monospaced source, editable in place, and a parse the view cannot use leaves the whole document monospaced with the reason in the notice bar while typing continues.
An HTML block at the top level of the document is drawn as its rendered HTML, a Markdown image `![alt](src "title")` and an inline `<img>` as the image, and the Live text of an issue body (read-only, never revealing source) draws them the same way.
Like a fenced code block, a block is one unit: a selection anywhere in it, edges included, shows its source, and an image shows its source while the caret is on its line.
A file opens with the caret at 0, which the operator did not put there, so a block or image there stays drawn until the operator first moves the caret or edits; every other mark follows the caret from the start.
An HTML block inside a quote or a list item, a comment, and a block with nothing visible left after cleaning stay source.
An empty Markdown file is an empty Live editor with the caret in it.
Links open on modifier-click only, a Markdown `[text](url)` link and a link in drawn HTML alike: HTTP(S) and `mailto:` in the external browser, a relative file inside the current checkout as a tab, and anything else (a path outside the checkout) as a diagnostic-log line; a plain click places the caret, and a drawn link never navigates the page.
Drawn HTML is cleaned by DOMPurify on an allowlist (paragraph, heading, list, table, `<a>`, `<img>`, `<details>` and the like; `align`, `width`, `height`, `alt`, `title`, `colspan`, `rowspan`): no `script`, `style`, `iframe`, `object`, `form`, event handler, inline style or script URL survives, and a link keeps only an http(s), `mailto:`, `#fragment` or relative address.
A link in drawn HTML opens as above, on modifier-click only (a relative file inside the checkout opens as a tab), and never navigates the page itself.
Images load from three places only.
A relative path (or one starting with `/`, taken from the checkout root) resolves against the document's folder and loads through the same file-bytes path an image View uses, so it works for a checkout on a connected device, and only inside the checkout in front; a path that leaves it, or a file that is not a png, jpg, gif, webp, avif, bmp or svg, is not loaded.
An `https:` address loads in the browser with no referrer.
A `data:image/*` address loads because it names its own bytes and opens no connection; it counts against the byte cap.
`http:`, `file:`, protocol-relative and every other scheme are not loaded, and an issue body, which has no folder, loads `https:` and `data:` images only.
An image that is not loaded, fails, or has not answered in 8 seconds (the request is cancelled) shows its alt text (its file name when it has none) in the muted caption size, with no banner; until then it takes no room.
A file that opens at its first line stays there while blocks and images above the fold are drawn and load; the first wheel, key, touch, caret move or edit ends that.
One Live text loads at most 64 images, 32 MiB of file and data bytes and 8 MiB for one file; past a cap the image shows its alt text and the diagnostic log records the crossing once.
A drawn block or image keeps its DOM, and its image, while the edit is outside it.
A document over 256 KB opens in Source with Live disabled and a notice that Live preview is off for files over that size, because Live re-parses the whole document after each edit.
That re-parse runs off the main thread, one at a time, with a burst of keystrokes coalescing into at most one more; attributes are re-applied only over the region whose plan changed.
No re-parse lands while an IME composition is marked, so composed input is not interrupted.

In both modes a Markdown document answers list-editing keys the same way: Enter after an item's text starts the next item with the same marker or the next number; Enter or Backspace on an empty item removes its marker and leaves the list; Tab and Shift-Tab move an item one level, with a numbered item counting in the column it joins and the column it left renumbering from 1; a lone `1. ` line stays as typed on Enter because it may be text.
None of this runs while a composition is marked, and a task box is not treated as a list item.
The text that results is ordinary Markdown, with nothing hidden or special in it.

## Projects and checkout context

Core owner: `herdr-core/src/sidebar.rs`, `herdr-core/src/project_context.rs`, `herdr-core/src/worktrees.rs`, `herdr-core/src/disk.rs` (measured on the node by `hide-host/src/disk.rs` and `hide-host/src/disk_layers.rs`), `herdr-core/src/worktree_cleanup.rs`, `herdr-core/src/runtime/projects.rs`. Web owner: `web/src/sidebar.tsx`, `web/src/projects.ts`.

### Sidebar type, rows and width

The project-row entry and Overview child-row requirements in PRD sidebar-row-click-overview-row D-03 and B5-B7 and web-project-overview are superseded by overview-remove.
Project rows now open checkouts; shared Overview owns the only Overview selection and destination.

The sidebar sets its words on three sizes: a checkout name, an agent title and a count 12/400, and a second line, a project name, a tab or group name and a time 11; a section header is 10/500 in sentence case (`Projects · Recent activity · 5`), 26 high with its words on the baseline.
A project's own name is its block's label, 11/600, muted while its agents only rest or sleep or it has none and in the ordinary text color while one of its awake agents draws a mark (the turn, the ring or a dot to read); the core decides which (`agent_presence.working`) and the next snapshot moves it.
A project's block is set off from the one above it by a hairline at its top and a 12px gap, so the last checkouts of one project never read as the next project's rows.
A project row and a one-line plain folder are 32 high, a checkout row 32 or 48 with its second line, an agent row 28 or 44.
A checkout name mutes its prefix up to and including the first slash (`prd/`), so the part that tells checkouts apart reads first; a name with no slash, or one that starts or ends with it, has no prefix.
The checkout whose Workspace is in front turns its name 500 and nothing else about the row.
These are the sizes at the default Interface text; the sidebar follows Appearance's Interface text with the rest of the interface, its rows growing with their text so every row still holds it.
The sidebar's right edge sets the width of its content column, the lists right of the device rail while one shows (see Device rail): under the pointer it shows a line and a column cursor, a drag moves the sidebar over the center between 220 and 440 and stops at either bound, and the release stores the width in the core's ui state, where it survives a reload and a relaunch; the center and its terminals take the new width once, on the release, never on each pointer move.
A double-click on the edge returns the width to 292, and a press that does not move changes nothing.
A width outside the bounds is refused into the diagnostic log: a sent one leaves the width as it was, and a stored one opens the sidebar at 292.

Sticky section markers update at most once per animation frame, with a pending frame cancelled when the list leaves.
Focus visibility uses current header geometry, so a row under a header is revealed even before its marker updates; a fully visible occurrence leaves scrolling unchanged.

### Sidebar hierarchy

This is the 프로젝트 list; the `프로젝트 | 태스크` switch above it, ⇧⌘B, and the 태스크 list are in [The sidebar's 태스크 list](#the-sidebars-태스크-list) (PRD sidebar-task-lens B1-B3).
The sidebar hierarchy is Project > Workspace > Group > Tab > Agents; a Workspace corresponds to one checkout path, including a plain folder.
Two checkouts of one repository share a project cycle.
A project is always a repository and a worktree is always a checkout under it: a worktree whose folder was removed while a pane still sits in it, or that a registration still names, stays a row of its repository, drawn as a missing folder, and is never a project of its own.
Only a folder that neither its node, Git's worktree list nor Herdr's record of the checkout places in a repository is a project row of its own ([ARCHITECTURE.md](ARCHITECTURE.md), Workspaces in the web shell).
Each checkout row shows a kind glyph: home for the stored primary checkout, otherwise the pull-request lifecycle icon when current GitHub data has a pull request, then branch, commit for detached HEAD, or folder for a plain folder.
A project row with checkouts draws no glyph of its own: its name starts on the row's inset and its checkouts' kind glyphs stand on that same column under it, one step shallower than a project glyph would put them (PRD sidebar-quiet-mark-indent D-06); its type is the block's label and the hairline above an open block is what tells it from the project above, while a folded project is one line set off by a small gap and no hairline; a plain folder's row keeps its folder glyph, which says its checkout's kind (D-07).
The primary choice belongs to the registered project and survives restart; a registration without a choice defaults to its root checkout.
Changing it moves the home glyph and first position together without moving focus; a vanished choice stays stored and is marked again when that checkout returns.
A checkout row's `Set as default checkout` sets it; the item is disabled with its reason on the checkout that already is the default, a plain folder, a checkout whose folder is missing, a project without a registration (pin it first), and a device's checkout, which is read-only for this action.
Remote catalogs still derive home from Git-root facts and do not yet display the primary checkout stored by the remote core.
Open, draft, merged, and closed pull requests keep their own lifecycle shapes and colors; they draw once a window is attached, without any other screen open, and a restart draws the previous run's answer muted as stale until the first read replaces it; stale GitHub data mutes only the icon, an unavailable GitHub lookup falls back to the branch glyph, and a missing folder colors its branch glyph as danger and omits the age.
In the web shell every row reads on the left and ends the same way on the right: its time or its status badge (a project or folder row has neither, and a GitHub project's row ends in its repository's mark instead), then a fold slot kept at rest, so nothing moves when a control shows and the times and badges of checkout and agent rows end on one column while their chevrons stand on another.
A folded chevron is always shown; an unfolded one shows under the pointer, while focus is inside the row, while its menu is open, and always on an input with no hover.
Under the pointer or keyboard focus a project or folder row shows `+` and `⋯` in a slot that keeps their width at rest, so the name never shortens when they show and they leave when the pointer does.
`+` (`Start agent in <project>`) opens the start panel aimed at the project's main checkout, or the folder itself; where that checkout cannot take an agent the panel picks its target by its own rule and the operator can change it there.
`⋯` (`More actions for <project>`) opens the row's menu under itself; both are keyboard stops and answer Enter and Space, and the menu keeps them shown while it is open.
Elsewhere nothing on a row stands for its menu: a right-click, or the menu key or ⇧F10 on the focused row, opens it, as it does on project and folder rows too.
Project, checkout and agent rows each have one (PRD sidebar-context-menus), and every other item runs at once; only `Remove project…`, `Delete worktree…` and `Close tab…` go through their existing confirmations.
A project row's menu is `New worktree…`, `New tab in main` (a new tab in the checkout the home glyph marks, brought to the front), then the reveal and `Copy path`, then `Pin` or `Unpin` and `Remove project…`, on every project row, registered or not.
A checkout row's menu is `Open` (the row's open, without unfolding its agents), `New tab here`, `Open pull request #n` while GitHub knows one, then `Set purpose…`, `Set as default checkout`, `Copy branch name`, `Copy path`, `Open in VS Code` and the reveal, then `Delete worktree…` in the destructive color on a linked worktree.
An agent row's menu, in Agents and under an opened checkout, is `Show` (the row's own open, with the ⌥n that selects the same row where the host has one), then `Copy name and ID` (the reference the pane menu copies) and `Copy session id` (the conversation id Herdr recorded, disabled when it recorded none), then `Close tab…`, which closes the tab holding the agent's pane, wherever it is, through the tab close flow, including its question about the agents spawned from it; Herdr 0.9.1 can neither mark a pane seen nor stop an agent, so neither is offered.
A Git project's menu has no `Open in VS Code`; a plain folder's has it before the reveal, since the folder is the checkout.
`Open in VS Code` is the desktop app's only and only while VS Code is installed on this machine, which the host checks once per launch: it opens the folder in a VS Code window, on another device's row it is disabled with the reveal's reason, and a failed open (VS Code or the folder gone) shows nothing and leaves its reason, never the path, in the desktop log (D-13, D-14).
`New tab in main` and `New tab here` show the registry's new-tab chord; the reveal follows [the reveal rule](#explorer-file-management): the desktop app's only, labelled by its OS, showing the folder selected in the OS file manager without opening anything.
On a device's rows the reveal and `Set as default checkout` are disabled with the reason; the rest act on that device as they do here.
`Delete worktree…` is never disabled on a linked worktree, on any device and before its Git state has been read; its confirmation says what would be lost and holds the choices, and reads `Reading the worktree's Git state…` until the row arrives.
Under the title and the path, one facts line says the folder is removed for good, how many panes close, and every agent those panes stop with its state; the core's warnings (uncommitted files with their count, a worktree inside it, the base branch, Git status unavailable, commits not merged, not pushed) follow as neutral badges.
`Keep worktree` is a neutral button, and only the widest action is in the destructive colour.
`Also delete branch <name>` deletes the branch with `git branch -d` when Git counts it merged, and otherwise with `git branch -D`, saying how many commits not on the base go with it or that Git could not tell; it is not offered for the base branch or a missing folder.
A locked worktree shows its name, Git's lock reason (or that Git supplied none), and the manual `git worktree unlock` action followed by refreshing its Git state; no Delete action is offered while locked.
A folder that holds uncommitted files, a worktree inside it, a status Git could not read, or ignored nested repositories shows a `Discard …` checkbox, and Delete stays disabled until it is ticked, because that loss cannot be undone.
The checkbox names every measured ignored repository by its path relative to the checkout, including repositories with gitfiles or separate Git directories; a warning badge counts the measured list.
An unreadable, invalid or bounded-out scan says why it could not verify the list and how to retry, and offers no Delete action.
A changed measured list, lock or scan availability clears the prior Discard choice, even when the previous facts return; the request carries the names actually shown.
Ticked, the folder is removed with one `git worktree remove --force`, which never unlocks it.
Before either deletion action closes any checkout or outside pane, `Rechecking Git state before closing any panes…` runs an authoritative host check.
A newly locked worktree, unavailable scan or changed repository set refuses with the reason, keeping all panes and files and requiring a fresh review.
When an agent in the worktree spawned agents that run outside it, the confirmation adds a short `N agents outside this worktree` heading over the same list, count line and states the close sheet draws, and its one Delete becomes two buttons named by their result on one row at the standard width: `Delete only`, the deletion as it is, and `Close N agents and delete`, the destructive one.
Both follow the dialog's own conditions, the Discard checkbox included, and neither holds the keyboard when the dialog opens; while one outside agent's status is unknown, the second is disabled and a `Check status` reads it again in place.
With the second, the core closes those agents deepest first as a subtree close does, and only once all are gone closes the worktree's panes and removes the folder; a close that is refused or times out starts no deletion and its reason shows in the dialog, and trying again continues from the agents that remain.
With the first, the agents outside keep running and the direct children become the operator's roots.
Files that change after the confirmation, typically written by an agent as it stops, stop an unticked removal with that reason and keep the worktree; every refusal and failure is shown in the dialog above the choices, and Delete tries again on the row as it is then.
From the confirmation until Git answers, the checkout row and its agent rows are dimmed with a spinner where the badge was, the row opens nothing, and a right-click or the menu key on it offers no menu at all rather than an empty one; the row leaves the sidebar as the removal finishes, and a failure gives it back as it was.
A copy that the clipboard refuses goes to the diagnostic log.
A web checkout row opens its checkout and unfolds its agent rows in one event; activating its already selected, unfolded Workspace folds the agents while keeping that Workspace in front.
An Overview opened over a Workspace while that event is still unanswered stays open when the checkout lands; the Workspace under the layer changes, the layer does not.
Click, Enter and Space have the same behavior, and a checkout without agents only opens.
Its chevron changes disclosure alone, and other checkouts keep their own expansion.
The core admits the workspace/checkout pair before changing focus or expansion and publishes both in the same snapshot; a stale pair changes neither and leaves a diagnostic.
A later Herdr projection failure retains these accepted values under the existing focus policy.
The last-commit age stays in place whatever the pointer does and while the menu is open.
A plain folder row puts its folder glyph where a project's name starts, so every top-level row shares one left edge, and its name and second line follow the glyph.
Opened folder projects use the same neutral selected fill and unboxed tab segments as opened checkouts, with the guide under the folder glyph.
Their required secondary lines and metadata remain visible.
An opened checkout keeps its rows on the sidebar background, without an enclosing card, border or group fill.
Checkouts start open, and `session_collapsed_checkout_ids` keeps only the checkouts the operator explicitly closes across launches.
An open checkout or plain folder lists its ungrouped collection and named groups in creation order, with tabs inside each group and the real agents beneath their tab.
The ungrouped collection wears a header only while the checkout holds a named group: a checkout with none draws its rows alone under the checkout, with no header, on the columns a checkout's own tabs have always stood on.
An empty ungrouped collection that is not the selected group draws nothing at all.
A group header stands one step in from the checkout's own row, its disclosure chevron on the checkout's kind-glyph column and its briefcase and name past it, so a collection never reads deeper than the rows beneath it, and the tab guide hangs under that briefcase.
The list uses the core's membership for every group, including groups whose canvases are hidden, rather than inferring membership from the front tab bar.
Empty groups, drafts and terminal-only tabs stay reachable, with no invented agent row or agent count.
Group disclosure changes only this page's fold state; selecting the group's name changes the checkout's selected group.
A tab line selects its owning group, area and tab, while an agent row continues to its pane.
The sidebar keeps a root and its complete lineage subtree together under the root's actual tab; children working in another tab do not inflate that tab's physical agent count.
Tab names use the manual name or localized `Tab N` from the core's stable number, while a draft reads New tab; automatic tabbar task titles remain independent.
The tab line's tooltip and accessible name give its name and recognized agent count, or its name alone when it holds no agent; shell panes never count as agents.
Every tab draws its guide, whatever it holds or is named, in both ungrouped and named collections, and every tab but an unnamed one holding one agent also draws its tab line: that tab's `Tab N` says nothing its one row does not, so its row stands beside the guide alone and opens its pane, while a named tab with one agent reads `<name>, 1 agent` and a terminal-only or operator-named empty tab keeps its line with its name alone.
A resolved session whose pane is still live stays a row under its tab, drawn dimmed whole like an unknown row with its mark and line from its own state and `해결됨 · ` leading its hover (PRD sidebar-every-tab-line B3); a live tab is never left as an empty tab line, and a closed pane's resolved session draws no row; a linked worktree whose only agents are resolved stays in the `Worktrees with no agents` fold, and shows the dimmed row once the fold is opened.
It takes a root shortcut number in physical order, opens its pane when pressed, and brightens again when a new request makes the session unresolved.
Group, tab and agent controls are stops in Arrow Up/Down and Home/End traversal, and click, Enter and Space use the same navigation.
Lineage children retain their `--size-lineage-indent` step and elbow rails, and each root's lineage disclosure stays in its trailing slot.
A tab has no disclosure state of its own; a stored tab fold from an earlier version is ignored on load and dropped on the next write.
A catalog without named-group metadata draws the same tab lines, and a named empty tab remains reachable.
A folded checkout keeps drawing and numbering only the roots that ask for the operator, and a resolved root asks for nothing even when its session ended on a question.
Focused sidebar agent and checkout rows use the neutral `--secondary` fill.
Root shortcuts follow the physical grouping order and deduplicate raised occurrences, with no numbers for group lines, tab lines or children.
A closed checkout draws its status badge and still shows questioning or blocked Needs You rows.
An agent row draws one of four marks in one 12 px slot ([status-model.md](status-model.md#the-mark)): the operator's turn as a `!` in a warning badge, in progress as a static three-quarter ring in the working colour (solid while the agent works, cut into dashes while it waits on proven work, status-marks D-25), to read as a 7 px dot that is green only when the label read the turn as finished, and quiet, an agent at rest, as a faint hollow ring in the muted colour, smaller and thinner than the in-progress ring (PRD sidebar-quiet-mark-indent D-01, D-02); a read turn keeps its badge dimmed.
A row of unknown state and a disconnected device's row keep the slot empty and are drawn dimmed whole, so neither reads as an agent at rest (D-04).
The mark's name and the word that leads its line are its hover text (`내 차례 · 질문`, `읽을 것 · 일 남음`); the line leads with that word (`질문 · …`, `끝남 · …`, `일 남음 · …`), and the end of checks the row waited on is its news (`CI 통과 · PR #12`, `CI 실패 · PR #12` in warning).
A working ring says its sentence alone or has no line; a waiting ring has a line in every list, the project tree included, led by what it waits for and its sentence or the cause's detail (`CI 기다림 · PR #12`, `자식 기다림 · 자식 2개 작업 중`, `백그라운드 기다림`, `답장 기다림`), or by `대기 · ` when no cause is proven, so the lead word tells working from waiting and never reads as waiting on the operator (status-marks B10, B29; needs-you-cleanup B17-B19).
A finished turn that left background tasks running draws the green dot and ends its line with `· 백그라운드 N`, and its hover with `백그라운드 N개 남음` (B27); at the narrowest sidebar the sentence is cut and the lead word and that tail stay.
An unreachable or unknown row draws no mark and is dimmed whole, its line saying why; a sleeping, waking or failed row keeps a grey moon and is not dimmed, the sleeping one with no line and `재움 · 열면 깨어남` on hover, the others with `깨우는 중` or `재움 · 다시 열지 못함` on their line (B31).
A status badge counts agents under the mark each agent's own row draws, one mark and count per mark in the order whose move is next (the turn, in progress, finished to read, to read), with quiet agents and zero counts left out (status-marks B19; docs/status-model.md, Workspace aggregation).
A web project row opens its device's last focused usable checkout, falling back to the primary checkout and then the first usable checkout.
It opens the Workspace and unfolds its checkouts in one admitted event; activating it while that target Workspace is in front and the project is unfolded folds the project while keeping the Workspace in front.
A project without a usable checkout opens the shared Overview at that project's scope.
Its chevron on the right folds and unfolds alone, without navigating; both folds are this machine's, so a selected SSH device's tree is drawn with nothing folded and its project row uses the same checkout selection rule.
A web project row carries no time and no agent counts: its checkouts and their agent rows say what runs, and a project whose agents all sleep folds as before without a moon cell.
A project whose GitHub repository is known ends in the GitHub mark, named `owner/repo` in its tooltip; pressing it opens `https://github.com/owner/repo` as a pull request's mark does (⌘ asks for the default browser) without selecting or opening the project.
The repository is the one Project Settings shows (`projectRepository` in `web/src/projectSettings.ts`): the confirmed override, else the detected one; a folder, a project with no GitHub repository and a non-GitHub remote draw no mark.
The mark is always shown, beside the `+` and `⋯` slot that appears on hover rather than inside it, so it stays pressable while they show, and it takes the row's other controls' hover, focus ring and keyboard reach.
Delegated children are reached through their root's tree, including work in another checkout or device.
The same root-only sidebar and 태스크 list rules apply while an SSH device is in front.
A checkout whose root came from another checkout prefixes its purpose line with the Return glyph and the parent checkout's branch, adding `+N` when more than one external root raised it.
Folding a project by its row or chevron, or using a checkout chevron changes the list only: the center, the focused pane and tab, read state, groups and running processes stay as they were.
Before the first snapshot arrives the Agents and Projects lists say they are connecting rather than drawing an empty list, and a local Projects list with no registered project says so in a line with no button of its own.
An `Add project` row sits directly under the Overview row, drawn whether or not a project is registered, with the current chord (`⇧⌘N`, `Alt+Shift+N` off macOS; none in a browser tab) on its right and no + on the section titles; it is the sidebar's one Add project (PRD first-run-onboarding B51, B52).
The sidebar has no Home row (PRD sidebar-titlebar-refresh D-06, D-07): a Home agent is found and opened from Needs You and Done, the 태스크 list and ⌘K, the ⌘N start panel can target the Home, and ⌥n numbers the visible rows without it.
Every device's sidebar starts with the Search row and the shared Overview row, under the device title in the window's top row, followed by one project/checkout/root list with no mode strip.
Overview shows its glyph, label, Needs You count and current shortcut; zero, an absent snapshot or a disconnected device hides the count.
The top row's Overview icon has an accessible Overview name and count description, a Needs You dot above zero and a tooltip with its current shortcut.
The top row's icons are Overview and Open server; the Files badge is on the rail.
⌘⇧P shows and focuses the sidebar, closing Overview first.
Retired `toggle_sidebar_view`, `project_home` and `sidebar_agents` (the removed Agents overview) stored bindings are ignored without losing other valid bindings, with a diagnostic.

A device that is not connected shows the top row and its reconnect view with no tab strip.
A device's Workspace that lost its connection keeps the last state on screen under one line, `<device> is not connected. Showing the last state it reported; nothing is sent until it reconnects.`, with `Reconnect`; the line names why the connection did not come up by the row's state (`Sign-in needed`, `Sign-in refused`, `Server key changed`, `Couldn't connect`), with the `ssh <alias>` to check a changed key, or Hide's sentence for a missing or old ssh, in its tooltip, and never carries the words the connection failed with, which are the device row's in Settings and the diagnostic log's.
A device waiting on a sign-in says `waiting for you to sign in` with `Open in Settings` in place of `Reconnect`, because the question is answered on its row there (`deviceAwayLine` in `web/src/settings.ts`, PRD system-ssh-device-link B15).
`Reconnect` pressed while the device still does not answer is a new attempt, not a new device: the last state stays, the Overview's age still says when it first stopped answering, and a Reinstall queued for it waits for the link.
A pane of a device whose link comes back by itself reads `reconnecting…` in its body and returns with the link (the device's Herdr answering again retries the link at once); `unavailable` is left for a pane that waits for the operator or has no terminal service.
The list shows the device's Needs You and Done and projects with their operator-owned roots.
Needs You and Done stand still above the projects: the project list scrolls under them and never carries them off, and past half the sidebar's height they scroll on their own.
Opening a raised row that leaves its section, a Done row once read, moves the next row up into its place, so the operator opens one after another from the same spot while the project list brings each one's tree row into view below (`web/e2e/projects-sidebar.spec.ts`).
Choosing a tile keeps each device's list apart, and ⌥n numbers its visible root rows in physical order.
The Search row at the sidebar's top, a field-shaped button reading `Search` with the current ⌘K chord (a rebound chord shows as bound), opens the ⌘K palette by a click or Enter on this machine or a selected SSH device alike; Add project is its own row under Overview (see Adding a project), and there is no bottom new-workspace button.
The Herdr status line sits under the top and above the list, and is not shown while a device is in front.
The web Projects list is the scope picker, starting at its first project.
The shared Overview row carries the selected fill and current-location marker while its modal or page is shown; project headings do not, and a project heading is filled only while a Workspace is in front and its fold hides the focused checkout (below).
Expanded projects contain checkout rows only, with no Overview child.
A Workspace selects its focused checkout and visible agent rows.
The sidebar's selected fill is the chosen accent at the selected-fill share (`--opacity-selected-fill`), apart from the neutral fill of the row under the pointer, so the row standing for the focus is never mistaken for a hovered one (`--color-selected` in `web/src/index.css`).
A focused delegated child has no row of its own, so the nearest ancestor row the list draws carries its selection.
While a fold hides the focused checkout's row, the row that hides it carries the selection instead: the folded project's row, or the Inactive, empty-worktree, cleanup or inactive-projects fold that holds it; no fold opens by itself, so the operator's folds stay as they left them.
When the focus moves, the list scrolls by as little as brings the selected row into view, the agent row before its checkout's, and not at all while one is already in view; an agent selected both raised and in its tree is brought in at its tree row.
A focus that has not moved since it was brought into view never scrolls the list, whether a snapshot arrives or the Workspace comes back in front from another screen, so a list the operator scrolled stays where they left it (`web/src/sidebarFocus.ts`, `web/src/sidebarFollowsFocus.test.tsx`).

A plain folder, a project that is not a Git repository and holds one checkout, is one web row instead of a project row over an identical checkout row.
Its first line is the project's folder glyph and name, set in the checkout row's columns, with no agent counts and no GitHub mark (a folder has no repository); its second line and trailing chevron are the checkout's, and a plain folder has no commit age.
It has no project fold of its own and keeps the checkout row's right slots; the row opens the checkout and is marked while that checkout's Workspace is in front, its menu lists the project's items and then the checkout's own `Open`, `Open pull request #n` and `Set purpose…` (the folder is the checkout, so its new tab, path and reveal items are the project's), and its project scope is reached from the shared Overview's scope tab.
While a checkout's agent rows are closed, its status badge ends line one; opening them takes the badge away, since their own marks now speak, and changes nothing else on the row.
A checkout's second line is its purpose, after the parent checkout it was raised from when there is one, with the last-commit age ending it on the time column; it is drawn only while the checkout's agent rows are folded and it has a purpose or a raising parent, so an opened checkout, or one with agents and neither, is one line.
A checkout with neither, or one whose Git facts have not been read yet, is one line, with its age on that line.
Workspace disclosure persists across launches and hides only the nested agent rows, preserving selection, running panes, and raised attention rows.
The web shell draws the raised groups at the top of the Projects list as `Needs You · N` and then `Done · N`, each left out while empty: the agents of that group whose pane a listed project's checkout or the Home owns, in the core's order, on the Agents list's own row with its place line, drawn whatever their project, checkout or parent has folded, never unfolded themselves, and opened as the Agents row opens.
A raised group draws at most its five (Needs You) or three (Done) most recent agents; the rest wait behind a `More N` fold row under them, styled as the inactive folds, which opens them below itself, and N in the group's heading stays the whole count.
The fold is the page's own and starts closed on a fresh load, so the list returns to its short form; an agent folded there keeps its ⌥n number on its checkout row.
A device's Projects list is its `Needs You · N` group, its `Done · N` group, then Pinned and the projects, then `No agents N` and `Inactive projects N`, and no row there names the device, since the whole list is that device's (with the rail rework of quick device-rail-badges).
An agent row's title is its identity label at both densities: the rolling task, or the provider's name when no task exists; a Herdr agent name and a Herdr workspace label never become display copy.
Worktrees with no agents folds worktrees with no unresolved agent, excluding primary, front, dirty, unpushed and already inactive checkouts and one holding a named tab the operator kept ([Agent areas](#agent-areas)); a worktree whose unresolved agents are all delegated children is in no fold and has no line (`session_folds.child_only`, same exclusions), because Worktrees with no agents would be false for it; Cleanup gathers agent worktrees and missing folders at the bottom without assuming they have no live pane.

### Purpose, pinning, and PR chrome

A checkout's one-line purpose is set from the checkout row's or Overview header's `Set purpose…` context item, with a character count and a warning near the limit; saving an empty purpose clears it, and display falls back through branch description, representative agent title, and pull-request title in that order.
Any project row can be pinned from its right-click menu: a row Herdr shows without a registration is registered with its device and root by the same Pin, so pinning never needs a separate add, unless its folder is outside that machine's home, which is not registered without Add a project's question and says `홈 밖 폴더는 프로젝트 추가에서 확인한 뒤에 고정할 수 있어요.` instead (PRD wsl-device B18); pinned projects are drawn once under a `Pinned · N` section, its header led by a pin glyph and its rows carrying none, between the raised groups and the activity-ordered project list, one hairline setting each section off from the one above, the first from the raised groups when there are any, in the tree's own order (device first, then latest activity), and only while at least one project is pinned.
The pin lives on the project's registration and survives a relaunch; removing the registration takes the pin with it.
A pinned project is exempt from its device's inactive fold whatever its activity; its own stale worktrees still fold behind their own `Inactive N` row.
PR state colour is a semantic-colour exception to otherwise neutral chrome, one colour per state on every surface, including during hover and selection (docs/status-model.md, Pull request visual states; operator approval, 2026-10-10): failed red, pending amber, mergeable green, draft grey, merged purple with the merge icon, and closed dim.
Every surface draws a pull request as the one PR mark (`web/src/components/pr-mark.tsx`): the state's icon in its colour and `#N` in the text colour, or, for a row or a root standing for several PRs, each state's icon and count in that state's colour, worst first, with no border and no glyph; a pressable mark fills under the pointer, a mark whose GitHub read is stale dims with the last read time in its tooltip, and where the row already names the pull request (a checkout's glyph, a search row) the mark drops the number.
Only a change request keeps a word beside the mark, `변경 요청` in red, as failed checks are; the Overview's task card and the PRs row (a second-line reason there) draw no separate checks mark.
On the web a checkout row whose glyph is a pull request's mark makes that glyph a button (PRD checkout-pr-glyph-card): a ring in the state's colour under the pointer and the pointer cursor say so, a click opens the pull request as a browser display of the Workspace in front (an address the Workspace already shows is brought forward instead), and ⌘-click opens it in the default browser.
While no Workspace is in front (the shared Overview page) or the checkout is an SSH device's, the click opens the default browser too.
The press never reaches the row, so the checkout neither opens nor unfolds; every other kind glyph (home, branch, commit, folder) is inert and a click there is the row's.
The row's menu offers `Open pull request #n` after `Open` and `New tab here` while GitHub knows one, with the same open as a plain click.
Hovering or keyboard-focusing a checkout row opens a card after the tooltip's delay, in place of the text tooltip (`Component / PR hover card`, `web/src/components/pr-card.tsx`): the pull request's badge, its number and `Open PR ↗` (the card's one control, opened as the glyph is), the title on up to two lines, a rule, then `Review`, `Checks`, `Branch`, `Agents`, `Commit` and `Path`, each row present only where the snapshot has the value.
The badge is the state's icon in its colour and the lifecycle word (`Merged`, `Closed`, `Draft`, `Open`) or, under review, the decision (`Approved`, `Changes requested`, `Review required`) in the text colour, with `Draft` muted beside a draft's decision; `Review` repeats the decision (`Approved` green, `Changes requested` red, `Review required` muted), `Checks` reads `Passing` green, `Failed` red or `Pending` amber and is left out while the checks are none or unknown; `Branch` is the worktree's branch or `Detached HEAD at <short sha>`; `Agents` draws the sidebar badge's marks and counts while any agent runs there; `Commit` is the last commit's age once Git has been read; `Path` is the checkout's full path in mono.
A checkout with no pull request has the same card without the badge line; a folder that is gone has `Folder missing` in the danger color over its path alone; a card that would hold one value and no header (a plain folder with no agents) is the plain text tooltip with that value.
On a local Git project's checkout the card's head adds `PR 카드 열기`, whose popover says `이 PR의 이슈와 에이전트 계보`, and whose click opens the pull request's card.
The card stays while the pointer crosses onto it, closes when the pointer leaves the row and the card, on Escape, and on a press on the row; a screen reader reads the row's detail sentence (the pull request, the agents by state, the branch and the path) as before.

### Project ordering and inactive folding

Projects and their checkouts sort by the latest authoritative agent activity timestamp or Git commit timestamp, descending; missing activity sorts after known activity, and no UI interaction or local clock invents recency.
Inside a project the primary checkout comes first whatever its activity, because it is the one checkout that never folds, and the rest follow in that activity order.
Activity orders projects inside one device group and never across two.
A project's secondary checkouts that are merged (as the Overview says) or whose pull request closed, and those inactive for seven days, move behind a trailing `Inactive N` disclosure, while its primary checkout and every checkout with live work, local changes, unpushed commits, or current focus remain visible.
A project of a node that dials the core lists every worktree that node reports, one with no pane open included, and folds them by these same rules; a device the core dials shows only the checkouts its session opens.
When every checkout in a project is inactive, the project itself moves behind the device's `Inactive projects N` disclosure.
A project with no agent row in any checkout, no named tab the operator kept, and that is not pinned, not the front project and not inactive, moves behind `No agents N` after the active projects, where opening it draws the same project rows; an agent started there brings the project back to the active list on the next snapshot (D-01).
Its fold is the core's per device (`no_agents/<device>`), so it survives a reconnect and a relaunch.
All three folds default closed and remember their expansion independently.
A project or folder whose agents all sleep is drawn folded (D-03); it shows no moon cell, since a project row no longer draws agent counts, and opening it shows its checkouts and sleeping rows as before, and it stays open until one of its agents wakes or starts, after which it folds again the next time all of them sleep (`asleep/<project>`, pruned by the core on waking, D-19).
While a folded project hides the checkout in front, its row carries the selection fill.

The `Pinned · N` and `Projects · Recent activity · N` headers stick to the list's top while it scrolls, the next header pushing the one before it off, with a hairline under the stuck one; a row brought into view by the keyboard lands fully under a stuck header, never behind it (D-05).
Search continues to index the complete project tree; choosing a folded result brings the focused row back into the active list without opening either fold.

### Tools and Overview

The rail's tools are the Project tool, Memory, the Explorer and Changes (History), with the documents button and, while it holds something, the 보기 slot's button above them (PRD lane-two-columns B2, B3); the device's task list is the sidebar's, described in [The sidebar's 태스크 list](#the-sidebars-태스크-list).
The top row's Overview button opens [Overview](#overview), `Work · Projects · Backlog · History · Usage`, with device and optional project scope for the four device views and global core-machine scope for Usage.


### Disk allocation and cleanup

Web owner: `web/src/DiskCleanupSheet.tsx`, `web/src/diskCleanup.ts`; core owner: `herdr-core/src/disk.rs`, `herdr-core/src/worktree_cleanup.rs`; node owner: `hide-host/src/disk.rs`, `hide-host/src/disk_layers.rs`, `hide-host/src/cleanup.rs`.
Allocated-on-disk sums main, linked worktree folders, and the shared Git directory once; nested roots belong to the longest matching root, hard links share one inode allocation, and descendant symlinks are not followed.
Each checkout is measured under its own limit of one million entries and 30 seconds, so a checkout that exceeds it or cannot be read is the only row that has no size; the reason is in the diagnostic log.
An incomplete measurement has no total, and allocated blocks are not a promise of reclaimable space, so the result states only the volume's free space before and after.
A node's project is measured and cleaned up on that node; a project of a device the core dials is neither measured nor cleaned.
The Overview's [Projects](#projects) tab is where a project's size shows: while it is on screen the core measures its projects one at a time in the order shown and keeps each answer for ten minutes, and its disk number, `머지됨 N → 정리` and the project panel's `정리` open the sheet below.

Only what the ignore rules hide can fall into a layer, so every path git sees is worktree source and is never counted in a layer cell.
An ignored folder holding a valid `CACHEDIR.TAG` is build cache, one Hide's ecosystem table names beside its marker file is build cache or dependencies, and every other ignored path is Other.
Measurement reads no git index, so a tracked file inside an ignored folder is found only when the cleanup runs, and that cell is then skipped.
The table covers Cargo, Node, Python, Gradle and Maven, Swift, Dart, Elixir, .NET and Composer, and applies only to ignored folders; a symlink, a folder holding another git repository and a folder holding tracked files are never removed.
Other has no checkbox and its folders are never removed, because Hide cannot tell whether what a tool made there can be rebuilt.

The sheet is one large dialog with no subtitle: a stacked usage bar whose segments say their layer and size on hover, the filter, and a table with a row per checkout and columns Build cache, Dependencies, Worktree, Other and a total.
A column head is its name and the column's total for the rows shown, with no second line.
Main leads, the rest follow by size, and checkouts under 1 GB fold into `작은 체크아웃 N · X GB`; a row that is still being measured is a skeleton with a disabled checkbox, and a row that could not be measured is dimmed with no size and cannot be chosen.
The filter is All, Finished, Resting and Working, each with its count: Finished is a linked checkout the Overview already calls done that nothing is using, Working is a checkout something is using, and Resting is the rest.
Checkboxes sit on each cell, on each row (its build cache and dependencies, never its worktree), on each column head and at the top left, and they reach only the rows the filter shows; a group that is partly chosen shows the middle bar, and a cell that cannot be chosen is skipped by every group.
Choosing a worktree cell shows the same row's cache cells as included: they cannot be chosen apart and are not counted twice, and they leave with the worktree folder.
A checkout is in use while an agent there is working or waiting on the operator (a question, an approval or a blocked prompt, or an activity Herdr does not report), an agent it delegated to is working or waiting wherever that agent runs, a terminal pane in it runs a process that is not a shell, or a server listens on a port opened from inside it.
A finished or idle agent and a shell prompt are not use: a pane that is merely open blocks neither a cache nor the worktree.
A worktree cell can be chosen only for a linked checkout that is merged into the repository's default branch, clean, is not the checkout in front, is not locked and holds no nested git repository, and has no live pane that is not one of its own; otherwise it is disabled and its tooltip says which.
The default branch is the base branch chosen in Project Settings, else the one GitHub names, else what `origin/HEAD` names (PRD git-freshness D-07), else, in a repository with no remote at all (none in its config and nothing under `refs/remotes`), the branch its main worktree has out once it holds a commit, which is what a clone of it would take as its default; never a branch the code assumes, and in a repository with a remote the main worktree's branch never counts (`hide_project::git::Repository::own_default_branch`, shared with the base branch protection's Auto).
Merged is the one judgment the sidebar's landed mark also reads (`worktrees::merged` in `herdr-core/src/worktrees.rs`, D-06): HEAD is in the local default branch or in its `origin/` copy as of the last fetch (the review never fetches; the background fetch keeps that copy current), or a pull request GitHub merged into the default branch has exactly this HEAD as its head; a squash or rebase merge needs that last proof, and it needs no merge commit in a local ref.
GitHub's answer is the pull request list the project's read already holds, the last one kept while a read fails; that list holds only the 200 newest pull requests, so the review asks GitHub by branch for a HEAD the list does not name, and a worktree whose lookup fails is unverified rather than not merged, and a device's project, whose pull requests this core does not read, always asks by branch; with no GitHub answer only the ancestry counts.
A head the review proved by branch is kept by the core for that project and default branch (at most 1024, `review_merge_proofs`) and joined to the list the sidebar's landed mark reads, so the row is marked landed when the review offers it (B17); before any review the sidebar knows only the list.
A HEAD that moved past its merged pull request's head is not merged, a pull request merged into another branch proves nothing until that branch is in the default branch, and with no known default branch, or neither it nor its `origin/` copy present, the sheet judges no worktree.
Deletion still happens only when the operator confirms the reviewed rows; the Hide AI attempt retirement keeps its own rule (GitHub answering that attempt's pull request merged or closed).
When Hide cannot read what is in use, the sheet says so above the table with a retry and every checkbox is disabled; no banner or alert appears.

The bottom line reads `항목 N개 · X` (`항목 N개 · 워크트리 M · X` when a worktree is chosen, and a part that is zero is left out, so a worktree alone reads `워크트리 M · X`) and `정리` runs at once for caches and dependencies alone.
A chosen worktree whose finished agents and shells still have panes shows a terminal mark with their count beside its size, in the cell and again in the confirmation, which is the state shown instead of a sentence about it.
With a worktree chosen a confirmation step titled `<branch> 폴더째 삭제` (`워크트리 N개 폴더째 삭제` for more than one) says only that the branch stays, lists the worktrees with their sizes and pane counts and offers `워크트리 N개와 캐시 정리` (`터미널 P개 닫고 캐시와 워크트리 N개 정리` when panes close, the same consequence-first naming as Delete worktree's `Close N panes and delete`) and `돌아가기`, neither focused; going back deletes nothing and keeps the choice.
Cleanup deletes cache and dependency folders permanently, with no trash, and removes worktrees without force, keeping the branch.
Each cell is checked again when `정리` is pressed and again per folder: a cell that became in use, a worktree that changed and a folder that gained tracked files are skipped with their reason and the rest go on.
A worktree's panes are listed again just before its own removal and closed through the same Herdr close and confirmation Delete worktree uses, then the folder is removed without force; a pane that does not close leaves the worktree in place as a failed row (`터미널을 닫지 못함`) and the other rows go on.
A finished descendant that runs in another checkout is not closed with its parent's worktree: it stays as its own root.
A folder is moved into the repository's Git directory (`hide-removed`) and disappears from the checkout at once; the cleanup thread then deletes it, and the sheet reads `비우는 중 · N/M`, keeps going when the sheet is closed and shows its progress or result when opened again.
Only one cleanup runs in a daemon, and pressing the same confirmation twice removes each folder once.
When the cleanup ends the result is the one line `여유 A → B GB` over each cell as removed, skipped or failed with its reason; `다시 검토` measures again and `닫기` closes the sheet.
The Projects tab's disk cell, its tooltip and its top line then read the new measurement.
Every removed, skipped and failed cell leaves one diagnostic line with its kind, checkout, layer, bytes and reason code and no file contents.
Hover, filter and checkbox changes are local: they dispatch no core event and start no disk work, opening the sheet sends one review event and `정리` one confirm event.

### Adding a project

Web owner: `web/src/AddProjectDialog.tsx`, `web/src/addProject.ts`; desktop owner: the `hide:pick-folder` channel in `desktop/src/main/host.ts`.
Add project opens one centered `Add a project` dialog from each of its entry points: the Add project row under Overview, the empty Home's center button (B27), ⌘⇧N and the File menu; Escape and × close it.
With projects registered the Add project row under Overview is the only one on screen; with none it and the empty Home's button are the two (PRD first-run-onboarding B51, B52).
A browser tab offers the same entry points without the chord (Chrome keeps ⇧⌘N); having no folder picker, it adds a project on this machine by a typed path, the `~/` field a remote host gets, which hided's boundary reads against this machine's home and asks about outside it as below, and the core answers `already a project` for a folder registered already (`hostWays` in `web/src/addProject.ts`, `Boundary::resolve_workspace` in `hided/src/boundary.rs`).
The Host selector lists this Mac and every registered device, and starts on the focused device.
This Mac here is the machine the window runs on, which on a window of a machine whose core runs elsewhere is not the core's (`hostWays` in `web/src/addProject.ts`).
On this Mac the primary `Browse folder` row holds the keyboard, so Enter activates it; it opens macOS's own folder picker as a sheet on the window, which can also make a new folder, and a cancelled pick leaves the dialog as it was.
The chosen folder is registered with one `create_workspace`; the dialog closes when the project appears, and says `Adding <folder>…` until then.
A project added on this machine, whichever way below and from the first-run wizard too, opens in front on the one terminal tab its registration starts there (`spawn_workspace_creation` in `herdr-core/src/live.rs`), never on the empty start screen, however many checkouts Hide has remembered before (`WorkspaceViews::entry` in `herdr-core/src/workspace_views.rs`).
A project added on a device opens on one terminal tab there once that device's helper has confirmed the folder, the terminal New tab opens; a folder whose checkout already has a tab there keeps what it has, and a terminal that cannot be opened leaves the project registered on its start screen with no notice, the reason going to the diagnostic log (`open_registration_terminals` in `herdr-core/src/runtime/device_catalog.rs`).
A refusal stays inside the dialog as an alert naming the folder, and the dialog stays open for another pick: `already_registered` from the shell's own registrations, before anything is sent, and `outside_home`, `home_root`, `not_found`, `not_a_directory` or `invalid_path` from hided's `$HOME` line, or the core's own error.
A folder in a Git repository that holds the whole home folder (dotfiles kept in `~/.git`), on this machine or a device, has its own line, `홈 폴더 전체를 담은 Git 저장소 안의 폴더라 프로젝트로 추가할 수 없어요.` (`This folder is inside a Git repository that holds your whole home folder, so it can't be added as a project.`), from hided's `project_holds_home` or a helper's `workspace.create_refused.project_holds_home`, so it is not read as home itself.
A folder outside home, picked or typed, on this machine or a device (PRD wsl-device B15, B21), is asked about once instead: hided's `needs_confirmation`, or a device helper's `workspace.create_refused.needs_confirmation`, shows `홈 밖 폴더예요. 이 폴더를 프로젝트로 추가할까요?` (`This folder is outside your home folder. Add it as a project?`) with the path of what would be added (the folder, or the root of the project outside home it belongs to, such as the repository a subfolder or a linked worktree is in, so the operator is asked once) and an `추가` button that holds the keyboard, and, when the folder is under the PC's C drive as WSL shows it (`/mnt/c`), the line `C드라이브 폴더는 WSL에서 조금 느릴 수 있어요.`; `추가` sends the path shown with its confirmation; the dialog closes when the project appears or keeps the folder's own refusal (`not_found`, `not_a_directory`, `home_root`, `outside_home` for a link under home that leads out of it) with the dialog open.
Nothing keeps the answer: picking another folder, changing the host, closing the dialog, and `Remove project…` on the project it added all end it, so adding that folder again asks again, and no setting lists allowed folders (B20).
Another machine's folders are not this window's to browse, so on a device, the core's machine seen from a node's window included, the dialog shows a `~/` path field instead, sent to that machine's node through the same event, with its refusal shown the same way.
On a device whose helper reports where it shows the Windows PC's drives (a WSL distribution, `host.windows_drives`), a Windows path pasted into that field, `C:\work\app`, `C:/work/app` or Explorer's quoted `Copy as path`, is shown at once as the device's own `/mnt/c/work/app`, and one typed is spelled so when it is sent (`wslDrivePath` in `web/src/addProject.ts`, PRD wsl-device B16); any other device, an ordinary Linux server included, keeps the text as written.
On the core's machine, the one a new project can be made on, an `Other ways to add` group sits under Browse folder or the path field, one row per way, each opening its own view of the same dialog with a `Back` that returns to the first view and puts the keyboard on the row it came from; on a node's window it sits under the core's machine and its views offer no folder picker, since the picker would show this machine's folders.
`Clone from URL` (web owner: `web/src/CloneFromUrl.tsx`) shows a `Git URL` field that holds the keyboard, and a `Parent folder` field with a button that opens the same native folder picker; the parent starts in the folder most local projects live in (the latest added wins a tie), else at home (`~`); Create new project starts there too.
The folder the URL names is shown as `Clones into a new folder <name>`, and hided is asked whether it is free under that parent, so `Clone` is enabled only for an https, ssh (`git@host:path`, `ssh://`) or `file://` URL, a parent inside home, and a name nothing holds there yet (`already_exists` covers even an empty folder, since a clone never writes into a folder it did not make); each refusal is shown inline under its field.
While the clone runs the dialog shows Git's stage, worded in the window's language (`web/src/coreError.ts`), and percent with a progress bar and a `Cancel` button, and the fields and `Back` wait; closing the dialog leaves the clone running, and opening it again shows it.
A finished clone is registered like a picked folder and the dialog closes when the project appears, or says it could not be added and that the repository is on disk for Browse folder; a failure (authentication, a stalled transfer, a missing repository) stays inline in plain words chosen by the core's `failure` code, with Git's own last line beneath it when Git gave one, and `Clone` enabled again, and a cancel says nothing was kept.
`Create new project` (web owner: `web/src/CreateProjectView.tsx`) shows a focused `Name` field and a location row naming the folder it goes in (`Will be created in ~/…`) with the full path under it, which follows the name as it is typed.
The folder starts as the parent of the most recently added project on this Mac, else home; the location row opens the native folder picker to choose another.
hided answers a `project_target` probe for every name and folder on the same `$HOME` line the create is checked on, so a name that is not one folder, a folder outside home, something already standing at the path (`already_exists`) or a project already added there is said inline and `Create project` stays disabled until the path is free.
`Create project` (or Enter) sends one `create_workspace` with `new_folder`; the core makes the folder, runs `git init` in it and registers it, and the dialog closes when the project appears or keeps the failure with its reason.
A folder holding nothing, or only `.git` and a Finder `.DS_Store`, is what a create that failed after making its folder leaves, so it is not `already_exists`: the view says the project is made in it, and a retry continues there rather than being refused or making a second folder; a failed create never deletes the folder it made.

### Opening a folder from a terminal

CLI owner: `open_path` in `hided/src/cli.rs`; daemon owner: `hided/src/project_open.rs`; core owner: `herdr-core/src/runtime/project_open.rs`; page owner: `showProjectOpened` in `web/src/opening.ts`; desktop owner: the `hide:come-forward` channel in `desktop/src/main/host.ts`.
`hide .`, `hide <path>` and `hide open <path>` open a folder as a project from any terminal, the way `code .` opens VS Code.
The path is read on the machine the command runs on, from that shell's working directory.
A folder a registered checkout holds, the checkout itself or a folder inside it, brings that checkout in front, the deepest one when worktrees nest, and registers nothing.
Any other folder is registered by the same `create_workspace` Browse folder sends, so it is judged by the same rules and opens on the first terminal tab its registration starts; a folder outside home is refused with the advice to add it from Add a project, since only the dialog asks that question, unless a project the operator already added holds it: that registration was the answer, so the checkout comes forward as one under home does, and a link inside it that leads out of it is still asked about.
Then every Hide window of the core's machine shows that project in place of All projects or an Overview, as a click on its sidebar row does, and the desktop window comes forward; a browser tab shows it too but cannot raise itself.
With no window open the command opens Hide as bare `hide` does, already on that project, and with no Hide running it starts Hide first.
A path that is not a folder, or one that cannot be registered, is answered with the reason and what to do next, and the command exits non-zero; nothing else opens.
A command's name wins over a folder of the same name: `hide status` is the command, `hide ./status` opens the folder `status`, and a word that is neither is answered as neither.
On a connected device the command works in a Hide pane there: the device's helper judges the folder against that device's home, the device comes forward on the project, and its first terminal opens there once the helper has confirmed the folder.
A plain SSH shell on the device works the same way while the device is connected: its helper takes this one command from a process of the account it runs as, and nothing else that shell runs gains a pane's reach ([INSTALL.md](INSTALL.md#what-each-system-supports)).
There it runs one open at a time and six a minute; past either, or with the device not connected or open in more than one Hide, the shell hears why and what to do, and the command exits non-zero.

### Removing a project's registration

`Remove project…` removes only Hide's registration and never deletes files, worktrees, sessions, or Herdr workspaces.
A project with panes is not refused; under the title and the path one facts line names the panes that close, the running agents that stop (left out at zero), that only the registration goes (a row Herdr shows without one goes with its panes), and that files stay on disk; `Keep project` is a neutral button.
On confirmation the core closes every pane in the project's checkouts and waits for confirmation before removing the registration and its row; a timeout or refusal leaves the project registered with the reason in the error banner, and a repeated request continues from the panes that remain.
When an agent in the project spawned agents that run outside it, the confirmation lists them as Delete worktree does, with `Remove only` and `Close N agents and remove` (the destructive one) in place of its one button, neither holding the keyboard; the second closes those agents deepest first and removes the project only once all are gone, a refusal or timeout leaving the project registered with the reason in the error banner and a retry continuing from the agents that remain.
Removing the project that holds the focused checkout moves focus and pane selection to the next project.
An add that lands mid-removal cancels the removal and says so, rather than losing the project it just opened a pane in; a completed removal disappears from the snapshot and a repeated request is a quiet no-op.
A row Herdr shows without a registration offers `Remove project…` too: its confirmation counts its panes the same way and says the row goes with them, the confirmation closes those panes, and the row leaves once Herdr drops its workspace; a timeout leaves the row with the reason in the error banner, and repeating the removal closes the panes that remain.

## Home

Core owner: `herdr-core/src/runtime/home.rs`, `hide-host/src/home.rs` (see ARCHITECTURE.md, Home). Web owner: `web/src/devices.ts`; the sidebar draws no Home row (see Sidebar hierarchy).

Each device has one Home, `~/hide` in that account's home directory, for work that belongs to no project or to several.
Nothing is made on a device until its Home is first used: the first agent or tab started there makes `~/hide` with one link per project registered on that device and Hide's `AGENTS.md` with a `CLAUDE.md` link to it, and a device whose Home was never used has no `~/hide`.
Once a device has a Home, registering a project there adds its link and removing the registration removes only that link, never the project's folder; two projects with one folder name get distinct link names.
A link whose project folder moved away is dropped at the next sync with nothing on screen, only a diagnostic line.
A `~/hide` that Hide did not make is left untouched, and the start that wanted it says so where it was asked for, in the start panel, with what to do.
A Home agent reads and edits the projects through their links, the change shows on that project's checkout row, and its own row is found in Needs You and Done, the 태스크 list and ⌘K.

## First run

The startup's real connecting state draws one flying owl around the lowercase hide mark and brand line, then a thin indeterminate bar with the host's localized preparing sentence.
It shows no progress percentage and never delays the shell; failure stops and hides the loading scene so the existing Retry and Copy diagnostics remain primary.
Reduced motion keeps a static owl and bar; hidden or offscreen decoration pauses, and leaving its screen releases its animation resources.
The wizard's existing agents, notifications and project steps carry their matching brand owl without changing their actions or platform policies.
The ready Home with no projects carries the ai owl.


Core owner: `herdr-core/src/runtime/first_run.rs`, `herdr-core/src/runtime/herdr_takeover.rs` (the Herdr step), `herdr-core/src/runtime/kit.rs` (`apply_agent_onboarding`). Web owner: `web/src/FirstRunWizard.tsx`, `web/src/firstRun.ts`, `web/src/HerdrTakeover.tsx`, `web/src/herdrTakeoverRules.ts`, `web/src/NotificationControls.tsx`, `web/src/agentNotices.ts`, `web/src/ScreenIntro.tsx`. Desktop owner: `desktop/src/main/notify.ts`, `desktop/src/main/notificationPermission.ts`, `desktop/src/main/host.ts` (`showBadge`, `openNotification`), `desktop/static/status.html`.

The desktop app shows `Starting Hide…` while it reaches Hide, and never the word hided; a failure shows its reason in one line with Retry and `Copy diagnostics`, which copies the app's build, the reason code and where the host log is, and shows the same text to select by hand when the clipboard refuses it (PRD first-run-onboarding B1, B2).
On a machine whose kit has never run, the first-run wizard opens over the shell: Agents, Devices when this computer knows another machine, Notifications and how to start.
When the Herdr server on Hide's socket is the operator's own Herdr at an older stable version than the one Hide carries, the wizard opens first on a Herdr step, which shows only while there is such a server; the core looks at the server off its lock for at most eight seconds, and a look that finds nothing goes on to Agents (PRD herdr-takeover B1, B17, D-09).
Most operators do not know what Herdr is, so Hide does every step it safely can itself and each screen is a title saying what happens, a few ✓ and ! lines saying what the step keeps and ends, and one button (operator decision 2026-10-23, which replaces PRD herdr-takeover D-14).
A look that does not answer in that time, or cannot read the server, is a failure, not nothing: while Hide cannot use the server the core asks again three times, three seconds apart, without a word, and only then the step says `Couldn't read Herdr` with `Your Herdr is unchanged.`, `Try again`, which looks again, and `Not now`; while Hide uses the server it shows nothing and leaves only a `herdr.handoff` diagnostic record (B8, B9).
A server an earlier Hide app started is still handed over by `hide connect` without asking, and a server at Hide's version, a newer stable or a preview on Hide's protocol gets no step (B14, B15).
On macOS and Linux the core starts the move as soon as a look offers it, with no press: the step is `Moving Herdr to <version>` with a spinner, the version the operator runs now (`Now`) beside Hide's (`After the move`), `✓ Tabs and agents keep running` and `! A Herdr screen open in a terminal disconnects once`, with no counts, no description and no button; the panes' processes keep running (B2, B3, B4).
The move is tried on its own once per server process and version: a move that fails leaves the old server and its work as they were, and the step names the result in one line (`Herdr declined. Nothing changed.`, `The move didn't finish. Try again.` or `Couldn't read Herdr.`) with `Try again`, which moves it on the press; Hide remembers that server process in `herdr-handoff.json` and never moves it again on its own, though the button always does, and forgets it once the process ends or the carried Herdr changes (B8, D-23).
A move that lands shows `Moved` with `Your work keeps running.` and Continue (⌘↵, `Ctrl+Enter` off macOS), which goes on to Agents.
When the operator's `herdr` on PATH answers an older protocol than the moved server, Hide brings it up itself, once per move, off its lock: `brew upgrade herdr` for a `herdr` Homebrew installed (one under `Cellar/herdr`), and otherwise `install -m 755 '<Hide's herdr>' '<that file>'`, which changes only the file the terminal finds and never the running server (B5).
While it runs the step says `Updating the herdr in your terminal`, and once that `herdr` speaks the server's protocol `✓ The herdr in your terminal is now <version> too`.
A command that would need `sudo` (the account cannot write that folder) is never run, and neither is one on Windows, which has no move; when Hide did not run it, or it ran and the `herdr` still speaks another protocol (Homebrew's latest can be newer than Hide's pin), the step says `! Update the herdr in your terminal (<version>) too. Until then its commands are refused.` with the command and Copy, and the command's output goes to the `herdr.path_update` diagnostic record.
Settings > About shows the same command with what it does: `Updates it with Homebrew`, or `Replaces it with Hide's herdr <version>` (Windows: `Copy-Item -Force`).
`herdr update` is not offered: it refuses while a server runs on 0.7.5 and 0.8.2, and its `--handoff` would hand Hide's server to Herdr's latest release, which a Hide pinned to an older one cannot read (measured 2026-10-23).
Agents in Hide's panes call Herdr as `"${HERDR_BIN_PATH:-herdr}"`, the Herdr that serves the pane, so an older `herdr` on PATH refuses only the operator's own terminal.
Outside the wizard a move shows nothing unless the terminal's `herdr` could not be brought up, which leaves that one line with the command above the shell until `OK`.
A server on Hide's protocol (0.9.x) is moved on its own too, and offers `Not now` whenever it is not moving, which goes on to Agents without moving it; one on an older protocol is the screen that holds Hide, below, with no `Not now` (B6, B7, B9).
Where Herdr says it cannot hand a running server over (Windows, or a release older than 0.6.3), the step is `Restart Herdr` for any older stable server, whoever started it, since a restart ends running commands and so still waits for the press: the same two versions (`Now`, `After the restart`), `✓ Tabs and folders come back` and `! Running commands end. Press after your builds finish`, and `Restart` is a destructive button with no keycap and no initial focus, so nothing changes before it is pressed; a server on Hide's protocol offers `Not now` here too (B10-B12).
`Restart` is the desktop host's: it stops the server with the carried Herdr, waits for it to end and starts Hide's Herdr, which restores the saved workspaces and panes; a server that does not stop says `Herdr didn't stop. Nothing changed.`, and a start that fails shows the startup failure screen with its reason, Retry and `Copy diagnostics` (B12, B13).
The core holds that failure, as it holds a failed move's, for as long as that server answers, and the step keeps `Restart` beside `Not now` (D-29); a restart that went through asks the core to look at the server again, since the connection state may not have moved, and holds its button for up to 15 seconds while that look takes the offer away, so a second press cannot end the panes' commands again.
The step's details (server pid, versions, the stage that failed, the update command's output) go to the `herdr.handoff` and `herdr.path_update` diagnostic records, never to the screen (B20).

The screen that holds Hide: while this machine's Herdr server speaks another protocol than Hide's, nothing Hide opens works on it, so the whole shell gives way to one screen with the window's drag strip above it, on every system and in a browser tab alike (`web/src/herdrTakeoverRules.ts`, `herdrGate`).
No title row, sidebar, wizard, Settings, palette, dialog or notice is drawn behind it, no key or menu item runs a command, and Escape and a click outside do nothing, so no path reaches a workspace, a tab or a project on that server; a notice raised before it came, such as a refused workspace, is cleared.
It names what Hide is doing or can do: for an older release Hide can move, the move step above, moving on its own and then `Try again` after a failure; for one Herdr cannot hand over (Windows, or a release older than 0.6.3, which has no live handoff), the restart step with `Restart`, which a browser tab replaces with `Restart Herdr from the Hide desktop app.`; for a server the look could not read, `Couldn't read Herdr` with `Try again`.
For a newer protocol it is `Hide needs an update` with `✓ Your Herdr and its work are unchanged`; Hide never touches that server (B18).
The desktop app looks for a Hide update as soon as this screen shows, not after its usual launch delay, and while one downloads the title is `Downloading the Hide update` with a spinner; once it is downloaded the app restarts into it through Settings > About's Restart, which keeps the core and its agents running, once per downloaded version.
When the check found no newer Hide the screen says `A Hide for Herdr <version> is coming soon.` and the app keeps checking on its usual schedule; a check that failed, or an app that cannot update itself, shows About's update row with its reason; `Try again` and `Check for updates` stay at the right, and a browser tab shows only `Try again`.
A mismatch the core offers nothing for (a daemon without a Herdr of its own, or a look still on its way) says `This Herdr isn't compatible with Hide` with `When your work is done, stop Herdr and open Hide again.` and `Try again`.
None offers `Not now`; the desktop app adds `View log`, which shows the core's log in the file manager, and `Quit Hide` at the left.
The hold ends on its own once the core reads the server on Hide's protocol again: after a move it hands over to the moved step or line above at once, though the connection still reads the old server for about a second, and after a restart or a stop to the shell.
Every Herdr from 0.5.12 to 0.8.2 reads as another protocol, not as a server that does not answer: a server that refuses the subscription is asked its protocol with `ping`, which every release answers.
The progress bar names the actual route, with three steps after a successful empty profile discovery and four while saved profiles are present or their discovery is loading or failed.
Each step after Agents has Back; Back changes the core-owned step alone and never repeats an installation, reconnect, permission request or test notification.
The core saves the agent selection, checked device profiles, project machine and path alongside progress, so returning or reloading retains explicitly disabled agents and the earlier notification choice.
Only complete clone URLs without user information, queries or fragments are saved as setup drafts; other clone input stays in the open form and is still passed unchanged to Git when Clone is pressed.
Finishing setup clears its project draft.
Nothing but each step's own buttons moves it: Escape, a click outside and a close button do nothing, and the step on screen is the core's (`ui_state.first_run`), so quitting, reloading or a failed save reopens the step not yet finished, and the wizard never returns once its last step is done or skipped (B5, B6).
Each step's main button sits at the footer's right with its keycap, `⌘↵` (`Ctrl+Enter` off macOS), which presses it from anywhere in the step; Enter on the step itself presses it too, while tiles, links and buttons keep their own keys; while macOS blocks Hide with the switch off that button is `Turn on notifications` (B40).
The agents step is the first-run agent choice (Settings: each machine's install kit, below); the core moves the wizard on once this Mac's kit pass for it has landed with every chosen agent in place.
Returning to Agents shows the saved choice, or the machine's enabled agents when no draft exists; installed agents start enabled only for the unanswered initial choice.
An old installation or discovery answer can settle its own operation but cannot advance a different step.
Devices lists the Host entries of this computer's `~/.ssh/config`, the same list and filter as Add device (no wildcard or negated Host), beside the machines saved in Herdr, so a person who never used Herdr still sees the machines they reach; reading both contacts no machine (`ssh -G` resolves an address without connecting).
Hosts and saved profiles that lead to the same `user@host:port` and Herdr session are one row however many aliases name it, while a different user, port or named session is its own row; rows follow the config's order, then the profiles no Host is.
Each row names the machine over its address (`user@host:port`, or the profile's target) and a small chip for where it came from, `ssh 설정`, `Herdr에 저장됨` or `Herdr에 저장됨 · 세션 <name>` for a session other than the default, or both.
The step is titled `다른 기기에서도 에이전트를 돌리나요?` with or without Herdr, and says the machines come from the ssh config and Herdr and that nothing is contacted or installed before Connect; under the list one line says a machine not listed is added later in Settings › Devices.
In the Windows app it also lists this PC's default WSL distribution first, read from WSL's own registry without starting it, as `WSL Ubuntu` over `이 PC (WSL)` (PRD wsl-device B1, D-13); another distribution is not listed here and is added later from Add device.
Loading, a failed read with Retry, a proven empty result, disabled or invalid profiles, a Host `ssh -G` could not resolve (with Add device's reason) and already-added machines are distinct states; a config that exists but cannot be read, or an Include whose folder cannot be opened, is a failed read with Retry, a list cut at a size, Include depth or file bound keeps the step, and only a complete read that found no profile and no Host (a missing config reads as none) skips Devices, in the first run and in Help's walk.
When Devices was skipped that way the last step says `다른 기기는 나중에 설정 › 기기에서 추가할 수 있어요.` under its two ways.
A WSL list that could not be read is a failed read too, with the same Retry, and never an empty one, so a PC that has WSL does not lose the step to a registry it could not read (B1).
Every row starts unchecked, and its name, address and chips wrap when they need to.
The WSL row starts checked until the operator unchecks it, which the draft keeps (`first_run_choices.devices` stays null until a row is checked or unchecked), and it names no target, session or `Herdr에 저장됨`.
Its Connect is the install consent, as Add device's is: nothing is installed in WSL before it, and it registers the distribution's device, which installs Hide's helper and hooks there, and brings its `WU` tile to the front in the same event, so the project step adds to it and a WSL agent can be started at once (B2).
Before Connect the WSL row lists no agents: only the distribution's own helper reads what is installed there, and reading it any earlier would start WSL before the consent, so the row is `WSL Ubuntu` over `이 PC (WSL)` alone (B1, by the operator's choice).
After Connect its install runs as any device's does, and its agents show where a device's kit already shows them: the project step's agent choice and Settings › Agents.
A default distribution on WSL 1 cannot be checked and says `WSL 1이에요. WSL 2로 바꾸면 연결할 수 있어요:` over its command `wsl --set-version <distribution> 2`, and one whose setup is unfinished says `배포판 설정을 먼저 마치세요` (B29); a PC without WSL has no WSL row, and its step is the Hosts and saved profiles alone, skipped when there are none.
Checking a device changes the draft alone; Connect selected explicitly allows Hide's helper, command and selected agent integrations on those checked devices through the existing Add device connector and install owner: a Host registers as Add device registers it, under its alias on the default server, and a saved profile keeps its target and session.
Existing Herdr sessions, tabs and workspaces stay in place.
After Connect a row whose ssh asks the operator shows the device row's own sign-in part in place (`web/src/settings/SignInPrompt.tsx`): ssh's sentence over a masked field with `로그인` and `취소`, a new server's fingerprint with `이 서버 믿고 연결`, a page to confirm in the browser with `열기`, or `보안 키를 터치하세요`, and the row reads `로그인을 기다리는 중`; one line under the list says moving on without answering ends only the questions still waiting.
A row that did not connect shows the device row's three states (`DeviceLink` in `web/src/settings/DeviceRow.tsx`): a sign-in needed or refused with `로그인` in Retry's place, a changed server key with `ssh <alias>` and Copy and no retry, or `연결하지 못했어요` over ssh's last line with `다시 시도`; one line says a failed device shows the same in Settings › Devices.
Connecting and installing hold the Connect control, and the main button reads `계속` meanwhile; neither it nor `이 컴퓨터만 쓰기` is ever held by a connection or a waiting question.
Leaving the step (Next, `이 컴퓨터만 쓰기`, Back, or Help's walk moving on or closing) ends the sign-ins its own Connect started (`first_run_sign_in_withdraw`), whatever they show, as closing a screen ends its own; the device then reads `로그인 필요` in Settings › Devices, a sign-in the device row or another screen started since is left alone, and connected devices stay connected.
Connect selected counts only checked rows not yet tried; a failed row is tried again by its own action alone, while successful machines remain added and are not connected again by Back or re-entry.
`이 컴퓨터만 쓰기` continues with this machine, and failed discovery can be skipped without treating it as proof that no machine exists.
Project reuses Add project's host selector and folder, clone and create views; adding a remote folder offers the enabled agents from that remote machine's kit and never borrows this machine's sign-in state.
Its What changes opens a list with one row per place Hide writes, the path in mono with the agents it is for and the piece (`Codex, OpenCode, Pi · Skill`), so a skill folder agents share is one row, and the `hide` command's link last (B11).
A chosen agent that did not go in keeps the step with `<agent> was not set up: <reason>` and Try again (B12).
An agent whose copy is older than Hide's minimum is still a switch, on or off as chosen, and its tile says `업데이트 필요` (`Update needed`) in the warning tone; under the tiles one line per such agent names it with its versions (`Claude Code 2.1.200 → 2.1.278 이상`) and the one command that updates it with Copy, or its install page, since a tile is itself a switch and holds no button.
Its pieces refused for that reason do not hold the step and are no `was not set up` line: the update puts them in at the next kit read ([agent-hooks.md](agent-hooks.md#updating-an-agent-that-is-too-old)).
A pass that could not run (the kit link failed, or the pass listed no agent) keeps the step, says `Could not set up agents on this device:` with why the kit could not run (it was unreadable, this Hide is not the installed app, or its installer did not start), leaves the choice unanswered, and reads the agents again; the step says the same whenever the kit could not run here, as after a relaunch, and never that no agent is installed; `Try again` then records nothing and only has the agents read again, and once they are back Continue sends the choice anew; the walk Help reopens does the same, staying on the agents step with that line and no banner (B12, B44).
A first pass that stopped before changing anything still opens the wizard on a Mac no pass has answered for, since nothing was set up and the `hide` command, hooks and letters wait on it: the agents step draws no tile and says the machine row's line (Settings: each machine's install kit, below), and `Try again` runs the same pass again, which checks again what stopped it, records no choice, and once a pass runs brings the tiles and the question back.
Clearing a refusal can take a terminal and a restart, so that step alone can be left: `Not now` closes the wizard with the question unanswered, and it opens again only for a different refusal, when Hide's daemon starts again, or with the question once a pass runs; an install that was set up before gets no wizard for a later refusal, only the machine row's line.
The notifications step's description names the system that shows the notification (macOS, Windows, or `your desktop`), and its footer offers `Not now` and `Turn on notifications` at the right, with `Also on your phone →` at the left.
On a fresh install, `Turn on notifications` enables Needs You only; Done stays off to avoid announcing every completion (operator decision 2026-10-13).
Turning them on stores that choice and sends one test notification, `Notifications work`, which is what makes macOS ask; while macOS's dialog waits the step says `Waiting for your answer…` and holds `Turn on notifications`, an allowed answer shows `Notifications are on` with `Send a test`, and a refusal shows `macOS is blocking Hide's notifications` with `Open System Settings`, which opens Hide's page of the system's notification settings; once answered the footer offers Continue, drawn as a secondary button while macOS blocks, and a blocked state is not counted as on (B14, B15).
The step reads macOS's permission every two seconds while it is open, one read at a time, so a change in System Settings shows within seconds; an already allowed Mac shows `Notifications are on` at once, with Continue, which turns the switch on with the step; nothing is said before the first read; a permission Hide cannot read shows `Hide can't read the notification setting` with `Send a test`, and the reason goes to the host log (B16-B18).
On Windows and Linux the step has no permission to show: turning on stores the switch and sends the test once (B19).
A desktop with no notification service (Electron says notifications are not supported, or on Linux the last notification failed to show, until one shows again) shows no `Turn on notifications`: the step says `This desktop has no notification service, so Hide can't show notifications here.` with Continue, and Settings > Notifications shows the same line under Needs You; both switches stay off and locked, with no test (D-24, B47, B48).
Settings > Notifications' desktop group has independent `Notify me when an agent needs me` (Needs You) and `Notify me when an agent finishes` (Done) switches; while either is on, the permission row says what macOS allows (`macOS permission`, `✓ Allowed` with `Send a test`, `! Blocked by macOS` with `Open System Settings`), or on Windows and Linux `Test notification` with `Send a test`, and the group ends with `Sent only while no Hide window is in front. The app icon counts agents in the notification groups you enable.` (B30, operator decision 2026-10-18).
`Not now` goes on with both switches off, even after `Turn on notifications` was pressed while macOS still asks, in the same event that leaves the step, and `Also on your phone →` opens Settings > Mobile over the wizard, which returns to the step when it closes (B21).
The last step asks `How do you want to start?` under `Either way, you can do the other one later. Practice is always in Help (?) > Practice Hide.`, with two ways, each a whole-width button with no choice to confirm and no Continue, then Back at the footer's left and `Skip for now` at its right (operator decision 2026-10-24).
The first way, with no chip that ranks it over the other (operator decision 2026-10-27), is `Try it on a practice screen first` over `Your real agents and files stay untouched`, drawn as the same plain bordered button as the second, only its graduation cap in the practice's amber (`--practice-accent-strong`) as on the sidebar's invitation; it ends the wizard and opens the [Practice window](#practice-window) on chapter 0, the Home behind it empty.
The second is `Add a project` over `Start right in your own repository`: it shows Add a project under `Add your first project`, with its Host select, Browse folder, Clone from URL and Create new project, its refusals and its clone progress, and `Skip for now` (B22, B25); a folder pick cancelled there leaves those views as they were, and their Back returns to the two ways.
Which of the two is on screen is the project draft's view (`first_run_choices.project_form.view`, empty for the two ways), so a reload reopens it.
`Skip for now` ends the wizard to the empty Home in either.
Add a project's Host list has no Add device, which would take the wizard's place, and only a project on the chosen host counts as the one added.
The first project added ends the wizard and opens the start panel once, on that project, with the first agent that is on and signed in (in the order Claude Code, Codex, Grok, Pi), else the first agent that is on, and the keyboard in its text box; with no agent on it opens no panel and the wizard closes to the empty Home, whose Start an agent is the way on; a later project opens nothing (D-15, B23, B24).
For a target on the core's own machine, the start panel says `! Not signed in` right after its agent menu whenever that agent is one whose sign-in Hide checks and its last read was signed out (B23, D-06).
A remote target never uses the core machine's login result; its kit reports installation and enablement, without claiming remote sign-in (B37).
A browser tab shows the agents step alone, with no progress bar, and Continue there ends the wizard, so the sidebar's invitation is its way into practice (Practice window, below); the notifications, start and Herdr steps are the desktop app's, and its Continue leaves a Herdr step unanswered for Help > Set up Hide and Settings > About, and the screen that holds Hide is drawn in a browser tab as in the app (D-14, B26, PRD herdr-takeover B19).
The desktop app's Help > Set up Hide opens the same steps from the first as this window's own walk, beginning with the Herdr step only while there is a server to move or restart (PRD herdr-takeover B17): the agents step starts from the agents switched on now and its Continue switches only the agents whose switch changed, the way Settings > Agents does; Escape and the close button end it, and a step not continued changes nothing (B45, B46).

With Needs You on, the desktop app shows a notification when a root agent newly needs the operator (a question, an approval, a block or an error); Done independently enables new completions.
Both are sent only while no Hide window has the keyboard: its title is the root's row title (the provider when no task or proven native title exists), and with a second-line sentence its body is `<translated row status> · <exact sentence> · <project>` (D-09 to D-11, B31, B39, operator-approved update 2026-10-13).
The sentence stays in its original language; without one the body is the row's word and the project (`Approval · <project>`), a frame with no status code says its section (`Needs You · <project>`, `Unread · <project>`), and an empty project omits its separator.
See [With and without Hide AI](status-model.md#with-and-without-hide-ai) for the title priorities, native and inferred questions, counts and both notification channels in each mode.
hided judges the changes with the phone push's rules (status-model.md, Row presentation and phone notifications), so a child's raise arrives on its root, the same state never notifies twice, an agent back within ten minutes is not announced again, and nothing already waiting when the app opened is announced (B32, B33).
A click brings Hide forward and opens that agent's pane, on a device too; an agent the operator has seen takes its notification back, and a newer one for the same agent replaces it (B34, B35).
On Windows a click on a notification in Action Center after Hide quit starts Hide and opens what it named once the shell is up, as a click while it runs does.
The app icon sums root agents in every enabled notification group across every device and project the window shows: Needs You under its switch and unread Done under its own, including rows folded into `more`, and nothing at zero (operator decision 2026-10-18).
Reading a Done row removes it from that group and the count; turning either switch off removes only that group's contribution.
macOS draws it on the Dock once it lets Hide notify, Linux where the desktop shows launcher counts, and Windows as a dot on the taskbar button whose description gives the count.
With both switches off nothing is sent and the app icon has no count; with Done alone on, it counts unread Done.
The phone's push follows its own settings whatever these switches say (B36, B38).

Settings > Devices keeps its own description.
The persisted `screen_intros_seen` field remains readable for older builds; this release has no Factory introduction.

## Factory

Factory is excluded from public source builds and official free app builds.
There is no sidebar entry, command, engine, question interception or Factory snapshot section.
Existing Factory stores and historical usage records are preserved as legacy data.

## Recent navigation

Web owner: `web/src/recent.ts`, `web/src/areaCycle.ts`, `web/src/viewFocus.ts`, `web/src/keyboard.ts`, and `CycleOverlay` in `web/src/Overlays.tsx`.

Recent Agent pane or View tab cycling (⌃Tab / ⌃⇧Tab in the desktop app, ⌥` / ⌥⇧` in a browser) follows where the keyboard is.
In a View area it walks the tabs of that exact area, in recent-use order.
The View scope includes device, checkout and area ID, so an Agent area with the same ID or another checkout cannot widen it.
A document, diff, View tab bar or visible native browser page names its View area.
Everywhere outside a View area it walks the agent panes the keyboard has been in, one row per pane, across every device, project and checkout, in recent-use order (issue #301); the order survives a reload, quitting the app and a daemon restart.
A terminal pane in a tab an Agent area of the Workspace in front draws (its normal tab or a delegated child's canvas), or that area's tab bar, puts the keyboard in the Agent area; from a tab bar the pane in use is the one the core focuses in that tab.
A pane joins the order the first time the keyboard is in it, and a commit's passing frames on the way to its pane are not visits; a pane that closes is no longer a row.
A pane row is titled by the agent it runs, with the place as Recent Panels names it and `Terminal` beneath, and the agent's status mark; a pane running no agent, a View display, an Overview and a project are never rows.
From a pane running no agent, or from the sidebar, tools, search, Settings, a dialog, Main or Overview, the first chord lands on the most recent agent pane.
Outside the Agent area, opening the cycle neither invents a pane origin nor records the underlying pane as visited.
The Overview modal and page supply no pane origin; their first Agent cycle chord selects the most recent visited agent, even with a Workspace View owner underneath.
Escape cancels the cycle and keeps Overview; committing closes Overview and brings that pane forward with one event and keyboard focus.
A focused View area keeps its own scope even when it has zero or one tab or its target retired; it never falls through to Agent panes.
With no visited agent pane, or no other visited agent besides the pane in use, the Agent cycle is a no-op.
Holding the chord's actual modifier freezes the order and scope and previews in Recent Agent panes or Recent View tabs without moving the committed tab, layout or keyboard owner.
Repeated forward and backward chords walk that frozen order; Escape or losing the window cancels without a selection event.
Releasing the modifier on a View tab commits one in-place selection; on a pane it brings the pane's device, project, tab and pane forward with one event, as choosing it in the Agents list does, and the pane receives the keyboard.
Closed or moved View tabs leave the frozen candidates; removal of the origin area or checkout cancels, and no other area replaces it.
Closed panes and panes whose agent ended leave the frozen Agent candidates; a highlight that left moves to the next surviving pane, and with none left the cycle ends.

Global Recent Panels is a separate, named command, unbound by default and assignable in Settings alongside the Agent pane and View tab commands.
It walks every unified surface in recent-use order across projects, checkouts and connected devices: terminal tabs, View files, diffs and browser displays, plus the single Overview page.
The Overview modal creates no Global Recent Panels visit.
An Overview page revisit moves its single device page row to the front; its tab, device and filters remain page-local, and a removed device retires its page row.
Recent Panels holds no Project Overview row, since that screen is gone (PRD overview-device-tasks B61).
A Global Recent Panels commit shows an Overview at once or brings one Workspace surface forward with one event; a View target also receives the keyboard.
Menu and palette invocations of either family select the next or previous valid target immediately, with no held modifier or second selection.
A bound Global Recent Panels chord keeps the same hold, release and cancellation behavior.
Recent Projects (⌥Tab / ⌥⇧Tab) retains its global project order and restores each Project's last Workspace surface, never its Overview.
Both global lists and the Agent pane cycle carry a device chip on a row outside the device in front and move the rail, sidebar and center together when committed.
The session uses one bounded recent-surface history; the View area cycle filters it and appends normal area tabs not yet visited.
The Agent pane cycle reads the core's order of pane visits (`ui_state.recent_pane_ids`, saved with the UI state); the page decides what a visit is and reports it with `pane_visit`, and the fifty most recent panes are kept, shell panes included, since an agent may start in one later.
Until the core's order moves after a report, the page puts the pane it reported first, so a cycle opened before the echo arrives starts where the keyboard last was.
The shown tab of the recorded keyboard area is the current visit, including native browser pages; intermediate commit frames are not visits.

Reopen Closed Tab is disabled when the session-local recent-close stack is empty or a restore is already running, and restoration works regardless of which surface currently owns focus.
Pressed while this machine's newest close is still being confirmed, it is accepted and waits: the closed item reopens as soon as Herdr confirms the close, and nothing reopens if that close is refused or fails, or if another close starts first.
Restoration is one action with no confirmation: an in-flight pane shows inline progress, and a restore without a target pane shows a compact inline warning.
Missing cwd, an unavailable prior conversation, a missing file, and a retryable failure all use the same inline notice vocabulary, without a banner, card, or modal.
A definitive close refusal removes its reserved reopen entry, while an unconfirmed result keeps the entry and explains inline that Hide could not determine whether the item closed.
With no other project or tab available, navigation keeps the current selection without a modal; selecting an empty project shows its existing empty state.
Automatic pruning and concurrent-selection recovery use structured diagnostics without a modal.

The project identity is scoped by device, following the sidebar's Project > Workspace > Agents hierarchy; two checkouts of one repository share a project cycle, and panel history is one order over every project, narrowed to a project for its own last-surface lookup.
Both the recent-panel and project switchers show at most nine rows around the highlight.
Project rows show the last surface and checkout; panel rows show their project and checkout (collapsed to the checkout alone when both share a name) and their surface type.
A row's device chip follows its detail line, truncated before it would crowd the detail, with the status mark, title and checkout intact; a device absent from the registration map is named by its actual remote ID rather than presented as local.
Every device chip on an agent, a child or a descendant names its machine as the device rail does (`machineName`): the window's own machine is This Mac whatever name the core registered it under.
A panel row whose tab holds exactly one agent pane is titled by that agent's identity with its status mark; a tab with no agent or several keeps the Herdr tab label.
Recent Panels and Recent Projects history is session-local and retains only existing projects and surfaces; a deleted highlight moves to the next surviving entry without reordering the held cycle, and if none survives, the cycle cancels and keeps the current selection.

Next tab and Previous tab (⇧⌘] and ⇧⌘[ in the desktop app on macOS, Ctrl+PageDown and Ctrl+PageUp on Windows and Linux) choose the tab beside the one in use, in the order its tab strip draws, wrapping from the last tab to the first and back, as every browser, terminal and editor does.
They step through the View area that holds the keyboard, and otherwise through the Agent area in use, the strip ⌘1-9 numbers; the choice is the one a click on that tab makes, and the keyboard follows it.
A strip with one tab stays as it is.
There is no overlay and nothing to hold: ⌃Tab stays the recent-use cycle.
A browser tab has no default chord for either, because Chrome keeps both pairs for its own tabs; Settings, Shortcuts can bind them there.

## Device rail

Web owner: `web/src/components/device-rail.tsx`, `web/src/devices.ts`, the rail branch of `web/src/sidebar.tsx`.

This section supersedes PRD home-device-rail rows D-10, D-11, D-27, B3, B4 and B12 (quick device-rail-badges): the Inbox, the rail that came and went with the first remote device, the Needs You-only badge and the footer device button no longer exist.
Quick device-rail-slack then narrowed the rail to `--size-rail` (48), ran it the sidebar's full height, took the names off the tiles and left Working to the session rows.

This Mac is this document's name for the machine the core runs on; the screen names it as its system does, This Mac on macOS, This PC on Windows and This computer on Linux, in the interface language (`common.thisMac`, `common.thisPc`, `common.thisComputer`: `이 Mac`, `이 PC`, `이 컴퓨터` in Korean).
The core sends no word for it: its device row has no `label` but the one it kept from a move (below), only its system in `host.platform`, and every surface that names it, the sidebar title, the rail, Home, Settings, the Overview's device switch, the start panel, ⌘K and the phone's start sheet, words it from that system (`systemMachineKey` and `machineName` in `web/src/screenMachine.ts`), so one screen never shows it in two languages.
The rail is always shown, even when This Mac is the only device: the sidebar's full-height left column, with the single sidebar list to its right and the device title above it in the window's top row, holding This Mac, each registered device in the core's order, and a `+` (`기기 추가`, a dashed tile) directly under the last device tile.
The stored sidebar width stays the content column's; the rail adds `--size-rail` to its left while it shows, and the drag edge sits on the content column.
Each tile is a 32 rounded square with no name under it: This Mac draws the laptop glyph and a device the monogram of its name, the first letter of each of its first two words, split at spaces, dots, dashes and underscores (`Mac mini` → `Mm`, `build-box` → `Bb`, `mini` → `M`); the tile's hint, shown to its right, is the name followed by `연결 안 됨` or each count it marks in full (`mini · Needs You 12 · Done 1`), since the pill stops at `9+`.
The selected tile is ringed (a 2px ring 2px off the tile), and one tile is selected at a time.
A tile carries at most one mark, notched into its top-right corner by a ring of the rail's fill, and it is the most urgent state: the Needs You count in a `--warning` pill, `9+` from 10, or, with no Needs You, a `--success` dot with no number while the device has unseen Done.
Working has no mark on the rail, since it is no reason to switch device; the 태스크 list counts it.
A mark's digits are `--status-foreground`, white in Light and near-black in Dark.
The counts come from the device's published `agent_scope` values; the browser does not rederive them.
A device that is not connected dims its glyph and wears a `×` at the bottom-right, with no mark, since its last counts are not current; selected, its sidebar shows only its name, `연결 안 됨` and `다시 연결`, which retries the connection in place, and never the tree it last reported.
A delayed or failed agent-state query alone leaves the connected device, its tree and terminals available; retained agent activity uses the existing Unknown state until a successful query confirms it, without a disconnected message or a new alert.
Selecting a device tile sends `focus_device`, and the sidebar becomes that device's single project and session list; no row there names the device.
In the desktop app ⌥⌘1-9 (Alt+Shift+1-9 on Windows and Linux) selects the tile at that number, top to bottom with This Mac first, whether or not the rail shows, and holding ⌥⌘ alone floats each tile's number at its top left (see Keycaps, tooltips, and icon buttons); a number with no tile does nothing.
⇧⌘ would have been the shorter chord, but macOS keeps ⇧⌘3, ⇧⌘4 and ⇧⌘5 for screenshots, so three tiles could never be reached; ⌘1-9 belongs to the tabs.
Every tile is a button reached with Tab and chosen with Enter or Space, named for assistive technology by the device, its connection and each count it marks (`mini, 연결 안 됨`, `This Mac, Needs You 2, Done 1`).
`기기 추가` opens Settings › Devices with the Add device dialog in front, as the hidden rail's `기기 추가…` and the Add a project dialog's host list do.
Removing the device in front moves the front to This Mac; that device's agents and its `~/hide` stay on it, and the rail stays.

A right-click on the rail offers `기기 목록 숨기기`.
While the rail is hidden the top row's device name becomes a `This Mac ⌄` menu that lists the devices (a check on the one in front), `기기 추가…` and `기기 목록 표시`.
The View menu of the desktop app carries `Toggle device rail` (`toggle_device_rail`, no default chord, bindable in Settings › Shortcuts like the sidebar switch); a browser tab uses the `This Mac ⌄` menu.
The hidden state is the core's `ui_state.device_rail_visible`, kept beside `left_sidebar_visible`, so it survives a restart.

A device's Workspace in front wears the device color, `--device-remote`: a band at the start of the top row's Workspace path with the server glyph and the device's name, truncated, and a border of the same color around its panes; this machine's Workspace has neither.
Which machine is a device is the window's to say (`awayDevice` in `web/src/screenMachine.ts`): on a window of a machine whose core runs elsewhere, its own Workspaces wear neither and the core's machine's wear both.

A machine whose core runs on another machine (ARCHITECTURE.md, A core on another machine) is a device on that core's rail like any other, named by its host name.
Its `다시 연결` is answered that the machine reconnects by itself, and removing it ends its connection until it attaches again; neither adds a state or a line of its own.
The window's own machine is always the first tile and the one named This Mac (`이 Mac`), the laptop glyph (`web/src/screenMachine.ts`, `windowFirst`); on a window of a machine whose core runs elsewhere, the core's machine follows it, named by the name the operator registered it under before the core moved to it, which the core keeps (`machine_label`, ARCHITECTURE.md, Moving the core), and by the name its machine gives itself when it has none.
The tile of the machine that runs the core wears a crown notched into its bottom-left corner (`--size-rail-crown` in a `--size-rail-badge` notch, the corner the `×` and the marks leave free), and its hint adds `core 실행 중`; the crown shows only once a machine dials in to a core, that is on a node's window or while any device dials in, since with one core and its devices nothing needs telling apart (PRD core-host-node-move B2).

This PC's WSL distribution is a device like any other, one tile per distribution lettered from its name (`WSL Ubuntu` → `WU`) (PRD wsl-device B5, D-12, D-20).
Every surface that names it says after its name where it is, the core's machine as this window names it, and never `원격`: the tile's hint and accessible name (`WSL Ubuntu · 이 PC · Needs You 3`), the sidebar title's smaller word, which is `원격` for an SSH device, the Overview's device switch, ⌘K's device row, its surface while it shows no tab, and the line under its name in Settings › Devices, in place of its Linux platform (`WSL Ubuntu · 이 PC · Herdr 0.9.5`) (`wslPlace` in `web/src/screenMachine.ts`).
Choosing its tile scopes the sidebar, 태스크, PR, Overview, the tile counts and the ⌥⌘ numbers to its own items by the rules above (B6), and a stopped distribution reads as any device that is not connected.

A window on a machine whose core runs elsewhere shows its core's state and takes its strip, in the place and size of the connection badge, when the core cannot draw it (W1 to W3, B16):

- while a move holds the window, `core를 Mac mini 기기로 옮기는 중 · 3/5 프로젝트와 기록 복사 · 다시 연결될 때까지 입력을 받지 않아요`, the move's step of five, with `…`, and the window sends nothing but the move's own give-up (a rollback says `옮기지 못해 이 Mac의 core로 되돌리는 중`); while the move waits on a machine that took its link and stopped answering, the strip ends with `그만 기다리기` (`여기로 가져오기` for a move back), which only opens the move dialog at its confirm;
- while this machine updates its core to this app's build, `Mac mini의 core를 이 앱의 빌드로 바꾸는 중 · 에이전트는 계속 실행돼요`, with `…`;
- while the core cannot be reached, `! Mac mini의 core에 연결할 수 없어요 · 다시 연결하는 중`;
- while the dial a window's `로그인` or retry started waits on the operator, that window alone shows `… Mac mini core에 로그인하는 중` with what ssh asks under it exactly as a device's row shows it (PRD system-ssh-device-link B33): ssh's own sentence and a masked field with `로그인` and `취소`, a server's fingerprint with `이 서버 믿고 연결` and `취소`, neither chosen, a page to confirm with `열기`, or `보안 키를 터치하세요`; another window shows the link connecting and cannot answer; the answer is cleared as it is sent; Cancel, five minutes on a question, a page or a key touch, or closing that window ends the attempt, before the first question or between two as well; one press asks at most one sign-in; a connect no window made (`hide connect`, a launch) and the dials Hide makes by itself never ask, and stop at the sign-in for the window's `로그인`;
- while the link stopped on what only the operator can fix, which nothing dials again until they act, the device row's three actions: a sign-in (`! Mac mini core에 연결하려면 로그인이 필요해요`, or `! Mac mini core가 로그인을 거절했어요`) with `로그인`, which dials again and may ask; a changed server key (`! Mac mini core의 서버 키가 바뀌었어요 · 터미널에서 확인해 주세요`) with `ssh mini` and Copy and no `다시 시도`, as on the device row, since a retry would meet the same key; a Hide an update replaced on disk (`! Hide가 실행된 뒤에 업데이트됐어요 · Hide를 종료하고 다시 열면 Mac mini core에 연결돼요`) with no action, since only Hide opened again can dial; and any other such failure (`! ssh로 Mac mini core에 연결하지 못했어요 · 터미널에서 ssh mini 명령으로 이유를 확인해 주세요`) with `다시 시도`;
- after the operator ended the link here, `! Mac mini core와 연결을 끊었어요` with `다시 연결` at its end, which links it again and is, with `다시 시도` (`로그인`) and an answer to what ssh asks, the only thing the window sends while it waits;
- otherwise the shell's own connecting state.

The window takes no input while its strip shows, and sends none of what was typed meanwhile once the core is back; the strip goes when the core's snapshot arrives, and a move that ends on another machine rewrites the window's address to it without a reload (`addressAfterMove`).

## Start panel

Web owner: `web/src/StartPanel.tsx`, `web/src/startTargets.ts`, `web/src/startDraft.ts`, `web/src/startAnswer.ts`, `web/src/agentPicker.ts`, `web/src/components/agent-picker.tsx`.

The start panel starts an agent supported by the shared adapter with a first instruction anywhere hide can reach (PRD home-device-rail D-17..D-22).
It remains independent of the inline group/draft start: a successful start makes its own tab in the target checkout's selected group rather than consuming the draft whose form happens to be open.
⌘N opens it in the desktop app, where it is also File › Start agent; in a browser tab ⌘N stays the browser's and `에이전트 시작…` in ⌘K, found by typing, opens it, on every screen.
It floats at the ⌘P palette's place and width with no backdrop, and the keyboard lands in its one-line text box, `에이전트에게 무엇을 요청할까요?`.
Opened while Settings is up, it takes Settings' place: Settings closes and the target is the front device's Home.
Under the text are the target, the agent kind and the model menus, a `⏎` keycap and `시작`; Enter or `시작` sends one `agent_start_in_checkout` with a fresh request id, the text as the agent's first instruction, handed to the CLI as its own argument (ARCHITECTURE.md, the first prompt).
The target defaults to what is in front: the checkout of the Workspace in front (a worktree when that is it), a project's main checkout while the Overview is filtered to that project, and the front device's Home while the Overview with no project chip or Settings is; a device's surface in front makes that device the target's.
The target menu lists the front device's Home and checkouts, then each other device's Home and checkouts prefixed with its name, with a separator between devices and a check on the chosen item; a device that is not connected is listed disabled with `연결 안 됨`.
The kind menu holds Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity with their provider marks; the model menu is `CLI 기본값` then the chosen kind's catalog (Antigravity's from `agy models`, started with `--model`), and changing the kind takes that kind's list and the model last chosen for it.
OpenCode's start takes no model, so with OpenCode chosen no model menu is drawn here or in a Factory's worker candidate, and a model remembered for it from before is not sent.
While the catalog is being read or cannot be read, the model menu shows the remembered model or `CLI 기본값`, is disabled, and gives the reason in its tooltip; starting is never held back by it.
Every supported agent start, here or in a dialog, is remembered by the core with its model, `CLI 기본값` included, so the next open preselects that kind and its model with no `recent` mark; the target is not remembered and follows what is in front on every open.
Escape or a press outside closes the panel and keeps the text for the next open; a start that goes clears it.
The panel follows only the answer carrying its own request id, even after it closes: a refusal or a failed start shows its reason inside the panel and keeps the text, and a start that went brings the center to the new pane, on a device by moving the rail, the sidebar and the center together.
The page opens that Workspace only after the core has selected the correlated pane and its checkout lists it, including the first start after setup or in Home.
A newer page visit cancels that pending presentation; the answer never sends a second focus request or closes an Overview opened over work.
An agent that fails to start after its tab opened puts its text back in the draft with the reason, unless a new draft took its place, so the next ⌘N shows both; the reason stays until the text changes.
A start with no answer in 90 seconds, the core's own limit for one, says so inside the panel.
New worktree, Start from an issue and 맡기기 carry the same kind and model menus with the remembered choice preselected, and what they start becomes the next default; New worktree's kind menu starts with `Terminal only`, which is never remembered.
A worktree created on this machine, from New worktree, the panel, Start from an issue or 맡기기, comes forward the way a start does: the core selects its new pane once its checkout is listed, and only while the operator has visited nothing else since, within the 90 seconds and with the same Herdr connection; otherwise it stays where it is, and the page never sends a focus of its own for it (with-hide/hide-private#311).
A worktree created on a device is opened once that device lists its pane, through the device's own focus.
A new worktree's branch starts from `origin/<base>` as Hide last fetched it when the local base has no commits of its own, so a base the operator fetched but never pulled is not where new work begins; a local base with unpushed commits, or with no fetched copy, is started from as it is, and the new branch tracks no upstream either way ([ARCHITECTURE.md](ARCHITECTURE.md), `live::new_branch_start`).
Settings > Agents > Starting work has no default agent of its own.

## Usage

Web owner: `web/src/components/usage.tsx`, `web/src/components/usage-history.tsx`, `web/src/components/usage-history-table.tsx`, `web/src/components/usage-amount.tsx`, `web/src/UsageOverview.tsx`, `web/src/usageHistory.ts`, `web/src/usage.ts`, `web/src/settings/UsageTab.tsx`.
The core owns account windows and shared preferences (`usage`, `ui_state.usage`); all limits refer to the core's own node, even with a remote device in front.
The declaration lists all seven agents; only agents with a declared limit source appear in the footer and popover.

The footer draws up to three pinned account windows at its left edge, and, while the device rail is hidden, the View menu, Help and Settings at its right edge.
Defaults pin the weekly window of each agent that is on by default and reports one (Claude Code and Codex), leaving a slot for the operator's own pin.
Each chip shows the agent mark and its rounded percent, naming the window when the agent has multiple visible chips.
A numberless not-started or expired chip always names its window.
Each chip has its own accessible name with agent, window and value.
Success is below 70%, warning starts at 70%, danger at 90%.
Loading or unavailable agents collapse their pins into one dimmed mark; recent stale and fallback readings keep their values until the reset.
At narrow widths the footer wraps rather than clipping its controls.

Any chip opens the 320-wide Usage popover above the footer.
Its first row shows today's estimated API-rate equivalent cost and new tokens side by side; token categories are behind `Token details`.
New tokens means input, output and cache writes; cache reads stay separate, and All tokens adds them to new tokens.
Cost is an estimate at public API rates, not the account's bill; its tooltip states this and the full history repeats it below the table.
An unpriced model or incomplete native usage makes a known subtotal a lower bound with its reason; a wholly unpriced amount says `Price unavailable`, a missing price table says `Pricing pending`, and a table older than sixty days is marked stale.
No calls means No data, never a zero dollar amount; a known free price remains zero.

Hide AI's own background calls (agent summaries, Memory, Factory, issues and worktrees), which no session file records, are inside the headline totals, bars and grass; the popover draws no separate Hide AI line, and the full history's table is where Hide AI is read apart from the agents.
Unknown Hide AI calls make the inspected day's summary and the selected period's total a lower bound, drawn `≥` with a mark and spoken as "at least", including the full-history summary, whenever that day or period contains unknown calls.
A missing model price alone does not make known tokens a lower bound.

Below the summary, a `7 days` / `30 days` segmented choice switches between seven daily bars and thirty 12-pixel contribution squares; thirty days is the initial view.
The bars show the most recent seven civil dates, labeled by weekday with Today at the end, and scale new tokens against the largest day in that period.
The squares show the most recent thirty civil dates in Sunday-first columns, with no calendar numbers or weekday labels.
The selected period's cost and new-token total stay visible while a day is inspected, below the bars or beside the grass.
The core prepares daily amounts, boundaries and quartile levels; the page selects and sums those amounts without repricing calls or reading sessions.
Closed nonzero days set quartiles, or quarters of the maximum with fewer than seven such days; level zero uses secondary and the other four use primary over secondary.
During initial backfill unread cells are outlined, and file progress and `so far` remain visible.
Hover or keyboard focus previews a day in the summary; clicking pins it, leaving restores the pinned day, and Today returns to today.
In the grass, Up and Down move one day and Left and Right move a week; in the bars, Left and Right move one day.
Home and End reach the visible range's ends.
Changing period clears the preview, keeps a pinned date within the new range and otherwise returns to today.
A day change keeps the closed detail row and grass geometry stable; expanded token categories wrap at the available width.
Hovering a weekly limit outlines the recent seven days, and a five-hour limit outlines today; leaving removes that outline.
Only the chosen seven or thirty date buttons are mounted, and closing the popover removes them.

Account limits follow the grass as compact rows without individual card borders.
Each visible agent has 5 hours and 7 days rows with a countdown, short inline bar and percent; model-specific limits are indented below their weekly row.
Not-started windows say `Not started`, never 0%.
Expired, not-offered and individually unreadable windows have no popover row; a whole malformed response makes the agent unavailable.
Loading rows show `…`; unavailable rows show `Unavailable`, with a sign-in command only for a signed-out account, or the missing CLI for an uninstalled agent.
Network and timeout detail stays in diagnostics.
The header's settings icon, named `Choose sidebar items`, opens Settings > Usage.
No usage state becomes a banner, toast or alert.
The countdown advances once a minute only while the popover or Usage tab is open; the redesign adds no polling or data collection.
With no history there is no grass section; available account rows remain, and with no visible account chips the footer's chart button opens the available history.

Full history opens Overview's global Usage tab, not a per-project ledger.
The table starts at the most recent thirty days, with seven-day, ninety-day and retained-year choices and an agent filter.
The retained year is the existing core projection's bound; Full history does not claim to expose an unlimited lifetime ledger.
Each recorded date has a summary row, followed by agent rows when expanded; today starts expanded.
Under the agent rows an expanded date has a `Hide AI` row with the same columns, itself expandable into one row per feature (agent summaries, Memory, Factory, issues and worktrees, other) with its call count and the providers that served it, and a `Calls with no reported usage` row that has a count and no amount.
The agent filter has a `Hide AI` choice that lists only the days Hide AI was counted on, with the feature rows directly under each date (no second Hide AI row); a date of only unknown calls is still a date, and its popover line reads `Calls with no reported usage` instead of an amount.
A footnote under the table says the amounts are lower bounds when any unknown call is in the period, and names the day Hide AI is counted from, since earlier days are not backfilled.
Choosing one agent hides Hide AI, which belongs to the all-agents range only.
The headline shows the selected period's estimated API cost, new tokens and separate cache reads above stacked daily new-token bars.
Only real agent and Hide AI contributions form each stack; there is no minimum fabricated usage height.
Left and Right move by day and Home and End reach the visible range's ends; selecting a day narrows the totals and detail, and Show full period clears it.
Changing period clears the selected day, while changing agent or daily/model view keeps a selected day still in range.
Columns are Date / Agent, estimated cost, new tokens, input, output, cache write, cache read and all tokens, with a leading selected-period total and an exact-number toggle.
The model view groups actual recorded date, agent, model and session-or-Hide-AI reports, never an allocation inferred from an agent total.
An absent model says Model not reported; an older history without model dimensions says those details are unavailable.
The newest 4,096 model records are retained in deterministic order with a visible limited marker when truncated; daily totals remain complete and model subtotals may then be incomplete.
Model shares use known new tokens, and Hide AI model reports may outnumber the backend attempts counted by its headline.
Year-view dates include the year, and changing the filter or page clears expanded rows.
At most thirty recorded dates are mounted per page, with agent rows only for expanded dates; an empty filter says there is no usage for the chosen agent and period.
The table scrolls sideways within its own region at narrow widths and large Interface text, never the whole page.
Account limits and agents without limit readings remain available in a collapsed Account limits section below the table.
Pi and omp show recorded provider provenance using known display names, account wording only for a recorded account provider, and no line for an unknown provider.
A bounded or partly unreadable source has the small limited marker in its own row and range.

The page sends `usage_window_visible` on every live connection and visibility change, and `usage_popover_open` while the popover or Usage tab is open.
Opening it requests a refresh only for reads older than one minute, subject to the existing backoff.
The core keeps one value of each hint, so with several pages open the last page to report decides.

## Help menu

Web owner: `web/src/components/help-menu.tsx`, `diagnosticsInputOf` in `web/src/settings.ts`, `web/src/updates.ts` (PRD sidebar-titlebar-refresh D-11, D-12; PRD auto-update D-03, D-21).

The footer's `?` opens the Help menu above it: `Keyboard shortcuts` with its chord, which opens the shortcut sheet; `Practice Hide`, which opens the [Practice window](#practice-window) or brings the open one forward; `Copy diagnostics`, which puts About's diagnostics text on the clipboard and says `Copied` in place, or the failure when the clipboard refuses, keeping the menu open either way; `Report an issue`, which opens the repository's new-issue page in the default browser; then, under a separator, the `hide <version>` line in mono, which takes no press.
The version is the release the app was built as, such as `0.4.0`, and a development build's own version, such as `0.4.0-main.<commit>`.
When the desktop app has an update ready ([INSTALL.md](INSTALL.md#update)), a dot in the primary color sits on the `?`, whose name and tooltip become `Help · Update ready`, and the last line becomes a menu item, `hide <new version> ready` with `Restart` at its end, which installs the update and reopens the app.
Nothing else of an update reaches the footer or this menu: looking, downloading and a failure show only in Settings > About.
A browser tab and the phone have no dot and no update line.

## Practice window

Web owner: `web/src/practice/` (`PracticeApp.tsx` the chapter list, `PracticePlayer.tsx` a chapter playing, `PracticeCoach.tsx` the guide, `chapters/` the chapters, `worlds/` what they play in, `channel.ts` what the window and the shell say to each other), `web/src/i18n/resources/practice.ts` and `web/src/i18n/resources/practice/`. Desktop owner: `desktop/src/main/practice.ts`, `desktop/src/main/host.ts` (`listenPractice`, `openPractice`). Core owner: `ui_state.practice` and `practice_progress_set`.

The practice window teaches Hide on a practice screen: the shell's own sidebar, panes, lane and keyboard, drawn over a scripted world with no agent, project, file or Herdr behind it.
`Practice Hide` in the Help menu and the line under Add project on the Home with no project open it on the chapter list; the first run's `Try it on a practice screen first` and the sidebar's invitation open it on chapter 0; the desktop app opens one window of its own, `Hide 연습` by name, 85% of the main window's size and against its lower right, so the main window's title row and sidebar still show beside it and the practice is never taken for Hide itself, and a browser tab opens one tab of its own; asking again brings the open one forward, and an ask for a chapter plays that chapter there from its first step.
A new window takes the chapter from its address (`chapter=<id>`); one already open is told over the channel below (`play`), the same way in the desktop app and a browser tab on every system.

An operator who never practiced (the core keeps no chapter's progress) and never closed it sees the sidebar's invitation, whether or not a project is registered: one quiet row above the footer, like a 태스크 row, with an amber graduation cap, `Practice Hide` and under it `Start with chapter 0, a look around the screen`, and a × at its right, with no progress bar and no count.
Pressing it opens the practice window on chapter 0; the first step a chapter reaches writes its progress, so opening any chapter, from anywhere, takes the row away, and opening the window on its list does not.
Its × says in its hint that practice stays in Help (?) > Practice Hide, and sends `practice_invite_close`, which the core keeps as `ui_state.practice_invite_closed` like the other UI choices, and the row never shows again; the Help menu and the empty Home's line stay.
The practice screen's own sidebar never shows it.
It is not a ⌘K row, because ⌘K goes to things and runs no command but two ([Search keyboard navigation](#search-keyboard-navigation)).

The window wears `Hide 연습` as its title while it lists and while it plays, and `Hide로 돌아가기` is always on it: at the right of the list's heading, and at the right of the top center's chapter name while a chapter plays.
It closes the practice window, and the desktop app brings the main window forward, as it does whenever the practice window closes; a browser tab the shell opened closes back to the shell's tab.
The list's lead says that no real agent, project or file is touched and that the learner may pick only the chapters they need, and a smaller line under it that Help (?) > Practice Hide opens it again any time.
It opens on the chapter list: each chapter's number, title, one line of what it covers and its step count, its state (`새로`, `<n> / <m>에서 멈춤`, `✓ 봄`, or `바뀜` when the chapter's steps changed since it was played), and one button: `시작`, `이어서`, or `다시 보기`, with `처음부터` beside `이어서`.
↑ and ↓ choose a chapter and ⏎ plays it; the first chapter not yet seen is chosen at first.
`이어서` plays the steps before the stopping point again on its own, quickly, the way ⏎ would, so the screen reaches the state that step needs.
A chapter seen once stays seen when it is played again.
Every start of a chapter, from the list, from `처음부터`, from `이전` over a step that changed the screen, or the next chapter from its end card, begins on the shell screen as the page first loaded it: an overlay, a focus, a relationship move still waiting or a copy left by the run before never carries into it (`freshShell` in `PracticeApp.tsx`).

A chapter playing dims everything but what its step marks, rings that, and puts a light bubble beside it with what happens and how: the command's chord as keycaps (`Or press ⌘D`), `Press the button, or ⇧⌘↩` for a step with a button, or `Press where it's marked` where this host has no chord for the command (⌥1 in a browser tab).
⏎ performs the step in the learner's place, pressing the chord, the button, dragging the pane with a hand cursor, typing what the learner would say into an agent's prompt and sending it (words the learner wrote there themselves are sent as they are, for the world to judge; the example is typed only into an empty prompt), or right-clicking, or pressing a control such as the pane head's `@`, and choosing from the shell's own menu or list with the hand; a held chord (⌃⇥, ⌥⇥) is let go a moment later, so the shell's own switcher is seen before it lands.
The bubble's button does what ⏎ does, because a press on the guide's bubble, task card or end card is no press on the practice screen.
It leaves the keyboard in the pane that held it, so a chord that acts where the keyboard is (⌘W closes that pane, ⌃⇥ starts from it) acts there, and so does the learner's own key afterwards; and it closes no popover or menu the screen has open, so a step that points into one (chapter 7's `크게 ›`) still finds it.
What ⏎ does is played at a pace a learner can follow, one pace for every move of it (`PACE` in `PracticeCoach.tsx`): the hand points at what it presses, drags or opens before it acts, and a step it performed stays a moment once done, so its result is seen before the next bubble; ⏎ in that moment moves on at once.
A step the learner does by their own key, click or drag moves on as soon as it is done, and the steps the coach replays on its own (`이어서`, `이전`) go at the quick pace, because they only bring the screen back to where it was.
→ moves on from a step that only looks; on a step that waits for something to happen, only ⏎ or the learner's own key, click or drag moves on.
Every step past the first offers `이전`, and ← does the same on a step that only looks: it goes back to the step before as that step began.
Over a look that left the world as it was, only the bubble moves back; over a step that changed the screen, or after the learner changed something, the chapter starts over and plays itself to that step the way `이어서` does, because a chapter can be started over from any of its steps.
The bubble's lower left carries `나가기` on every step, a step that does not move on included, and it goes back to Hide as `Hide로 돌아가기` does; the step reached is kept, so the list offers `이어서` from there next time, and the top center's `← 장 목록` is the way to the list.
The window never reads or writes the operator's clipboard: a copy made there, a menu's Copy name and ID or the page's own ⌘C, goes to the practice world, and a paste, a menu's Paste or ⌘V, takes the last copy made there (nothing before the first).
Nothing on the practice screen leaves it: a link, a window the shell would open in the system's browser (a PR card's `GitHub에서 보기`), a page navigation and a call on the desktop host (open in another app, reveal in the file manager, a notification) are held where the page meets the browser and told to the practice world (`practice/outside.ts`), and the desktop app refuses whatever gets past; a browser without the Navigation API holds all of them but a navigation the shell starts by script.
The learner can also do it themselves, by key, click or drag: a step that changes the screen is done when the world says so, however it was done.
⌘K search and ⌘P Open file open over the practice screen as they do in the shell.
While one is open, or a held chord's switcher is up, everything the guide points with steps aside (the dim, the area tags, the ring, the pressed chord, the `예시` marks and the hand), and its bubble moves beside the palette without buttons; the bubble also moves beside a right-click menu it would cover.
Every key is then the palette's but ⏎ where the guide types into the palette's field: it keeps ⏎ while the field of a step that types there is empty, to type the words, and while it is still typing them, so ⏎ opens no row on half the words ([Adding a chapter](#adding-a-chapter)); ⏎, → and ← go back to the guide when the palette closes.
A pressed chord shows large in the middle for a moment, so the keys are seen as they are used.
Every chord is the operator's own: the host's set, the system's, and the operator's stored bindings.
A chapter with tasks lists them on a card in the lower left with a check as each is done and `↺ 처음부터`; the top center names the chapter and leads back to the list.
The last step's button is `마치기`, and past it the chapter stops on an end card in the middle of the dimmed practice screen, in place of the bubble and the task card: `<n>편 완료` with the chapter's title, the tasks it taught with their chords as the task card listed them (a chapter without tasks lists none), the next chapter's number and title, and `↺ 다시`, `장 목록으로` and `<n>편으로`; ⏎ is the next chapter, and on the last chapter, which has none, the card offers only `↺ 다시` and `장 목록으로`, which ⏎ presses.
Chapter 0 (화면 둘러보기) is where the first run and the sidebar's invitation land, so it does not lead on into chapter 1 as if the course were the way in: its last step says the look around ends here and the learner chooses what comes next, and its end card asks `어떻게 할까요?` in place of the summary, with three ways drawn alike: `바로 써 보기` (`연습을 닫고 Hide로 돌아가요`) closes the practice as `Hide로 돌아가기` does, `1편 이어서` with chapter 1's title plays it and carries the ⏎ mark, since ⏎ still plays it, and `필요한 것만 고르기` (`장 목록에서 골라요`) goes to the list; under them the list's line says the Help menu opens the practice again, beside `↺ 다시` (`ChoiceCard` in `PracticeCoach.tsx`).
Chapter 5 (찾고 다시 보기) finds yesterday's ended session in ⌘K by a word of its requests, opens its record in the lane's 보기 slot, finds the PR it made by `#3` and opens that card, and puts a finished task in 다시 볼 것 from its right-click menu; any word of the session finds it, the record's `세션 복구하기` brings the session back in a tab of its own, and the invented PR wears a `예시` mark so it is never taken for a real one.

Chapter 1 closes its working agent by mistake, through the shell's own Stop-work sheet, and brings it back with Reopen closed tab, where it stood and continuing its conversation; its world answers `close_pane` and `reopen_closed` and draws `recent_closed` the way the core does.
The practice screen mounts the shell's close sheet, so a learner's close in any chapter asks as it would in Hide.
Chapter 4 (`delegate`) hands work on by saying it to a lead agent, in three ways: a child it starts in a tab of its own, under the lead in the list (and the line that says `open it beside me` opens it in the right half of the lead's pane instead), then tells something and hears back from by letter; an open agent it finds by what it does and writes to; and an agent named by its reference, which the lead pane's `@` types into the lead's prompt with no Enter, whose terminal ID the lead sends to, with the work added to the request.
The step says that a sidebar row dropped on the terminal, or the menu's Copy name and ID pasted, types the same reference; the world takes whichever arrives, since all three reach it as typed input.
The lead takes the learner's own words for the first three; the last goes only when the request holds the signup agent's ID, and the lead asks for it otherwise; a reference alone is not yet the learner's words, so ⏎ types the example, reference and work, in its place.
The agents' answers and letters are the world's script, drawn in the panes and the list; the learner only types the requests and chooses from `@`'s list.

Chapter 8 hands a Conductor a job of several pull requests as a mission: its world draws the plan, the workers in delegated tabs under the Conductor and the one decision it asks as an ordinary question, in `missions` the way the core keeps them, with its choices and the one it recommends; the first plan opens the pane chip's popover by itself under the shell's own rule ([A mission on its Conductor's pane](#a-mission-on-its-conductors-pane)), the big map opens only from a press on the pane's mission chip, as a lane call the world numbers, and the learner answers on the map's decision banner: a choice pressed or `직접 답하기…` reaches the world as the core's `mission_answer`, and words said to the Conductor answer it too; either changes the plan.

Chapter 7 asks an agent to plan before it changes anything, opens the plan's checklist from the pane's plan chip and the plan large in the 보기 slot, and approves it in the conversation, because Hide has no button that approves a plan; its world draws `plans` and lays the review on the agent's row as the core's question, and any words sent to the agent put the plan up, then approve it and start the first step.

The window holds no daemon connection and carries no token.
It asks the shell of the same origin, over one `BroadcastChannel` (`hide-practice`), for the operator's language, chords, theme and progress, and hands each step it reaches back; the shell dispatches `practice_progress_set`, and the core keeps `ui_state.practice` like the other UI choices.
With no shell answering, the chapters still play and the list says `진행 기록은 Hide 창이 열려 있을 때 저장돼요.`.
In the desktop app the window has no preload: it stands in for the desktop host from its address (`host=electron&platform=…`), so its keyboard runs the desktop chords, and a menu command while it holds the keyboard is dropped rather than sent to the main window's shell; it closes with the main window.

Chapter 2, going back and forth fast (`chapters/switch.ts`), plays on two invented projects: ⌃⇥ (`recent_area_tab`) goes back to the agent used before through the shell's own switcher, ⌥⇥ (`recent_project`) to the project used before, and ⌘P finds a file of the project by name and opens it in the lane's View beside the agent.
The agent step ends on whichever agent the switcher stopped on and the project step on the other project, and a palette closed with Esc comes back with ⏎.
Chapter 3, following agents, plays a lead with two children in tabs of their own beside a second project's asking and finished tasks: the 태스크 list by state, the pane head, its turn card, a child opened from the head's child button and the parent mark at the front of the child's head back to the parent, and the Overview's map, opened with the lead chosen from `개요에서 보기`, first in the pane's menu. Coming back to the lead finishes one child and turns the lead to Needs You, so the list and the head move while the learner watches. Going to an agent from an OS notification is not taught, because the notification is drawn outside the window.
Chapter 6, the project at a glance (`chapters/project.ts`), opens the right lane beside an agent at work with the Project tool's rail icon (`toggle_lane_project`), so ⏎ reaches the Project tool even after the learner showed another one, and reads its 현황 tiles and 할 일, whose two invented PRs wear the `예시` mark; ⇧⌘E (`toggle_lane_explorer`) shows the files the agent changed, ⇧⌘L (`open_project_memory`) the project's Memory cards, and ⌘E puts the lane away.
A tool step ends when the lane shows that tool, whether by its chord or its rail icon, and the closing step when the lane is no longer drawn, by ⌘E or by the tool's ×; the world answers the pin too, and nothing opens from a tile, a row or a file, which the bubbles say.

### Adding a chapter

A chapter is three new files and no change to a shared one, so chapters written side by side do not meet in a file.

- `web/src/practice/chapters/<id>.ts` default-exports the chapter (`chapter()` in `chapter.ts`): its `id` (the file's name and its catalog prefix), a `number`, a `version`, the `world` it plays in, and its steps.
  `chapters.ts` lists every file there by number and refuses a file whose chapter has another id or a number another chapter has; the next chapter is the next number there is, so a gap is allowed.
- `web/src/i18n/resources/practice/<id>.ts` default-exports the chapter's words in every language, under `practice.<id>.title`, `practice.<id>.summary` and `practice.<id>.<step>.kick|title|text|quest`.
  `web/src/practice/words.ts` joins every file there when the chapters load, refusing a key outside its file's chapter, and adds them to the window's words; they stay out of the shared catalog (`practice.ts` keeps only the guide's own words), because the desktop host bundles that catalog without Vite, and the catalog and copy checks read them beside it.
- `web/src/practice/worlds/<id>.ts`, when no world there fits: the world the chapter plays in.

A step names what it rings (`target`, a selector into the shell), where the bubble sits (`side`), the command it teaches (`command`, never a chord), a `gesture` when ⏎ should press a button, drag a pane, type into a pane's terminal (`type`), choose from a right-click menu (`menu`) or press a control and choose from the list it opens (`pick`, the pane head's `@`) instead of pressing the chord, and `done`, which reads the world, or `shown`, a selector whose appearance finishes a step that opens a card, a popover or a page with no core event behind it; a step with neither is a look.
A world (`web/src/practice/world.ts`) gives the first state, answers the core events the chapter's steps dispatch as the core would (an event it does not need answers nothing), and draws the snapshot, what the core sends beside it (`frames`: the ⌘P index's answer, the open documents, ⌘K's session search, the 보기 slot's record and link record, the Explorer's folder listings and Git changes, and the Memory tool's Project, each empty when a world sends none), and each pane's screen.
What is typed into a terminal reaches the world as the terminal's `key` events and a copy as `practice_copy`; a pane whose screen the world changes is cleared and drawn again, so an agent's answer appears as the world writes it.
A world whose chapter types into a pane keeps that pane's unsent words (`prompt`), which the coach reads to send the learner's own words rather than its example.
A `type` gesture can name a field (a palette's search box) instead of a pane: ⏎ types the words in letter by letter, the coach keeping ⏎ while the palette is open until they are in, and a field the step's `command` opens is opened by the first ⏎; a field the learner typed into on that step keeps their words, which the palette answers as they are typed.
A step done inside a palette names the command that opens it and, when the palette forgets what was typed, the words to type, so ⏎ alone reopens a palette Esc closed: it opens, then types into the empty field, then is the palette's own ⏎ once the field holds words.
A world names in `examples` what it invented that could pass for a real record (a pull request's title), and the coach marks each `예시` just past the first words it shows (a row's title, a card's heading), never past its own right edge, so the mark covers nothing beside it; the shell's own components stay as they are.
`worlds/kit.ts` holds what several worlds build with: a checkout's desk of tabs, panes and groups and the events that change it (`applyDesk`), agent rows (`agent`, with a parent named as the core names it), and the snapshot of one or more projects' desks (`deskSnapshot`), which leaves a tab of only delegated children off the strip and lists each pane's, tab's and desk's close consequence as the core does.
A tool only one chapter needs stays in that chapter's world file, and moves to `kit.ts` when a second chapter needs it.
A world's data is the practice window's own, invented for it: no world imports the gallery's fixtures.
`worlds/tour.ts` plays on `data/tour.json`, a whole snapshot kept with each time written as how long before now it was (`$msAgo`, `$sAgo`, `$isoAgo`, `$daysAgo`).
Bump a chapter's `version` when its steps change, so a learner's old progress reads as `바뀜`.
`web/e2e/practice.spec.ts` plays every chapter to its end with ⏎ alone, so a new chapter is played there without a change to the spec; a step whose ⏎ moves nothing stops it.
`web/src/practice/worlds.test.ts` sends every chapter's world, at each step the shell's commands and pane focus can reach, every command the keyboard runs and every pane focus, and fails when the world, its snapshot or a pane's screen throws; a step only a drag, typing or a palette reaches is checked from the state before it.

## Search keyboard navigation

Web owner: `web/src/SearchPalette.tsx` (⌘K), `web/src/search.ts`, `web/src/relations.ts`, `web/src/searchDetail.ts`, `web/src/searchGithub.ts`, `web/src/Palette.tsx` (⌘P and the diff palette), `web/src/components/sidebar-header.tsx`; core owner for the GitHub search: `herdr-core/src/runtime/issues.rs`.

⌘K goes to things; it runs no command except `에이전트 시작…` and `브라우저에서 URL 열기`.
On the web, the sidebar's Search row, which shows `Search` and the bound chord, opens the same palette Command+K opens, and the query row carries an `Esc` keycap.
Its own wide layout is a list on the left and the highlighted row's detail on the right, one and a half times the width ⌘P, the shortcut sheet and the start panel share; a dialog under 800 px, too narrow for both, draws the list alone.

### What a query finds

A nonempty query also searches saved session goals, goal flow names and original requests in a separate `세션` group, including ended sessions within Copied history.
Enter on that group opens the record in Tools; the existing Agents group still opens the live agent.
With summaries Off it searches saved raw requests only, without generated goal or flow matches.
The core's bounded local record search runs independently of other groups, drops an answer for an older query and makes no GitHub request while typing.
The session group carries its current loading or failure state; a successful search with no session matches draws no group, preserving the palette's existing no-match state when no other kind matches.
The creator-session action in an output panel opens the same record.

A query finds an agent, project, checkout, device, issue or pull request by name, and an issue or pull request by `#number` (`#273`, or `273`); an exact number match lists first, as its own row, issue and pull request separately.
After them come the agents the core links to a pull request of that number (its session made it or its branch has it; every one the core links, a merged one included, which is more than the agent's PR mark draws), in `Agents` on every device, so `#275` reaches the agent that works on pull request 275 though no word of its row holds the number; an issue's number brings no agent.
An agent is found by its title, by the name Herdr knows it by (`hide agent spawn --name`, `herdr agent rename`), by its place and by its state sentence; the core carries the Herdr name on the agent row (`herdr_name`), on this machine and on a device alike, and leaves it out when Herdr has none or has only a name Hide made up from a pane id (a task's `hide-claude-w9j-p52`, a wake's or a fork's), which would print the id the rule below keeps off every row.
A query holding `:`, as every pane's Herdr id does (`w9J:p52`, as Copy name and ID carries it or `$HERDR_PANE_ID` gives it), lists first the agents whose id holds it in its case, on every device, the whole id before longer ones (`:p5` lists `w9J:p5`, then `w9J:p52`); without a `:` no id matches, and no row prints the id.
Only digits in the query match a number by substring on other rows.
The rows are `Issues`, `Pull requests`, `Agents`, `Projects`, `Checkouts` and `Devices` (while another device is registered), then `Commands` holding `에이전트 시작…` and `GitHub`; a group stands where its best result ranked and keeps its results in rank order, so grouping never moves the best match off the first row.
Search covers every connected device; a result not on the device in front carries that device's chip after its title, and choosing it brings that device forward with it.
An agent row is the agent's status mark, its title, and under it its Herdr name, place and state (`observer-instant-pane-topology · hide › main · Working`); a Herdr name the title already reads as, ignoring case, or no name at all draws nothing for it; an issue or pull request row carries its number, state and, for a pull request, its CI.

`브라우저에서 URL 열기` is listed first while the query is a web address: one written with `http://` or `https://`, or a loopback host a dev server runs on (`localhost:5173`, `127.0.0.1:3000`), read by the same rule as the View address field (`addressUrl`); any other text is a name to search.
Enter opens the address as a browser display in the Workspace in front, in its active View area, as New tab does.
Pages are drawn by the desktop app, so in a browser tab, and on a screen with no Workspace in front, the row stays listed, dimmed, with the reason under its title, and Enter does nothing.

### What an empty query shows

Nothing typed lists what is connected to the thing in front: the issues the thing works on or closes on top, then one group per checkout with a head, the checkout's pull request, and the agent lineage under it.
The agent the keyboard was in is tagged `여기` and choosing it does nothing; a parent that works in another checkout is one `↑ 부모` line under the agent, and a child delegated to another checkout stands in that checkout's own group.
In front of an agent pane the thing is that agent; in the Workspace elsewhere it is the checkout in front; a connection the snapshot does not name has no row.

Under Related, `Recent` lists the checkouts last brought to the front (PRD cmdk-recent), newest first, at most five, as the same checkout row Related uses: the branch over the project, and the device's chip after the title while the checkout is not on the device in front.
It leaves out the checkout in front and every checkout Related already lists, and fills the five from the rest of the record.
The core keeps the record (`ui_state.recent_checkouts`, ten checkouts, newest first, saved with the rest of the UI state and so still there after a restart): a checkout is recorded whenever it comes to the front by any path (the sidebar, ⌥1-9, ⌘K, Herdr's own focus, a device's focus), moves to the top if it is already there, and the oldest leaves past ten.
A device's Home is never recorded.
Choosing a Recent row opens that checkout, bringing its device forward when it is on another one.
A checkout of a connected device is that device's live row; one a connected catalog no longer lists is not drawn, and the core drops the record of a project unregistered, a worktree removed through Hide or a device removed (a worktree deleted outside Hide only stops being drawn, and its record ages out of the ten).
A checkout of a device that is not connected stays as a dimmed row drawn from the names the record kept (the branch, the project and the device's name): arrows pass over it, its detail names the checkout and offers nothing, and Enter does nothing and says nothing.
Its row is the live one again when the device reconnects.
A typed query hides Recent and searches as before, and recency does not rank results.
Recent does not hold pull requests, issues or agents; ⌃Tab stays the agent pane cycle and Recent is the way back to a checkout. Unlike Recent Panels and Recent Projects, which the page holds for the session, it is the core's saved record and is still there after a restart.
On a screen with nothing in front, such as Settings, the palette shows Recent alone; with no record and nothing in front it is the input alone with the placeholder `이름이나 #번호를 입력하세요`.

### The detail

The highlighted row says what it is with the facts the snapshot carries and no others (a missing value has no line): a pull request's review, branch and the issues it closes, an issue's owner checkout and closing pull request, an agent's checkout, the pull requests the core links to it (`#275, #260`) and last words, and `N분 전 읽음` for the GitHub-backed ones.
A pull request or an issue with sessions in the link record adds `세션 N`, the count the record's summary carries (PRD link-graph B35); with none there is no line, and the list and Related are unchanged.
Under `관계` it draws the same groups the empty list draws for the row, when it has more than itself; the footer line says what ↵ does.

### GitHub

Typing never calls GitHub.
Opening ⌘K asks once per app run for the project in front when nothing has read it (one of the core's own, or one of a node that dials the core; never a dialed device's), and a checkout row shows a spinner while that read has no answer and a warning mark with the last value's age when it failed; the reason is in the log.
A query with a GitHub project the core reads (its own or a node's that dials it) ends with `GitHub에서 "…" 검색`; choosing it sends one `github_search`, and the row shows a spinner, then the pull requests and issues GitHub holds under `GitHub` (an exact match to one already held is not listed twice), `GitHub에도 없음` (or `검색 결과 없음 · 일부 저장소만 검색` when the core covered only some of the projects), or `GitHub 검색 실패 · 다시 시도`.
A pull request's row, held or found on GitHub, leads with its PR mark without the number and says its state in the muted word; a found one's state is the core's (`pr_state`), as a held one's is.
The query's words match like a search box, and a query is capped at 200 characters.
The same query already searching is not started again, and an answer for an older query is dropped.
Search targets use confirmed project repository choices and valid catalog repository names; an unknown or malformed local name is resolved by the worker, while a node project without a valid name is left out.
Choosing a GitHub result opens it in the browser.

### Keys and results

Up and Down move the selection in display order, stopping at either end, while typing continues in the query field.
Return opens the highlighted result through the existing agent, checkout, Overview or Pull requests actions (an issue or memo opens the Overview on the device that holds it, at the item in 백로그 while it is not started, and once it is, at its task in the sidebar's 태스크 list with its record, and a pull request opens its PR card); Escape closes the sheet.
The selected row scrolls into view.
Filtering preserves a surviving selection by identity; a retired selection moves to the first remaining result.
Empty results have no selection, and arrows or Return require no modal acknowledgement.
A stale result is checked against the live result set before execution.

### File search

⌘P lists what hided's index ranked for the typed query, with the same focused query field and first-result selection, and never executes results from a previous query or checkout while its asynchronous index is updating.
The index follows files made or removed after it was built, on this machine and on a device: opened five seconds or more after its last walk, ⌘P lists the old files at once and says `Indexing…` only while it has none, and the list updates in place when the new walk lands.
↵ opens the highlighted file; ⌘↵ opens it beside the active View area, and the footer hint says so.

## Pane header lineage and ownership

Web owner: `web/src/PaneRelations.tsx`, `web/src/lineage.ts`, and the [Agent panes and the Agents explorer](#agent-panes-and-the-agents-explorer) subsection of Web Workspace above; the [agent workflow contract](../design/agent-workflow-review.md) is the authoritative lineage/ownership document.

The header identity, the second row, the body's notices and shared badge follow [Agent panes and the Agents explorer](#agent-panes-and-the-agents-explorer).
A relationship Open or ancestor step publishes one request-scoped pending state; repeats are ignored until it settles and Retry starts a new request.
Target retirement, refusal, timeout or remote failure preserves pane geometry and topology and retains the scoped reason, retryability and Dismiss outcome.
The shell never infers success from an old layout, sends a rollback focus or attributes an unrelated global error to the control.
A root has no Return control; a child on another device carries that device's real name in the popover.
Ordinary delegated rows stay subdued, while a child raised by the core receives operator attention under [status-model.md](status-model.md#delegated-escalation).
A descendant change still turns its ancestors unread without changing their own group.
A pane whose native session Hide does not hear wears a small, muted broken-link icon in its header (`web/src/PaneConnection.tsx`), named `Hide integration limited` (`Hide 연동 제한`) on hover and keyboard focus.
A detected process that has not reported a native session yet carries no connection judgement or icon, just as a connected pane, a pane the core has nothing to judge for, a sleeping agent and a plain shell draw none.
The icon uses the existing ghost icon button and Hint, with no warning triangle, text badge or banner, and remains distinct from the agent's work-state mark.
The one exception is a copy too old for Hide (`update_agent`): the header wears a small ghost "Update needed" (`업데이트 필요`) badge with a warning dot instead of the icon, because the operator can fix it only outside Hide and needs to know which pane runs it; its popover reads "Not connected to Hide" (`Hide와 연결되지 않았어요`), says status, letters and Memory are off for that agent and version and the version it needs, and shows the one update command with Copy (or the agent's install page), Not now and no Reopen; nothing else on the screen says it.
Click, Enter or Space opens its existing popover; Esc closes it and returns focus to the icon without opening another hint over the popover.
A Codex on the shared server reads "Hide can't follow this Codex" with its reason, Reopen with Hide, Not now, and the existing confirmed shared-server action; a session with no hook report reads "Reconnect this session to Hide", says chatting can continue and offers Reopen and Not now without claiming when the hook was installed; a setup problem names the cause and offers no Reopen, since a Reopen would change nothing; a session Hide cannot reopen (an agent other than Claude Code and Codex, or one on another device) that started before its piece went in, as after an update, says to restart the agent to connect it.
Reopen is hidden where the core says the pane cannot be reopened (a pane on another device).
Not now closes the popover only and the chip stays; Settings has no Reopen button.
Reopen sends one `pane_reopen` however often it is pressed before the core answers, shows the core's pending state as a busy button, and on a refusal known before the agent was touched leaves one line from the refusal code (the agent is working, no saved conversation, the folder is gone, the agent would not stop) while the pane stays as it was; a session that reconnects loses its chip and closes the popover with it.
A start refused after the agent was ended leaves a pane with no agent and so no chip to carry a line, and it is the one Reopen failure that is a notice rather than only a diagnostic (design principle 13): the operator can act on it, because the agent is gone from the pane and its conversation is kept, so they start it again.
The shell turns `last_error` kinds `terminal.input_hold_failed` (a new tab, split or fork exceeded its input hold bound) and `pane_reopen.not_restarted` into the one-line notice bar (`web/src/errorNotice.ts`), worded by kind (`web/src/coreError.ts`), and the notice stays until dismissed or until the front device changes, because the core clears `last_error` at its next event of any kind; every other failure of Reopen stays on the chip.
The link opens a confirmation before anything changes, in the device-removal pattern with no button focused (PRD codex-daemon-apply B1, B2).
It names the machine's Codex shared server, says autostart goes off and the server running now stops, that every Codex attached to it disconnects and continues in its pane with `codex resume`, and that Codex Hide started keeps running; it shows no count of Codex, since Codex reports none.
Its buttons are "Leave it running" and "Turn off and stop", and Escape, a click outside or "Leave it running" changes nothing.
The confirmation closes by itself whenever the popover stops offering the link, and the popover stays open behind it to show the answer.
"Turn off and stop" sends one `codex_daemon_disable` for the pane's machine and shows the answer the kit reads back: turning off, done, or the failure code's line (B3).
A failure before autostart went off leaves the setting unchanged; when autostart went off but the running server did not stop, the line says so and the link stays to try again, which then only stops it (B7, B8).
A finished answer is shown only to the operator who asked in that popover.
`PaneChildren.connection` carries the reason, whether Reopen is offered, and a Reopen's pending or refused state as a code, and `docs/status-model.md`, Not connected and what fixes it, owns what each one means; `web/src/paneConnectionRules.ts` names what each code asks of the operator.
An Overview agent row reuses the same agent identity and state presentation as the sidebar and shared child popover; a missing row means the current live projection has no agent there.
The accent header tint and 2px perimeter mark only the pane that owns input; moving input into another surface removes both while the pane remains selected.
Keys typed right after New tab or a split go to the new pane once Herdr names it and its shell reads keys itself, in the order typed, so its line editor takes them: they are not echoed on a line above the first prompt, and a Backspace over a character typed in several bytes (a Hangul syllable) erases the whole character; a shell that does not read keys within 2 seconds of the first key, or a pane Herdr makes later than that, gets them as they were typed. Outside a fork holder, keys typed while a pane's terminal is still opening reach it once it opens; nothing typed is dropped unless more than 64 KiB waits, which discards what waited and shows a dismissible notice asking the operator to type again, the pane can take a key only more than 3 seconds after it was typed, which discards that key because it is no longer what the operator means to run, or the keys have no pane to go to: the creation is refused or makes no pane, the new pane opens in the background, or the pane closes, is released or its terminal fails before it opens (`docs/ARCHITECTURE.md` names every reason).
Keyboard focus follows the core's answer, not the drawing, so a second ⌘D pressed before Herdr confirms the first splits the original pane again, as Herdr does.
Keys typed after a click go to the pane clicked last, whatever order snapshots arrive in: the header may briefly follow an older snapshot, but keyboard focus does not leave the last-clicked pane until the core has answered that click, and a focus move the core makes afterwards (Herdr's own move, or a refused click) is followed.
Opening a pane from a sidebar row, a 태스크 row, a list, a link or a View's opener mark takes the keyboard to its terminal even when that pane is already the focused one, so the control pressed never keeps the keys and the accent perimeter shows which terminal has them.
Unread weight is never reused to mean parent, child, delegated, or selected.

## Project settings

A Git project or plain folder's gear, project menu and the Overview's project panel (Projects tab) open the same Project settings dialog without selecting or folding that project.
Home has no project settings entry.
The gear shares the reserved action space with New work and More, appears on hover, keyboard focus or an open menu, and stays visible on devices without hover.
Closing the dialog with Escape or its close button restores the opener's focus and leaves the screen, selected device and checkout unchanged.

The dialog is 880 by 640 when the window permits it, with a fixed 200-wide navigation list for Connections, Working style and Memory; only the page scrolls.
Choices are scoped to the owning device and canonical main-worktree root, shared by all of that project's checkouts.
Each change saves immediately through the core's existing state writer, shows pending, saved or failed beside its row, and keeps the last confirmed value until durable acknowledgment.
Closing does not cancel a save, and reopening preserves the failed intent's Retry.
A later window's save does not turn an already completed change back into Pending.
An unavailable device or missing project retains its last observed values read-only, with a retry at that surface; a newer settings version is also read-only with an update reason.

Connections shows the actual GitHub repository or why none was detected, with automatic detection or an explicit owner/repository override, and the existing Auto, GitHub and Local issue choices.
An invalid repository has an inline reason and is not saved.
Changing the connection does not change Git remotes, account login or permissions; provider failures stay with the provider rather than silently switching to Local.
Linear is a disabled Coming soon row.
Previously linked issues and recorded issue, pull-request and session addresses keep their original repository identity, including an unchanged legacy bare issue token.

Working style chooses the initial location for project-scoped New work: an existing checkout or a new worktree.
A new-worktree start asks for its branch and actual base before starting and preserves the task, agent and model on failure.
An explicit checkout, issue or pull-request start, CLI target, resume, wake, fork and running agent keep their existing behavior.
The base branch is also the project's existing Git comparison and merge base; an absent saved branch falls back to an existing branch with a reason.
Plain folders cannot select worktree creation or a Git base.
Check the remote in the background (원격 자동 확인) is on for every Git project, including one whose record predates it; turned off, Hide stops fetching that project and its header detail falls back to the operator's last fetch, and turned on again it fetches within the minute (PRD git-freshness B9).
A plain folder shows it off and unavailable with the existing Git project reason.

Below the base branch, the Guardrails group (보호) holds four switches, each with one line on what it stops: Guard working folders, Guard the base branch, Block risky Git commands and Guard files (PRD agent-guardrails B1).
Every switch starts off, and a record saved before the group existed reads with all four off (B2, B30).
Guard the base branch shows the checkbox Block agents from merging pull requests only while it is on (B10), and Guard files shows its path chips only while it is on: `.env`, `.env.*`, `*.pem`, `*.key` and `~/.ssh/` at first, each with its own remove button, and Add path takes a gitignore-style pattern or a `~/` path (B15, B16).
A path past 32 entries, past 200 characters, already listed or not a valid pattern is not saved, and its reason shows under the field, which keeps what was typed and offers no Try again, since the same path would be refused again; the field stays open until the path is saved, Escape closes it without closing the dialog, and focus returns to Add path after a path is added or removed.
Each change sends the whole set of protections and shows saving, then saved, beside the control that moved; saved appears only once the machine that owns the project has the new policy, so a failed delivery shows failed with Try again and keeps the last confirmed values (B3, B4, B31).
A plain folder keeps the three Git switches off and unavailable with the existing Git project reason, once, and Guard files stays usable; a Git project whose base branch is Automatic with no remote default cannot turn on Guard the base branch and says to choose a base branch first (B17).
When an agent installed on the project's machine is switched off in Hide or has a Hook that is not in place, Codex's untrusted hook included, one line under the group names those agents, says the guardrails do not apply to them and links to Settings > Agents; with every agent covered the line is absent (B25).
A refusal reaches only the agent, as an English reason naming the protection and what to do instead; the operator's screen shows no alert, banner or badge for it (B18), and [agent-hooks.md](agent-hooks.md#project-guardrails) owns what each protection stops.
The group promises a guardrail, not security (B28).

Memory uses the existing consent and kit; its row shows hook preparation, pending writes and project-scoped failure with Retry, while retaining the confirmed value.
That Retry applies only to enabling or disabling Memory; card actions stay in the Hide AI tool's Memory tab with their existing confirmations.
Memory promotion PR rows retain their original repository-qualified link after Connections changes; checks, status and agent identity join only that exact URL.
A legacy promotion whose original repository cannot be proven shows PR verification unavailable and keeps its PR button disabled.
Turning it off keeps existing cards.
While the dialog is open its project owns the daemon's single Memory read slot, and the tool rail's project is restored when it closes.
Open project Memory is one scoped navigation intent that closes the dialog and opens that project's usable checkout with the right lane on the Hide AI tool's Memory tab; a delayed remote focus changes the lane only on arrival, and refusal leaves the earlier checkout's lane unchanged.
The shell waits for both the target checkout and its Hide AI tool; the core's lane call then brings the lane forward, over the agents in a narrow window.
A newer explicit navigation cancels the delayed Memory choice so its later arrival cannot take the screen back.
Without a usable checkout it gives a local recovery reason.
All pages use the four interface languages, existing settings rows, system controls, dialog keyboard containment and theme tokens.
`web/src/ProjectSettingsSheet.tsx` owns the dialog and `web/src/projectSettings.ts` reads its confirmed scoped choices.

## Settings: each machine's install kit

Settings opens as one modal, from ⌘,, the sidebar's gear and the desktop menu's Settings…, about 880 wide (`--size-settings-sheet-w`) and as wide and tall as the window allows with the window inset (`--size-settings-sheet-window-inset`) clear on every side (PRD settings-left-nav B1, B13).
Its left is a list of pages and its right the chosen page; the list keeps its width (`--size-settings-nav-w`) and its place while only the page scrolls, down to the smallest window, 720×480 (B12, B13).
The list reads, top to bottom: the group name `App` with General, Notifications, Usage, Devices and Shortcuts, the group name the core's machine goes by (`This Mac`, as the sidebar calls it, and its name on a window of a machine whose core runs elsewhere, since Hide AI, Mobile and GitHub are the core's machine's) with Agents, Hide AI, Mobile and GitHub, and About followed by Privacy apart at the list's foot (B2).
Its top inset uses `spacing-sm` and group labels use `spacing-xs` above and below, so the additional Usage page fits at 720×480 with the default Interface text without shrinking row targets.
When Interface text makes the list overflow, its grouped pages scroll independently while About and Privacy stay pinned at the foot; every page remains reachable by pointer or keyboard, and scrolling either the list or the page does not move the other.
Each page has its mark and its name, the names are the same in every language, and a group name is a label, never a page.
The chosen page has the selection fill; Up and Down move through the list and show each page as they go, Home and End reach its ends, and Tab leaves the list for the close button (and a remote device's chip before it) and then the page, which takes focus itself with a ring so the keyboard can scroll it before reaching its first control (B3).
A page has its title and no description line (B4).
While another machine than the window's is selected, a small device chip beside the page title names it and says that selecting it changes where files, Git and panes run, not where Settings are kept; with This Mac selected the chip is absent and no sentence about ownership is shown, and on a window of a machine whose core runs elsewhere This Mac is that machine and the core's machine has the chip.
Esc and the close button close Settings and leave the screen under it as it was (B14).
Settings opens on General unless the way in names a page: Explorer's and a device row's Add device open Devices with its Add dialog in front, Memory's and Factory's Hide AI links open Hide AI, and the first-run wizard's phone link opens Mobile (B11).

### General

General has the Appearance group.
Appearance contains Language, Color theme, Mode, Accent, Interface text, Terminal text and Terminal font; Accent shows its swatches and no hex value (PRD settings-left-nav B5; hide-color-theme B1).
Interface text, 11 to 17 pt with 13 the default, sizes every text outside the terminals and the editor's code, the sidebar included.
Terminal text, 10 to 24 pt with 14 the default, is every terminal's size; it carries one note that a single pane is sized with Larger text and Smaller text in that pane (⌘= and ⌘- on macOS) and the editor's text with the same keys in a document.
A pane's own size multiplies Terminal text, so moving the slider resizes every terminal and keeps a zoomed pane that much larger than the rest; each terminal takes its new rows and columns at once.
The core keeps both sizes in its ui state, where they survive a reload and a relaunch, and refuses a size outside its range into the diagnostic log, keeping the last one.
Terminal font is a text field for the name of an installed font, empty by default with `Default monospace` as its placeholder; leaving the field or pressing Enter sends it, Escape puts the stored name back, and the core keeps it in its ui state beside the sizes, refusing a name that is untrimmed, longer than 128 characters or holds a control character into the diagnostic log.
Every terminal draws in that font first, then in the default monospace font, and its note says that icons and Korean, Japanese and Chinese text are fitted to the font's cells and that with some fonts a gap can remain between CJK characters; a name this machine does not have turns the note into a warning that terminals use the default font.
Whatever the font, Nerd Font icons come from the bundled Symbols Nerd Font Mono shrunk to one cell, and Hangul, kana and Han from the system's own CJK fonts (Apple SD Gothic Neo, Hiragino Sans and PingFang on macOS, Malgun Gothic, Yu Gothic and Microsoft YaHei and JhengHei on Windows, Noto Sans CJK on Linux), led by the operator's first CJK language, each enlarged to fill its two cells without rising above or sinking below its row; a chosen font that has its own icons or CJK glyphs keeps them.
Color theme chooses the palette every window draws in, Hide or Neutral, as a segmented toggle like Mode's, each name led by a small swatch of its brand color, the same in Light and Dark (Hide the owl mark's blue-grey, Neutral a grey), and no color code; it waits for a live connection, as Language does.
Hide is the landing page's Owl Field palette and the default, including for an install that never chose; Neutral is the grey chrome.
Both have their own sixteen terminal colors, each readable at 4.5:1 on its background, and the choice changes the sidebar, tabs, pane headers, terminals, editors, sheets and popovers at once, keeping a terminal's content, scroll position, half-typed line and selection.
Status colors, the accent choices and what uses them, the destructive color, and the diff, PR and file icon colors are the same in both.
The status marks' colours (the warning badge, the working ring, the green and the plain dot) are the same in both, while quiet text beside them, such as a read request line, follows the palette's `--subtle-foreground`; a quiet row's faint ring follows the palette's `--muted-foreground`.
Mode is the former Theme row unchanged: System, Light or Dark, each color theme having both.
The choice is the core's, one for every window attached to it, and survives a restart of the app or of hided; the first paint, the desktop window before the page draws and the host status page are Hide Dark, and a paired phone draws the operator's color theme in its own light or dark.
A stored value this build does not know opens Hide with only the `ui_state.color_theme_unknown` diagnostic.

### Privacy

Privacy is the last Settings tab and holds Send usage statistics, enabled by default and disabled while disconnected.
Its short note says that connection, AI call and agent start counts are collected with the app version and operating system, and that conversations and code are not sent.
The core persists the choice, shares it between windows and preserves it against stale generic UI saves.
Turning it off discards waiting events and prevents new ones; a request already in flight may finish, and turning it back on never replays disabled events.
Product analytics has no autocapture, session recording, person profiles or location enrichment; [the architecture contract](ARCHITECTURE.md#product-usage-statistics) owns the PostHog US destination, exact wire fields and worker limits.

### Notifications

Notifications holds every notification Hide sends, in two groups (PRD settings-left-nav B6).
The first, named for this machine and only in the desktop app, has two independently stored switches in the existing row group: `Notify me when an agent needs me` (Needs You) and `Notify me when an agent finishes` (Done).
Both default off before the wizard's choice; the fresh wizard enables Needs You only, and `Not now` disables both.
An existing install whose old shared switch was on migrates to both on; an old off or absent switch migrates to both off, so no existing notification choice silently disappears.
Each toggle sends one event and persists through the core's existing UI-state writer; a repeated choice publishes nothing and a stale generic UI save cannot replace either choice.
While either switch is on, the group shows what macOS allows with `Open System Settings` when it blocks Hide or `Send a test` when it allows, or a `Test notification` row on Windows and Linux.
The note says notices arrive only while no Hide window is in front, and the icon count sums the notification groups enabled by their own switches (First run above).
The second, Phone, is the push to a paired phone, the dropdown Settings > Mobile below describes.

### Usage

Usage follows Notifications in the App group, with a gauge mark and the localized Usage title.
The page walks the shared adapter declaration so all seven agents appear even before a read.
An agent with limit support has one card: its mark and name with Hide, then a row for each declared account window with its current value and a pin switch.
A window the account reports as not offered has no row or switch.
When a fresh reading explicitly marks a window not offered or unreadable, the core removes its saved pin so an invisible row cannot hold a chip slot; loading, failed and retained readings preserve choices.
The initial choices are the weekly window for Claude and Codex; the core stores the choices for every connected window and retains them across restart.
At three pins, an unselected switch is disabled with “The sidebar shows at most three chips. Switch one off to pick this one.” beneath that row; removing a pin enables it again.
Every window switch stays in one right-aligned column beside the first line of its label; a long value wraps below the label, including unavailable and refused-fourth-pin rows.
Hide folds the card to its header, Hidden and Show, preserves the pins, and removes the agent from chips and the popover.
The five agents without supported limits appear in No limit reading with a reason from the adapter declaration, without Hide or switches.
Grok's weekly reader is not built, OpenCode Go would need an API key Hide does not hold, Pi and omp count toward their signed-in account.
The page reuses the settings group, row and switch parts; loading is an ellipsis and a missing reading is Unavailable, never an invented zero.

### GitHub

GitHub is the GitHub row: its description, whether `gh` is signed in, and Check again, which asks the core to read GitHub again for the registered projects (PRD settings-left-nav B8).
The core reads GitHub with its own machine's `gh`, so in a window on another machine a signed-out `gh` is named on the core's machine ("Not signed in to gh on Mac mini") with the command that fixes it there (Run on Mac mini, `gh auth login`, with Copy) in place of `gh`'s own reason; on the core's own machine the row says it plainly ("Not signed in to gh") with the same command under Run in a terminal (PRD core-host-node-move B15, B1).
`gh`'s own reason stays in the core's log on every machine; a missing `gh` has no one command that installs it on every system, so its row names the state only.
Hide has no GitHub sign-in of its own, so there is no connect or reconnect action beyond that read.

### About

About shows the `hide` version and two Herdr lines: `Herdr server` with the version Hide is connected to and `· same as Hide` or `· Hide has <version>`, or its failure in one line, and `herdr in your terminal` with the version the operator's terminal finds, absent when it finds none (PRD herdr-takeover).
A line carries a button only where there is something to do: the server's `Move` while a server on Hide's protocol was left with `Not now` or its move failed (`Moving…` while the core moves it on its own), and, while the terminal's `herdr` speaks an older protocol, the quiet `<version> · its commands are refused` with the command to copy and what it does, which Hide runs itself after a move unless it needs `sudo`; only versions Hide read are shown, and opening About reads them again, so a `herdr` the operator just updated reads as it is now.
A closed Details disclosure holds the rest: the process, lifetime, state file, page connection, Herdr's runtime message, version, protocol, socket and binary paths, the environment rows, the recent diagnostics and Copy diagnostics (PRD settings-left-nav B9).
The Protocol row shows one number while the protocol Hide expects and the one Herdr answered are the same, and `<received> (expects <expected>)` when they differ.
Version and Protocol come from the answer to `session.snapshot` the core last accepted, so a Herdr that is connected never shows a missing version or protocol.

In the desktop app, an Updates row and a preview switch sit under the `hide` version (web owner `AboutTab.tsx`, `web/src/updates.ts`; host owner `desktop/src/main/updater.ts`; PRD auto-update D-20 to D-22, B25 to B28):

| State | The row says | Action |
| --- | --- | --- |
| Up to date | `Up to date · checked <how long ago>`, which moves with the clock while About is open, or `Not checked yet` | `Check now` |
| Looking | `Checking…` | none |
| Downloading | `Downloading <version> · <percent>` | none |
| Ready | `<version> ready · installs when you quit`, or on Linux `<version> ready · restart to install` | `Restart now` |
| Failed | `Couldn't check for updates` or `Couldn't update`, with the reason in the app's log | `Check again` |
| Off | `Updates are off in development builds`, or on macOS `Move hide to Applications to get updates` for an app on the disk image or in a folder it cannot write | none |

A ready update stays ready while a later check looks; a newer release that check finds is downloaded in its place, so the row shows that download and the dot and Help's line return with the newer version once it is ready, since the updater keeps one pending update.
`Get preview versions (rc)`, with a note that turning it off never goes back to a lower version, is off at first, kept in the desktop profile, and cannot be turned while updates are off.
`Check for updates…` in the app menu (macOS) or the Help menu (Windows, Linux) opens About and looks at once.
A browser tab and the phone show neither row.

### Language

General includes Language, using the existing Settings row and selector pattern.
Its options name English, 한국어, 简体中文 and 日本語 in their own scripts, plus Use system language.
The selected value comes from the core snapshot; an edit keeps the confirmed value visible until the next snapshot.
The selector remains available while connected, so a concurrent choice from another client cannot strand a local pending edit.
An explicit choice is shared by connected shell windows; Use system language resolves separately in each client, with English for unsupported languages.
[LOCALIZATION.md](LOCALIZATION.md) owns the policy and the remaining integration surfaces.

Settings > Devices lists This Mac and every device, one row each (PRD settings-cleanup B54, B55, B56): the name, a line with the SSH alias, the platform its helper runs on and the Herdr version it reported, and the connection state, with Test (a device) and a ⋯ menu at the end.
`+ Add device` is one button above the list on the right; the Add dialog is on the page only after it is pressed, and every other way in (the rail's `+`, the hidden rail's `기기 추가…`, Add a project's host list) opens Settings › Devices with that dialog in front.
The dialog closes by itself once the device is listed.
The Add dialog lists the Host entries of the daemon machine's `~/.ssh/config` with the address each leads to (`user@host:port`, which `ssh -G` resolves without connecting); it asks hided for them each time it opens (`ssh_hosts_list`, answered in `status.ssh_hosts`) and says it is reading until they arrive (PRD settings-cleanup B50 to B52).
Choosing a Host fills the Name with the alias, which the person can rewrite and which later choices no longer replace; Add stores only that name and the alias, so the dialog has no input for a username, port or key and no way to type an alias.
A Host a registered device already uses, or one that reaches the same machine as one (its host name in any case) on that device's default server, is dimmed with `Added as <name>` and cannot be chosen, and the first-run Devices step reads the same Host as the same device; a Host whose address could not be read is dimmed with a short reason (no `ssh`, `ssh` could not read it, `ssh` took too long); a wildcard Host is not listed, and a config with more concrete Hosts than the list holds says so.
With no Host and no WSL distribution the dialog shows only `Not listed? Add a Host entry…` with an example entry, and no form.
In the Windows app this PC's WSL distributions come first in the same list, the default one first, each named `WSL <distribution>` with `WSL on this PC` under it (PRD wsl-device B4); the list is read from Windows without starting WSL, so opening the dialog never wakes a distribution.
Choosing one fills the Name with that name, which cannot be rewritten, and Advanced shows what Hide installs and no Herdr socket; Add sends the distribution with the same consent as a Host, and the dialog closes once the device is listed.
A distribution already added is dimmed with `Added as <name>`; one whose first-run setup is not finished is dimmed with `Finish setting up the distribution first`, and one on WSL 1 is dimmed with the command that switches it, `wsl --set-version <distribution> 2`.
A PC without WSL lists no WSL row, and a WSL whose list could not be read says so in one line under the list while the Hosts stay choosable.
The list is a radio group: the arrow keys move between the Hosts that can be chosen and Enter adds.
Advanced, closed at first, holds `What Hide installs there` in three lines (the helper and the `hide` command, hooks and Herdr integrations for the agents turned on, and that removing the device takes them off) and the Herdr socket; the description above the list says Hide installs its helper on the device, so adding is still the one place the kit is agreed to (device-parity D-12).
The ⋯ menu offers Select, Connection details…, Allow and install… (a device whose helper is not allowed) or Revoke helper… (one that is), and, after a separator, Remove…; This Mac's menu has Select and Connection details… only.
Revoke helper… and Remove… ask before they act, with the same confirmations and events as before; no button is focused when a confirmation opens.
Revoking the helper stops the device's terminals with its link, and the Revoke confirmation says so: its panes read `helper not allowed · allow it in Settings > Devices` in their body at once, take no key, and come back when Allow and install… gives the link again.
Connection details is a dialog with the helper's state and folder, the helper program, the folder of the `hide` command and the SSH host key the consent is bound to, then every part of Hide's kit with a mark, its name and where it is when installed or its state and reason when not.
Installed is ✓, not on this machine is –, turned off is ○, outdated, not installed or removed is !, and failed is ✕; the state is also read out, since the mark is hidden from assistive technology, and a part that is not on the machine says so once, without repeating its reason.
A healthy kit is on no row.
The missing-Herdr and older-protocol lines explain that Hide's allowed helper installs, starts or moves the pinned Herdr; they do not ask the operator to install Herdr first.
If the helper cannot start Herdr, the row says so even while the device session is unavailable and offers Retry, which runs the install pass again; it is disabled while that pass is running.
An ordinary kit status read keeps this failure visible until an install answers with its next server outcome, and diagnostic process details stay in the log.
A line grows under a device only when something needs the operator: what its connection waits on, a connection that did not come up with its class and what to do, a failed helper with its reason and Retry helper even before the session connects, a helper not allowed (Allow and install…) even when a missing Herdr or an older protocol prevents the session connection, a part of Hide's kit that failed or was removed, outdated or never installed, as one line naming it with Reinstall, or the result of a test they asked for.
What a device's ssh asks is answered on its row, in place of the failure line (`web/src/settings/DeviceSignIn.tsx`, PRD system-ssh-device-link B4-B12): a password, a key's passphrase or a one-time code shows ssh's own sentence over a masked field with `Sign in` and `Cancel`, and for a passphrase the `ssh-add` command (`ssh-add --apple-use-keychain <key>` on a Mac) with Copy, so the key is asked once; a server not yet in known_hosts shows its address and key fingerprint with `Trust this server and connect` and `Cancel`, neither focused nor primary; ssh's yes or no shows its sentence with `Continue` and `Cancel`; a security key shows `Touch your security key`; a sign-in page shows `Waiting for confirmation in the browser` with `Open`, which opens it in the default browser, its address, and that it waits up to 5 minutes; a device waiting for its turn behind the four that dial at once reads as connecting and shows no line, since there is nothing to do about it.
An answer is sent for the question on screen and cleared from the field at once; a wrong answer brings a new, empty field when ssh asks again, and no Retry shows while a question waits.
A connection that did not come up shows one of three states on a warning line, each with its one action (the operator's 2026-10-30 decision, superseding the per-class row of PRD system-ssh-device-link B18-B21): `Sign-in needed` or `Sign-in refused`, with `Sign in` in Retry's place, after a reconnect Hide started on its own, a question that went unanswered or was cancelled, a browser check that timed out, or a sign-in the server refused; `Server key changed`, with `Check in a terminal` and `ssh <alias>` with Copy as its one action and no Retry, since a retry before that check is refused again; `Couldn't connect` with `Hide was updated after it started. Quit Hide and open it again to connect.` and no Retry, when an update replaced the running Hide on disk, since every attempt of that Hide is refused the same way and only opening it again connects; and `Couldn't connect` with Retry, over ssh's last line in monospace (a name not found, no answer, a failed proxy command, an access policy's refusal, any other failure), or, where ssh has no useful line, Hide's sentence: `ssh` is missing, with how to add it on the core's system (Optional features on Windows, `openssh-client` on Linux), OpenSSH is older than 8.4, or the device's system is one Hide does not run on (an Intel Mac, for one).
ssh's other words and the finer class stay in the diagnostic log, and a helper's `unavailable` or `unsupported` line is not repeated under an ssh failure, since the helper is the link (`deviceProblemLine` in `web/src/settings.ts`, B13).
The helper failure includes a device kit download or integrity failure and a build with no matching device release; a missing Herdr or an older protocol must not hide that reason or its retry.
A device reached over ssh whose system Hide does not run on, such as an Intel Mac, shows `Couldn't connect` with `Hide does not run on this device's system.` and the row's Retry, not its helper's `unsupported` line, and offers no helper retry; the Apple Silicon reason goes to the diagnostic log; only a device not reached over ssh keeps the helper's `unsupported` line with its reason.
Reinstall repairs only the parts the line names, reads Reinstalling… while the machine's kit work runs (B8), and the line goes when the parts are whole again; nothing else on the screen reacts, and the detail of every install goes to the diagnostic log (B18).
A kit pass that stopped before changing anything (a folder another account can change or a link in the way, an earlier Hide's open work or unreadable record, another Hide changing the same account) is one line under the machine with Retry, never a line per part: `Hide stopped before changing anything.`, what stopped it with the folder or file at fault, and what to change; when the folder is the state folder `HIDE_STATE_DIR` or `XDG_STATE_HOME` chose, the line names that variable and both ways back, another folder only the operator can change or unsetting it to return to `~/.hide/state`, and on the machine that runs the app the command that stops Hide's daemon there, which reads the variable only when it starts and outlives the app (INSTALL.md, When a kit pass stops before changing anything).
More registered projects than a pass checks is its own line, and a cause this build cannot name says only that the pass stopped and to update Hide, with Retry like the rest.
Retry runs the same pass again, which checks again what stopped it, and reads Installing Hide's kit… while it runs; a read while Settings is open answers the same way, so the line goes only once a pass can run (`web/src/settings/MachineKit.tsx`, `kitRefusalLine` in `web/src/settings.ts`).
A machine whose kit does not run says why in that place: a daemon run from neither the installed app nor a build Hide placed (a moved core runs from one), a device not allowed yet, a device that must be allowed again, or a platform this build does not carry (B11, B17, B21).
The first install reads Installing Hide's kit… under the device; the tab reads every machine once when it opens.
The add form has one Add button and lists, once, what the kit puts on the device and where (B12); a device registered earlier without the helper offers Allow and install… in its menu, with the same list.
The list says Hide installs its Herdr, starts a missing server and moves an older server with its panes kept, preserves saved Herdr profiles, and leaves the server and files it still uses after removal.
Removing a device asks once, names in one line what comes off that device (with its helper folder, `~/.hide/host-helper` by default) and that the records in `~/.hide` stay, or, when its helper is not connected, that the kit stays there; no button is focused when the confirmation opens (B22).
When another registered device reaches the same account on that machine, such as a second Herdr server there, the line says the kit stays for it instead.
The page has no per-device agents line, no coordination retirement row and no Codex per pane row: the kit no longer turns Codex's shared daemon off, and a machine's agents are on the Agents tab.

The rows put the window's own machine first, named This Mac, and name the core's machine on a window of another machine by the name it was registered under before the core moved to it, else the name it gives itself (PRD core-host-node-move B2, `web/src/screenMachine.ts`).
The row of the machine that runs the core always wears a `core` badge with the hint `core 실행 중`, a one-Mac setup included, unlike the rail's crown (Q11 of the plan; B1 keeps the one-Mac rail unchanged).
Test and Retry are offered for a device this core dials, never for a machine that dials in, which has no SSH alias here.
A connected device this core dials offers `core를 이 기기로 옮기기…` in its ⋯ menu, dimmed with `연결 안 됨` while it is not connected; a window of a machine whose core runs elsewhere offers it on no row.
That window's own row offers instead `core를 이 Mac으로 되돌리기…` and `Mac mini core와 연결 끊기…`, and its menu has no Remove…, since a machine does not remove itself.
`연결 끊기…` asks once, saying that the core refuses this Mac's pane input, file reads and browser actions at once and that reconnecting allows them again, and the window's strip then offers `다시 연결` (B16).

A move is one dialog over Settings whose body turns with the move, and every form names the reason first, `Mac mini 기기는 늘 켜져 있어서 이 Mac을 덮어도 에이전트와 폰 연결이 계속돼요.` (B3 to B5):

- checking: `점검하는 중…` while both machines are checked and nothing is changed; when signing in to the other machine needs the operator, what ssh asks shows under it as on a device's row (PRD system-ssh-device-link B33), in the window that asked for the checks only, and only it answers; closing the dialog while the request it sent runs, before the core's first answer too, or closing that window, ends the checks' sign-in whether it shows a question, a page, a key touch or nothing yet, and five minutes on any of them does too, which ends the check;
- checks failed: `고칠 것 2개 · 통과 9개` (the passed count only when every check ran) and one line per failing check, naming the machine and either the one command that fixes it, with Copy (`gh auth login`, `sudo pmset -c sleep 0`), or what to do in words; `다시 점검` checks again;
  a connection that stopped on ssh says what the device row says (`로그인 필요`, `로그인이 거절됐어요`, `서버 키가 바뀌었어요` with `ssh mini` and Copy, `연결하지 못했어요`), and for a sign-in `다시 점검` reads `로그인`, since the checks it starts may ask, while a changed server key and a Hide an update replaced (`Hide를 종료하고 다시 열면 연결돼요`) offer no `다시 점검`;
  a line names the machine the failure is on, so a Hide AI with no agent chosen names the machine whose core moves, since that setting moves with the core, `이 Mac의 Hide AI` with `설정 › Hide AI에서 켜고 에이전트를 고르세요`, while an agent of it not signed in names the machine taking the core (`checkMachine` in `web/src/coreMove.ts`);
  earlier core records on the machine taking the core fail `Mac mini의 이전 core 기록` with `Mac mini에서 정리하세요`, and a folder the copy cannot reach there (a file on the way to its state folder, a link that leads nowhere, a folder another account owns) fails `Mac mini의 상태 폴더` with the cause as its line and `Mac mini에서 폴더를 고치세요`; a link on the way is no failure, since the machine answers the folder by its resolved path;
- confirm: what happens (`몇 초 동안 창에 입력할 수 없어요. 폰에서는 QR을 한 번 다시 스캔해야 해요.`), that it can be undone, and `Mac mini 기기로 옮기기`;
- moving: the five steps with ✓, `…` and `-`, and, while the other machine cannot be reached, `Mac mini에 연결할 수 없어요 · 연결되면 옮기기를 이어 가요`; where that machine may already have taken the core, so only it can say whether the move committed, the footer offers `그만 기다리기`, and for a move back whose core machine does not answer its retirement, `여기로 가져오기`;
- give up (amendment 4): the confirm of that action in the same dialog, `되돌릴 수 없어요.` first, then that this Mac's core starts again as it was and that a core coming back on Mac mini is stopped and set aside, not merged, and one plain line, in the body color like the others, that a Mac mini only cut off, not gone, keeps its core running until this Mac reaches it; `취소` goes back to the steps and `그만 기다리기` is the destructive action, never the default; the move back's `core를 여기로 가져올까요?` says instead that this Mac's core starts from the copy it received and that Mac mini's core stays stopped until it is retired, with no risk line;
- stopped waiting: `이 Mac의 core가 원래대로 실행 중이에요.` and that Mac mini's core stops once this Mac reaches it;
- failed: the step that failed with ×, `이 Mac의 core는 그대로예요.` with ✓, and Retry;
- done: `폰에서 Mac mini의 QR을 한 번 다시 스캔하세요.` and `되돌리기`, which opens the same dialog for the move back.

The dialog only asks; the supervisor's `core_move` frame says where the move is, so a window that closed it and opens Settings again finds the same form, and a request another move holds is answered `다른 이동이 진행 중이에요`.
In Korean a fixed noun follows the machine's name (`Mac mini 기기로`), so no particle depends on how the name ends.

Settings > Agents shows the seven agents Hide supports, Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity, in that order, for one machine at a time (PRD settings-cleanup B8 to B20, B67).
With a device registered, a switch at the top chooses This Mac or a device and the list below is that machine's; with none there is no switch.
This Mac comes first, as in Devices, and each machine is listed once.
`Checking this machine's agents…` shows only while a read runs or before the machine's first answer; a machine whose kit answered without listing an agent (another Hide held its account, a retirement it refused) says `Hide couldn't read this machine's agents.` with Check again.
A device that cannot be reached shows one line and Try again in place of its list; a machine whose kit cannot run says why above a list it may still have.
`Installed N` lists the agents whose program is found on the machine ([agent-hooks.md: Installed means the program is found](agent-hooks.md#installed-means-the-program-is-found)), or that are on and whose program went away, so they can still be switched off; a folder an agent creates does not count.
An agent installed where Hide has nothing to attach to on that machine's system yet is listed here too, with `Not on Windows yet` (that system's name) in place of its switch (PRD first-run-onboarding D-25, B49).
`↻ Check again` at its right asks the kit to read the machines again, reads `Checking…` while it runs and rests again after a moment, since a read that changes nothing sends no frame; a failed read leaves the list as it was and shows its reason in one line.
`Not installed N` is a folded list of the rest, each with its official mark, its name and `Install ↗`, which opens the vendor's installation guide (`web/src/settings/agentRows.ts`); an agent with no program has no switch.
A row is the agent's official mark (the same in light and dark), its name and a switch, with no line about what the switch puts there; `Docs ↗` to the kit's skills page for the agent shows only while the row is hovered or has keyboard focus, and always on a screen with no hover.
The switch is that machine's: turning an agent on installs its skill and hook and the Herdr integration there, turning it off takes out only what Hide installed (docs/agent-hooks.md), and an agent that is off wears no status.
A Claude Code, Codex, Grok, Pi, omp or Antigravity row that is on, whose sessions Hide counts from their session files, says one of two things: `N sessions` for its sessions running on that machine now, or `Ready` when it is set up and has none; a sleeping agent is not running and is not counted.
The count is all the row says about sessions: it lists none and does not say whether Hide hears each one, because a session that runs without Hide is fixed from its own pane header (docs/status-model.md).
Grok wears a `Basic` chip (`기본`, `基础`, `基本`), on or off; Claude Code, Codex, OpenCode, Pi, omp and Antigravity have both prompt intake and spawn refusal and wear no chip.
Every other agent's row shows no status, on or off.
The chip opens a popover with every feature of the kit's feature table, grouped under Herdr basics, session reading and multi-agent collaboration, with a heading, `✓ Supported` or `– Not supported` rather than color alone.
The chip, headings and feature text wrap within the popover in every interface language.
Every supported agent has Herdr's integration, so no row says its status is judged from the screen; a row whose id this build does not know is omitted and its id and device are diagnosed once.
The support order, names, links, logo ids and start eligibility come from the shared adapter's generated contract rather than web-owned lists.
Pi has native titles, Sleep and Fork from its complete session reader beside letters, Memory and the spawn guard from Hide's extension; it has no structured question/plan wait or subagent count.
Pi starts with the existing first-prompt argument boundary and CLI model default, without an extra-folder argument.
omp has native titles, Sleep and Fork from its complete session reader beside guidance, letters, Memory, subagents and the spawn guard from Hide's extension; its row wears no Basic chip.
Its current native/manual title wins over a generated goal; an explicit clear returns to the existing fallback without replaying the conversation.
An unanswered native `ask` enters the existing question state with bounded text and choices even with summaries off, and a correlated answer clears it.
Its question keeps the existing Answer tag in the pane header even while the native session holds for an operator reply.
Grok has native titles, Sleep and Fork from its complete session reader beside the spawn guard, subagent count and letters from its own hook.
At rest its bell opens a turn that takes letters at the first tool call or at Stop; while working it takes due letters at either hook without a bell.
It has no new-prompt letter context or Memory, so its row keeps the Basic chip.
Its `/rename` title wins over a generated goal, an unanswered `ask_user_question` enters the question state with bounded text and choices, and a plan awaiting approval enters the approval state with the plan text.
Cursor is retired from new agent choice, onboarding and installation.
Its retained native chat reader preserves existing conversations, search and minute-precision timestamps without enabling a new start, resume or Sleep action.
Its Basic chip and existing guidance and spawn-guard facts remain, and it offers no model or effort choice.
Archive resume sends the selected source path to the core, which decides whether the native reader requires that exact file proof before starting.
OpenCode has native titles, Sleep and Fork from its database reader beside letters, Memory, subagents and the spawn guard from Hide's plugin; an unanswered native `question` enters the existing question state with its text and choices even with summaries off.
Its question keeps the existing Answer tag in the pane header, as omp's does.
Antigravity has titles from agy's conversation index, Sleep, Fork (Hide opens the same conversation in a new pane with `agy --conversation <id>` and types `/fork` there once its composer reads empty by agy's composer declaration, first erasing what it holds (8 Backspaces a read, at most 64, within 10 s of its own), since Herdr's answer to agy's start-up mode query can leave text such as `27;1$y` in it and `/fork` typed behind text is a prompt, not the command (#1529); a screen the declaration cannot confirm empty (no composer found, a cut screen, text that outlasts the erase) is typed into as before and logged as `fork.composer_unconfirmed`; it is refused in a folder agy does not trust, where the new pane would open on agy's trust question; a read of agy's index that meets agy writing it is read again, up to 20 reads in a row inside the fork's 30 s wait; a fork that still fails closes its pane like any agent's failed fork, logged as `pane.fork_failed`, and leaves the conversation `/fork` made in agy's history, since agy 1.4.0 has no command that deletes one) and its conversation from its transcript, beside guidance, letters, Memory, subagents and the spawn guard from Hide's `hide` block; its row wears no Basic chip, so no feature popover opens for it, and the bell it does not ring yet is named in [agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block).
Its tool-permission question reaches Needs You as an approval, as Herdr 0.9.3 reports it (seen 2026-10-16); its folder-trust question is unmeasured ([agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block)).
Escape closes the popover and focus returns to the chip.
A part that failed, was removed or is outdated shows one line on that agent's row naming it (`Hook: Removed`, `Plugin: Outdated: …` for OpenCode's hook piece, `Herdr integration: Failed: …`) with Reinstall, only while the agent is on; a hook the operator removed stays removed until Reinstall or switching the agent off and on.
An agent that is on whose copy is older than Hide's minimum shows, under its row and without Reinstall (which would be refused again), the versions (`2.1.200 → 2.1.278 이상으로 업데이트하면 Hide와 연결돼요`), the one command that updates that copy with Copy, or the agent's install page when Hide cannot tell how it was installed, and the program's path; its refused pieces leave the problem line, and the next kit read after the update puts them in without a press ([agent-hooks.md](agent-hooks.md#updating-an-agent-that-is-too-old)).
An agent that is on and found whose hook Hide could not put in says why in one muted line without Reinstall, since nothing is broken: its program has not created its configuration folder yet, a file of that name is not Hide's, or Hide does not write that hook on this system (`Plugin: Hide puts it in once the agent makes this folder: ~/.config/opencode`).
A switch-off whose removal did not finish keeps the row from reading Off and says so in its own line.
There is no row or switch for Codex per pane, no per-agent CLI group (that is Hide AI's) and no hooks diagnosis list: the row statuses are the diagnosis.
Idle agents (`Sleep after`) and Starting work (`Link the issue in pull requests`) follow the list.

The first-run agent choice is the first step of the first-run wizard (see First run below), PRD agent-adapters-onboarding and PRD first-run-onboarding, shown when this Mac's kit has never run.
It lists the seven agents Hide supports, Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity in that order, as tiles in a grid, each with the agent's own mark (or a two-letter monogram where no official mark is bundled, `docs/BRAND.md`) and its state: the agents installed here are on and show a check, an agent not installed is dimmed, has no switch and links its official install page (`Install ↗`), an agent installed on a system where Hide has nothing to attach to yet (the kit's `unsupported_system`) is dimmed and says `Not on Windows yet` with that system's name, with no switch, no install link and no entry under `What changes` (D-25, B49), and a tile is a switch (`role="switch"`) pressed with Space or Enter.
Claude Code and Codex are on when they are installed; every other installed agent is on too, since the operator chose the full set, and `Continue` installs what is left on.
Under each name the tile says what Hide knows: `Checking…`, then `Ready` or `Not signed in` for the agents whose sign-in Hide checks (Claude Code, Codex, Grok, Pi and Antigravity; Antigravity says `Installed` until Hide AI's folder for it is made, because Hide reads agy's sign-in only once that folder is ready), and `Installed` for OpenCode and omp, whose sign-in Hide cannot read (`tileStatus` in `web/src/firstRun.ts`).
With no agent installed, one line says so and links Claude Code's and Codex's install pages, and Continue still goes on.
One sentence above the tiles says that Hide adds its own entries to the config of each agent turned on and changes nothing before Continue, and `What changes` opens the list of what Continue writes for each agent left on (its hook file, skill, plugin or extension, and Herdr integration, at the places the kit reports, with the account home written as `~` on every system) and the `hide` command's link; the list follows each flip (B11, D-26).
`Continue` is the only button and the only way on: Escape, a click outside and a close button do nothing, so a stray key cannot finish a choice that leaves Claude Code and Codex off with no hooks; with every tile off, Continue installs nothing and moves on.
Continue holds, reading `Applying…`, from the press until this Mac's kit pass lands, and a second press meanwhile changes nothing; a chosen agent whose piece failed keeps the step on screen with the agent and the kit's reason in one line, and the button reads `Try again`.
The question belongs to the kit record (`awaiting_choice` in `~/.hide/kit/installed.json`), not to the window: closing the app, reloading the page or a failed save leaves it asked, and the next launch asks again until Continue has saved.
The tiles are drawn from the live availability plus the operator's own flips, so an agent that becomes installed while the step is open appears on, and Continue sends exactly what is shown.
A Mac whose kit has already run (an existing install) never sees it, and nothing is held back from it.
Until the choice is made the kit puts nothing for Claude Code or Codex on a fresh Mac either, so the hooks wait for Continue or for Settings, Agents.
A device whose own record waits gets the saved choice once per run, by what is installed there, as soon as it reports (also after it was not connected when Continue was pressed); with nothing chosen it is told that, so its record stops waiting, and nothing is installed for agents.
A device that still waits after it was sent the choice is logged once (`first_run_choice.unanswered`) and not sent it again until it reconnects.
Under `Continue`, one line names the agent Hide AI will use: the first agent in that fixed order that is switched on in the step and signed in (`firstRunAgent` in `web/src/hideAi.ts`, "Hide AI uses Claude Code", `Hide AI가 쓰는 에이전트: Claude Code`), read from this machine's rows of `status.background_ai.providers[].state`, which the step asks the core to probe while it is open; with no such agent the line is absent, and it follows each flip of a tile (PRD settings-cleanup D-18, B45, B46).
An agent installed later appears in these rows off and is never turned on by a pass.

### Conductor

Settings › Conductor, in the machine group after Hide AI, keeps what a Conductor plans with on this machine (PRD software-factory-conductor D-31, B48, B49).
`동시 실행 상한` is how many agents one mission runs at once, reviewers and verifiers included and the Conductor not counted, from 1 to 16; `기본 범위` is how far a mission goes when the operator does not say, up to ready pull requests or through the merge.
`작업자 후보` lists the agents a Conductor chooses workers from, at most 16: each row's agent, its model from the start panel's list (disabled with why when that list cannot be read), its effort automatic or fixed, and what it is for; an agent this machine has not installed is dimmed with `설치 안 됨`.
With no candidates the Conductor chooses among the installed agents; with some, Hide refuses a worker that is none of them (`mission_candidate_refused`, [delivery.md: Missions](delivery.md#missions)).
`검증` holds two controls, and the checkers come from the candidates above.
`검증 강도` is a new mission's verification strength, `빠르게`, `보통` (the default) or `꼼꼼하게`, one segmented choice with what it has the Conductor do, how often a node is reviewed again and the work it suits under it, as `hide mission help` states the rules; `검사는 만든 에이전트와 다른 회사 모델로` (on by default) has the Conductor prefer a checker of another company than the node's worker.
It is only the default: the Conductor proposes each mission's strength in its plan and changes it when the operator says so, before or during the mission, and the map's head shows it ([A mission's map](#a-missions-map)).
A setting kept before strength (the review and verify defaults) reads as `보통`, with the switch off only where the operator had turned another company off.
Every edit sends only the field it changes and the core checks the result; a refusal shows inline, and a mission already created keeps the cap, scope, candidates, strength and switch it was created with.

### Hide AI

Settings > Hide AI is the agent Hide runs behind the scenes (labels, worktree names, Project Memory analysis), separate from the agents the operator runs (PRD settings-cleanup D-14 to D-18, B33 to B47).
Every value is the core's `status.background_ai`; the tab sends `ai_settings` events, one per intent, and an edit is pending until the snapshot carries it, so a choice the core refused simply returns to what the snapshot says.

- **Use Hide AI** is a switch on its own at the top.
  Off, Hide makes no model call: agent titles show each session's own text, worktree names are typed, and Project Memory analysis waits while saved Memory search keeps working.
  Everything below it dims and is out of reach (`inert`), and the stored values stay as they were, so turning it on resumes with them.
- **Runs on** has an Agent select and a Model select.
  The Agent select lists only agents the core would accept (`selectable`: installed, signed in, a Hide AI backend, a call that cannot change files) with each official mark, and the chosen agent stays in it with its state even when it has a problem; the same row shows its state as symbol and words (Signed in, Not signed in, Out of usage until 3:10 PM).
  An agent signed in on a connected device is listed beside this machine's as its name and the device's, such as `Claude Code · WSL Ubuntu`, and choosing it runs Hide AI there on the device's own login (PRD wsl-device B11, B12).
  A first run takes an agent signed in on the core's machine before a device's, and a choice the operator made never moves on its own, not even when the same agent signs in here later.
  While the chosen device is not connected its row reads Unavailable, nothing wakes the device for Hide AI, and its work resumes once the device is back (B13); it cannot be chosen again until then.
  The Model select lists what the agent reported, with `CLI default` first where the agent can be asked without a model, and a marker "Fixed list" when an older peer reports that catalog form; the stored model is always offered, and a list that failed to load leaves it in place with one line.
- **If <Runs on> can't answer** lists the agents the operator added, in the order added, each with a number, its mark and state, its own model select and a remove ×; there is no switch and no reordering, so an empty list is no fallback and a different order is a remove and an add.
  `+ Add agent` opens a menu of selectable agents not yet used; installed agents Hide AI cannot use yet are listed under "Hide AI can't use these yet", dimmed, each with its reason (not signed in, can't guarantee it only reads, not supported yet), and an agent that is not installed is not listed.
  Its note explains that exceeding the configured usage threshold, signing out, exhausting provider usage or provider unavailability tries the next listed agent.
  Only an empty list adds that Hide AI waits for Runs on to become available again.
  With nothing to offer the button is disabled.
  Choosing a listed agent as Runs on takes it off the list.
- **Usage limit** (PRD hide-ai-usage-limit D-01 to D-09) is one select of 80%, 90%, 95% or No limit under the fallback list, 90% by default and for an install that never chose; a stored percent that is none of them is still shown.
  One value covers every agent: the row says "Up to 90% of each agent's week" and a note says that past it Hide AI moves to the next agent in the list while work the operator starts (worktree names, issue review) still runs.
  The select shows the core's value, so a choice stays as it was until the core accepts it.
  The Runs on and fallback rows add the week to the state when the weekly usage read gave a figure ("Signed in · 86% this week"), and an agent at or past the limit reads "Past your limit · 93% this week · resets Mon 9:00"; an agent with no figure shows none and is never held back, and the Add agent menu shows the state alone.
- While another agent answers, one line under Runs on reads "Using Codex · Claude Code is out of usage until 3:10 PM" (or "is past your 90% limit until Mon 9:00") and disappears when Runs on answers again.
  With the list empty and Runs on unable to answer, the Agent row says why and that Hide AI is paused; with every listed agent past the limit it reads "Hide AI is paused · every agent is past your 90% limit this week", agent labels show each session's own text and Memory analysis waits, and both resume on their own when the limit is raised or the week turns; nothing else on screen changes, and a failure the operator cannot act on stays in the diagnostic log.
- With no agent signed in, one line says "Sign in to <first agent that is on> to use Hide AI" (or to turn an agent on in Agents when none is); it appears only after every agent has been probed, and Hide's other features keep working without AI.
- Hide AI runs the core machine's agents unless the operator chose a device's, so in a window on another machine every sign-in state and line above names the core's machine ("Not signed in on Mac mini", "Claude Code is signed out on Mac mini", "Sign in to Codex on Mac mini to use Hide AI"); the agents have no one sign-in command, so the fix is the machine in words, as the move's Hide AI check says it; on the core's own machine nothing names a machine, and a device's agent is already named with its device (PRD core-host-node-move B15, B1).
- **Features** holds Agent summaries (the `agent_summary` switch of `ai_settings`: off, no agent label is asked for, the one being made is dropped, and every surface names each agent by its session's own title or its provider with no AI line or written question; on again, the kept labels return at once and each pane's current turn is asked for) and Worktree names (the same `issue_settings` value the Issues tab held).

## Mobile companion

### Settings > Mobile

Mobile is a page of Settings' machine group, after Hide AI; Tailscale runs on the core's machine, so a failing step and the serve entry line name that machine (`Tailscale isn't installed on Mac mini` on a window of another machine, `This Mac` on its own).
Its switch, 폰에서 Hide 열기, is off at first, and its one-line description says it works over Tailscale and only inside the tailnet (PRD settings-cleanup B59).
While the switch is off hide runs no Tailscale command.
Turned on, the three Mac checks (Tailscale installed, logged in, MagicDNS and HTTPS on for the tailnet) fold into one line, `Tailscale is ready`, once they all pass (B57); a failing step shows only itself with a warning and its action (a download link, "Tailscale 앱에서 로그인", or the admin console's DNS page with a link) and nothing for the steps that passed or still wait.
The phone side, Tailscale on the phone with the same account, is guidance hide cannot check and is part of the `폰 연결하기` row's description.
The page rechecks every three seconds while it is open, so logging in or turning HTTPS on continues without reopening it.
Once every Mac step passes and hide has confirmed its serve entry, the `폰 연결하기` row offers `QR 보기`; opening the page makes no code and voids none (B58).
Pressing it shows the QR, 폰 카메라로 스캔하세요, the ts.net address and the code's m:ss countdown, and the row's button becomes `QR 숨기기`, which takes the code out again; a pairing phone spends the code and the QR goes, so the next phone needs `QR 보기` again.
An HTTPS entry hide did not make shows its target in one line and no pairing row; a failed serve command shows the failed step and its message in one line, and so does a Funnel that would publish the address, which also disconnects every phone when it is turned on later.
A removal that fails when the switch goes off keeps that line under the switch until hide finishes it.
The phones group is titled 연결된 폰 · n / 4; each row shows the phone's name, when it was last seen, whether notifications are on or off, the days left before the seven-day revoke once it has been away a day, and 연결 해제, which closes that phone at once.
The push to the phone is on Settings > Notifications, not on this page (PRD settings-left-nav B7): one dropdown of 끔 (the default), 앱이 닫혀 있을 때만 and 항상, with the description of the chosen mode only under it, and the choice survives a restart (B59).

### The phone app

A reply or quick key sent while its pane is being forked waits for that fork under the same 64 KiB and 30-second holder bounds as desktop input.
The phone reports success only after Herdr confirms the write; a holder refusal keeps the reply text and shows the existing send-failure line so the operator can retry once the pane is ready.
A pane outside the desktop's attached tabs remains writable without attaching it or moving the desktop's keyboard.

The QR opens a page with the hide icon, "<Mac>에 연결", a line on what the phone can do, 연결, and a note that the code expires in five minutes.
연결 opens the list and a one-time hint to keep hide on the Home Screen; an expired or spent code says so, a fifth phone is told the limit and to revoke one on the Mac, and a page opened with no code or credential says to scan the QR in Settings > Mobile.
The list's header shows hide, the Mac's name, how many other phones are connected, and a connection dot.
Agents sit in 내 차례, 읽을 것, 작업 중 and 확인함, the sidebar's own section names, with their counts, each row with its status mark (the sidebar's five marks and the word that leads its line), task name, project and branch, the SSH device's chip, the elapsed time and the request or news line, and the list updates live.
Local sleeping conversations appear as separate rows in the core-selected group, with Wake or Check status and no conversation detail or reply bar.
Those controls are disabled while disconnected or while one sleep request is waiting; a refused request keeps the row and asks the operator to check connection and session status before trying again.
With no agents the list is one line, 실행 중인 에이전트가 없어요.
The header's `+`, 에이전트 시작, opens the start sheet: a text box for what to do, the target (This Mac's Home first, then This Mac's checkouts, then each device's Home and checkouts, a device that is not connected disabled with 연결 안 됨), the shared kind and model menus, both preselected from the desktop's remembered choice, and 시작.
The target names the core's machine as its system does in the phone's language (`이 Mac · 홈`, `이 PC · 홈`), and reads `홈` alone until the targets arrive.
시작 starts the agent there; once it appears in the list the phone opens its detail.
The desktop does not follow a phone start: the visible tab and the keyboard stay where the operator has them, and the new agent only appears in the sidebar and the lists like any other.
Pressing 시작 again for the same text and choice after a lost answer sends the same request id, and hided starts one agent for it.
While hided is out of reach the sheet shows the unreachable line and keeps what was written; a refused start shows its reason inside the sheet and keeps the text, which is cleared only by a start that went.
When hided is out of reach the last list stays dimmed under "연결 안 됨 · Mac의 Hide가 꺼져 있거나 폰의 Tailscale이 꺼져 있어요. 다시 시도 중", the app retries on its own, and it shows the same line when opened without a network.
A Mac that went to sleep shows it too: the phone asks hided when it has heard nothing for 20 seconds and shows the line 10 seconds after an unanswered ask, and on returning to the phone it asks at once, so the green dot never outlasts a hided that has stopped answering by more than 30 seconds.
A row opens its detail: ← 목록, the elapsed time and the row's head, with 대화 | 터미널 between ← 목록 and the elapsed time when the agent has a conversation to show, a Claude Code, Codex, Grok, Pi, omp, Cursor or OpenCode agent on this Mac whose session Herdr reports.
대화 is the agent's own conversation, its newest 30 messages with the newest at the bottom: the operator's messages as ❯ blocks on a grey ground, the agent's Markdown drawn at full width (headings, emphasis, lists, tables, and code wrapped without highlighting, a link or an image as its text), an interruption as 중단됨, and the time after each turn; pulling to the top loads 30 older messages at a time up to the first, and at 300 it says 최근 300개까지 볼 수 있어요.
What the agent writes next arrives on its own, and tool output and injected context are never shown.
Pi, omp and Grok use their recorded native message units and times (a Grok prompt's blocks and a turn's answer runs joined into one message each), with the same 30-message paging and text bounds; every page, idle poll and copied pager proves the reported native owner and exact checkout under its default root again.
Cursor uses its native graph units under the same page and text bounds; a recorded time is shown at minute precision and an unrecorded time is omitted.
OpenCode uses its own messages from its database, paged by message with the same 30-message and text bounds; every page, idle poll and copied pager proves the root session and exact checkout again, and a session rewound under the phone is read anew.
An unavailable or changed native source falls back through the existing feature-local refusal path; no transcript is borrowed from another session and no new banner is added.
터미널, and the whole detail of any other agent, is the pane's recent rows read-only in the terminal's colours with the newest at the bottom, each row wrapped at the phone's width so the detail never scrolls sideways; pulling to the top loads older rows until the pane has no more.
Every detail has the quick keys (Enter, Escape, 위 화살표, 아래 화살표, Ctrl-C by accessible name) and a one-line reply with 보내기; a reply is sent with Enter after it and clears on success, a failure keeps the text with the reason under it, and a reply over 2,000 characters is named before it is sent.
When hide cannot tell whether a reply reached the pane, the line says so and asks the operator to check the terminal before sending it again.
An agent whose process Hide ended and whose pane holds a bare shell (a keep-pane sleep, described with the pane body above) shows its state in the terminal's place, in the desktop pane's words: Sleeping with its last progress line and Wake agent; Waking… with how old the resumed conversation is; or `Couldn’t resume this conversation` with the core's reason, Retry and Start new session.
The 대화 | 터미널 switch is not offered there, the reply bar and the quick keys are disabled, and the core refuses an input to such a pane at the write (`asleep`, drawn as one line under the bar), from the moment its end begins until the agent is back, so nothing a phone sends reaches the shell under the agent; a repeated request keeps the answer its first send recorded.
Wake agent, Retry and Start new session send the desktop's own wake for that pane, and the agent resumes in the same pane.
Opening the detail does not wake it, and neither does a tap on its row: the desktop wakes on a committed move of its own tab, and a look from the phone is not one, so the row reads `Sleeping`, without the desktop's "wakes when opened".
A closed pane shows "이 pane은 더 이상 열려 있지 않아요" and disables the reply bar; a disconnected SSH device shows that its device is not connected.
With push on, the list offers 알림 켜기; a Safari tab without push is told to open hide from the Home Screen first, and a refused permission shows "알림이 꺼져 있어요 · 설정 > 알림에서 hide를 켜세요".
A notification's title is the root's row title and, with a sentence, its body is `<translated row status> · <exact sentence> · <project>`; without a sentence it is the row's status word with the project (`승인 · <project>` in Korean), and a payload with no status code says the section (내 차례 or 읽을 것) in the phone's language.
The sentence is the core's chosen row detail, untranslated, and may appear in lock-screen previews according to the phone's setting (operator-approved update 2026-10-13); tapping the notice opens that agent's detail.
See [With and without Hide AI](status-model.md#with-and-without-hide-ai) for native question and plain-text question behavior, and cached-worker compatibility.
A revoked phone shows "이 폰의 연결이 해제됐어요. Mac에서 QR을 다시 여세요." and drops its own push subscription.
The app follows the phone's light or dark setting, draws text a quarter larger than the desktop, and keeps every control at least 44 points tall.

## Keyboard shortcuts per system

Web owner: `web/src/shortcuts.ts` (the registry, the rule and its exceptions), `web/src/keys.ts` (terminal keys), `web/src/shortcutLabels.ts` (every chord a screen prints), `web/src/keyboard.ts` (the window listener); desktop owner: `desktop/src/main/menu.ts` (the menu's accelerators); test owners: `web/src/shortcuts.test.ts`, `web/src/shortcutsDoc.test.ts` (fails when the Every command table below and the registry disagree), `web/src/keys.test.ts`, `desktop/src/main/menu.test.ts`, `web/e2e/korean-input.spec.ts` (Hangul composed through the browser's input-method path: the ⇧↩ order and a composing Escape), and `web/e2e/chords.ts`, through which every web e2e presses its chords as the runner's system does.

The rest of this document writes chords as macOS has them; this section is how Windows and Linux press each one.
Which system applies is the one the operator types on: the desktop app's, or the browser's for a browser tab, whatever system the daemon runs on.

### The rule

On Windows and Linux every ⌘ is Ctrl+Shift, as in Windows Terminal, GNOME Terminal, WezTerm and kitty, so a plain Ctrl key reaches the shell and the agent CLIs (Ctrl+C, Ctrl+R, Ctrl+W, Ctrl+K, Ctrl+D and the rest); the only plain Ctrl keys the app takes are text size and, in a browser tab, the ones Chrome keeps for itself.
A ⇧ or ⌥ that a macOS chord adds to ⌘ makes it Alt+Shift there, the layer Windows Terminal puts its panes on (Alt+Shift+D splits a pane there too): ⌘T is Ctrl+Shift+T and ⇧⌘T is Alt+Shift+T, so no chord needs more than three keys.
The chords the rule puts on that layer take no key a shell or an agent CLI documents: readline reads Alt+Shift+B as Alt+B (`do-lowercase-version`), so Alt+B, Alt+D and Alt+T still reach the shell and Claude Code.
None of them is a key Windows, GNOME or Chrome lists as its own, except Chrome's Alt+Shift+T (below).
Windows can be set to switch the input language on Alt+Shift, and a Linux layout can be too (`grp:alt_shift_toggle`); a chord still runs there, because chords match the physical key, but whether the layout switches as well is unconfirmed on a real Windows machine.
A chord without ⌘ is the same keys on every system: ⌃Tab is Ctrl+Tab and ⌥1 is Alt+1, so those Alt chords take keys a shell reads as Meta, as they do on a Mac whose terminal sends Option as Meta.
Chords are written in words joined with `+`, in Windows' order Ctrl, Alt, Shift, as Windows Terminal writes them, so every chord the rule makes begins with `Ctrl+Shift+` or `Alt+Shift+`.
AltGr types a character on Windows and Linux layouts (Windows reports it as Ctrl+Alt), and a key pressed with it is never a chord.
A stored chord set keeps macOS chords, so a set a Mac saved means the same keys through the rule on Windows and Linux, except that ⇧⌘X, ⌥⌘X and ⌥⇧X are all Alt+Shift+X there, so a set binding two of them, or binding one onto a default that is another, is refused whole and the defaults run, and a chord recorded there as Alt+Shift+X is stored as ⇧⌘X; on those systems a desktop pane chord needs Ctrl+Shift or Alt+Shift, a browser chord Ctrl or Alt, and the Windows or Super key is refused because the system keeps it.

### Exceptions

Each exception has a dominant convention on Windows and Linux, or a chord the system keeps, behind it.

- Text size is Ctrl and =, - or 0, as in Windows Terminal, GNOME Terminal, WezTerm, VS Code and every browser.
- The tab beside is Ctrl+PageDown (previous Ctrl+PageUp), as in Chrome, Firefox, VS Code and GNOME Terminal; the rule's Ctrl+Shift+] would be a chord no one there knows.
- The recent project cycle is Ctrl+Shift+` (previous Ctrl+Alt+Shift+`): Alt+Tab is the system's window switcher on Windows and Linux and never reaches an app, so the cycle moves to ` under the rule, the key macOS keeps for its own window cycle.
  Going back adds Alt to the Ctrl+Shift the cycle already holds, the one four-key desktop chord: a held cycle commits when its Ctrl is released, so the rule's Alt+Shift+` could not step back inside it, and GNOME keeps Alt+Shift+` for switching between an app's windows.
- In a browser tab, Reopen closed tab is Ctrl+Alt+Shift+T: Chrome keeps Alt+Shift+T to focus its toolbar, which keyboard users rely on, and the macOS browser chord ⌥⇧T is those same keys there.
- In a browser tab, Switch sidebar list is Ctrl+Alt+Shift+B for the same reason: Chrome keeps Alt+Shift+B to focus its bookmarks bar.
- Not yet handled: a browser tab's area cycle is Alt+` and Alt+Shift+` on Windows and Linux (Chrome keeps Ctrl+Tab, as it keeps ⌃Tab on macOS), and stock GNOME and KDE take both to switch between an app's windows, so on those desktops the cycle reaches the page only once the desktop's keys are changed or the cycle is bound to another chord in Settings, Shortcuts.
- Move to Trash in the Explorer is Delete, as in Windows Explorer and the Linux file managers.
- Copy Relative Path is Alt+Shift+H: the rule would give it Copy Path's Alt+Shift+C, VS Code's own chord for it is a two-step sequence (Ctrl+K, then Ctrl+Shift+C) that the registry cannot hold, and Alt+Shift+R opens the PR list.
  A browser tab has neither path chord, because Chrome keeps ⌥⌘C (Inspect); the menu items work there.
- Cut, Copy and Paste in the Explorer are the system's own clipboard chords: ⌘X, ⌘C and ⌘V on a Mac, Ctrl+X, Ctrl+C and Ctrl+V on Windows and Linux.
  The tree answers them only while it holds the keyboard, so a terminal keeps ⌘C for its selection and Ctrl+C for the interrupt, and a text field keeps its own.
- Inside a text field, a palette, a board or a document, no terminal holds the keyboard, so the system's own command key applies: ⌘↵ is Ctrl+Enter (send a form, open a ⌘P result to the side, open a pull request on GitHub), ⌘-click is Ctrl-click (a link, a pull request, an issue), and Ctrl+S saves the document in front as well as Ctrl+Shift+S.
  A file or diff View's comment chord ⇧⌘. is Ctrl+Shift+. there for the same reason ([View openers and comments](#view-openers-and-comments)).
- The digits follow the rule: a tab is ⌘1-9, so it is Ctrl+Shift+1-9 there, a device is ⌥⌘1-9, so it is Alt+Shift+1-9, and an agent row stays ⌥1-9, Alt+1-9.
  No digit is on plain Ctrl: ⌃1-9 would take the control bytes ⌃2 to ⌃8 send a terminal (NUL, Escape, ^\ to ^_).

### In a terminal

| Action | macOS | Windows and Linux |
| --- | --- | --- |
| Copy the selection | `⌘C` | `Ctrl+Shift+C` (the terminal's even with nothing selected, so Chrome's element inspector never opens), or `Ctrl+C` while text is selected |
| Interrupt the program | `⌃C` | `Ctrl+C` with nothing selected |
| Paste | `⌘V` | `Ctrl+Shift+V` |
| Delete to the line start, go to the line start or end | `⌘⌫`, `⌘←`, `⌘→` (sent as ^U, ^A, ^E) | `Ctrl+U`, `Home` and `End`, which the terminal sends as typed |
| A newline the agent keeps in its prompt | `⇧↩` | `Shift+Enter` |

Ctrl+C copies only while text is selected and clears the selection as it copies, as Windows Terminal does, so the next Ctrl+C interrupts; with nothing selected it reaches the program.
Every other plain Ctrl key reaches the program, apart from text size (Ctrl+=, Ctrl+- and Ctrl+0) and the tab beside (Ctrl+PageDown and Ctrl+PageUp).
Settings, Shortcuts refuses a binding on the terminal's copy or paste chord.

While an input method composes (Hangul, kana, pinyin), the keys in this table are the input method's and none is sent; a ⇧↩ that ends a syllable reaches the program after that syllable, never before it (`web/src/terminals.ts`).
The syllable being composed is drawn in the terminal's own colours, underlined.
Anywhere in the window, an Escape pressed while a field composes is the input method's too: it never closes the dialog, sheet or palette the field is in (`web/src/keyboard.ts`).

### Every command

The hold hint follows the same table: holding Ctrl+Shift alone in the desktop app on Windows and Linux floats the tab numbers, holding Alt+Shift alone the device numbers, and holding Alt alone the Agents-list numbers.
A command marked none has no chord until the operator binds one in Settings, Shortcuts, where it can be bound; the browser tab's moved chords are the ones Chrome keeps for itself on that system.

Settings, Shortcuts (PRD settings-cleanup B60-B63, B66) lists the editable pane and navigation commands, each as its name and a chip holding its chord.
Pressing the chip, or Enter on it, waits for the next chord, which applies at once; there is no second Apply step, Escape cancels the recording without closing the sheet, and a refused chord leaves the old one with the reason beside the row.
The chip's accessible name carries the command and its current chord.
The restore-default and clear controls of a row show while the pointer is on the row or the keyboard is inside it (and always where there is no hover), and keep their place so the chip does not move.
The eight area commands have no chord until the operator sets one, so they fold into one line that says how many are still unset and opens to a row each.
The numbered chords are fixed and read as one row per family under Fixed, whose subtitle names the modifier to hold to see the numbers ("Hold ⌘ to see the numbers", the system's own modifier names elsewhere); a browser tab has no numbered chords, so those rows say so and carry no subtitle.
The sheet carries no row or explanation for a command the removed native app alone ran (`toggle_conversation` stays in a stored set and is never run here).

| Command | Desktop app, macOS | Desktop app, Windows and Linux | Browser tab, macOS | Browser tab, Windows and Linux |
| --- | --- | --- | --- | --- |
| New tab | `⌘T` | `Ctrl+Shift+T` | `⌥T` | `Alt+T` |
| Close focused view or pane | `⌘W` | `Ctrl+Shift+W` | `⌥W` | `Alt+W` |
| Reopen closed tab (exception) | `⇧⌘T` | `Alt+Shift+T` | `⌥⇧T` | `Ctrl+Alt+Shift+T` |
| Add project | `⇧⌘N` | `Alt+Shift+N` | none | none |
| Start agent | `⌘N` | `Ctrl+Shift+N` | none | none |
| Next recent tab | `⌃⇥` | `Ctrl+Tab` | `` ⌥` `` | `` Alt+` `` |
| Previous recent tab | `⌃⇧⇥` | `Ctrl+Shift+Tab` | `` ⌥⇧` `` | `` Alt+Shift+` `` |
| Next tab (exception) | `⇧⌘]` | `Ctrl+PageDown` | none | none |
| Previous tab (exception) | `⇧⌘[` | `Ctrl+PageUp` | none | none |
| Next panel (all projects) | none | none | none | none |
| Previous panel (all projects) | none | none | none | none |
| Next recent project (exception) | `⌥⇥` | `` Ctrl+Shift+` `` | `⌥⇥` | `` Ctrl+Shift+` `` |
| Previous recent project (exception) | `⌥⇧⇥` | `` Ctrl+Alt+Shift+` `` | `⌥⇧⇥` | `` Ctrl+Alt+Shift+` `` |
| Select agent 1-9 | `⌥1 … ⌥9` | `Alt+1 … Alt+9` | none | none |
| Select tab 1-9 | `⌘1 … ⌘9` | `Ctrl+Shift+1 … Ctrl+Shift+9` | none | none |
| Select device 1-9 | `⌥⌘1 … ⌥⌘9` | `Alt+Shift+1 … Alt+Shift+9` | none | none |
| Search | `⌘K` | `Ctrl+Shift+K` | `⌘K` | `Ctrl+Shift+K` |
| Open file | `⌘P` | `Ctrl+Shift+P` | `⌘P` | `Ctrl+Shift+P` |
| Overview | `⇧⌘O` | `Alt+Shift+O` | `⇧⌘O` | `Alt+Shift+O` |
| Add to backlog | `⇧⌘M` | `Alt+Shift+M` | `⌥⇧M` | `Alt+Shift+M` |
| Focus sidebar | `⇧⌘P` | `Alt+Shift+P` | `⇧⌘P` | `Alt+Shift+P` |
| Toggle left sidebar | `⌘B` | `Ctrl+Shift+B` | `⌘B` | `Ctrl+Shift+B` |
| Switch sidebar list (exception) | `⇧⌘B` | `Alt+Shift+B` | `⌥⇧B` | `Ctrl+Alt+Shift+B` |
| Toggle device rail | none | none | none | none |
| Toggle right lane | `⌘E` | `Ctrl+Shift+E` | `⌘E` | `Ctrl+Shift+E` |
| Toggle Project tool | none | none | none | none |
| Toggle Hide AI tool | none | none | none | none |
| Open Project Memory | `⇧⌘L` | `Alt+Shift+L` | `⇧⌘L` | `Alt+Shift+L` |
| Open PR list | `⇧⌘R` | `Alt+Shift+R` | `⌥⇧R` | `Alt+Shift+R` |
| Toggle documents | none | none | none | none |
| Toggle Changes tool | `⇧⌘G` | `Alt+Shift+G` | `⇧⌘G` | `Alt+Shift+G` |
| Toggle Explorer tool | `⇧⌘E` | `Alt+Shift+E` | `⇧⌘E` | `Alt+Shift+E` |
| Find in pane | `⌘F` | `Ctrl+Shift+F` | `⌘F` | `Ctrl+Shift+F` |
| Save file | `⌘S` | `Ctrl+Shift+S` | `⌘S` | `Ctrl+Shift+S` |
| Keep open | `⇧⌘K` | `Alt+Shift+K` | `⇧⌘K` | `Alt+Shift+K` |
| Split right | `⌘D` | `Ctrl+Shift+D` | `⌘D` | `Ctrl+Shift+D` |
| Split down | `⇧⌘D` | `Alt+Shift+D` | `⇧⌘D` | `Alt+Shift+D` |
| Zoom pane | `⇧⌘↩` | `Alt+Shift+Enter` | `⇧⌘↩` | `Alt+Shift+Enter` |
| Show plan | `⇧⌘U` | `Alt+Shift+U` | `⇧⌘U` | `Alt+Shift+U` |
| Close pane | `⇧⌘W` | `Alt+Shift+W` | `⌥⇧W` | `Alt+Shift+W` |
| Larger text (exception) | `⌘=` | `Ctrl+=` | `⌘=` | `Ctrl+=` |
| Smaller text (exception) | `⌘-` | `Ctrl+-` | `⌘-` | `Ctrl+-` |
| Reset text size (exception) | `⌘0` | `Ctrl+0` | `⌘0` | `Ctrl+0` |
| Focus next Agent area | none | none | none | none |
| Focus previous Agent area | none | none | none | none |
| Grow Agent area | none | none | none | none |
| Shrink Agent area | none | none | none | none |
| Focus next View area | none | none | none | none |
| Focus previous View area | none | none | none | none |
| Grow View area | none | none | none | none |
| Shrink View area | none | none | none | none |
| Move to Trash (exception) | `⌘⌫` | `Delete` | `⌘⌫` | `Delete` |
| Copy Path | `⌥⌘C` | `Alt+Shift+C` | none | none |
| Copy Relative Path (exception) | `⌥⇧⌘C` | `Alt+Shift+H` | none | none |
| Cut File or Folder (exception) | `⌘X` | `Ctrl+X` | `⌘X` | `Ctrl+X` |
| Copy File or Folder (exception) | `⌘C` | `Ctrl+C` | `⌘C` | `Ctrl+C` |
| Paste File or Folder (exception) | `⌘V` | `Ctrl+V` | `⌘V` | `Ctrl+V` |
| Settings | `⌘,` | `Ctrl+Shift+,` | `⌥,` | `Ctrl+Shift+,` |
| Keyboard shortcuts | `⌘/` | `Ctrl+Shift+/` | `⌘/` | `Ctrl+Shift+/` |

## Keycaps, tooltips, and icon buttons

Every icon-only control has a tooltip and an accessible name carrying the same words as the tooltip.
A chorded tooltip reads the label followed by the shortcut chord; a chordless control shows only the label.
The sidebar Settings tooltip and accessible name append the host's current shortcut in parentheses to the translated label.
There is no native platform tooltip layered underneath the shared one; the shared tooltip is the only tooltip in the main shell.
Tooltip hover has a short reveal delay, and an exact modifier hold reveals shortcut hints faster than a hover tooltip does.
In the desktop app, holding ⌘ alone (Ctrl+Shift on Windows and Linux) floats each tab's number at its top left in the Agent area in use, holding ⌥ alone floats each Agents-list row's number at its top right, and holding ⌥⌘ alone (Alt+Shift on Windows and Linux) floats each device tile's number at its top left, the corner a tile's marks leave free: a keycap in the popover colors with a border, a small shadow and one mono digit, positioned over the row or tile rather than in it, so a title, a row's time, its fold slot and a tile's marks never move.
The number is the screen order at that moment, first to ninth, top to bottom for the rows the Agents list draws (a folded parent's descendants are not rows) and for the rail's tiles, and an item past the ninth carries none.
A tab is the exception, as in a browser: ⌘1-8 select the first eight tabs of the Agent area in use, left to right, and ⌘9 its last, so with ten or more tabs the last one carries 9 and the ninth to the second-to-last carry none, and with fewer the last tab carries its own place and ⌘9 reaches it too.
Adding ⌥ to a revealed ⌘ hold turns the numbers to the devices at once, and releasing it turns them back.
A chord an earlier version let the operator bind that a numbered chord now holds (⌥⌘1-9 went to the devices) is dropped from the stored set with a diagnostic, the command keeping its default, and the rest of the set stays.
While Projects is on screen the same hold shows those Agents-list numbers, since ⌥n still selects by them, each once: on the agent's raised row, else on its row under the checkout that owns its pane.
The hint appears only after a short hold of the exact modifier; releasing it before then shows nothing, so a ⌘C never flashes numbers.
Releasing the modifier, adding another, pressing any key during the hold (including the numbered chord itself), losing the window, hiding the page, or opening a sheet, menu, dialog, palette, or cycle clears the numbers at once; the same modifiers still held after that show nothing until they are released and held again.
The keycaps and the hover tooltip never share space: a tooltip hangs beside its trigger and a keycap sits inside the trigger's own box.
In the desktop app a tooltip avoids the browser pages on screen: it opens on its usual side, else the opposite one, else across, whichever first fits the window clear of every page, because a page is drawn over the shell (Browser displays).
A browser host has no numbered chords, so holding ⌘ or ⌥ (Ctrl+Shift or Alt on Windows and Linux) there shows nothing.
Pane focus, active tab, tab order, zoom state, and disappearing anchors all update which controls can show a hint or tooltip; pointer exit, mouse down, scroll, key down, losing key window status, and anchor removal all dismiss an open tooltip.

Destructive buttons are named by their result (`Move to Trash`, `Close 3 panes and remove`, `Stop work and close`), never by a generic "Delete" or "OK" that hides the consequence; the non-destructive option is the default/cancel action, except in the sheet that closes an agent with the agents it spawned.
There the operator chose `모두 닫기` as the Enter default (PRD close-agent-subtree D-07, D-18), and a descendant whose status is unknown gives the default back to `취소`; the removal dialogs that ask the same question keep no default.
Escape closes the innermost open layer and returns focus to whatever held it before that layer opened, including a terminal that was focused when a sheet, menu, or overlay opened over it.
A context menu returns focus to its real target; a delayed focus return from any layer leaves a newer control or layer's focus in place.
A tooltip or hover card is not a layer: an Escape pressed while one shows still reaches the terminal or the screen it was meant for, and closes the tooltip on its way, whether the tooltip's own dismiss or the shell answers the press.
A disabled control cannot activate, and destructive meaning always comes from the control's role rather than from its text color alone.
Hover and focus are local presentation state: they never publish core snapshots, dispatch core events, or trigger Git/disk work by themselves.
Pending operations show explicit progress and remain disabled for the duration; a shared control style never invents its own pending or error state independent of the actual operation.

## Sheets, overlays, and abnormal states

Search, New Agent, Settings, and file search each host the same tooltip overlay and the same sheet container language: headline titles, supporting text, and consistent spacing.
The Settings sheet grows with the presenting window up to a maximum, so an ordinary window reads a tab without scrolling; the Settings scene (its own window) keeps its smallest size.
A selected provider card uses a stronger fill and border while keeping its status readable; disabled Start/Add controls keep their existing enablement conditions.

Loading, local-only, no-workspace, disconnected, and unreadable states are each distinct and each drawn in its smallest form: a badge or a dimmed row before a banner, following design principle 9.
A value Hide cannot read is shown as unreadable (a question mark or equivalent), never as a false zero or a silently empty state.
An alert, banner, or sheet for a failure is a deliberate product decision, never a default: a failure the operator cannot act on goes to the diagnostic log instead (design principle 13).
Retryable failures offer Retry attached to the exact target that failed; a definitive refusal and an unconfirmed/timed-out result are told apart in their copy, because only the definitive case can safely remove state (such as a reopen entry or a pending pane).

### Action menu presentation

The terminal, pane header, Agent and View tabs, Projects session rows and Tasks rows show a shared leading icon column and one muted heading naming the target.
Opening a menu still acts on that target without selecting another tab or marking a session read.
The heading follows the target's current name; the terminal close action reads `Close terminal` without repeating the name.
Copy, Paste, Select all, Find, Split right, Split down and Zoom remain direct terminal actions, with the existing platform-specific chords.
Related sessions, fork destinations and copied session details are submenus; Agent and View tab directions are grouped under Split and Move, with Rename beside New tab on an Agent tab.
Only directions already offered by the layout are shown under Move.
The command identities and their eligibility stay the same; a submenu whose choices share the same refusal shows that reason once on its disabled trigger, and mixed eligibility keeps the reason beside each disabled choice.
Escape closes the innermost submenu first, returns focus to its trigger, then closes the root menu and restores the original target's focus.
Close and archive stay neutral, while removing an archived session keeps the destructive color and existing confirmation.
The Tasks action formerly labelled `pane으로` reads `터미널로 이동`; the four supported interface languages use the same structure.
The approved B layout is implemented by the shared action-menu components; original visual boards are maintained separately.
