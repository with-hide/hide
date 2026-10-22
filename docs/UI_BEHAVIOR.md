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
The fetch is `git fetch --prune --no-tags --no-recurse-submodules origin` without `FETCH_HEAD` and with automatic gc and maintenance off, so it moves only remote-tracking refs and never a branch, the index, a file or a submodule, and it never opens a terminal prompt, askpass, keychain or Credential Manager window; Git before 2.29 cannot leave `FETCH_HEAD` alone, so there the fetch writes it.
A ref lock left by the operator's own Git skips that turn; a lock still held on the third fetch in a row, or two refs that cannot both exist, is a failure like any other.
The detail opens even at 0/0 and names the tracking ref and when the remote was last checked (`N분 전 확인`), or `아직 확인 안 함` before Hide's first successful fetch.
A failed fetch keeps the last counts dimmed and says since when they are old; a credential or network failure waits for the window to come forward, any other failure retries after 1, 2, 5 and then 15 minutes, and the failure goes to the diagnostic log once.
With the background fetch turned off in Project settings the project is not fetched and the detail shows the last `FETCH_HEAD` time, the operator's own fetch.
`herdr-core/src/runtime/fetch.rs` owns the schedule, `herdr-core/src/fetch.rs` runs it, and `hide-host/src/worktrees.rs` (`fetch_remote`) is the node's fetch.
Only the ↑ arrow uses the existing success green and only the ↓ arrow uses warning orange; both counts and the file count remain neutral, including when the branch is ahead and behind at once.
At the right, the current HEAD's subject and relative age open a compact history popover directly below that trigger, with up to five newest commits from the captured HEAD; no commit is invented for an unborn branch.
Above them, dimmed and marked `들어올 커밋`, sit up to five of the tracking branch's commits not yet in HEAD, with `+N개 더` when more are behind; while the fetch fails they are the ones last received and the list says since when.
A local commit no remote has is marked `푸시 안 됨` when `origin` is on GitHub.
When Git cannot say in time which commits a remote holds, or the upstream was pruned during the read, the header keeps its other facts, lists no incoming commit, and no commit opens a page; the reason goes to the diagnostic log once.
A commit GitHub can show, one some remote already has or an incoming one, opens its GitHub commit page in the Browser View from its subject; a commit only this checkout has does not.
Each compact item has its subject on the first line and short SHA and relative time on the second, with the exact local time available on the timestamp.
Its associated CI result sits at the right of that second line; commits with no CI omit the result.
The adjacent CI mark describes only that exact SHA, using GitHub's aggregate statusCheckRollup even on main without a PR, and distinguishes passing, pending, failed, none, unknown and unavailable.
The header's Git read, each CI result and the remote check share one observed shape (PRD git-freshness D-09, `herdr-core/src/observed.rs`): the last value read stays, with when it was read and whether it is fresh, stale or behind a failed read.
A CI read that fails, or ages past 90 seconds, keeps each commit's last result dimmed, not openable, with `GitHub 읽기 실패 · N분 전 값` or `N분 전 값` in its hint (B22); a commit GitHub never answered for reads unavailable.
A Git read that fails keeps the ↑/↓ counts dimmed with `Git 읽기 실패 · N분 전 값` instead of hiding them.
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
`Open server` (a globe): one known listener opens directly as a page in the files tool, and multiple listeners open a compact keyboard picker.
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
The sidebar, at its own width, and the center screen show one quiet line in their place, `이 영역을 그리지 못했습니다. 다음 변경 때 다시 그립니다.`; the top row keeps an empty strip of its height the window is dragged from, and a notice, an overlay or the bridge that fails is left out.
A part is drawn again on the next change to the core's snapshot or to the page's own state, a new diagnostic not counted, and nothing listens for that change while every part draws.
The failure is a diagnostic naming the part and the error, once per error until the part draws again; there is no banner.
Only when the window's frame itself cannot draw does the line take the whole window, under the drag strip.

### The right lane

The Workspace body, under the top row, is Agent Views (the Agent area), one right lane, and the tool rail, 48px wide, at the window's right edge.
The rail lists Memory, PRs, Files and Changes from the top, the Project's tools before the checkout's, each an icon alone (a brain, a pull request, files, a file diff) whose name and scope are its hint on hover and its accessible name, and the lane shows one of them at a time: Memory, PRs (see The PRs tool), Files (the View areas and the file tree) and Changes (History).
There is no Work tool; its Sessions list is the sidebar's 태스크 list and its PRs lens is the PRs tool (PRD sidebar-task-lens D-16, D-17).
A rail icon is drawn pressed, with a primary bar at the rail's edge, while the lane shows its tool; pressing it opens the lane on that tool, and pressing the pressed one closes the lane.
Each icon's hint names the tool and its scope and ends with its chord (`Memory · herdr-ide  ⇧⌘L`, `PRs · all of hide  ⇧⌘R`, `Files · settings-ui  ⇧⌘E`, `Changes · settings-ui  ⇧⌘G`).
A short rule between PRs and Files marks where the Project's tools end and the checkout's begin (PRD work-tab-pr-lens D-37); it is derived from each tool's scope, so a tool of the other scope moves it.
Memory, PRs and Changes headers name the tool, then its scope behind one small icon, a folder for the Project (Memory, PRs) and a branch for the checkout (Changes), in the scope text's colour; the icon is decoration.
Files omits the title, branch icon and branch name from its header; its rail hint still names the tool and checkout scope.
While the lane does not show Files, its icon carries a badge counting the views open, also given as its accessible description; Memory's carries a dot while its Project has questions waiting, and PRs' a warning-coloured number counting the PRs tool's Needs You rows (folded rows not counted, a session's Needs You never).
While the 보기 slot holds something, a fifth button stands above Memory with the mark of what it holds, a pull request as its PR mark in its state's icon and colour or a record as a page, and one short rule under it sets it apart from the tools (see The 보기 slot).
The rail's `…` menu holds Toggle File Tree with its chord, Pin or Unpin, and Expand.
A Workspace seen for the first time shows Agent Views alone, its lane closed.
The lane's header has Pin, Expand and Close at its right.
Memory, PRs and Changes name their tool and scope, `Memory · <project>`, `PRs · all of <project>` and `Changes · <checkout>`; a new checkout in front updates the checkout scopes in Changes and the Files rail hint and leaves Memory and PRs as they are.

Unpinned, the lane opens over Agent Views at their right edge, set apart by a shadow and a left border in both themes; the terminals under it keep their grid and layout, are not dimmed, and keep their input and output.
An unpinned lane stays up until the operator takes it down: a click on a terminal pane, choosing an agent or a tab, another Workspace in front and a restart all leave it as it was.
Close, ⌘E or its rail icon closes it, and Escape inside the lane or on its rail puts it away and hands the keyboard to the focused pane; Escape first closes a menu, popover or search open in the lane, and Escape in a terminal is the program's.
A lane put away keeps its documents, tabs, scroll, previews and searches, and its rail icon, ⌘E, an open or a reveal brings it back as it was.
Pinned (a filled pin; an empty one says `Pin · keep it beside the agents`), the lane docks beside Agent Views, which narrow by its width once; unpinned (`Unpin · back over the agents`), Agent Views take their width back and the lane is over them again.
The pin is the lane's, whatever tool it shows, and a pinned lane closed and opened again comes back docked.
A pinned lane docks in any window where its minimums (360px of documents, 180px of tree) fit beside 480px of Agent Views, giving up its own width first and taking its stored width back as the window widens; a window too narrow for even that opens the lane over them as if unpinned, with the pin still filled and its tooltip saying `Pin · too narrow, drawn over`.
Expand gives the lane the whole body between the sidebar and the rail, with the file tree as a column beside the documents; Expand again or Escape returns to the pin, widths and terminal layout as they were, no terminal resizes meanwhile, and Agent Views under an expanded lane take no pointer or keyboard.
While the keyboard is in the lane, a document, a page or the tool, the Zoom pane chord (⇧⌘↩, Alt+Shift+Enter on Windows and Linux) is Expand and Expand again, so one key fills the window with whatever the operator works in; anywhere else it zooms the focused pane, and the Expand tooltip shows the chord.
Close (the header's ×, the pressed rail icon, or the tool's chord) closes the lane and leaves the file tree as it was; it closes no view, and the Files badge counts what stays open.
A close keeps the tool the lane showed, so ⌘E and Show right lane bring it back on that tool.

The files tool shows the View areas below, with their tabs, splits, previews and browser displays, each page inside its area's bounds.
The active document's header starts with the checkout's name, then the path in ` › ` steps, cutting a long path at its start so the file name stays.
The Files top header starts at its left edge with one folder-and-Explorer toggle, with localized text, pressed background and accessible pressed state.
It stays visible with no View and is disabled while the tree alone must remain.
Document and page toolbars retain their path or address and controls, without another tree toggle.
The tree button or ⌘⇧E (`Toggle File Tree`) turns the file tree, the Explorer, on and off, the button pressed while it is on; ⌘⇧E while the lane does not show Files opens it on Files with the tree.
On, the tree is a column left of the documents and the lane widens by its width, so the documents keep theirs: over Agent Views it covers more of them, and docked, Agent Views give up the tree's width as well.
The tree stays on when a file is picked, when the lane is put away or closed and comes back, and when another tool shows and Files comes back; only the tree button and ⌘⇧E turn it off.
With no document open, the files tool shows the tree across the lane, its head saying `No open document · tree only`; a file picked there, or anything opened meanwhile (⌘P, a terminal link, a page, `hide` from the front tab), opens beside the tree, which goes back to its own width and stays on, and ⌘⇧E there changes nothing.
Closing the last view leaves the lane on Files in that state when the operator turned the tree on; with the tree off, the lane closes with that view and the keyboard goes to the focused pane, so a tree nobody asked for never appears.
⌘E on a Workspace with no view opens Files in that state; nothing opens a blank page in its place and the lane is never empty, while a View area's New tab still opens a browser display (see Browser displays).

Opening a file, a diff or a page from the tree, Changes, ⌘P, a terminal link, Open in Browser or Open server opens the lane on Files, over the agents or docked as its pin says, with the opened tab in its active View area; another tool showing gives way to Files.
Select in File Tree, a reveal, or a terminal link that names a folder opens Files with the tree on and the row unfolded and selected.
⌘E (`Toggle right lane`) closes the lane while it shows and otherwise brings it back on the tool it last showed in that Workspace, whichever way the lane was closed (its × button, the pressed rail icon, ⌘E or a tool's chord), and on Files for a Workspace whose lane never showed a tool; the tool is kept with the Workspace, so it holds after an app restart, and the path menu's Show right lane brings back the same tool.
Files coming back with a view takes the keyboard into its active View area, the editor of the document there or the page, as an open does, so typing, ⌘S and ⌘W act on what came back rather than on the terminal it was pressed from; another tool, or Files with no view, leaves the keyboard where it was.
⇧⌘L (`Toggle Memory tool`), ⇧⌘R (`Toggle PRs tool`) and ⇧⌘G (`Toggle Changes tool`) press that tool's rail icon: the lane opens on it, or closes when it already shows it; ⇧⌘E opens Files with its tree as before, and `Toggle Files tool` has no chord until one is bound (PRD sidebar-task-lens D-19, D-29).
On Windows and Linux they are Alt+Shift+L, R and G; in a browser tab Chrome keeps ⇧⌘R for reload, so PRs is ⌥⇧R there, and ⌥⌘1-4 do nothing anywhere.
A chord the operator had bound to the retired `Toggle Work tool` is dropped without an error, and the rest of their set stays.
The desktop app's View menu names the lane commands and `Toggle File Tree`, and a chord bound to any of them in Settings, Shortcuts keeps doing the same.
Each sends the value the lane should end at, so the same command arriving twice lands where it did once.
A right-click or the menu key on the Workspace's path offers Show or Hide right lane with the same effect.
A file read opens the files tool when its document is ready, in the Agent tab that requested it.
Choosing another Agent tab while it reads leaves the checkout's Views and selection as they are, and closing a requesting tab drops only that tab's pending open.
When several tabs request the same file during its read, they share the read and document buffer while each keeps its own display request.
A buffer opened through another path before the read answers retains its draft and save state, and the pending GUI displays still open in their requesting tabs.
Closing that buffer's last current display does not cancel another tab's pending GUI open.
Choosing an agent or a tab from the sidebar, the palette, a tab cycle or the tab strip, or a terminal taking the keyboard, leaves the lane where it is, pinned or not; the lane keeps its tool, pin and tree, and the files tool keeps showing the checkout's Views (see File Views per checkout).

⌘⇧P shows and focuses the sidebar.
`Toggle device rail` has no default chord either and is bound the same way; it shows or hides the device rail.

The lane has a divider at its left edge, docked or over the agents, and the tree a divider between it and the documents.
A divider shows its grip on hover, keyboard focus and while dragging; a drag moves a guide with the pointer and lands once on release, and a focused divider moves one 32px step per arrow key, each a separator that reports its width.
Fast arrow presses retain every step before the preceding width has been confirmed, and reloading restores the final width.
The tree's divider trades width between the tree and the documents, so the lane and Agent Views keep theirs; the lane's divider changes the width of what the lane shows, against Agent Views when docked, and over them otherwise, where it may reach the body's left edge.
The lane keeps two widths: Files draws its documents at one, and Memory, PRs, Changes and the 보기 slot share the other, so documents widened to read a page leave Memory as it was, and Files comes back at its own width after another tool.
Each width is never narrower than 360px and starts at 520px, the tree never narrower than 180px and starts at 240px, and Agent Views keep 480px beside a docked lane; the widths are the Workspace's and the same over the agents and docked, so a width dragged over the agents is the width the pin docks at.
A web page opened into the lane widens the Files width, when it is narrower than 900px, to 900px, so a site lays out its desktop screen: an open from the Explorer, a terminal link, `hide browser open` from the operator's pane, History, Recent Panels or a page's own new tab that puts a page in front of its area (PRD browser-width-zoom B1-B5).
It widens over the agents and docked alike only as far as Agent Views keeps 480px beside the lane and its tree, so a narrow window gives the page less than 900px, and it never narrows a wider lane.
The widened width is stored as a divider landing is, so a narrower width the operator drags afterwards stays through a file tab and back; choosing a page tab already open, a restart, an agent's open behind the page in front and an expanded lane widen nothing, and a width the core refuses leaves the page at the width it opened at, with only the log saying so.
The pin turning on or off, a docked lane opening or closing, a docked divider landing, the tree turning on or off beside a docked lane, and a docked lane turning between Files and another tool whose width differs resize the agents' terminals once.
A lane over the agents opening, going away, coming back, expanding or closing, its divider landing, a drag in progress, and anything inside the lane, an open, a close, a tab change, a split or a drag of a view, resize nothing.
Changing Workspace or lane geometry during a divider drag cancels its guide without saving a width, and releasing the old pointer cannot change the new Workspace.
The lane's tool, pin, tree, its two widths and the tree's are stored per Workspace and survive a restart: a pinned lane comes back docked, an unpinned one over the agents, and neither expanded.
A Workspace stored with the earlier File Views and Tools columns comes back once on Files where File Views was on, else on the tool Tools showed, unpinned (see View states).

Agent Views is always live: clicking a pane or a tab focuses it and typing goes to it, and chords such as ⌘F, ⌘T and ⌥W act where the keyboard is, a View area, the lane's tool or a pane.
A lane going out of sight with the keyboard inside it, or a files tool another tool replaces while the keyboard is in its documents, hands the keyboard to the focused pane.
Tab never walks into a lane out of sight: a lane put away is inert and holds no Escape layer or focus return, and a native page in it is hidden before it goes inert.

An Agent tab shows the focused pane's status mark, then title, using the sidebar row's title and mark rules; the agent's logo stands between them only while the agent logo view mode shows it ([Agent logo view mode](#agent-logo-view-mode)); a tab with no agent keeps the neutral `>_` in both modes.
A custom Herdr label wins; an empty, numeric, or `Tab N` label instead follows the focused pane's chosen agent title (manual name, Hide AI goal, automatic name, then provider, as [Task identity](status-model.md#task-identity) defines), foreground process name, then `Tab N` using Herdr's stable number; a Herdr workspace label is never a tab or agent name.
A plain terminal tab shows the terminal icon and that name; terminal titles are never used to guess a process.
Local and remote process names are read only for focused panes of attached tabs, outside the runtime lock, on focus or agent-state changes and a 30-second recheck.
The foreground process is the process-group leader, or the last returned process when the leader is absent; its name is the basename of `argv0`, falling back to `name` only when `argv0` is empty, and to `Tab N` when both are empty.
The tooltip and accessible name carry the kind, full name and agent state; widths, scrolling, truncation and close behavior stay the same.
Every Agent tab reuses the same name and Rename field in its area menu described below.
Rename opens an inline field with the displayed name selected: Enter saves to that host's Herdr, an empty name restores automatic naming, and Escape or blur cancels editing.
While saving, the committed name stays unchanged; a refusal or timeout keeps the entered text with “이름을 저장하지 못했습니다 · 다시 시도” below it, Enter retries, and Escape returns to the prior name.
Failure details go to diagnostics, with no banner; a successful name survives reconnect while Herdr keeps the tab.
Copy name copies the displayed name.
A View tab carries the file-type mark, and a diff tab the comparison mark, so a kind is never told by color alone.
Its title is cut at the tail to fit, and its tooltip and accessible name carry the kind, the full path, and `Preview` or `Unavailable` while the view is one, so a long path stays readable.
A dirty view shows a warning-colored mark after its title.
Closing a view is always called Close view, distinct from moving a file to the Trash and from closing a pane or tab.
The close chord, ⌘W in the desktop app and ⌥W in a browser, closes the smallest unit that holds the keyboard: the focused View area's active display, or the focused terminal pane.
The page records its last focused region as View area, pane, the lane's tool, or none from focus events, including native browser-page focus; the chord does not walk the active DOM element or choose a region from panel size.
When the desktop app hands the keyboard back from a native page to deliver a menu command, the focus the shell's last element regains is not a move while that command runs, so ⌘W and ⌘T pressed in a page act on that page's View area; once the command has run, the keyboard's owner is the shell element holding it, so after any other command from a page the next chord acts where the operator now types.
While the lane draws the 보기 slot, the chord closes the slot as its `✕` does when the keyboard is in the lane or nowhere (the sidebar row or the rail button that opened it); a pane or a View that holds the keyboard keeps the chord.
Otherwise, with the lane's tool or nothing focused, nothing closes and a diagnostic states why.
Tabs close only from their own close control or menu; closing a tab's only pane still removes its tab through Herdr.
An unavailable or retired keyboard target never falls through to closing a different pane or a whole tab.
When a connected pane selects, splits, moves, or closes a View through `hide view`, the same View layout rules apply to that pane's Workspace even while another Workspace is in front.
These commands do not move the keyboard target; a close that would lose the last View of an unsaved document reports the refusal and keeps the draft.
`hide file open` and `hide diff open` put the calling pane's file or changed-file diff into its own Workspace and never select that Workspace or another tab.
Only an agent in the pane the operator is in, the selected pane of the front Workspace, changes what the operator sees: its open or `hide view select` brings the View to the front and opens the lane on Files as a click does, the tree staying on when it showed alone.
Another pane of that Workspace, of the same tab or another, adds its View to the checkout's list and leaves the selection, the active area, the keyboard, the lane and the tab on screen as they were, and the Files icon's badge counts the new view (see File Views per checkout).
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

Agent Views draws the selected group's area tree, separate from the files tool's View tree.
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
Agent Views and the files tool keep separate active areas, so the chord follows the recorded keyboard owner, never whichever active area changed last.
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

The files tool holds one or more View areas, each with its own tab bar above its own view, split left/right or up/down as often as the limits allow.
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
When the last view closes, the lane stays on Files with the tree across it if the tree was on, and closes otherwise (see The right lane).

### File Views per checkout

The checkout owns the Files View list, its split, its selected Views and its active area, whichever Agent tab is in front.
Every tab sees the same list.
The lane, its tool, pin, tree and widths, and document editing and saving remain shared by the Workspace.
Changing Agent tabs through the strip, sidebar, search, cycling or a confirmed local or device switch changes no View selection, split or list.
Closing an Agent tab keeps its Views and documents.
Putting the lane away or closing it preserves its documents, unsaved text and save state.
Closing the selected input View chooses its area's most recently visited surviving View, then the right neighbour and then the left when none was visited, and the keyboard follows the confirmed result.
Closing a background View, including the selected View of another area, leaves input where it was.
A View context menu borrows the current input without selecting its target, and its automatic focus return does not cancel a pending close follow.
An emptied area collapses and input follows its surviving neighbour.
The last View returns input to the existing tree if it is on, else closes the lane and returns to the existing Agent pane; with no pane there is no invented input target.
A last document reference waits for save protection; failure or conflict leaves it open, and a newer input destination wins over a delayed close completion.

From the pane the operator is in, a file, diff or browser open or a select brings the View to the front, its area becomes the active one and the lane is called to Files.
From another pane of the Workspace on screen, of the same tab or another, the View is added at the end of the area it lands in; the selection, the active area, the keyboard, the lane and the tab on screen stay as they were, and the Files icon's badge counts it.
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
Authoritative topology drops a closed tab's pending opens and keeps the Views and documents.
An unanswered connected-device session or unresolved catalog preserves saved state.
Local and connected-device callers follow the same rules; the phone keeps its existing flow.
No additional badge, notice or setting is introduced.
The checkout keeps at most 32 Views in its six View areas and depth three.
An open that would add the 33rd View first closes the View looked at longest ago in the checkout, the way closing its last View closes a file (Reopen Closed brings it back); a View in front of any area and a document with unsaved work never close.
Only when nothing can close is the open refused, with the View notice, and existing content stays as it was.
Web owner: none, the shell draws the existing snapshot projection.
Core owner: `herdr-core/src/runtime/{view_callers,view_areas,workspace_control,workspace_view}.rs` and `herdr-core/src/workspace_views.rs`.
Regression owners are `runtime/tests/view_bookmarks.rs`, `web/e2e/{view-bookmarks,right-lane}.spec.ts` and `desktop/e2e/browser.spec.ts`.

### Opening, preview, and Open to the side

A single click on an Explorer file or a History row opens it in the active area's preview view (italic title), and the next single click replaces that preview in place, so browsing leaves one tab per area rather than a trail.
Each area has at most one preview, and a click never touches another area or a pinned view.
A double-click on the row or the tab, Keep open, or the first edit pins the preview where it is, including a double-click that lands while the file's first read is still running; because the first edit pins, a document that is dirty, saving, or whose save failed is never a preview in any area that shows it.
Opening a file that is already shown moves to its view instead of adding a tab, choosing the one used last when several views show it.
The keyboard goes with every open from the Explorer (a click, a double-click or Enter), a History row, ⌘P, Open to the side or a terminal link into the view that shows it once the core places it, so ⌘W, ⌘S and typing act on that file rather than on the pane that printed the link; a page the operator opens (a terminal link's URL or HTML file, the Explorer's Open in Browser, Open server, or a row's, a card's or ⌘K's link) takes it the same way once the core names the display it opened, so ⌘W closes that page, while a View area's New tab gives it to its address field and a page's own new tab leaves it with that page; a folder a link reveals opens nothing to type in, so the keyboard stays in the pane; the tree's arrow keys only move its selection, so the keyboard stays in the tree until something opens.
Opening a file while the lane is closed, put away or on another tool opens it on Files first.
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
A Workspace stored with the File Views and Tools columns, or with the side panel or layout before them, restarts once on Files where File Views was on (a panel open or expanded, a layout that did not show the agents alone), else on the tool Tools showed, the Explorer becoming Files with its tree, unpinned, its views, tabs and areas as they were.
File Views' stored width becomes Files' width; a Workspace stored before the columns, the tree, and the width the other tools share start at the default widths.

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
In a plain browser tab the view keeps its address on the toolbar row, level with a document header beside it, and below it reads `Pages open in the hide desktop app.`; a web address offers Open in browser, and nothing else is drawn in its place.
[BROWSER_DISPLAYS.md](BROWSER_DISPLAYS.md) owns which addresses a page may hold, the `file:` boundary, and the page's lifetime.

### View openers and comments

Each area's selected View shows its actual opener's 14px provider mark at the right of the address or path toolbar, while tabs retain their type mark and title without repeated opener logos: a page, a file, a diff, an image or a PDF opened by `hide file open`, `hide diff open` or `hide browser open` from the agent's pane, or by the operator clicking a path or an address in that agent's terminal (PRD view-opener-feedback D-02, D-04, D-24).
A View the operator opened from the Explorer, ⌘K, the file tree or a new tab's address field, and one a caller Hide could not bind to a pane opened, wears no mark.
The mark follows the agent that last opened the View; a link inside the page and an agent's `hide browser` command keep it, and an address the operator types takes it off.
Its hint, above the toolbar mark, says the agent's name, `Opened this · <product>` and `Click to go to its pane`, and the mark's accessible name says the same; the tab's tooltip also identifies its opener, including on background tabs.
Opening, waiting and unavailable Views retain a path toolbar with the same opener, and a missing or unknown opener creates no invented identity.
Pressing it brings the agent's pane forward with the keyboard, leaving the View selected; an agent whose pane closed is found again by its session in another pane, and one found nowhere draws its mark dimmed with `(closed)` in the hint and goes nowhere.
A pane whose agent opened Views that are open shows `N views` after its PR mark, a list of each View's kind mark, name, address or path and unsent comment count, counted once however many tabs show it; a row brings that View forward.

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
A pinned lane docks while its minimums and Agent Views' 480px fit side by side with its divider, the lane narrowing first; in a body too narrow for even those it opens over Agent Views with its pin still filled, and widening the body docks it again.
The lane opens at any width: in a body narrower than the lane it takes the whole body, giving up document width down to its 360px minimum first and tree width after.
When the View areas cannot all have their minimum, only the active area shows, with an area switcher to the others.
Widening the window brings back what the Workspace stores, the lane's tool, pin, tree, widths and the area sizes, because none of these narrow arrangements is stored, and another Workspace's lane never changes while one is narrow.

### Library masters

PRD right-lane B25 draws the lane, rail, pin and tree parts of its approved v3 boards in `Component / Workspace columns`, and the closed, over-the-agents, docked, tree-on, tree-only, expanded, Work and narrow states in `Screen / Workspace`.
Delivery reviews actual captures in both themes through the public [design workflow](DESIGN_WORKFLOW.md).
The production gallery and View components provide the public visual reference; original boards are maintained separately.
Their sheets draw every state as refs: the View tab sheet draws preview, pinned, hover, active-in-the-active-area, active-in-another-area, dirty, unavailable, diff, a long title, and the floating drag copy; the placement sheet draws a reorder, a move into another area, a right and a down split, and an ineligible target; the tab menu sheet draws a preview's menu and a pinned view's menu with a disabled Split and its reason; the area states sheet draws each view's opening, waiting, and unavailable states.
The browser display's toolbar and its loading, load failed, and plain browser tab states are on `Component / Browser file diff toolbars`.

### Agent panes and the Agents explorer

A pane header is a quiet identity row: the ancestor path, the status mark, the agent's logo, title, the fork mark, the letters-waiting pill, the task chip, the PR mark, the limited-integration icon, and the retained overflow, zoom and close controls at the right end.
An agent pane's header also holds `개요에서 보기`, a graph icon before zoom whose tooltip and accessible name say the same, drawn only while the header is pointed at or holds the keyboard, and keeping its place while hidden so the title and chips never move; a shell pane has none (PRD pane-open-in-overview B1-B3).
It opens the Overview on Work with the pane's session chosen, as a click on it there would, and the pane menu offers the same item first among the agent's own actions, with no shortcut (B11).
Dragging the title or unused space in the first header row moves that pane after the pointer crosses the ordinary drag threshold; a short title click still opens its record.
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

The task chip (PRD task-split-checklist D-05, D-06) is a checklist icon and `<done>/<tasks>` in figures, drawn from the pane session's first task (`0/1`) and never for a session with no task, whatever children it has, a plain shell, or with Agent summaries off; it carries no child count, and the second row with the child badge does not move it.
The pointer on the chip or the keyboard's focus on it opens the checklist below the header, which never covers the header it belongs to and stays inside the window at narrow widths: `시킨 일 <done>/<total>`, each task's checkbox, name (Korean wrapping between words) and right-hand word with a split request's PR marks once on its first task; right under the tasks this pane session's own `지금` box, the record's state box part with its words, `지금 · 질문 <line>` edged and written in the warning colour behind `!` while it asks or is blocked and `지금 · 작업 중 <line>` quiet otherwise, drawn only while the agent has a current line; then `맡긴 일 N · 끝남 k` in one line (`맡긴 일 N` while a child has no judgment, nothing without children), never a child's words and never a bar; and `기록 열기` with the chord, which opens the session record on the same task list (PRD record-card-readability D-08, B15); the task marks are the record's checkboxes, and nothing turns in the list.
Leaving the chip and the list, or blurring the chip, closes it; a press on the chip or Show task list (⇧⌘U, Alt+Shift+U on Windows and Linux, remappable) pins the focused pane's list, and a second press, the chord again or Esc closes it; Esc on a pinned list stops there, so an agent CLI in the focused terminal is not interrupted, and opening the list never takes the keyboard from the terminal (`web/src/PaneTaskChip.tsx`).
The chip, the list and the record follow the ledger in the same snapshot burst, and a chip whose count did not change is not redrawn.
A plain shell keeps its terminal mark.
On an agent pane whose session Hide knows, the status mark, the logo and the title are one button: under the pointer or focus they light together and open nothing, since a card under the head kept rising in the way of the terminal (the turn card opens from the request on the second row, and the record shows the whole title), and a press shows that session's record in the lane's 보기 slot ([The 보기 slot](#the-보기-slot)), opening the lane when it is closed or put away, while the keyboard and the input cursor stay in the pane's terminal (PRD overview-device-tasks B22, sidebar-task-lens B15); the Overview's `lane에서 보기` lands the same way.
A press while the lane already draws that record turns the lane back to its tool, and the fork mark and the PR mark keep their own presses outside the button (B23).
Whether the lane draws that record is read from the slot's front item and the lane's tool, both of which the core keeps.
A plain shell, and an agent pane whose session is not known yet, keeps the plain head with nothing to press; the head becomes the button in place once the session is known, on a device's pane as on this Mac's (B24).
Over a narrow window the record shows in the lane floating over the panes (`web/src/PaneHeaderBand.tsx`, `PaneRecordButton`).
The provider logo is drawn on a pane's own header, and in the heads and places that are about the agent or its provider itself: the start picker, Settings, the footer's usage chips and the Usage pages, the session record's head, the tree popover's head, the last-words popover's head, the phone's detail head, the Memory tab's Hide AI chip, the first-run tiles, and the selected View's toolbar ([View openers and comments](#view-openers-and-comments)).
The sidebar's 프로젝트 and 태스크 rows and the Agent tabs draw it only by the operator's choice or while ⌥ is held ([Agent logo view mode](#agent-logo-view-mode)); ⌘K and the recent-pane list and the phone's list lead with the status mark alone in both modes (PRD agent-mark-quiet); a session record's children, the PR card's session rows and the child tree popover's rows draw the logo after the status mark in both modes (PRD record-card-readability D-06, D-09).
A pane header draws exactly one logo, the pane's own agent, for each of the seven supported agents: Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity, the same logo Settings and the first-run choice draw.
A row without a logo names its agent by product name (`Claude Code`, `Codex`, the adapter's label, never the raw kind) in its hover hint and its accessible name, and the pane header's logo and title say it the same way.
Only a plain shell and an agent Hide does not support draw the neutral `>_`.
A logo drawn dark on a transparent ground (Grok) sits on the light plate so it stays visible on the dark theme; the others are drawn as published ([BRAND.md](BRAND.md#third-party-agent-marks)).
A child pane's ancestor path names each ancestor from the root to its parent, `›` between them, and each name opens that pane; a root has none.
The names drop before current identity or actions when the pane narrows, leaving `›` with its accessible name.
The PR mark is the one the 태스크 row of the same agent draws: a pane without children shows its own PRs, and a parent shows the PRs over itself and every descendant (the `#number` when that is one PR, each state's count when several), pressing which lists each PR with who holds it, so a child's pull request is seen from the pane that delegated it; under the pointer or focus it opens below the header what the 프로젝트 row's mark shows, the PR card for one PR and the list for several.
The fork mark, a small fork icon after the title, is drawn only on a pane Fork agent made; its tooltip and accessible name read `Forked from <source name>` while the source still runs the conversation it was forked from, and the mark is then a button, reachable by keyboard, that opens the source; once the source closed or moved on to another conversation it reads `Forked from another pane` and opens nothing (`web/src/PaneHeaderBand.tsx`, `PaneForkMark`; the fact is the core's, `PaneForkSnapshot.forked_from`).
A delegated child and a handed-off root have none, and neither does a fork made while forks were children (PRD fork-operator-root B20, B21).
A device's pane header wears the mark too, as its rows do, although a device's pane offers no fork.
The letters-waiting pill follows the title and the fork mark while letters wait on the pane for a possible draft, as the core decides ([delivery.md](delivery.md#delivering-a-held-letter-now)): the envelope, the count from two letters, and `지금 전달` (`Deliver now`) on the popover fill with a border edge, which stands apart from both a focused and an unfocused header in Light and Dark.
The whole pill is one button, reachable by keyboard and pressed with Enter, and its hover and accessible name read `대기 중인 편지 N통 지금 전달`; a pane narrower than the pane breakpoint drops the word and keeps the envelope and the count, and pressing it still delivers.
Pressing sends `deliver_letters_now` and nothing else: the pill goes, and the bell line appears in the terminal as the doorbell submits it through Herdr's `agent.prompt`, with no confirmation and no warning about a draft, because the pill sits right above the composer.
The line begins with 🦉 (U+1F989, no variation selector); Codex CLI on Windows may omit it, and the saved bell still identifies the letter ([delivery.md](delivery.md#recognizing-the-bell)).
No other hold of a letter reaches the screen, and nothing changes on the sender's pane (`web/src/components/letter-mark.tsx`).
The tree popover's `Overview에서 일감 보기` opens the Overview on the agent's device with the card of the task it belongs to selected and the agent's own record in the panel (PRD overview-device-tasks B59); a resting or cleanup card's fold opens with it, so the selected card is drawn.
The zoom control names the panes it hides and unzooms the tab; the input pane keeps its accent perimeter and header tint.

The second row over the terminal shows the pane's turn, `<request> → <answer>` with no word before the request (the button's accessible name still begins `요청:`), with a child-tree button at its left when direct children are known (PRD pane-last-request).
The request is the operator's last one, the label's one-line summary of it, read against the conversation (`제안대로 진행 + 애매한 상태 더 점검` for `ㅇㅇ 그렇게 해 … 점검들 좀 해줘`), in quiet text; until the label has summarized it, with summaries off, after a failed judgment or for a turn before the summary existed, it is the request's own first line, and a request of images alone reads `이미지 N`.
A letter another agent sent after it does not replace it; a delegated child with no operator request shows its sender's last request the same way.
The answer is the operator's ask, the agent's own Working progress (`작업 중` without a sentence), for a finished turn the operator has not looked at, `끝남` and its result sentence, or, while the agent waits on working children, `대기 · 자식 N명 작업 중`, the words its sidebar row says; once the operator has looked at a finished turn the row goes.
The request shows only with an answer: a pane that waits on anything but working children, rests or finished and was read has no row (only the children button when it has live children).
When the request and the answer do not fit one line the answer moves to a second row under the request, each cut only when it overflows its own row, and the two return to one line when the pane widens; the children button stays at the first row's left, the request is never folded into an icon, and both rows overlay the terminal without changing its grid.
The request is a button the keyboard reaches: the pointer or focus on it opens the turn card under it, a press pins the card and a second press or Esc closes it, and Esc on a card neither pinned nor focused stays the terminal's.
With the card open, Tab from the request or the title moves the keyboard into the card to `기록 열기`, the card stays open while focus is in it, Shift+Tab before its start goes back to the trigger, Tab past its end and Esc in the card close it and return focus to the trigger, which keeps it closed until the pointer or focus comes anew; only Tab moves focus in, so the pointer never takes the terminal's keyboard, and a card the pointer only shows is no Escape layer, so Esc reaches the terminal.
The card reads the session as one conversation top to bottom on a rail: up to two earlier turns first, oldest first and dimmed, each ringed in how it ended (the warning colour for a question or a block, the success colour for done); then who sent the request (`나` or the sender) over a bubble with the request's summary and its own first two lines, and who sent a later request under it; then the agent's half, the answer's verb as a pill in its state's colour (warning while it asks, the working colour while it works, success when finished) with the label's words for the turn, never the agent's own words, which the record keeps and which could still be the previous turn's while a new one starts; and `기록 열기`, which shows the session record in the lane; a part with nothing to say is not drawn (`web/src/PaneTurnCard.tsx`).
The second row is absent when there is no turn to show and no child button; a pane whose terminal is not live has no ask, request or progress on it, because its body says why.
The core publishes the ask (`herdr-core/src/agent_state/header.rs`, `Header.band`): the pane's own Approval, Answer or Blocked first, the lead raised descendant next, then the pane's own Stopped.
A pull request's verb (Fix, Review, Merge) is the header PR mark's colour and a finished turn is the status mark's dot to read, so neither draws a row; the PR mark carries no review word either.
An ask stays until it is answered: once the operator has had the pane focused it dims to `--opacity-read-status`, as its row's request line does, and a question the operator read but did not answer keeps its row (`Band.read`).
The pane's own ask starts with its verb, `승인`, `질문`, `막힘` or `멈춤` in the warning colour, then what is asked and the stable wait; an Approval names no command, since the core reads none, and shows the ask's own default words.
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

The pane menu (from its overflow control or a right-click on the header) groups the parent and other siblings under Related sessions and the pane name and ID under Copy details, then offers Close terminal, which asks about the agents it spawned as Closing an agent that spawned others says; opening it moves no focus and marks nothing read.
The pane's own children are not in it: its tree button opens them.
The menu names the pane as its header does and is drawn from the pane as it is now, not as it was when the menu opened: a session's label arrives after its first turn, and the heading already open then reads the new label and offers Fork session and Sleep agent as the core now says.
Fork session groups the new-tab and right-side destinations without changing which sessions can fork.
A right-click in the terminal focuses that pane, like a click, and opens a longer menu: Copy (only over a selection), Paste, Select all, and Find; then Split right, Split down, and Zoom terminal or Exit zoom (disabled on a tab's only pane); then the pane menu's items; an item with a chord that does the same shows it, ⌘C and ⌘V included.
The Zoom pane chord on a tab's only pane does nothing either, on this machine and on a device: the core does not send it and logs `pane.zoom.single_pane` (`remote.control.zoom_single_pane` for a device).
Copy pane ID, here and in an agent row's menu, copies the id the pane's own Herdr knows it by (`w9J:p52`), the one `herdr pane read` takes on that device, without the `remote:` scope Hide gives a device's pane.
The right-click leaves the drag selection as it was, so Copy copies what the operator selected; the program in the pane never hears it.
A wheel over the pane moves a drag selection with the text it covers: the pane has no scrollback of its own, so the highlight is placed again on the rows that text moved to, drawn only for the part still on screen.
Clipping the highlight preserves the original selection ends and Copy text; reversing the scroll while part remains visible restores the complete highlight.
Once none of it is on screen the highlight goes, but Copy (⌘C, the menu, Ctrl+C where a selection makes it copy) still takes the text that was selected, until the next click, drag, select all or key; Ctrl+C interrupts again only after that.

A pane whose agent sleeps (PRD agent-sleep) shows its state in place of the terminal, which stays hidden until the agent is back because the shell under it is not what the operator was talking to: Sleeping with the last progress line and Wake agent; Waking… with how old the resumed conversation is; or `Couldn’t resume this conversation` with the core's plain reason, Retry, and Start new session.
Its second row carries no ask while it sleeps, and typed input to the pane goes nowhere, from the first moment its end begins until the agent is back: keys, a dropped file's path, the find keys and a phone's reply alike ([The phone app](#the-phone-app)).
Opening the pane's tab by a committed move (a row, a tab, a checkout, a relation) wakes it in the same pane with its conversation; a Recent Panels preview does not, and neither does a click inside the tab already on screen.
Wake agent, Retry and Start new session take a mouse click as they take Enter: everything a pane draws over its terminal (these bodies, the reconnect caption, the attachment refusal, the header band) lies above every layer the terminal's renderer draws, so a click there never reaches the terminal (`web/src/terminals.ts` gives each terminal its own stacking context).
The pane menu offers Sleep agent on an agent pane that is awake on this machine, or on the machine the window runs on when that machine's core runs on another machine, disabled with the core's reason when the agent is working, waiting for the operator, of another kind, or has no conversation Herdr reported; a device's pane offers none.
Archive session (`세션 보관`, PRD session-archive) stands right under Sleep agent, or first in that group when the agent cannot sleep, on a root agent's pane; a delegated child's pane draws it disabled (`루트 세션과 함께 보관돼요`), as does an agent whose conversation Hide has not seen and a pane of a device that is not connected, each with its reason; a shell and Hide AI's own agents offer none ([The sidebar's 태스크 list](#the-sidebars-태스크-list) says what it does).
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
A row carries the PR mark (Purpose, pinning, and PR chrome) the 태스크 row of the same agent draws: a root with children gathers its own and every descendant's PRs, each once by URL, whether its children are folded, behind `N개 더`, or in another checkout or on another device, and a child or grandchild row, like a root without children, shows its own (PRD project-tree-pr-family).
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
Web owners: `web/src/PaneView.tsx`, `web/src/PaneHeaderBand.tsx`, `web/src/PaneRelations.tsx`, `web/src/components/sidebar-agent-row.tsx`, `web/src/components/agent-tree.tsx` (the verb, ask line, descendant mark, rails and chevron the three surfaces share), `web/src/components/pr-mark.tsx` and `web/src/prMark.ts` (the PR mark every surface draws), `web/src/components/letter-mark.tsx` (the letters-waiting pill and envelope), `web/src/components/agent-tree-popover.tsx`, and `web/src/sidebarTree.ts`.

### Work Hide AI does for a home

Hide AI starts agents for the operator by itself (today a Memory card's `올리기`), and the work belongs to a screen, its home, not to the operator's agent lists (PRD hide-ai-work-home).
The Memory tab's To resolve row is the home of a promotion: it shows the progress, the pull request, the failure and the agent link, and it is where the work is read and acted on.
The fact that Hide AI started an agent comes only from the coordination record of the spawn (`docs/delivery.md`), never from a branch or worktree name: a checkout the operator made by hand on a branch called `hide-memory-m9` is an ordinary checkout.
The work is named in words, `Memory 올리기 · m72`, and never by an icon of its own, because Hide AI will start other work, each kind with its own home.
`docs/status-model.md`, Work Hide AI does for a home, owns the ownership value, the group rule and the turn table; this section owns what the screen draws.

While the agent works, finishes or rests, the work is in no list: not the sidebar's tree or Working, not the 태스크 list's 일하는 중, 끝남 or 쉬는 중, not the phone's list, not Overview's requests and graph, and it moves no badge, rail count, checkout count, desktop notification or push.
The agent is still found by ⌘K, the recent panel, the tab strip and `hide agent list`, and its pane header keeps the owl; only the lists that enumerate the operator's work leave it out.
The sidebar tree has no `Hide AI` head, and a checkout Hide AI made is drawn as an ordinary checkout only while it is the front checkout or an agent the operator started works in it; the Hide AI agent's own row is never in a tree.
The same rule applies inside the empty-worktree and Inactive folds, so a fold's count is of the checkouts it draws; the cleanup fold lists every cleanup candidate, because reviewing them is its job, and that is where a worktree kept after its pull request settled is found.

The operator is called out in two cases, and both are one row in the sidebar's Needs You and the 태스크 list's Needs You with the owl beside the status mark, the task name and a place line naming the task where the branch would stand (`herdr-ide › Memory 올리기 · m72`).
The agent asks: a question, an approval or a block, with the same read rules as an operator's row, so an AI-inferred question leaves once read and a block or native question stays until answered.
The home has a turn: the attempt's pull request is open, ready, passing or unchecked and nobody asked for changes, so the second line is `PR #924 리뷰` and the row stays after it is read until the pull request is merged, closed, made a draft, or gets a requested change or a failed check; or checks failed and the agent is no longer working, so the second line is `CI 실패` until the agent works again.
A draft, running checks and a requested change are the author's turn and draw nothing.
A row has one line, and its second line is the agent's ask or its block's cause, then `CI 실패`, then `PR #n 리뷰`; the core decides the turn and the screen only words it.
The row's accessible name carries `Hide AI`, the task and the turn (`Hide AI · Memory 올리기 · m72 · PR #924 리뷰`).
An attempt with no live agent (a failure with no pull request, an open pull request after the operator closed the pane) never has a row; the Memory tab alone shows it, as it did.

Choosing the row, by click, by its digit or from its desktop notification, does not go to the pane: it opens the Tools lane on the Memory tab of the row's Project with that attempt's To resolve row open, and the `Agent ↗` link there opens the pane.
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

The sidebar footer carries a View menu between the usage chips and Help: an eye button whose hint and accessible name are `보기` (`View`).
It opens above the footer with the label `에이전트 로고`, two choices, `행 왼쪽에 표시` and `숨기기`, with a check on the current one, a separator, and a muted line that is not a choice: `⌥ 누르고 있으면 잠깐 보기` (`Alt` on Windows and Linux).
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

### Subagent count

Hide draws no count of the subagents an agent runs inside its own session, on any surface: not on a sidebar row, in a pane header, in the 태스크 list, in an Overview line, in ⌘K and not in Settings (operator decision on issue #810).
A subagent has no pane of its own, so it is not a delegated child: it has no row to open and the operator can neither enter it nor act on it, so the number is not a state they would act on (design principle 13).
The child marks on screen mean only the agents Hide started, which have their own panes and tree rows: the descendant mark `! N` on a row, the tree button with its `자식 에이전트 N` tooltip, and the child popover.
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
A target whose own status is unknown still shows only the status notice.
The Stop-work confirmation has one line under its title and lists every pane that closes, named as the sidebar names it, with its status mark; a working, asking or unknown pane adds its status word, and a quiet one is dimmed with its state in the mark's tooltip and its accessible name.
While it is open the list is live: a pane that settles stays listed and dims, one that starts working or asking brightens, and its title, line, buttons and focus stay as they opened.
While a listed pane's status is unknown, `Stop work and close` is disabled, `Keep open` takes the keyboard if the button held it, and a `Check status` in the confirmation reads the status again without closing it.
`Stop work and close` sends the close the confirmation shows at the press.
Otherwise one sheet opens in place of the Stop-work confirmation, titled `이 에이전트와 자식 N개를 닫을까요?` (a tab: `이 탭과 자식 N개를 닫을까요?`), N counting only the descendants outside.
It has no sentence under the title, and lists the target first and then its descendants in tree order, indented by depth, each with the sidebar's status mark and name and a device chip when it runs on another device than the target; the list scrolls inside the sheet when it is long.
Closing a tab lists every agent in the tab as a target, each followed by its own descendants, so a working agent beside the parent is shown before it closes.
A line above the list counts, with the sidebar's marks and neutral text, the descendants waiting for an answer to a question, approval or block (`답 대기`), working (`진행 중`), holding an unread result their row marks with a dot (`확인 안 한 결과`; a delegated child's finished turn is its parent's news, so it reads quiet here as on its row, status-marks B24) and unreadable (`상태 모름`), leaving out a kind with none; those rows are bright and add their status word in neutral text, and a quiet one (idle, read, asleep) is dimmed with no word, its state in its mark's tooltip.
Each row is focusable, and its accessible name is its name, its device when it differs, and its status word.
`이것만 닫기`'s tooltip and accessible description say what it leaves: the children keep running and come up into the operator's own list.
Colour in both sheets is the status marks' and the one destructive button's (`모두 닫기`, `Stop work and close`); `취소`, `Keep open` and `이것만 닫기` are neutral.
Its buttons are `취소`, `이것만 닫기` and `모두 닫기`, and `모두 닫기` holds the keyboard when it opens, so Enter closes the whole subtree; Escape or `취소` sends nothing and gives focus back.
While a descendant's status is unknown, `모두 닫기` is disabled, `취소` holds the keyboard, and a `상태 확인` in the sheet reads the status again without closing it; while a target's own status is unknown, `이것만 닫기` is disabled too.
`이것만 닫기` closes the target alone as the ordinary close, with no second question; its direct children become the operator's roots and their own children stay under them.
While the sheet is open its list is live, as in the removal dialogs: a descendant that appears shows up, one that goes drops out, and the title's N and the count line follow.
`모두 닫기` sends one event naming the target and exactly the descendants the sheet shows at the press: one that appears after the press is not closed, and one already gone counts as closed.
The core checks each listed pane again as it arrives; if one now needs a status check, an earlier close of one is still unresolved, or the close slots cannot take them all, nothing closes and the one-line notice says why and what to do.
A pane whose earlier close was refused is simply closed again, since asking again is the retry.
If every descendant goes while the sheet is open, it turns in place into the target's Stop-work confirmation, a quiet target dimmed, whose `Stop work and close` closes the target alone; a descendant that appears under an open Stop-work confirmation turns it into this sheet the same way.
Either one closes by itself only when its target pane or tab is gone or its device disconnects; otherwise it waits for the operator's answer and runs nothing on its own.
A sheet that turns into the other leaves the keyboard on the sheet itself, never on a close button an Enter meant for the old one would press.
The core closes the deepest descendants first, each pane only after every descendant below it is gone, and the target last, so no descendant ever surfaces as a root on the way; each row reads `closing…` until it goes.
A descendant whose close is refused, times out, or loses its device keeps itself and its ancestors, the target included, open while the other branches finish; the close failure notice shows and the detail goes to the diagnostic log.
Closing the same target again lists only what is left.
Each local pane or tab closed this way gets its own Reopen closed tab entry; a device's close leaves none, as before.
Every entry to a pane or tab close uses this: ⌘W or ⌥W, ⌘⇧W or ⌥⇧W, the desktop menu's commands, a tab's ×, the tab menu's `Close tab…`, the pane header's ×, the pane menu's `Close pane`, and the agent row menu's `Close tab…`; closing a View, the phone app and the Overview are unchanged.
Web owner: `web/src/close.ts` (which sheet, which panes and descendants, and their states), `web/src/Overlays.tsx` (both sheets), `web/src/components/subtree-list.tsx` (the list both the sheet and the removal dialogs draw); core owner: `herdr-core/src/runtime/tree_close.rs`.

### Terminal links

A terminal pane links what its program prints, whichever program it is: an http(s) URL, a path, and a link the program marked itself (OSC 8).
A plain-text path is a link only where it names something on this Mac, so a word that merely looks like a path draws nothing; the check runs when the pointer reaches a row, never per frame.
A relative path is looked for under the pane's folder and then under its checkout's root, `~/` names the home folder, and a location written after it (`:12`, `:12:5`, `#L12`, `(12,5)`) is where the file opens.
The original spelling is checked before treating punctuation, attached prose or a location as context: if `docs/a.md)에`, `docs/a.md)` or `src/a.ts:12:5` is itself a file name, that complete name wins.
Composite punctuation, prose and locations also preserve prose-only literal names such as `(src/a.ts:12:5)` before dropping their leading or closing punctuation or interpreting the location.
Among the bounded interpretations, the longest existing path wins; a failed or budget-skipped longer check never establishes a shorter link.
In Korean prose such as `보드 보기 (docs/README.md)에 C안을 추가했습니다.`, only `docs/README.md` is underlined and clickable when that is the actual path.
Korean, Japanese and Chinese attach the next word to a path without a space, and Claude Code draws a backticked path without its backticks, so `view-mode.html에` reaches the pane as one word.
The run of Hangul, kana or Han characters at a word's end is that attached prose, and it is cut once, where the script changes: `view-mode.html에`, `docs/a.md)에서도` and `view-mode.htmlに` link `view-mode.html`, `docs/a.md` and `view-mode.html`, with no list of particles.
A run that starts right after `/` is a folder entry's own name and is never cut, so `docs/회의록에` is checked only as written; a word in another script attached to a path is not prose either.
A URL cannot be checked, so its attached prose is cut only after a closing mark, where it is always prose: in `PR #682 (https://github.com/o/r/pull/682)이 끝났습니다`, only `https://github.com/o/r/pull/682` is the link, while `https://ko.wikipedia.org/wiki/서울이` links all of it.
Ranges follow the terminal's actual cells, including wide glyphs, combining sequences and wrapped rows.
A path or URL the terminal wrapped at its last column, or a TUI broke at its own margin and indented, is one link across its rows; a URL spans rows only where the terminal wrapped it, because it cannot be checked.
Under the pointer a link is underlined and the pointer becomes a hand; a click on it is the link's and never reaches the program, and a drag across it selects its text and opens nothing.
A click opens it in the front Workspace: a URL or an HTML file (`.html`, `.htm` or `.xhtml`, case-insensitive) as a browser display, another file in View at its line (an ordinary tab, not a preview), a folder revealed in the Explorer.
A file in another registered checkout opens the same way in the front Workspace and moves no checkout: the pane that printed it stays on screen, the Explorer keeps its tree and selection, and the file stays that checkout's document, so its own Workspace shows the same text and closing one view leaves the other.
A folder in another registered checkout brings that checkout forward to show it in its Explorer, as does any path while the Overview is on screen, on any machine: a checkout on a node or device comes forward once that machine's Herdr has moved there, and the file opens then; with no Workspace in front a URL goes to the default browser.
Right-clicking a URL offers Open in Hide, Open in default browser and Copy link above the terminal's ordinary editing and pane actions.
Right-clicking an existing path offers Open, Open in default application, the host's Reveal in Finder/File Explorer/file manager and Copy path; an HTML file in a registered checkout offers Open in browser and Open source instead of Open.
Open source opens that HTML file in View at the line its link names.
These choices use the exact target under the pointer, including an OSC 8 label, and keep that target after the pointer leaves it; Copy path copies its physical path without the line suffix.
Opening or dismissing the menu moves no checkout, opens nothing and sends no click to the terminal program; a late path check cannot reopen a dismissed menu or replace a newer one.
⌘-click (Ctrl off macOS) hands the link to the operating system: a URL to the default browser, a path to its default application or, for a folder, a Finder window, within the limits below.
A path outside every registered checkout, such as `/tmp`, goes to the operating system on a plain click as well.
The operating system opens only a plain document (text, source, a PDF, an image, audio or video) or a folder whose name has no extension; anything else, such as a script, a program, a spreadsheet, an application bundle or an installer, is revealed in Finder, never opened.
A program's OSC 8 link takes the same routes with no confirmation dialog: `file://` and `vscode://file/` addresses are paths, any other scheme is not opened, and a refused link goes to the diagnostic log.
A pane on an SSH device, and a page outside the desktop app, links URLs only, because its paths name files this Mac cannot check.
A path with a space in it is a link only where the program marks it (OSC 8), since plain text gives no way to tell where it ends.
Web owner: `web/src/terminalLinks.ts` (detection), `web/src/terminalLinkProvider.ts` (the check and the routes), `web/src/terminals.ts` (pointer-to-cell lookup), `web/src/PaneRelations.tsx` (target-specific menu), `web/src/actions.ts` (`openLink`, `openTerminalPath`), `web/src/editor/lineRequest.ts` (the line); desktop owner: `desktop/src/main/localPath.ts`.

## Project Memory: the Memory tab

Web owner: `web/src/MemoryTab.tsx`, `web/src/memoryList.ts`; core owner: `herdr-core/src/runtime/project_memory.rs`, `herdr-core/src/runtime/memory_promotion.rs`.
Memory is the rail's second tool, after Work and before Files and Changes, and shows the Project of the checkout in front (PRD project-memory-v2 D-17, B19-B32).
The core opens one Project's Memory per daemon; the rail, whatever the lane holds, names its Project when it shows, when the front Project changes and when its window comes forward after another window named another Project, so the Memory icon's dot is current, and the tool reads until the core answers for its own.
Its head is the `Auto memory 사용` switch and the Hide AI chip, the agent Hide AI asks now with its logo (the listed agent standing in while Runs on is past the usage limit or cannot answer); with Memory off, one line says what is sent where and a `데이터 삭제` link sits under it while the Project has cards.
Turning the switch on when this Mac's agent hooks do not carry Memory yet updates them first and turns it on once they land.
A press before Hide has first read those hooks, right after it starts, shows the switch pending, waits for that read and then runs as pressed; a read that fails refuses the press, and a press for another Project meanwhile is refused as busy.
While a pass analyzes the Project, a small `분석 중` chip sits beside the switch; with Hide AI off or unable to run, the chip dims and one line with `Hide AI 설정 열기` says verified cards are still used; when every agent is past the usage limit the same dimmed state reads "Hide AI is paused: every agent is past your usage limit this week. Saved cards are still used." (PRD hide-ai-usage-limit D-06).
A Project on any linked device shows the same Memory controls and cards as this machine's Projects, read from the core's store with the device's own Project identity.
The disclosure says cards stay in Hide's Memory, and the delete confirmation removes them from this Project's Memory; neither claims the screen's device owns the store.
An unavailable link or Memory reader uses the existing unreadable state; its reason is recorded in the diagnostic log.
`처리할 것` appears, and the tab's name carries a dot, while the Project has a suggestion, an unresolved document comparison, a failed placement or a decision nothing builds yet.
Each compact row has a one-line title and one-line reason, truncating long text; `Compare` reads and opens the document excerpt and original operator words with their dates only when requested.
A confirmed enduring operator correction offers `Fix the doc` and `Card is wrong`; unresolved authority first offers `Card is right` and `Doc is right`, and cannot be selected until answered.
Dates remain provenance and never decide authority.
Choosing the document marks the card Outdated; choosing the card turns the row into a document correction proposal.
Eligible rows have checkboxes; selecting one to five shows `N selected · up to 5 per PR`, `Clear` and `Open one PR`, and at five the other checkboxes are disabled.
The row's single-card action uses the same batch path.
While any row is selected, per-row promotion and dismissal buttons are hidden; unknown-authority answer buttons remain so those comparisons can be settled before selection.
Promotion reasons omit zero counts and use short source-session counts.
Started single or batch promotions remain in To resolve, in the same two-line format as the questions: a PR mark in the checkbox slot, the first title and `+N` for additional cards with the disclosure indicator, then the status, elapsed time where measured, subdued card numbers and links.
Before a PR exists the row says Writing PR with the durable attempt's elapsed time; cards bound to the same running agent and attempt share a row.
After confirmation, cards with the same PR number share a row, and `PR <number>` opens the existing PR panel.
The catalog's failed checks use destructive text, requested changes and an agent asking the operator use warning text, and writing or awaiting review is neutral.
`Agent ↗` appears only while writing or needing attention and only when a real pane is known; a quiet review row keeps only its disclosure indicator.
After PR creation the agent link requires a Hide AI promotion origin and that agent's live PR association with the catalog's full repository-qualified URL, rather than a name or branch guess.
This applies equally to promotions already open before upgrade; PR numbers remain plain identifier digits in all four languages.
Every To resolve row expands from its title, reason or empty space, or its single disclosure button with Enter or Space; the chevron only indicates expansion.
Checkboxes, links and action buttons perform only their own actions.
An ordinary question opens its current body, cited files and sources below its action buttons, while a document disagreement opens Compare content.
A progress row opens a two-line body preview aligned with its title and status; a single card does not repeat its title, while a batch labels each preview with its card number and title.
The progress row keeps medium inset spacing around its header and preview, with a small gap above the expanded body, so its content stays clear of the hover surface and bottom rule.
PR and agent actions appear once, in the status line, and an expanded row also exposes its known agent there when the collapsed status does not need that link.
Merged promotions leave To resolve and become document pointers; closed-unmerged promotions leave To resolve as ordinary cards and are not suggested again at that revision.
A promotion row whose agent is alive is also in Needs You while the operator has something to do (`PR #n 리뷰` or `CI 실패`), and choosing that row opens this tab with it open (Work Hide AI does for a home); the row here still shows the progress, the pull request and the agent link whether or not it is in Needs You.
When the pull request merges or closes, its agent ends and its worktree goes with the branch kept, so a closed attempt's `Agent ↗` link leaves with the pane.
A card that was not processed returns with `Card was not processed · Ended without a PR` in warning text and the agent's reason behind the row; every failed row offers Retry, continuing the shared branch and PR, and `Don't promote`, which leaves an ordinary card.
When a promotion reports a card outdated, its empty, clean worktree is released after its agent has left; committed or uncommitted work stays and a later promotion continues on that same branch and PR.
An accepted outdated report returns the card to verification with its original sources intact.
An outdated card sharing a batch waits while any sibling still runs, has an open PR or follows a building branch.
A card whose decision the code does not have yet, which another branch builds, stays in To resolve as a progress row, `Goes in with the PR that builds it`, with `Agent ↗` to that branch's agent and `PR <number>` once its pull request is open; it asks nothing, puts no dot on the tab and is still given to agents, and that pull request's merge makes it a document pointer like any merged promotion.
When nothing builds it, one row says `Not built yet · nothing builds it` with a clock mark in the checkbox slot and the agent's reason behind the row, and offers only `Keep as is`, which leaves an ordinary card that is not asked about again, its document disagreement included, until its text changes.
A new rule, lesson or decision suggestion keeps its source-session counts and dismissal until its revision changes; at most three new suggestions are open.
`카드 N` lists the other cards, two lines each: the number and title, then the kind, how many sessions it was given to, how many times it was pulled, how many of its sources are the operator's words, and when it changed.
The first five show, the rest fold under `그 외 N`, and a card shown in full is never folded; the search above them shows every match, best first, and is the Project's, so every window shows the same query and only the one typed into sends it.
A searched promotion card also shows its running status and the name that opens its pane, with no name before the session appears; expanding a progress row does not duplicate it in Cards.
Once merged, its ordinary card dims and shows the file it went into, or the merged pull request's mark when no file was named, with `파일로 올라감 · 주입 안 함`.
A personal preference carries `Personal`, remains available to agents and never appears as a document suggestion.
Unresolved fact disagreements remain withheld and readable in ordinary card detail, with no unsupported document correction action; rule, lesson and decision disagreements are eligible for correction.
A confirmed enduring correction says `Correction confirmed · document update pending` in the existing comparison row and remains available to agents as untrusted reference data.
It retains the original human source and document quote; unrelated document commits do not ask the same authority question again.
When the owning document incorporates it, background reconciliation turns the card into the existing document pointer without another promotion.
Turning Memory off retains cards and analysis progress; its setting explains that enabling it again catches up on unprocessed conversations, including conversations from the disabled period.
A Project at its 10,000-card limit makes room for a new card by forgetting its least recently used one, and the tab says so in one quiet line under the head, `N older cards were cleared to make room for new ones.` (`새 카드를 남기려고 오래된 카드 N개를 지웠습니다.`), counting the cards forgotten that way since the tab last said so.
The line is said once: it counts as seen once it is in view on a visible page, stays while the tab is open, and the next time the tab opens it is gone until another card is cleared, when it counts only the new ones.
`hide memory undo mN` brings one of the newest 1,000 cleared cards back, and so does `되돌리기` on a card just forgotten, but not while the Project is at its limit: the undo is refused with `mN cannot come back while this Project's Memory is full; forget another card first`.
A card added by hand is never cleared this way.
Only when every card is one a person made or settled (added by hand, a confirmed correction, or a card whose promotion is under way, waiting on the operator, merged or kept) does the line become the warning `This Project's Memory is full, so new cards are not kept.`, from the same check that refuses a new card; merges and corrections of existing cards still land.
An Outdated comparison dims its card and names the document and its date, asks nothing and is excluded from injection.
Document references in card metadata and Compare headers show a single-line filename, truncated when needed, with the full path on hover and the date alongside.
An unverified card waiting to be checked against the repository again is dimmed; only the card the worker is checking shows `확인 중` in its existing status line, and a promotion keeps priority there.
An already verified card awaiting its first document comparison stays usable and keeps its suggestions while waiting, even when the checking budget is spent.
The waiting reason or exhausted checking budget is explained once above the list, without repeating it on every row.
When no card is usable and cards await checks, the existing empty slot says why and that accepted cards return one by one.
The core supplies the checking budget's next reset timestamp, and its explanation draws that date and time in the viewer's timezone and interface language, with no UTC wording.
With Memory off, stored cards are retained but the list and its empty slot stay hidden behind the existing off disclosure, and the usable count is zero.
An older consent shows a short re-consent explanation under the existing off disclosure and points to the switch above; it never enables or injects on its own.
The core supplies the usable count, current card and reasons, and the shell only draws them; no banner, alert, toast or panel is added.
The checking mark is one line in the lane at its 360px minimum in all four languages, and the card is still listed and opens in full; an unverified card is not given to agents until the check ends (PRD memory-card-quality D-13, B6, B19).
When the agent is gone and GitHub cannot say whether its pull request came (gh missing or logged out, or GitHub not answering), the line reads `PR 확인 못 함 · <reason>` with no spinner and nothing to press, since restoring GitHub settles it; it is not asked about in `처리할 것`.
A card on its way into the repository is never folded under `그 외 N`, and a card opened from a conversation clears a search that leaves it out.
A card opens in place: `N분 전에 바뀜 · 이전 내용` when a text replaced another (the earlier text with `되돌리기`), its body, its cited files, its sources with their runtime, time and whether they are the operator's words, and `지우기`.
A forgotten card leaves `mN 지움 · 되돌리기` at the bottom of the tab until it is undone or another card is forgotten.
The `⋯` beside `카드 N` holds `데이터 삭제`, which asks first, naming the Project and the number of cards, because it cannot be undone.
An empty Project says no cards yet and when sessions are analyzed.

## Terminal image attachment boundary

Web owner: `web/src/attachments.ts`, `web/src/PaneView.tsx`, `web/src/explorerDrag.ts`, `web/src/terminals.ts`. Core owner: `herdr-core/src/runtime/attachments.rs`; device staging `hide-node/src/ssh/attachments.rs`.

Dragging an Explorer file onto a terminal focuses that pane and inserts the original absolute path as quoted text, without submitting it.
Image files instead read through the checkout's existing bounded file-bytes stream and enter the same binary attachment flow as an external image drop, retaining their original format and filename.
One Explorer image read is admitted at a time, capped at 20 MiB, and a page reconnection, receiving pane closure, or terminal transport generation change during that read discards the result.
Folders and files from another device show the forbidden drop cursor and insert nothing; moving files inside Explorer keeps its existing behavior.
Image acceptance and display remain the receiving program's responsibility, as for other terminal attachments.

Dropping local file URLs into a visible terminal focuses that receiving pane and starts one attachment intent through the existing ordered writer.
Paste captures PNG or TIFF clipboard images at the same ingress; ordinary text and keys keep their existing terminal behavior.
The core reserves the original pane and connection generation before asynchronous image preparation, file validation, or transfer, so later focus changes cannot redirect the attachment.
Clipboard images are normalized into private PNG files off the input thread, while the core owns transfer state, held input, retry, and cancellation.
Local files keep their original paths after validation; remote attachments are uploaded through authenticated SFTP and insert only the resulting remote paths.
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

Web owner: `web/src/Overview.tsx` (the modal and the page, the title and the device switch), `web/src/MainScreen.tsx` (the tab row with its counts and controls, the unreachable device, the Home with no project, the side panel), `web/src/overviewTasks.ts` (a device's tasks as the record and the backlog read them), `web/src/OverviewRecord.tsx` (a session's record beside the tabs at Home), `web/src/BacklogTab.tsx` and `web/src/overviewBacklog.ts` (백로그), `web/src/ProjectsTab.tsx` and `web/src/overviewProjects.ts` (Projects), `web/src/QuickAdd.tsx` and `web/src/quickAddChoice.ts` (the quick add), `web/src/issueMarks.tsx` (an issue's glyph and labels), `web/src/IssueDialogs.tsx` and `web/src/issueStart.ts` (Start).
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
Its head is the title `Overview` and the device switch: every connected device as its rail tile and name, the rail's front device marked `rail`, a device that does not answer marked `연결 안 됨`, and every other device's tooltip carrying `내 차례 N` (B2).
With This Mac alone there is no switch, since there is nothing to choose.
Choosing a device changes only what the Overview shows; the rail, the sidebar and the Workspace in front stay, and a project chip that was set clears.
The tab row shows Work, Projects, Backlog, History and Usage, with counts derived from the same scoped records each view shows.
A device that does not answer dims the live Work, Projects and Backlog tabs, and their body holds `<device>가 응답하지 않는다 · <언제부터>` and `다시 연결`; the age is the core's `unreachable_since_unix_ms`, and the diagnostic reason stays in the line's tooltip.
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

The Home with no project shows `Add a project to start an agent.` and Add project with its chord (⇧⌘N), which a browser tab, unable to pick a folder, shows as the sentence alone.
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
The Overview reads the same tasks for the Projects tab's `지금` counts and the order of Backlog's sections.

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
Selection, the project search, the graph/list choice and the camera are not remembered.

The graph is a force map centred on the operator, the way Obsidian's graph view is drawn (PRD overview-main-map D-01, D-20): a `나` circle is fixed at the centre, and every session without a lineage parent (a root) is held loosely toward it by a faint line.
Each project gathers around its own place on an ellipse around the operator, so a project's sessions settle as one cluster apart from the others.
A delegated session is a small dot in its state colour, tied short to its parent by a faint line, and a grandchild sits outside its parent the same way (D-02).
Behind each project's cluster is a soft glow in the project's accent colour, and its name sits in bold, larger than a session's title, in that colour above the cluster; there is no band or box (D-17).
The accent is one of sky, violet, lime and amber, chosen by a stable hash of the project's id.
Pointing at a project's name or its row in the side column, focusing either from the keyboard, or choosing one of its sessions so its details open, makes that project's glow and name clearly stronger, and they return when the pointer leaves or the choice closes; the current pane's highlight at entry lights nothing, and reduced motion changes them without a transition (B21).
Placement is computed in the web shell and settles before the first paint, so nothing wobbles on open; every start position comes from the session's id, so the same sessions open in the same shape (D-13).
Status, labels, letters, filters, selection, hiding and folding never enter the layout key.
When a session appears, every placed session holds still while only the new one finds a place beside its parent or in its project; a session that leaves takes only itself away.
Admission keeps the map within the 400-node budget (below); a map the layout cannot place fails into the existing Retry and session list.
A root's circle shows its title and its sidebar second line (what it is doing, what it asks, what it waits for); a descendant shows only its title, plus what it asks in amber while it is the operator's turn.
Provider, proved launch model, state words and judged tasks are not drawn under circles; the accessible name still reads title, state, provider and proved launch model.
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
Hiding never affects the list view or the filter counts.
Hiding the project of the selected or highlighted session clears that selection.
Below the project list, Waiting on you lists each root whose turn it is, drawn on the map or not, with its project, title and question, and Go answer focuses the asking pane: the root when it asks, else its first asking descendant.
A root in a hidden project stays in the list with a `그래프에서 숨김` mark, because hiding a project from the map does not make its question wait less (D-21).
Folding a team leaves its root circle with a chip that counts its descendants and shows their My turn, Working and Unread marks; choosing the chip unfolds it, and the descendants come back to their places (B24).
Folding a project, from the faint fold button beside its name in the map (clear while the name is pointed at), turns its cluster into one circle in the project's place, filled faintly and bordered in the project's colour, keeping the project's glow and its emphasis, with its name at the expanded name's size and colour, its session count and nonzero counts; a line between that project and another ends on the circle, and choosing the circle unfolds the cluster to its old shape; both places share one fold state, and the button never starts a project drag (B25).
Fold all folds every project into its circle and leaves each team's own fold as it was; Unfold all opens every project and every team; both sit in the toolbar of the graph view (B26).
A project folds and unfolds only on the map: its name's fold button, its folded circle, and Fold all or Unfold all; the side column does not fold (PRD overview-graph-remembered D-03).
Choosing a session by click, Enter or Space replaces the column with its panel: title, state, project and branch, provider and proved launch model, what it does now, what it asks the operator when it asks, its team (every descendant with its state; choosing one selects it), its counterparts with sent and received counts only, and `세션 열기 · 기록 보기`.
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
The panel's head is ✕ and `Workspace 열기 · lane에서 보기` (B18).
`Workspace 열기` closes the Overview and brings the session's pane forward; `lane에서 보기` closes the Overview, opens the checkout and shows the same record in the lane's 보기 slot.
A task on another device reads `<device>로 가서 열기`, which brings that device forward, and a record of a task whose root runs on another device than the one the Overview shows carries `<device>에서 도는 자식은 <root device>의 이 일감에 속한다` above it (B14).
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
It opens in place to every open missed item of the device's sessions, live or ended, newest first: the missed sentence with `백로그로 · 무시` as in a record, and under it the session's title and the item's kind, which open that session's record in the panel.
With no open missed item, or with summaries off, the line is not drawn.

백로그 has one section per project, the projects with a task that needs the operator first, then the most recently active (B25).
A section identifies its project, source-qualified items and open or started counts.
Status, source, author, assignee, labels and literal title search filter before the initial fifty visible rows; Load more increases the visible bound.
The add control opens the shared quick add, whose explicit destination and repository disclosure apply here too.
Enter submits the title through that quick add: a memo stands as an `L-N` row, and an explicit GitHub destination shows a dimmed `만드는 중` row that becomes the `#N` row once the issue exists (B29).
The input locks during creation, then shows its success line and closes as described below; an empty or blank Enter does nothing, and a title stops at 256 characters.
A GitHub issue made here, by the quick add or by `백로그로` carries the title alone: no request, answer or body leaves the device.
A failed GitHub create keeps its row with one line of why (`gh가 없음`, `GitHub에 로그인 안 됨`, `GitHub 원격 없음`, `네트워크 또는 사용량 제한`), `다시 시도` and `지우기` (B30).
A failure no retry can change (an add the core refused, or a repository that moved) offers `지우기` alone; the core keeps at most eight such rows, the oldest leaving first.
`다시 시도` names the same add, so an issue the first try made is joined rather than made again, even after a restart; past eight waiting creates a new add fails at once with `만들 것이 너무 많이 밀려 있음`.
A GitHub create is made only in the repository the project was connected to when it was asked; when Connections changes meanwhile, the create stops before the issue exists, fails with `프로젝트 저장소가 바뀜`, and its `다시 시도` makes nothing in the new repository.

Open unstarted items are the initial view; status filters can include started and closed items without changing their identity or Start eligibility.
A GitHub row is `#N`, the title, a flag when it came from missed work, its labels, its age, `↗` (GitHub) and `▷ 시작`; a memo row is `L-N`, the title, the flag, its age and `▷ 시작` (B27).
Memo flags belong to the memo's device and path/number; GitHub flags remain shared by repository/number.
When one source cannot be read, its last known rows and failure remain beside the other source's usable rows; an unreadable source is never an empty successful result.
On a device the core dials every section is the one line `이 기기의 <project>는 이슈를 읽지 않는다`, with no input and no `▷ 시작` (B36).

A row opens the item in wide detail: its title, `#N` or `L-N`, its source, when it was made, its labels, `어디서 왔나` for an item from missed work (the session's title, the assigned work it came from and the missed sentence, which open that session's record), `▷ 시작`, `GitHub에서 열기` for a GitHub issue, and `되돌리기` while it is not started and Hide made it: every memo, and a GitHub issue made by the quick add, the backlog's input or `백로그로`; an issue someone else made never offers it (B31).
The wide detail renders the real body as sanitized Markdown with actual metadata and comments, retaining loading, stale and failed states.
Comments load fifty at a time up to two hundred, with each comment bounded to four thousand characters; truncation remains visible.
`되돌리기` deletes a memo or closes a GitHub issue and the row leaves the backlog; a missed line it came from opens again with `백로그로 · 무시`, and a close that fails leaves why and `다시 시도` in the panel with the row in place (B32).
Undoing a memo on one device preserves another device's memo, origin and missed line even when their path and number match.
The record that names a Hide-made GitHub issue keeps the newest 256 typed items, so an older issue no longer offers `되돌리기`; a refusal from the core (the issue is not one Hide made, or not the project's repository), which sent nothing anywhere, reads `되돌리지 못함` with no `다시 시도`, while Hide's record that could not be read just now (`unavailable`) says so with `다시 시도` (`undoRetryable` in `web/src/overviewBacklog.ts`).

`▷ 시작` opens the Start dialog, which turns an item into work in one step: a worktree named for the item, its base, the agent, and the agent's first prompt.
The name opens as the issue number and the title's English words (`192-hided-sigterm-handler`, `L-3-...`, or `issue-192` when the title has none), and when Settings > Hide AI > Features allows it the background AI's name replaces it once it answers, unless the operator has typed; `↺ AI 이름으로` brings the AI's name back.
A name that is already a branch says so, and when a worktree has it the primary button opens that worktree instead.
The first prompt is filled from the item's body, which the dialog reads when it opens, names the item, and for a GitHub issue asks for a pull request that closes it when Settings > Agents > Starting work says so; for an item that came from missed work it also carries the original request and the answer of that turn, which stay on this device and are never written to GitHub (B33).
The detail, origin, Start prompt and naming body use the selected workspace and device as well as the item key; a same-path memo or a late read from another device supplies none of their words.
Closing a panel or Start dialog releases only its owner's origin, including when the connection closes.
The operator edits the prompt before starting.
Starting creates the worktree, writes the link into it, starts the agent with the prompt as its first instruction, then brings the new pane's Workspace to the front; a folder project has no worktree, so its agent starts in the folder with the prompt.

### Adding to the backlog from anywhere

`백로그에 추가` (⇧⌘M; Alt+Shift+M on Windows and Linux; ⌥⇧M in a browser tab, where Chrome keeps ⇧⌘M) opens one input at the top center of any screen, even while a terminal holds the keyboard, and takes the keyboard; ⌘K or the find bar closes first (B37).
A second press closes it.
Above the input stand the project it will go to and why it was chosen: `고른 일감의 프로젝트` (the project of the record, item or project the Overview's panel shows), `걸러 보는 프로젝트` (the Overview's project chip), `앞에 있는 체크아웃`, or `백로그가 있는 첫 프로젝트` when none of them keeps a backlog (B38).
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
Its top line is `<device> 디스크 여유 X / 전체 Y · 프로젝트 합계 Z · 정리하면 W`, the free space in the warning colour below 10 GB (`LOW_FREE_BYTES`, the cleanup sheet's own threshold), and a small `재는 중` while the sweep runs, the sizes in mono and the words in the body font; a device the core dials has no top line (B47).
A row reads the project's name with its GitHub mark (a Git project whose issues are not on GitHub has the folder-with-branch mark) or `폴더`, `지금` (a colour bar and the counts that are not zero among `내 차례 N · 일하는 중 N · 대기 N · 읽을 것 N`, `쉬는 중 N` when all four are zero, `일감 없음` with no task), the backlog's `N ›`, the disk, `챙길 것`, the last activity and `열기 ↗` (B48).
The backlog's `N ›` opens 백로그 on that project, and `열기 ↗` closes the Overview and opens the project's most recent checkout.
`챙길 것` is `N merged → 정리` while the project holds merged checkouts and empty otherwise; pressing it opens the disk cleanup sheet on that project's finished checkouts (B49); a project of a device the core dials says `N merged` with nothing to press, since its cleanup is that device's.
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
Generated titles and task summaries are an explicit additional search scope; no raw copied-transcript index is restored.
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

## Tasks, the 보기 slot and the PRs tool

Web owner: `web/src/TaskLens.tsx` and `web/src/taskList.ts` (the sidebar's 태스크 list), `web/src/DetailPanel.tsx` (the lane's 보기 slot), `web/src/SessionRecord.tsx` and `web/src/sessionWork.ts` (the session record), `web/src/PrsPanel.tsx`, `web/src/PrList.tsx`, `web/src/PrCard.tsx`, `web/src/prWork.ts`, `web/src/prCardModel.ts` and `web/src/prReason.tsx` (the PRs tool and the PR card), `web/src/PrDialogs.tsx` (이슈 잇기 and 맡기기), `web/src/prDelegate.ts` (맡기기's first prompt), `web/src/LinkSessions.tsx` and `web/src/linkPanel.ts` (the link record's sections); a row's agent line follows `web/src/agentRow.ts`.
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


Under the sidebar's head (search ⌘K, Overview ⇧⌘O), right above the list, a `프로젝트 | 태스크` switch picks the list (PRD sidebar-task-lens D-10, B1, B39): 프로젝트 is the projects tree as before, with its raised Needs You and Done, and 태스크 the device's tasks.
⌘B shows and hides the sidebar, and ⇧⌘B (`Switch Projects and Tasks`; Alt+Shift+B on Windows and Linux; in a browser tab ⌥⇧B on macOS, where Chrome keeps ⇧⌘B, and Ctrl+Alt+Shift+B on Windows and Linux, where it keeps Alt+Shift+B) turns the switch; with the sidebar hidden it shows the sidebar on the other list (B2).
A window starts on 프로젝트, and then the last list and view option chosen stay, through a restart too (B3).

The list follows the rail's front device, every project of it at once, and turns with the rail (B19).
Its first line reads `태스크 N`, the open tasks, with a `상태별 | 프로젝트별` option at its right (B4, B6).
상태별 groups the tasks as `내 차례`, `읽을 것`, `다시 볼 것`, `일하는 중` and `대기` (`Needs You`, `Unread`, `Revisit`, `Working`, `Waiting`), in that order, each head with its count, 내 차례 in the warning colour, empty groups not drawn, and every row naming its project before its second line (status-marks D-22, B32; PRD revisit-group D-01).
다시 볼 것 holds a question or approval the operator read and has not answered, which keeps its own mark and `질문 · …` line, and a quiet task the operator starred, which wears a ★ in its mark's slot (PRD revisit-group B2, B6); a starred task that asks, finishes or works stands in that group with a small ★ after its title, a read block stays in 내 차례, and nothing in 다시 볼 것 sends a notification, a push or a badge count (B3, B4, B7, B18).
내 차례 holds the questions, approvals and blocks; a block is a task whose root agent errored (the core's `demand == "error"`), not a failed CI, and its line reads `막힘 · <cause>` in the warning colour.
대기 holds the tasks in progress that wait on proven work (the core's task state `waiting`), 일하는 중 those whose agent works.
Tasks that rest, those resolved included, fold into `쉬는 중 N ›`, and tasks whose pull requests all merged while a worktree remains into `정리할 것 N ›`, at the end; each opens in place and closes with `접기` (B5).
Entering a resting task's pane, or a descendant's pane, opens its `쉬는 중` fold in either 상태별 or 프로젝트별 and brings the selected row into view; a focused task moving into rest opens that fold too.
A manual collapse stays closed through snapshot updates while focus remains there; entering another pane in that fold or returning to it reveals it again.
Archived sessions fold last, into `보관됨 N ›`: under 정리할 것 in each project of 프로젝트별, at the very bottom in 상태별, closed until opened, newest archived first, and a project whose sessions are all archived keeps its group (PRD session-archive B14, B15).
프로젝트별 draws one group per project, the projects with a 내 차례 first, its head the project's name with its counts by state in the same order and names (`내 차례 1 · 일하는 중 2`), the same order inside and its own folds, and no project name on the rows.
Where a project's rows change state, a small grey label names the band (`내 차례` in the warning colour, `읽을 것`, `다시 볼 것`, `일하는 중`, `대기`); it is skipped by the arrow keys and hidden from assistive technology, and each band's list carries the state's name instead; 상태별 draws no band labels, since its heads already name the state.
Inside a group 내 차례 and 다시 볼 것 stand longest-waiting first and the others stand most recent first (B9; PRD revisit-group B13).
Inside `쉬는 중`, both views order rows by their latest actual native prompt, newest first, so the order matches the age on each row; unknown prompt times stand last and equal times retain their existing order.

A row is one task, its root agent; descendants are its children only, and a memory question, a pull request with no session, a delegated child are never rows of their own (B7).
Keep the sidebar and task list (the former Sessions list) easy to scan: the root task and its parent-child hierarchy lead, PRs stay summarized in their mark, and check details belong in the PR card rather than additional list rows.
When refining these surfaces, simplify the amount of visible detail and the relationship between PRs and delegated work without hiding the operator's next action.
Its first line is the row's own status mark (the sidebar's, status-marks D-12), the title, while folded no count of its children but `! N` in the warning colour when a hidden descendant asks the operator (status-marks D-24, B35), the PR mark and the elapsed time (B8, B10).
In both 프로젝트 and 태스크, a session's elapsed time measures its latest actual native prompt, including a real request letter from another agent, rather than its latest state change.
State changes and a daemon restart preserve that reference; a new session never inherits it, and an unknown prompt time leaves the slot empty until the native read succeeds.
The existing localized units and shared clock also apply to child rows; open-task ordering, unread state, waiting and escalation still use their own existing state times.
Its second line is the sidebar's line in its parts: on 내 차례 the verb and the question (`질문 · …`) or the block's cause (`막힘 · …`) in the warning colour, a waiting ring's `대기 · ` and its cause, a stop's word (`끝남 · `, `일 남음 · `) and the background tail, work in progress its sentence alone, a waking, failed or unknown row its state, a sleeping row none (status-marks B10, B27, B29, B31, B33); on 쉬는 중 only unfinished work has a line, and a read one draws the quiet ring; with nothing to say there is no second line, and no stage bar is drawn.
An unknown row is dimmed with no mark (status-marks B16).
A row with children opens them in place under its chevron, five per parent and the rest behind `N개 더`, indented to the third level; a child in another checkout carries a branch line, a child on another device a device chip, a finished child a quiet title, and pressing a child goes to its pane, bringing its device to the front (B10, B11).
Folds are kept apart from the 프로젝트 tree's (`sessions_expanded_agent_pane_ids`).

A press or Enter goes to the task's Workspace, tab and pane; the arrows, Home and End move through group heads, folds and rows, Right and Left open and fold a row, and Space opens its record (B12).
The row of the pane with the keyboard, or its nearest drawn ancestor while folded away, carries the sidebar's selected fill and `aria-current`; the list scrolls only as far as brings a newly selected row into view, and never while the operator's scroll position holds the selected row unchanged (B13).
While ⌥ is held (Alt on Windows and Linux) each task row carries its number at its top right, as the 프로젝트 tree's roots and the Agent tabs do (PRD electron-digit-shortcuts-hints): the rows are counted top to bottom as the list draws them, a group's rows then the rows of a fold the operator opened, a task's children never, nine at most, and ⌥1 to ⌥9 open that row's task the way a press does.
The `프로젝트 | 태스크` switch decides which list ⌥n counts, so on 태스크 the numbers are this list's and on 프로젝트 the tree's, whether or not the sidebar is shown.
Under the pointer or focus the small ★ after the title is not drawn and the time's slot holds a ☆/★ button, the record button and `⋯`, which offers `기록 보기` (Space), `터미널로 이동` (↩), on a task `다시 볼 것으로 표시` or, on one in 다시 볼 것 or starred, `다 봤음`, then Resolve (not while the agent works or waits, or an approval prompt holds it) and, on a task, `세션 보관`, offered and disabled as the pane menu's Archive session is; a right-click on the row opens the same menu (B15, B18; PRD session-archive B1, D-27; PRD revisit-group B8, B9).
☆ stars the task and ★ is `다 봤음`, which takes the ★ and puts the read ask away so the task rests until something new; a root agent's pane menu offers the same item before Archive session, the 프로젝트 tree, Overview's list and its detail head draw a starred task's small ★, an archived row keeps its ★, the phone lists the same group between 읽을 것 and 작업 중, and a ★ or 다 봤음 that could not be saved, or a ★ past 256, leaves the row as it was and says so once (PRD revisit-group B10, B11, B15, B19, B20, B22, B23).
`세션 보관` closes the task's root and every descendant pane with it and keeps their conversations: a session that is not working, or only waits on the operator, goes at once, and one with work running first asks `작업을 멈추고 보관할까요?`, listing the agents working now, with `작업 멈추고 보관` and `취소`, neither holding the keyboard (B5-B7).
The row leaves the open groups for 보관됨 once every pane closed; a save that fails closes nothing, and a close some agents refuse keeps the session as it was, with one notice, the members that did close listed in its record, and `세션 보관` again finishing the same archive; the notice says whether a close was refused, timed out or lost its device, and once that root closes some other way its archived members stand in 보관됨 as their own row (B8-B12, B24).
An archived row reads its title, its project in 상태별, its PR marks and how long ago it was archived, with no status mark, since nothing runs; `보관하는 중…`, `이어가는 중…`, `이어가지 못함`, `지우는 중…` or `상태 모름` stand on its second line while they hold, and a device that is not connected dims it (B14-B18, B13).
Pressing it opens its record; `⋯` and a right-click offer `기록 보기`, `이어가기`, `상태 확인` while Hide cannot say whether its close finished, and `보관함에서 지우기…`, which asks `<name> 세션을 보관함에서 지울까요?` with `보관함에서 지우기` and `남겨두기`, neither holding the keyboard (B19-B21, B28, B34).
`이어가기` brings the root back in a new pane, which takes the screen as any start does, and its descendants after it, one at a time, behind it; a member that does not come back keeps its reason and its own `이어가기` in the record, and the row leaves 보관됨 once every member is back (B21-B24).
Deleting removes only Hide's record of the session; the agent's own conversation file stays, so the agent's own resume list still shows it (B28-B30).
Resolve reads the row at once and moves it to 쉬는 중 only after the core saved it; a failed save leaves the row with the current error, and input, a letter or new work brings it back.
A merged pull request rests a task on its own only once its finish was read and nothing is left to wait on; until then it stays under 읽을 것 or 대기.
The PR mark shows one pull request as its state's icon and `#number`, several as each state's icon and count, worst first; one opens its card in the 보기 slot on a press, several list each first, and ⌘-press opens GitHub (B14); under the pointer or focus it shows what the 프로젝트 row's mark shows, the PR card with the title, review and checks for one PR and each PR's state and title for several.
A stale GitHub read dims the mark, with `GitHub 읽기 실패 · N분 전 값` in its tooltip, and never draws a zero (B20).
While the first snapshot is read the list shows a small skeleton and no number; with no task it reads `진행 중인 일감 없음` and `New agent`, which opens the start panel; a device with no project reads `Add a project to start an agent.` with Add project (⇧⌘N, the sentence alone in a browser tab); and a device that does not answer keeps its last rows dimmed under `<device> 응답 없음`, their actions off (B20, B42).
Every control is reachable from the keyboard with a visible ring and an accessible name, and a state reads as a word as well as a colour (`묻는 중`, `일하는 중`; B21).
A hidden list draws no rows, the elapsed clock updates only its own text, and the list adds no work to a key or a terminal frame (B22).

### The 보기 slot

The lane has one 보기 slot per window, which holds a pull request's card, a session's record or a mission's map and belongs to no checkout and no Agent tab, so changing either leaves it as it was (PRD sidebar-task-lens D-50, B48).
Whatever opens a card or a record opens it there, and the Workspace in front, its Agent tab and the keyboard stay (B14, B15, B50); no `PR #N` or record tab appears in Files.
Its head reads `←`, the item's mark and name (`PR #N · <project>`, `기록 · <title>` or `미션 · <title>`), `↗` (GitHub) on a card, and `✕` (B47); a card's mark is its PR mark, in its state's icon and colour, here and on the rail.
The head keeps the lane's surface and what the slot holds sits on the page's background, a card and a record alike, as the 보기 screens draw it.
Opening another item puts it in front, and `←` steps back through the ones opened before, at most 20, the oldest leaving first; `✕` empties the slot and the lane goes back to the tool it showed, or closes when it had none.
The close chord (⌘W) is the slot's `✕` while the keyboard is in the lane or nowhere, as the close chord's rule says ([The right lane](#the-right-lane)).
While the slot holds something, the rail carries a fifth button above Memory with its mark, pressed while the lane draws the slot; calling a tool turns the lane to that tool and the button stays, so pressing it comes back to the slot (B46).
A changed file pressed on a card opens its diff in the checkout's Files and the lane turns to Files, the slot staying on the rail (B49).
At Home, with no Workspace in front, a record opens in the Overview's panel beside the tabs and a PR mark opens the pull request's Workspace with its card in the slot (B17).
A pane head's agent-tree button and a started backlog item (from ⌘K, a record or a missed line's `· 백로그 #N`) show the sidebar on 태스크 with the row unfolded and selected, and open the agent's record in the slot, or in the Overview's panel at Home (B40).

### A mission's map

A Conductor's mission (PRD software-factory-conductor) is drawn as a map in the 보기 slot, read from the core's mission record; the map edits nothing, and the operator changes a mission by talking to its Conductor.
The Conductor's first plan opens the map once, widening the lane for it as a page does; after that only the mission chip, the rail's slot button or the Overview opens it again (B14).
Before the first plan the map is one `계획 중` line (B19).
A summary row counts the elapsed time, the agents running over the cap, the nodes done over those in the plan, the decisions waiting and, while it is gone, `Conductor 없음` (B21).
The nodes are drawn top down by what they wait on: every node that waits on nothing starts on the top row, and the edges meet one `보고·인계` card at the bottom.
A card carries its status mark, title, worker (`<model>·<effort>`, `자동` while the Conductor chooses the effort), elapsed time and one line: its PR mark and number, `↰ #N 위` when it is stacked on another, its branch for a direct change, or its last line (B15, B52).
A change card's left band is its pull request's state colour and its bar has four steps, work, PR or branch, review, merge or main; a research or verify card has one step (D-42).
A card that waits on a failed or dropped node says so, and a stacked edge is dashed in the merged colour.
Pressing a card opens its detail beside it with `터미널로 이동`, `기록` and `PR 카드` where they apply.
A decision the Conductor waits on is one banner with `대화로 가기`; a mission whose Conductor is gone shows `Conductor 없음` with `이어받기`, which starts a new agent in the mission's checkout told to take the mission, and the core refuses a second Conductor while the first is there (B44).
More than twelve nodes fold the done ones under one line that unfolds them (B19).
When the device that runs the mission disconnects, the map keeps its last values dimmed under a note and invents no number (B20).
A build from before missions opens with an empty 보기 slot when the saved slot held a map; the rest of the saved screen loads as it was.

### The session record

A 태스크 row's record button or Space, and an agent pane head's status mark, logo and title, open the record in the 보기 slot ([Pane header lineage and ownership](#pane-header-lineage-and-ownership)); pressing the title again while the slot shows that record turns the lane back to its tool.
The Overview's panel draws the same record under its own head ([A record beside the tabs](#a-record-beside-the-tabs)); the two differ only in that frame, and a window reads one record at a time (PRD overview-device-tasks D-07, B17).
The record starts with its title and agent logo, then `<device> · <agent product> · <project> · ⌂ <checkout>`; long titles and paths stay within the lane.
Its native status and elapsed time share a quiet row with a `pane 열림` or `pane 닫힘` chip; there is no coloured action box.
An open pane offers `터미널로 이동` even after its work ends, or `답하러 가기` while asking.
A closed pane offers `세션 복구하기`, using the existing native-session resume in a new pane in the registered checkout containing its original working folder on its device.
A missing conversation path, unsupported agent, disconnected device or missing checkout disables restore and states why beside the action.
A restore in flight shows `복구하는 중…` and accepts no second click; a failed start shows an inline reason with Retry, and a successful start focuses the resumed pane.
The record remains open until the resumed pane is confirmed; a timeout or an unknown launch result offers `시작 결과 확인`, which checks the original request instead of starting another pane, and Retry after a confirmed failure keeps that request's identity and destination.
Archived sessions keep their existing tree-aware `이어가기` action.
`요청한 작업` follows, with the completed count over the total only when judged tasks exist.
Tasks stay first-asked first, using the existing checkbox and task verdict; working and completed tasks also name their state.
Each task shows the full original request and its timestamp underneath, rather than hiding the request in a tooltip.
The latest request without a task remains readable, including while it has not been judged, when it is a question, and with summaries Off; none of these invents a task or a `0/0` count.
The live agent's latest reported line follows as `지금`; native session identity, device and agent product together select that live agent.
Immediately below are `PR`, `발견사항` and `요청 기록` tabs, with counts for this session alone.
The findings tab opens first when this session has an open finding, otherwise PR opens first.
PR results are rows with a state mark, number and wrapping title, with merged and closed results folded under their completed count; issues saved as outputs appear in the same results list.
The results tab is named `PR · 이슈` when it includes issues.
Every output opens the existing PR card or issue action.
Findings distinguish problems with a warning triangle and suggestions with a lightbulb, using the stored kind rather than inferring it from the sentence.
Only this session's open findings appear here; a child's findings remain in that child's expanded details.
Each finding offers `백로그 추가` and `넘기기`, retaining the public-repository notice, repository identity checks, retry policy and idempotent creation contract.
Creating replaces the controls with progress; a failure keeps the reason and retry, and a handled item leaves the open list.
A device whose core keeps no backlog offers only `넘기기`.
The latest raw request shows its local `YYYY-MM-DD HH:mm` timestamp.
Request history groups visible requests under local `YYYY-MM-DD` date headings, keeping `HH:mm` at each row.
Request history keeps the full saved text, author (`나`, the parent title or `부모`) and timestamp, oldest first, with the newest four shown and earlier requests behind `이전 N턴`.
`함께 진행한 세션` is a separate flat list, five initially with a way to show more, rather than being nested under an inferred parent task.
Each row shows status, agent logo and product name, wrapping title and its own PR and open-finding counts; a large result set never becomes a horizontal strip of marks.
Pressing a child expands its tasks and tabs in place, one child at a time, while the current session stays on screen.
An explicit `기록 열기` opens that child's record as the next slot item with the existing back action; a live child also offers its terminal action.
With summaries Off, the same structure keeps native/manual title, native status, raw requests, outputs and child sessions, while hiding task judgments and findings.
Loading, failure, retry, disconnection, archive and resume continue to use their existing runtime policies.
At narrow widths and the largest interface text, all sections and controls wrap within the lane; no session section introduces horizontal scrolling.
An archived session's record keeps every part above, and its `지금` box reads `보관됨 · <경과>` and the last line the agent wrote, with `이어가기` as its one button; while it resumes the button reads `이어가는 중…`, a member that could not resume adds its reason in the failed colour, `상태 확인` stands in its place while Hide cannot say, and a device that is not connected dims the box and holds the button (PRD session-archive B19-B24, B34).
A live root whose archive left members closed lists them under its `지금` box, each with its state and its own `이어가기` (B24).
A first record read uses a small skeleton; a failed read shows Retry, and a disconnected device retains the last read dimmed until reconnection refreshes it.
GitHub values that cannot be refreshed stay dimmed with their last-read time in the tooltip; failure is never a false zero.
Web owners: `web/src/SessionRecord.tsx`, `web/src/DetailPanel.tsx`, and `web/e2e/session-panel.spec.ts`.


### The PRs tool and the PR card

PRs is the rail's second tool, a pull request icon, its hint and lane header `PRs · <project> 전체` (PRD sidebar-task-lens D-17, B26).
Its head holds the scope chip, `All checkouts` or `<checkout> only`, remembered per Workspace across restarts, then ⚠ while the last GitHub read failed or left a question unanswered, and `열린 PR N`.
A Workspace saved on the old Work tool (`work` or `agent_sessions`) opens once on PRs, and the rest of its lane (pin, tree, widths) and its View layout stay (B30).
The rail icon's number is the tool's Needs You rows, the folded lines left out; a session's Needs You is not counted (B29).
A project with no GitHub remote reads `GitHub 원격 없음` alone.

The PRs tool lists the pull requests of the project that concern the operator: their own, those their Hide sessions made or worked on, those asked of them or of their team for review, and bots'.
At Home it gathers the device's projects and puts the project's name on each row, and a project on a connected device lists like the rest with the device's name on its rows.
`<checkout> only` keeps the pull requests that checkout's sessions made or worked on, and with none reads `이 체크아웃의 세션이 만든 PR 없음` with `All checkouts`.
The core decides the group, the reason and the order (`herdr-core/src/agent_state/work/board.rs`; the rules are `docs/status-model.md`, Pull request groups); the web draws them, and every pull request is in the first group it fits.

The groups, in order, are `Needs You`, `Agent is working`, `Blocked · no agent`, `Waiting`, `Drafts`, `Bots` and `Merging · merged`; the last four start folded, a group with no row is not drawn, and `Needs You` carries the review asks of a team as one folded line, `팀 리뷰 요청 N`, which opens to rows `리뷰 · <팀>에 요청 · N시간째`.
A pull request nothing has moved on for 7 days folds into `오래된 PR N` inside its group, each row of which shows the date of its last move once opened; the two folded lines are not counted anywhere.
`Merging · merged` puts a pull request in the merge queue first, `머지 큐 · N번째 대기`, then the merged ones newest first, for 14 days or while their worktree is here; a merged row with a worktree has `정리`, which opens the Delete worktree dialog unchanged.

A row is two lines.
The first reads the PR mark with its number, the title (ellipsized, in full in the tooltip), the agents' marks (a teammate's pull request shows its author instead) and the time; the second begins with a word for what to do, in the text colour, and the rest in the group's colour: `머지 · 검사 통과`, `리뷰 · <작성자>가 N시간째 기다림`, `답변 · <에이전트>: <질문 첫 줄>`, `확인 · <에이전트>가 N분째 움직임 없음`, `충돌 · <base>와 충돌`, `변경 요청`, `✗ <검사 이름>`, `일하는 중 · <에이전트> N분째`, `CI · 검사 N개 도는 중`, `리뷰 대기 · <리뷰어> · <기간>`, `머지 큐 · N번째 대기`, `업데이트 · <base>보다 뒤`, `리뷰어 지정 · 필수 리뷰 없음`, `초안 · N일째`, and `확인 중` while GitHub has not computed the merge state.
A blocked row ends `맡은 에이전트 없음`; the words come from GitHub's values and Hide's session facts alone, and turning Hide AI on changes none of them.
A click or Enter opens the card, ⌘-click or ⌘Enter opens GitHub, an agent's mark opens that agent's pane, and the arrow keys move between rows, group headers and folded lines.
Under the pointer or the keyboard the time's slot holds the row's buttons and nothing moves: `▷ 맡기기` on a blocked row, `정리` on a merged one with a worktree, otherwise the GitHub icon and `⋯` with 맡기기, 이슈 잇기, 새 이슈 만들기 and 브랜치 이름 복사.
The list has no search field, so Escape has nothing to clear in it.

Until GitHub has answered the tool shows three skeleton rows and no zero is drawn; with nothing concerning the operator it reads `열린 PR 없음` above the folded `Merging · merged`.
Without `gh` or its sign-in the tool reads one line of why with a `Settings › GitHub` button.
When a read fails the last pull requests stay, PRs carries ⚠ whose tooltip says `GitHub 읽기 실패 · N분 전 값`, and there is no banner.
When the list was read but a question beside it was not (the signed-in account, the review requests, the merge queue; `docs/status-model.md`, Pull request groups, owns what each leaves unknown), the list stays whole and current and the same ⚠ names the question in plain words: `GitHub 읽기 일부 실패 · 못 읽음: 로그인한 계정` (several are joined by commas), and after a failed read the names follow the age (`GitHub 읽기 실패 · 6분 전 값 · 못 읽음: 머지 큐`).
There is no banner or sheet for it, the sidebar's PR marks and the card are unchanged (a mark counts states the list alone decides, so it does not dim), and the failure's own words stay in the diagnostic log (PRD work-tab-pr-lens D-25 is replaced by issue #1189 here).
A pull request that cannot be judged because of such a question sits in the plain Waiting group, never in one it may not belong to, and its reason line reads `판정 못 함 · ` followed by what could not be told (`내 PR인지 알 수 없음`, `내게 리뷰가 요청됐는지 알 수 없음`, `머지 큐에 있는지 알 수 없음`) in the muted colour of `머지 상태 확인 중`; its card keeps GitHub as the primary action and draws Merge only for a pull request known to be the operator's, and it adds nothing to the PRs icon's number.
While the tool or a card shows, every listed open pull request whose checks run, an agent's or not, is read again every 60 seconds; hover, focus and a rest publish nothing.
The tool and a card each hold that one demand, and it is released only when the last of them closes.

#### The PR card

A card opens in the lane's 보기 slot (PRD sidebar-task-lens D-43, D-50): the Workspace in front, its Agent tab and the keyboard stay where they were, and the slot's head reads `←`, `PR #N · <project>`, `↗` (GitHub) and `✕`.
A card opens from a row, a 태스크 row's PR mark, the sidebar's PR card, ⌘K, a session's PR chip, Memory's PR button, a session record's PR and Recent Panels, and `이슈 없는 PR N` opens the PRs tool instead; at Home the pull request's Workspace opens with the card in its lane.
A stored `PR #N` tab from an older build is dropped when the app starts, and the rest of the checkout's Views stay; a pull request on a device this core does not read opens on GitHub.

From the top the card reads (B32-B36):

- the head shows the PR mark, number and lifecycle, an explicit `GitHub에서 보기 ↗` button and `⋯`; the wrapping title follows, with the author on a teammate's PR and the mono `branch → base · 파일 N · +a −d` facts below it; Workspace opens the known checkout, and the links line shows actual linked issues and branch worktrees only; an absent issue draws no placeholder, and `이슈 잇기` remains in the menu;
- a quiet CI and merge section replaces the coloured agent action box: `CI 검사 18개 중 3개 진행 중`, followed by the real nonzero counts for passed, running, waiting, failed, skipped and unknown checks; missing counts say they are not confirmed, never an invented total; `전체 검사 보기 ↗` opens this PR's checks, and review/conflict facts, failed-check logs and unresolved threads retain their existing actions;
- `타임라인 · 세션 N`, a small quiet head (D-03, D-05, B3-B6; PRD record-card-readability D-01, D-09, B16), the count left out while no session worked on it: the sessions that made the pull request or worked on its branch and the pull request's own milestones, one list oldest first, sessions by their start and milestones by their time, joined by one vertical line behind their marks, every time in the right column in mono, a row with no time leaving that place empty, and with no axis or bar; the milestones are only what happened, `PR 열림`, `검사 통과` or `검사 실패` (the latest completed run, read from GitHub's check runs, D-15), `검토 요청 · <누구에게>`, `승인` or `변경 요청`, `머지됨`, each a small square in its colour where a session row has its mark (green for a pass or an approval, red for a failure or changes, yellow for a request, purple for the merge), its word in normal weight and its time at the right, a check or review row whose time could not be read standing right after `PR 열림` with none; an open pull request ends on one line, `지금 · 검사 N개 도는 중` with the dashed ring of proven work it waits on while checks run, and `지금` with an empty slot otherwise (status-marks D-06, D-25; PRD record-card-readability D-12, B21); there is no `작업 → PR → 검토 → 머지` stage bar; the session rows are below;
- `바뀐 파일 N`, one folded line with +/- at the right that opens in place and stays as it was left while the app runs, to the files with files `.gitattributes` marks `linguist-generated` and the common lock files gathered into `생성 파일 N개 접음` (B11);
- `본문`, never folded (D-02, B12, B13): a row of the body's `##` headings that sticks to the top of the card while the body scrolls under it, a chip scrolling to its heading and the chip of the heading last passed pressed, then the whole body as Markdown, with the HTML GitHub draws in a body drawn too (PRD record-card-readability D-11, B19, B20): `<details>` and `<summary>` as a folded box, `<img>` and Markdown's `![]()` as the picture when its address is https on a public host, because the card loads it by itself as it opens and a body must not make the operator's machine ask this computer or its network for anything (its description where it cannot be read), `<br>`, `<p>`, `<b>`/`<strong>`, `<i>`/`<em>`, `<code>`, `<kbd>`, `<sub>`, `<sup>`, and `<a>` and Markdown's links to an http or https address as links that open in the browser; any other tag, an attribute outside that list (an `on…` handler among them) and a `javascript:` or relative address stay as the text they are, nothing runs, an opening tag past 32 open elements stays as its text, and an HTML comment is not drawn; a body with no heading has no row, an empty one reads `본문 없음`, and one that could not be read says `읽지 못함 · 다시 시도` for that part alone.

There is no AI summary, no four-step stage strip, no `만든 세션` line, no folded `연결` line and no body preview (B22).
No part of the body is lifted out as the card's own judgment.
A changed file opens as the Hide diff in the checkout's Files when the Workspace in front is a checkout of the pull request's branch and its committed changes already hold the file, and the lane turns to Files while the 보기 slot stays on the rail and brings the card back as it was (B49); otherwise it opens the pull request's files page on GitHub.
A part that could not be read keeps the head, the decision box and the milestones from the list, and says `읽지 못함 · 다시 시도` on that part (the folded files line says `읽지 못함`), which reads again only that part; a check or review time that could not be read leaves its row timeless, and the reason goes to the diagnostic log (B21).

The merge section's button is the case's: `Merge` when the pull request can merge, in a method the repository allows (a choice when it allows several) or `머지 큐에 넣기` in a repository with a merge queue; `브랜치 업데이트` when it is behind; `GitHub에서 리뷰` and `에이전트에게 리뷰 맡기기` for a review asked of the operator; `맡기기` for a blocked pull request with no agent; `리뷰어 지정` opens the pull request on GitHub; agent terminal actions belong to the timeline's session row, and GitHub stays visible in the card's header.
Merge and `브랜치 업데이트` are the app's only GitHub writes besides a pull request's body, a new issue and the close `되돌리기` makes, and appear for the operator's own pull requests.
A Merge that cannot run is disabled with its reason beside it (`검사 13개 도는 중`, `에이전트가 일하는 중`, `main과 충돌`, `필수 리뷰 없음`).
Pressing it asks once, `#N을 <base>에 머지 · <방식> · <짧은 커밋>` with `#N 머지` and `취소` (focus starts on `취소`), and the write goes out only for the head commit the card showed and the list judged: the confirmation closes by itself when the card's head, method or queue moved, or Merge became unavailable, and the core refuses a write whose head differs from the head the list was read at, so a push between the two reads is never merged unseen.
When the pull request moved meanwhile nothing is written and the card says `PR이 바뀌었습니다 · 다시 보기`, whose button reads the list and the card again and clears the notice.
Why Merge is unavailable names the real cause: checks running, checks failed, changes requested, no required review, conflict, draft, an agent working, no permission.
A merged pull request's card reads and draws like any other, with no Merge.
A merged pull request moves to `Merging · merged`; asking again for one already merged or queued writes nothing a second time.
A branch update asks the same way and takes the same head condition; a conflict turns the second line into `충돌 · <base>와 충돌`.
A failure leaves one line beside the button that says what the operator can do (`권한 없음 · GitHub에서 확인`, `GitHub 응답 없음 · 다시 시도`), the same button tries again, and the detail goes to the diagnostic log.
`에이전트에게 리뷰 맡기기` starts an agent on the branch's checkout (a new worktree when there is none) with a review instruction; it reports to the operator and neither reviews on GitHub nor merges.

Hide AI makes no summary of a pull request; the `PR 요약` feature, its Settings › Hide AI › Features item, its usage accounting and its stored summaries are gone, and a value an older build left in the settings file is read past (PRD pr-card-and-session-record-redesign D-19, B23).

The links line and the timeline's session rows are the link record (PRD link-graph), read when the card opens:
An issue on the links line opens that item in the Overview's 백로그, or, once it is started, its task in the sidebar's 태스크 list, or GitHub for one the project does not list; a worktree opens its Workspace.
What is true now (the pull request, CI, the review, whether a worktree is still here, whether a pane is live) is the snapshot's; the links and the sessions are the link record's.
`세션 N` counts the sessions that made the pull request or worked on its branch while it lived, from the record.
A session row is the sidebar's agent row shape (D-07, B7-B9; PRD record-card-readability D-09, B17): the status mark of its live pane, or an empty slot once it ended, its caption saying 끝남 (status-marks D-06), then the agent's logo; the title (the live agent's label, else the record's title as the Sessions list shows it, else the agent's product name); the `PR 만듦` chip on the session that made the pull request, only when the card lists two or more sessions; and at the right the time, `10/16 15:36 – 20:43` for an ended session and the time since its start for a live one.
Under the title an ended row reads `<agent> · 끝남`, a live one `<agent> · <state word>`, and an asking one its question in the warning colour; a live row keeps its one button under that line, `답하러 가기` (the card's only accent button) or `pane으로 가기`, and an idle live row reveals `대화 보기` and `pane으로 가기` instead.
The row's tooltip holds what the line leaves out: the request (for the session that made the pull request, the request just before its address was printed; otherwise the last request on the branch; `요청 없음` without one), `<부모>가 맡김` on a delegated session, `이어짐 N` for sessions joined by compaction or resume, and the device's name for a session on a connected device.
While the pointer or the keyboard is on an ended row, `대화 보기` and `이어서 하기` take the time's place (B9): `대화 보기` opens that session's record (in the lane while a Workspace is in front, otherwise in the Overview's panel), and `이어서 하기` starts `claude --resume <id>` or `codex resume <id>` in a new tab of the worktree it worked in and moves the screen there; a failed start leaves one line of why and `다시 시도` on the row.
An off button stays reachable and its tooltip says why: the worktree was cleaned up, the device is not connected, another device's conversation cannot be opened here yet (PRD link-graph D-41), the agent's sessions cannot resume (D-42), or the file is gone; a row whose file is gone is dim with `대화 파일 없음` and copies the path the file had.
Six sessions or more fold the oldest ended ones past five behind `이전 세션 N개` at the top of the timeline, a live session never and a milestone never (B10); with none the timeline reads `이 PR에서 일한 세션이 없어요` with `▷ 맡기기` above the milestones.
While the record fills for the first time or the panel reads, a small spinner line turns at the top of the timeline and the rows read before stay; a record that cannot be read puts one line of why and `다시 시도` there, and the rest of the card stands.
A pull request the list does not hold (an old one a session's chip asked for) is drawn from the record; while that read runs or after it failed the card is only `#N`, × and the timeline's sessions with its spinner or its failure, and it closes only once this open's read says the record has no such pull request.

이슈 잇기 opens the project source's open issues, searchable, with `새 이슈 만들기` last; closed issues and another repository's are not in it.
A GitHub issue asks once, `PR #N 본문에 "Closes #M"을 씁니다. 머지되면 GitHub가 이슈를 닫습니다.`, with `그만두기` first and `본문에 쓰기`; confirmed, Hide links the branch to the issue (its issue link and the Workspace token, when the branch has a worktree here) and writes the line at the end of the body after a blank line, reading the body just before, and the panel's issue line fills.
When the body already closes that issue by a closing keyword, nothing is written and the link succeeds, so a retry never adds a second line.
A failed body write keeps the link and the filled line; the dialog stays with one line of why and `본문 다시 쓰기`, which writes only the body, and `닫기`.
A Local issue is Hide's link alone: choosing it asks nothing and writes nothing to GitHub, and the dialog appears only when the link fails.
`새 이슈 만들기` opens the pull request's title and body (read when it opens) in editable fields and one confirmation, `이슈를 만들고 PR #N 본문에 "Closes #(새 번호)"를 씁니다`, `만들고 쓰기` then makes the issue and writes the body; when the body write fails the issue stays made and linked, the dialog says `이슈 #M은 만들었고 PR 본문 쓰기는 실패했습니다`, and `본문 다시 쓰기` writes only the body.
On a Local source the same form says `Local 이슈를 만들어 이 PR에 잇습니다. GitHub에는 쓰지 않습니다.`, and `만들고 잇기` makes a Local issue and links it, with nothing written to GitHub.

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
Dropping on a folder row puts them in that folder, on the root row or on the empty room under the rows puts them at the root, and the receiving row is highlighted as a drag inside the tree highlights its folder; a file row accepts nothing.
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
Dropping on a folder puts the item inside it, on a file puts it beside that file, and on the empty area puts it at the root; the receiving folder row is what highlights.
The same parent, the item itself, and a folder inside the item show no drop indicator and accept nothing.
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
The sidebar keeps these sizes at every Appearance font size, because its rows are scanned, not read; the font size scales the rest of the interface.
The sidebar's right edge sets the width of its content column, the lists right of the device rail while one shows (see Device rail): under the pointer it shows a line and a column cursor, a drag moves the sidebar over the center between 220 and 440 and stops at either bound, and the release stores the width in the core's ui state, where it survives a reload and a relaunch; the center and its terminals take the new width once, on the release, never on each pointer move.
A double-click on the edge returns the width to 292, and a press that does not move changes nothing.
A width outside the bounds is refused into the diagnostic log: a sent one leaves the width as it was, and a stored one opens the sidebar at 292.

Sticky section markers update at most once per animation frame, with a pending frame cancelled when the list leaves.
Focus visibility uses current header geometry, so a row under a header is revealed even before its marker updates; a fully visible occurrence leaves scrolling unchanged.

### Sidebar hierarchy

This is the 프로젝트 list; the `프로젝트 | 태스크` switch above it, ⇧⌘B, and the 태스크 list are in [The sidebar's 태스크 list](#the-sidebars-태스크-list) (PRD sidebar-task-lens B1-B3).
The sidebar hierarchy is Project > Workspace > Group > Tab > Agents; a Workspace corresponds to one checkout path, including a plain folder.
Two checkouts of one repository share a project cycle.
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
An agent row's menu, in Agents and under an opened checkout, is `Show` (the row's own open, with the ⌥n that selects the same row where the host has one), then `Copy details` with `Session title`, `Session ID` (the conversation id Herdr recorded, disabled when it recorded none) and `Terminal ID`, then `Close tab…`, which closes the tab holding the agent's pane, wherever it is, through the tab close flow, including its question about the agents spawned from it; Herdr 0.9.1 can neither mark a pane seen nor stop an agent, so neither is offered.
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
Every tab draws its guide and tab line, whatever it holds or is named, in both ungrouped and named collections: an unnamed tab with one agent reads `Tab N, 1 agent`, and a terminal-only or operator-named empty tab keeps its line with its name alone.
A resolved session whose pane is still live stays a row under its tab, drawn dimmed whole like an unknown row with its mark and line from its own state and `해결됨 · ` leading its hover (PRD sidebar-every-tab-line B3); a live tab is never left as an empty tab line, and a closed pane's resolved session draws no row; a linked worktree whose only agents are resolved stays in the `No agents` fold, and shows the dimmed row once the fold is opened.
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
A working ring says its sentence alone or has no line; a waiting ring has a line in every list, the project tree included, led by `대기 · ` and its sentence or the proven cause (`대기 · CI · PR #12`, `대기 · 자식 2명 작업 중`, `대기 · 백그라운드 작업`, `대기 · 답장`), so the lead word tells working from waiting (status-marks B10, B29).
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
In the desktop app an `Add project` row sits directly under the Overview row, drawn whether or not a project is registered, with the current chord (`⇧⌘N`, `Alt+Shift+N` off macOS) on its right and no + on the section titles; it is the sidebar's one Add project (PRD first-run-onboarding B51, B52).
The sidebar has no Home row (PRD sidebar-titlebar-refresh D-06, D-07): a Home agent is found and opened from Needs You and Done, the 태스크 list and ⌘K, the ⌘N start panel can target the Home, and ⌥n numbers the visible rows without it.
Every device's sidebar starts with the Search row and the shared Overview row, under the device title in the window's top row, followed by one project/checkout/root list with no mode strip.
Overview shows its glyph, label, Needs You count and current shortcut; zero, an absent snapshot or a disconnected device hides the count.
The top row's Overview icon has an accessible Overview name and count description, a Needs You dot above zero and a tooltip with its current shortcut.
The top row's icons are Overview and Open server; the Files badge is on the rail.
⌘⇧P shows and focuses the sidebar, closing Overview first.
Retired `toggle_sidebar_view`, `project_home` and `sidebar_agents` (the removed Agents overview) stored bindings are ignored without losing other valid bindings, with a diagnostic.

A device that is not connected shows the top row and its reconnect view with no tab strip.
A device's Workspace that lost its connection keeps the last state on screen under one line, `<device> is not connected. Showing the last state it reported; nothing is sent until it reconnects.`, with `Reconnect`; the line names a refused host key or sign-in by its class (`Host key changed`, `Sign-in refused`) with what to do in its tooltip, and never carries the words the connection failed with, which are the device row's in Settings and the diagnostic log's.
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
No agents folds worktrees with no unresolved agent, excluding primary, front, dirty, unpushed and already inactive checkouts and one holding a named tab the operator kept ([Agent areas](#agent-areas)); a worktree whose unresolved agents are all delegated children is in no fold and has no line (`session_folds.child_only`, same exclusions), because No agents would be false for it; Cleanup gathers agent worktrees and missing folders at the bottom without assuming they have no live pane.

### Purpose, pinning, and PR chrome

A checkout's one-line purpose is set from the checkout row's or Overview header's `Set purpose…` context item, with a character count and a warning near the limit; saving an empty purpose clears it, and display falls back through branch description, representative agent title, and pull-request title in that order.
Any project row can be pinned from its right-click menu: a row Herdr shows without a registration is registered with its device and root by the same Pin, so pinning never needs a separate add; pinned projects are drawn once under a `Pinned · N` section, its header led by a pin glyph and its rows carrying none, between the raised groups and the activity-ordered project list, one hairline setting each section off from the one above, the first from the raised groups when there are any, in the tree's own order (device first, then latest activity), and only while at least one project is pinned.
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

The rail's tools are Memory, PRs, Files (the View areas and the Explorer) and Changes (History), with the 보기 slot above them while it holds something; the device's task list is the sidebar's, described in [The sidebar's 태스크 list](#the-sidebars-태스크-list).
The top row's Overview button opens [Overview](#overview), `Work · Projects · Backlog · History · Usage`, with device and optional project scope for the four device views and global core-machine scope for Usage.


### Disk allocation and cleanup

Web owner: `web/src/DiskCleanupSheet.tsx`, `web/src/diskCleanup.ts`; core owner: `herdr-core/src/disk.rs`, `herdr-core/src/worktree_cleanup.rs`; node owner: `hide-host/src/disk.rs`, `hide-host/src/disk_layers.rs`, `hide-host/src/cleanup.rs`.
Allocated-on-disk sums main, linked worktree folders, and the shared Git directory once; nested roots belong to the longest matching root, hard links share one inode allocation, and descendant symlinks are not followed.
Each checkout is measured under its own limit of one million entries and 30 seconds, so a checkout that exceeds it or cannot be read is the only row that has no size; the reason is in the diagnostic log.
An incomplete measurement has no total, and allocated blocks are not a promise of reclaimable space, so the result states only the volume's free space before and after.
A node's project is measured and cleaned up on that node; a project of a device the core dials is neither measured nor cleaned.
The Overview's [Projects](#projects) tab is where a project's size shows: while it is on screen the core measures its projects one at a time in the order shown and keeps each answer for ten minutes, and its disk number, `N merged → 정리` and the project panel's `정리` open the sheet below.

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
The default branch is the base branch chosen in Project Settings, else the one GitHub names, else what `origin/HEAD` names (PRD git-freshness D-07), never a branch the code assumes.
Merged is the one judgment the sidebar's landed mark also reads (`worktrees::merged` in `herdr-core/src/worktrees.rs`, D-06): HEAD is in the local default branch or in its `origin/` copy as of the last fetch (the review never fetches; the background fetch keeps that copy current), or a pull request GitHub merged into the default branch has exactly this HEAD as its head; a squash or rebase merge needs that last proof, and it needs no merge commit in a local ref.
GitHub's answer is the pull request list the project's read already holds, the last one kept while a read fails; that list holds only the 200 newest pull requests, so the review asks GitHub by branch for a HEAD the list does not name, and a worktree whose lookup fails is unverified rather than not merged, and a device's project, whose pull requests this core does not read, always asks by branch; with no GitHub answer only the ancestry counts.
A head the review proved by branch is kept by the core for that project and default branch (at most 1024, `review_merge_proofs`) and joined to the list the sidebar's landed mark reads, so the row is marked landed when the review offers it (B17); before any review the sidebar knows only the list.
A HEAD that moved past its merged pull request's head is not merged, a pull request merged into another branch proves nothing until that branch is in the default branch, and with no known default branch, or neither it nor its `origin/` copy present, the sheet judges no worktree.
Deletion still happens only when the operator confirms the reviewed rows; the Hide AI attempt retirement keeps its own rule (GitHub answering that attempt's pull request merged or closed).
When Hide cannot read what is in use, the sheet says so above the table with a retry and every checkbox is disabled; no banner or alert appears.

The bottom line reads `N칸 · X` (`N칸 · 워크트리 M · X` when a worktree is chosen, and a part that is zero is left out, so a worktree alone reads `워크트리 M · X`) and `정리` runs at once for caches and dependencies alone.
A chosen worktree whose finished agents and shells still have panes shows a terminal mark with their count beside its size, in the cell and again in the confirmation, which is the state shown instead of a sentence about it.
With a worktree chosen a confirmation step titled `<branch> 폴더째 삭제` (`워크트리 N개 폴더째 삭제` for more than one) says only that the branch stays, lists the worktrees with their sizes and pane counts and offers `워크트리 N개와 캐시 정리` (`pane P개 닫고 워크트리 N개와 캐시 정리` when panes close, the same consequence-first naming as Delete worktree's `Close N panes and delete`) and `돌아가기`, neither focused; going back deletes nothing and keeps the choice.
Cleanup deletes cache and dependency folders permanently, with no trash, and removes worktrees without force, keeping the branch.
Each cell is checked again when `정리` is pressed and again per folder: a cell that became in use, a worktree that changed and a folder that gained tracked files are skipped with their reason and the rest go on.
A worktree's panes are listed again just before its own removal and closed through the same Herdr close and confirmation Delete worktree uses, then the folder is removed without force; a pane that does not close leaves the worktree in place as a failed row (`pane을 닫지 못함`) and the other rows go on.
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
Only the desktop app offers it, because the native folder picker is how a local project is added: a plain browser tab draws none of those entry points and has no chord for it.
The Host selector lists this Mac and every registered device, and starts on the focused device.
This Mac here is the machine the window runs on, which on a window of a machine whose core runs elsewhere is not the core's (`hostWays` in `web/src/addProject.ts`).
On this Mac the primary `Browse folder` row holds the keyboard, so Enter activates it; it opens macOS's own folder picker as a sheet on the window, which can also make a new folder, and a cancelled pick leaves the dialog as it was.
The chosen folder is registered with one `create_workspace`; the dialog closes when the project appears, and says `Adding <folder>…` until then.
A refusal stays inside the dialog as an alert naming the folder, and the dialog stays open for another pick: `already_registered` from the shell's own registrations, before anything is sent, and `outside_home`, `home_root`, `not_found`, `not_a_directory` or `invalid_path` from hided's `$HOME` line, or the core's own error.
Another machine's folders are not this window's to browse, so on a device, the core's machine seen from a node's window included, the dialog shows a `~/` path field instead, sent to that machine's node through the same event, with its refusal shown the same way.
On the core's machine, the one a new project can be made on, an `Other ways to add` group sits under Browse folder or the path field, one row per way, each opening its own view of the same dialog with a `Back` that returns to the first view and puts the keyboard on the row it came from; on a node's window it sits under the core's machine and its views offer no folder picker, since the picker would show this machine's folders.
`Clone from URL` (web owner: `web/src/CloneFromUrl.tsx`) shows a `Git URL` field that holds the keyboard, and a `Parent folder` field with a button that opens the same native folder picker; the parent starts in the folder most local projects live in (the latest added wins a tie), else at home (`~`); Create new project starts there too.
The folder the URL names is shown as `Clones into a new folder <name>`, and hided is asked whether it is free under that parent, so `Clone` is enabled only for an https, ssh (`git@host:path`, `ssh://`) or `file://` URL, a parent inside home, and a name nothing holds there yet (`already_exists` covers even an empty folder, since a clone never writes into a folder it did not make); each refusal is shown inline under its field.
While the clone runs the dialog shows Git's stage and percent with a progress bar and a `Cancel` button, and the fields and `Back` wait; closing the dialog leaves the clone running, and opening it again shows it.
A finished clone is registered like a picked folder and the dialog closes when the project appears, or says it could not be added and that the repository is on disk for Browse folder; a failure (authentication, a stalled transfer, a missing repository) stays inline in plain words with `Clone` enabled again, and a cancel says nothing was kept.
`Create new project` (web owner: `web/src/CreateProjectView.tsx`) shows a focused `Name` field and a location row naming the folder it goes in (`Git repository in ~/…`) with the full path under it, which follows the name as it is typed.
The folder starts as the parent of the most recently added project on this Mac, else home; the location row opens the native folder picker to choose another.
hided answers a `project_target` probe for every name and folder on the same `$HOME` line the create is checked on, so a name that is not one folder, a folder outside home, something already standing at the path (`already_exists`) or a project already added there is said inline and `Create project` stays disabled until the path is free.
`Create project` (or Enter) sends one `create_workspace` with `new_folder`; the core makes the folder, runs `git init` in it and registers it, and the dialog closes when the project appears or keeps the failure with its reason.
A folder holding nothing, or only `.git` and a Finder `.DS_Store`, is what a create that failed after making its folder leaves, so it is not `already_exists`: the view says the project is made in it, and a retry continues there rather than being refused or making a second folder; a failed create never deletes the folder it made.

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


Core owner: `herdr-core/src/runtime/first_run.rs`, `herdr-core/src/runtime/kit.rs` (`apply_agent_onboarding`). Web owner: `web/src/FirstRunWizard.tsx`, `web/src/firstRun.ts`, `web/src/NotificationControls.tsx`, `web/src/agentNotices.ts`, `web/src/ScreenIntro.tsx`. Desktop owner: `desktop/src/main/notify.ts`, `desktop/src/main/notificationPermission.ts`, `desktop/src/main/host.ts` (`showBadge`, `openNotification`), `desktop/static/status.html`.

The desktop app shows `Starting Hide…` while it reaches Hide, and never the word hided; a failure shows its reason in one line with Retry and `Copy diagnostics`, which copies the app's build, the reason code and where the host log is, and shows the same text to select by hand when the clipboard refuses it (PRD first-run-onboarding B1, B2).
On a machine whose kit has never run, the first-run wizard opens over the shell: Agents, saved Herdr Devices when present, Notifications and the first Project.
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
Devices reads only the saved Herdr machine configuration, without probing SSH or contacting a listed machine.
Loading, a failed read with Retry, a proven empty result, disabled or invalid profiles and already-added machines are distinct states; only the proven empty result skips Devices.
Every profile starts unchecked and names its label, SSH target and Herdr session, including when these need to wrap.
Checking a device changes the draft alone; Connect selected explicitly allows Hide's helper, command and selected agent integrations on those checked devices through the existing Add device connector and install owner.
Existing Herdr sessions, tabs and workspaces stay in place.
Connecting and installing hold the Connect control; per-device failures remain available for explicit Retry, while successful machines remain added and are not connected again by Back or re-entry.
Not now continues with this machine, and failed discovery can be skipped without treating it as proof that no profiles exist.
Project reuses Add project's host selector and folder, clone and create views; adding a remote folder offers the enabled agents from that remote machine's kit and never borrows this machine's sign-in state.
Its What changes opens a list with one row per place Hide writes, the path in mono with the agents it is for and the piece (`Codex, OpenCode, Pi · Skill`), so a skill folder agents share is one row, and the `hide` command's link last (B11).
A chosen agent that did not go in keeps the step with `<agent> was not set up: <reason>` and Try again (B12).
A pass that could not run (the kit link failed, or the pass listed no agent) keeps the step, says `Hide could not set up agents on this Mac:` with the reason, leaves the choice unanswered, and reads the agents again; the step says the same whenever the kit could not run here, as after a relaunch, and never that no agent is installed; `Try again` then records nothing and only has the agents read again, and once they are back Continue sends the choice anew; the walk Help reopens does the same, staying on the agents step with that line and no banner (B12, B44).
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
The project step is Add a project under `Add your first project`, with its Host select, Browse folder, Clone from URL and Create new project, its refusals and its clone progress, and `Skip for now` (B22, B25); its Host list has no Add device, which would take the wizard's place, and only a project on the chosen host counts as the one added.
The first project added ends the wizard and opens the start panel once, on that project, with the first agent that is on and signed in (in the order Claude Code, Codex, Grok, Pi), else the first agent that is on, and the keyboard in its text box; with no agent on it opens no panel and the wizard closes to the empty Home, whose Start an agent is the way on; a later project opens nothing (D-15, B23, B24).
For a target on the core's own machine, the start panel says `! Not signed in` right after its agent menu whenever that agent is one whose sign-in Hide checks and its last read was signed out (B23, D-06).
A remote target never uses the core machine's login result; its kit reports installation and enablement, without claiming remote sign-in (B37).
A browser tab shows the agents step alone, with no progress bar, and Continue there ends the wizard; the notifications and project steps are the desktop app's (D-14, B26).
The desktop app's Help > Set up Hide opens the same steps from the first as this window's own walk: the agents step starts from the agents switched on now and its Continue switches only the agents whose switch changed, the way Settings > Agents does; Escape and the close button end it, and a step not continued changes nothing (B45, B46).

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

## Device rail

Web owner: `web/src/components/device-rail.tsx`, `web/src/devices.ts`, the rail branch of `web/src/sidebar.tsx`.

This section supersedes PRD home-device-rail rows D-10, D-11, D-27, B3, B4 and B12 (quick device-rail-badges): the Inbox, the rail that came and went with the first remote device, the Needs You-only badge and the footer device button no longer exist.
Quick device-rail-slack then narrowed the rail to `--size-rail` (48), ran it the sidebar's full height, took the names off the tiles and left Working to the session rows.

This Mac is this document's name for the machine the core runs on; the screen names it as its system does, This Mac on macOS, This PC on Windows and This computer on Linux, in the interface language (`common.thisMac`, `common.thisPc`, `common.thisComputer`: `이 Mac`, `내 PC`, `이 컴퓨터` in Korean).
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
In the desktop app ⌘1-9 (Ctrl+Shift+1-9 on Windows and Linux) selects the tile at that number, top to bottom with This Mac first, whether or not the rail shows, and holding ⌘ alone floats each tile's number at its top left (see Keycaps, tooltips, and icon buttons); a number with no tile does nothing.
Every tile is a button reached with Tab and chosen with Enter or Space, named for assistive technology by the device, its connection and each count it marks (`mini, 연결 안 됨`, `This Mac, Needs You 2, Done 1`).
`기기 추가` opens Settings › Devices with the Add device dialog in front, as the hidden rail's `기기 추가…` and the Add a project dialog's host list do.
Removing the device in front moves the front to This Mac; that device's agents and its `~/hide` stay on it, and the rail stays.

A right-click on the rail offers `레일 숨기기`.
While the rail is hidden the top row's device name becomes a `This Mac ⌄` menu that lists the devices (a check on the one in front), `기기 추가…` and `레일 표시`.
The View menu of the desktop app carries `Toggle device rail` (`toggle_device_rail`, no default chord, bindable in Settings › Shortcuts like the sidebar switch); a browser tab uses the `This Mac ⌄` menu.
The hidden state is the core's `ui_state.device_rail_visible`, kept beside `left_sidebar_visible`, so it survives a restart.

A device's Workspace in front wears the device color, `--device-remote`: a band at the start of the top row's Workspace path with the server glyph and the device's name, truncated, and a border of the same color around its panes; this machine's Workspace has neither.
Which machine is a device is the window's to say (`awayDevice` in `web/src/screenMachine.ts`): on a window of a machine whose core runs elsewhere, its own Workspaces wear neither and the core's machine's wear both.

A machine whose core runs on another machine (ARCHITECTURE.md, A core on another machine) is a device on that core's rail like any other, named by its host name.
Its `다시 연결` is answered that the machine reconnects by itself, and removing it ends its connection until it attaches again; neither adds a state or a line of its own.
The window's own machine is always the first tile and the one named This Mac (`이 Mac`), the laptop glyph (`web/src/screenMachine.ts`, `windowFirst`); on a window of a machine whose core runs elsewhere, the core's machine follows it, named by the name the operator registered it under before the core moved to it, which the core keeps (`machine_label`, ARCHITECTURE.md, Moving the core), and by the name its machine gives itself when it has none.
The tile of the machine that runs the core wears a crown notched into its bottom-left corner (`--size-rail-crown` in a `--size-rail-badge` notch, the corner the `×` and the marks leave free), and its hint adds `core 실행 중`; the crown shows only once a machine dials in to a core, that is on a node's window or while any device dials in, since with one core and its devices nothing needs telling apart (PRD core-host-node-move B2).

A window on a machine whose core runs elsewhere shows its core's state and takes its strip, in the place and size of the connection badge, when the core cannot draw it (W1 to W3, B16):

- while a move holds the window, `core를 Mac mini로 옮기는 중 · 3/5 프로젝트와 기록 복사 · 다시 연결될 때까지 입력을 받지 않아요`, the move's step of five, with `…`, and the window sends nothing but the move's own give-up (a rollback says `옮기지 못해 이 Mac의 core로 되돌리는 중`); while the move waits on a machine that took its link and stopped answering, the strip ends with `그만 기다리기` (`여기로 가져오기` for a move back), which only opens the move dialog at its confirm;
- while this machine updates its core to this app's build, `Mac mini의 core를 이 앱의 빌드로 바꾸는 중 · 에이전트는 계속 돌아요`, with `…`;
- while the core cannot be reached, `! Mac mini의 core에 연결할 수 없어요 · 다시 연결하는 중`;
- after the operator ended the link here, `! Mac mini core와 연결을 끊었어요` with `다시 연결` at its end, which links it again and is the only thing the window sends while it waits;
- otherwise the shell's own connecting state.

The window takes no input while its strip shows, and sends none of what was typed meanwhile once the core is back; the strip goes when the core's snapshot arrives, and a move that ends on another machine rewrites the window's address to it without a reload (`addressAfterMove`).

## Start panel

Web owner: `web/src/StartPanel.tsx`, `web/src/startTargets.ts`, `web/src/startDraft.ts`, `web/src/startAnswer.ts`, `web/src/agentPicker.ts`, `web/src/components/agent-picker.tsx`.

The start panel starts an agent supported by the shared adapter with a first instruction anywhere hide can reach (PRD home-device-rail D-17..D-22).
It remains independent of the inline group/draft start: a successful start makes its own tab in the target checkout's selected group rather than consuming the draft whose form happens to be open.
⌘N opens it in the desktop app, where it is also File › Start agent; in a browser tab ⌘N stays the browser's and `에이전트 시작…` in ⌘K, found by typing, opens it, on every screen.
It floats at the ⌘P palette's place and width with no backdrop, and the keyboard lands in its one-line text box, `무엇을 시킬까요?`.
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
Settings > Agents > Starting work has no default agent of its own.

## Usage

Web owner: `web/src/components/usage.tsx`, `web/src/components/usage-history.tsx`, `web/src/components/usage-history-table.tsx`, `web/src/components/usage-amount.tsx`, `web/src/UsageOverview.tsx`, `web/src/usageHistory.ts`, `web/src/usage.ts`, `web/src/settings/UsageTab.tsx`.
The core owns account windows and shared preferences (`usage`, `ui_state.usage`); all limits refer to the core's own node, even with a remote device in front.
The declaration lists all seven agents; only agents with a declared limit source appear in the footer and popover.

The footer draws up to three pinned account windows at its left edge, and the View menu, Help and Settings at its right edge.
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
The header's settings icon, named `Choose chips`, opens Settings > Usage.
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

The footer's `?` opens the Help menu above it: `Keyboard shortcuts` with its chord, which opens the shortcut sheet; `Copy diagnostics`, which puts About's diagnostics text on the clipboard and says `Copied` in place, or the failure when the clipboard refuses, keeping the menu open either way; `Report an issue`, which opens the repository's new-issue page in the default browser; then, under a separator, the `hide <version>` line in mono, which takes no press.
The version is the release the app was built as, such as `0.4.0`, and a development build's own version, such as `0.4.0-main.<commit>`.
When the desktop app has an update ready ([INSTALL.md](INSTALL.md#update)), a dot in the primary color sits on the `?`, whose name and tooltip become `Help · Update ready`, and the last line becomes a menu item, `hide <new version> ready` with `Restart` at its end, which installs the update and reopens the app.
Nothing else of an update reaches the footer or this menu: looking, downloading and a failure show only in Settings > About.
A browser tab and the phone have no dot and no update line.

## Search keyboard navigation

Web owner: `web/src/SearchPalette.tsx` (⌘K), `web/src/search.ts`, `web/src/relations.ts`, `web/src/searchDetail.ts`, `web/src/searchGithub.ts`, `web/src/Palette.tsx` (⌘P and the diff palette), `web/src/components/sidebar-header.tsx`; core owner for the GitHub search: `herdr-core/src/runtime/issues.rs`.

⌘K goes to things; it runs no command except `에이전트 시작…` and `브라우저에서 URL 열기`.
On the web, the sidebar's Search row, which shows `Search` and the bound chord, opens the same palette Command+K opens, and the query row carries an `Esc` keycap.
Its own wide layout is a list on the left and the highlighted row's detail on the right, one and a half times the width ⌘P, the shortcut sheet and the start panel share; a dialog under 800 px, too narrow for both, draws the list alone.

### What a query finds

A nonempty query also searches saved session goals, assigned task names and original requests in a separate `세션` group, including ended sessions within Copied history.
Enter on that group opens the record in Tools; the existing Agents group still opens the live agent.
With summaries Off it searches saved raw requests only, without generated goal or task matches.
The core's bounded local record search runs independently of other groups, drops an answer for an older query and makes no GitHub request while typing.
The session group carries its current loading or failure state; a successful search with no session matches draws no group, preserving the palette's existing no-match state when no other kind matches.
The creator-session action in an output panel opens the same record.

A query finds an agent, project, checkout, device, issue or pull request by name, and an issue or pull request by `#number` (`#273`, or `273`); an exact number match lists first, as its own row, issue and pull request separately.
After them come the agents the core links to a pull request of that number (its session made it or its branch has it; every one the core links, a merged one included, which is more than the agent's PR mark draws), in `Agents` on every device, so `#275` reaches the agent that works on pull request 275 though no word of its row holds the number; an issue's number brings no agent.
An agent is found by its title, by the name Herdr knows it by (`hide agent spawn --name`, `herdr agent rename`), by its place and by its state sentence; the core carries the Herdr name on the agent row (`herdr_name`), on this machine and on a device alike, and leaves it out when Herdr has none or has only a name Hide made up from a pane id (a task's `hide-claude-w9j-p52`, a wake's or a fork's), which would print the id the rule below keeps off every row.
A query holding `:`, as every pane's Herdr id does (`w9J:p52`, as Copy pane ID or `$HERDR_PANE_ID` gives it), lists first the agents whose id holds it in its case, on every device, the whole id before longer ones (`:p5` lists `w9J:p5`, then `w9J:p52`); without a `:` no id matches, and no row prints the id.
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
A query with a GitHub project the core reads (its own or a node's that dials it) ends with `GitHub에서 "…" 검색`; choosing it sends one `github_search`, and the row shows a spinner, then the pull requests and issues GitHub holds under `GitHub` (an exact match to one already held is not listed twice), `GitHub에도 없음` (or `찾은 결과 없음 · 일부 저장소만 검색` when the core covered only some of the projects), or `GitHub 검색 실패 · 다시 시도`.
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
Click, Enter or Space opens its existing popover; Esc closes it and returns focus to the icon without opening another hint over the popover.
A Codex on the shared server reads "Hide can't follow this Codex" with its reason, Reopen with Hide, Not now, and the existing confirmed shared-server action; a session with no hook report reads "Reconnect this session to Hide", says chatting can continue and offers Reopen and Not now without claiming when the hook was installed; a setup problem names the cause and offers no Reopen, since a Reopen would change nothing.
Reopen is hidden where the core says the pane cannot be reopened (a pane on another device).
Not now closes the popover only and the chip stays; Settings has no Reopen button.
Reopen sends one `pane_reopen` however often it is pressed before the core answers, shows the core's pending state as a busy button, and on a refusal known before the agent was touched leaves one line from the refusal code (the agent is working, no saved conversation, the folder is gone, the agent would not stop) while the pane stays as it was; a session that reconnects loses its chip and closes the popover with it.
A start refused after the agent was ended leaves a pane with no agent and so no chip to carry a line, and it is the one Reopen failure that is a notice rather than only a diagnostic (design principle 13): the operator can act on it, because the agent is gone from the pane and its conversation is kept, so they start it again.
The shell turns `last_error` kinds `terminal.input_hold_failed` (a new tab, split or fork exceeded its input hold bound) and `pane_reopen.not_restarted` into the one-line notice bar (`web/src/errorNotice.ts`), and the notice stays until dismissed or until the front device changes, because the core clears `last_error` at its next event of any kind; every other failure of Reopen stays on the chip.
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
That Retry applies only to enabling or disabling Memory; card actions stay in the Memory tool with their existing confirmations.
Memory promotion PR rows retain their original repository-qualified link after Connections changes; checks, status and agent identity join only that exact URL.
A legacy promotion whose original repository cannot be proven shows PR verification unavailable and keeps its PR button disabled.
Turning it off keeps existing cards.
While the dialog is open its project owns the daemon's single Memory read slot, and the tool rail's project is restored when it closes.
Open project Memory is one scoped navigation intent that closes the dialog and opens that project's usable checkout with the right lane on its Memory tool; a delayed remote focus changes the lane only on arrival, and refusal leaves the earlier checkout's lane unchanged.
The shell waits for both the target checkout and its Memory tool; the core's lane call then brings the lane forward, over the agents in a narrow window.
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
Appearance contains Language, Color theme, Mode, Accent and Interface text; Accent shows its swatches and no hex value, and Interface text carries one note that the sidebar keeps its own sizes (PRD settings-left-nav B5; hide-color-theme B1).
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
Its short note says that connection and AI call counts are collected and that conversations and code are not sent.
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
The core reads GitHub with its own machine's `gh`, so in a window on another machine a signed-out `gh` is named on the core's machine ("Not signed in to gh on Mac mini") with the command that fixes it there (Run on Mac mini, `gh auth login`, with Copy) in place of `gh`'s own reason; on the core's own machine the row is as it was (PRD core-host-node-move B15, B1).
Hide has no GitHub sign-in of its own, so there is no connect or reconnect action beyond that read.

### About

About shows the `hide` version and Herdr as `Connected · <version>` or its failure in one line, and a closed Details disclosure holds the rest: the process, lifetime, state file, page connection, Herdr's runtime message, version, protocol, socket and binary paths, the environment rows, the recent diagnostics and Copy diagnostics (PRD settings-left-nav B9).
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
A Host a registered device already uses, or one that reaches the same machine as one, is dimmed with `Added as <name>` and cannot be chosen; a Host whose address could not be read is dimmed with a short reason (no `ssh`, `ssh` could not read it, `ssh` took too long); a wildcard Host is not listed, and a config with more concrete Hosts than the list holds says so.
With no Host at all the dialog shows only `Not listed? Add a Host entry…` with an example entry, and no form.
The list is a radio group: the arrow keys move between the Hosts that can be chosen and Enter adds.
Advanced, closed at first, holds `What Hide installs there` in three lines (the helper and the `hide` command, hooks and Herdr integrations for the agents turned on, and that removing the device takes them off) and the Herdr socket; the description above the list says Hide installs its helper on the device, so adding is still the one place the kit is agreed to (device-parity D-12).
The ⋯ menu offers Select, Connection details…, Allow and install… (a device whose helper is not allowed) or Revoke helper… (one that is), and, after a separator, Remove…; This Mac's menu has Select and Connection details… only.
Revoke helper… and Remove… ask before they act, with the same confirmations and events as before; no button is focused when a confirmation opens.
Revoking the helper stops the device's terminals with its link, and the Revoke confirmation says so: its panes read `helper not allowed · allow it in Settings > Devices` in their body at once, take no key, and come back when Allow and install… gives the link again.
Connection details is a dialog with the helper's state and folder, the helper program, the folder of the `hide` command and the SSH host key the consent is bound to, then every part of Hide's kit with a mark, its name and where it is when installed or its state and reason when not.
Installed is ✓, not on this machine is –, turned off is ○, outdated, not installed or removed is !, and failed is ✕; the state is also read out, since the mark is hidden from assistive technology, and a part that is not on the machine says so once, without repeating its reason.
A healthy kit is on no row.
A line grows under a device only when something needs the operator: a refused connection with the step and what to do, a helper that is not running (Retry helper) or not allowed (Allow and install…) while the connection is fine, a part of Hide's kit that failed or was removed, outdated or never installed, as one line naming it with Reinstall, or the result of a test they asked for.
Reinstall repairs only the parts the line names, reads Reinstalling… while the machine's kit work runs (B8), and the line goes when the parts are whole again; nothing else on the screen reacts, and the detail of every install goes to the diagnostic log (B18).
A kit pass that stopped before changing anything (a folder another account can change or a link in the way, an earlier Hide's open work or unreadable record, another Hide changing the same account) is one line under the machine with Retry, never a line per part: `Hide stopped before changing anything.`, what stopped it with the folder or file at fault, and what to change; when the folder is the state folder `HIDE_STATE_DIR` or `XDG_STATE_HOME` chose, the line names that variable and both ways back, another folder only the operator can change or unsetting it to return to `~/.hide/state`, and on the machine that runs the app the command that stops Hide's daemon there, which reads the variable only when it starts and outlives the app (INSTALL.md, When a kit pass stops before changing anything).
More registered projects than a pass checks is its own line, and a cause this build cannot name says only that the pass stopped and to update Hide, with Retry like the rest.
Retry runs the same pass again, which checks again what stopped it, and reads Installing Hide's kit… while it runs; a read while Settings is open answers the same way, so the line goes only once a pass can run (`web/src/settings/MachineKit.tsx`, `kitRefusalLine` in `web/src/settings.ts`).
A machine whose kit does not run says why in that place: a daemon run from neither the installed app nor a build Hide placed (a moved core runs from one), a device not allowed yet, a device that must be allowed again, or a platform this build does not carry (B11, B17, B21).
The first install reads Installing Hide's kit… under the device; the tab reads every machine once when it opens.
The add form has one Add button and lists, once, what the kit puts on the device and where (B12); a device registered earlier without the helper offers Allow and install… in its menu, with the same list.
Removing a device asks once, names in one line what comes off that device (with its helper folder, `~/.hide/host-helper` by default) and that the records in `~/.hide` stay, or, when its helper is not connected, that the kit stays there; no button is focused when the confirmation opens (B22).
When another registered device reaches the same account on that machine, such as a second Herdr server there, the line says the kit stays for it instead.
The page has no per-device agents line, no coordination retirement row and no Codex per pane row: the kit no longer turns Codex's shared daemon off, and a machine's agents are on the Agents tab.

The rows put the window's own machine first, named This Mac, and name the core's machine on a window of another machine by the name it was registered under before the core moved to it, else the name it gives itself (PRD core-host-node-move B2, `web/src/screenMachine.ts`).
The row of the machine that runs the core always wears a `core` badge with the hint `core 실행 중`, a one-Mac setup included, unlike the rail's crown (Q11 of the plan; B1 keeps the one-Mac rail unchanged).
Test and Retry are offered for a device this core dials, never for a machine that dials in, which has no SSH alias here.
A connected device this core dials offers `core를 이 기기로 옮기기…` in its ⋯ menu, dimmed with `연결 안 됨` while it is not connected; a window of a machine whose core runs elsewhere offers it on no row.
That window's own row offers instead `core를 이 Mac으로 되돌리기…` and `Mac mini core와 연결 끊기…`, and its menu has no Remove…, since a machine does not remove itself.
`연결 끊기…` asks once, saying that the core refuses this Mac's pane input, file reads and browser actions at once and that reconnecting allows them again, and the window's strip then offers `다시 연결` (B16).

A move is one dialog over Settings whose body turns with the move, and every form names the reason first, `Mac mini는 늘 켜져 있어서 이 Mac을 덮어도 에이전트와 폰 연결이 계속돼요.` (B3 to B5):

- checking: `점검하는 중…` while both machines are checked and nothing is changed;
- checks failed: `고칠 것 2개 · 통과 9` (the passed count only when every check ran) and one line per failing check, naming the machine and either the one command that fixes it, with Copy (`gh auth login`, `sudo pmset -c sleep 0`), or what to do in words; `다시 점검` checks again;
  a line names the machine the failure is on, so a Hide AI with no agent chosen names the machine whose core moves, since that setting moves with the core, `이 Mac의 Hide AI` with `설정 › Hide AI에서 켜고 에이전트를 고르세요`, while an agent of it not signed in names the machine taking the core (`checkMachine` in `web/src/coreMove.ts`);
  earlier core records on the machine taking the core fail `Mac mini의 이전 core 기록` with `Mac mini에서 정리하세요`, and a folder the copy cannot reach there (a file on the way to its state folder, a link that leads nowhere, a folder another account owns) fails `Mac mini의 상태 폴더` with the cause as its line and `Mac mini에서 폴더를 고치세요`; a link on the way is no failure, since the machine answers the folder by its resolved path;
- confirm: what happens (`몇 초 동안 창이 입력을 받지 않고, 폰은 QR을 한 번 다시 스캔해요.`), that it can be undone, and `Mac mini로 옮기기`;
- moving: the five steps with ✓, `…` and `-`, and, while the other machine cannot be reached, `Mac mini에 연결할 수 없어요 · 다시 연결되면 이어서 해요`; where that machine may already have taken the core, so only it can say whether the move committed, the footer offers `그만 기다리기`, and for a move back whose core machine does not answer its retirement, `여기로 가져오기`;
- give up (amendment 4): the confirm of that action in the same dialog, `되돌릴 수 없어요.` first, then that this Mac's core starts again as it was and that a core coming back on Mac mini is stopped and set aside, not merged, and one plain line, in the body color like the others, that a Mac mini only cut off, not gone, keeps its core running until this Mac reaches it; `취소` goes back to the steps and `그만 기다리기` is the destructive action, never the default; the move back's `core를 여기로 가져올까요?` says instead that this Mac's core starts from the copy it received and that Mac mini's core stays stopped until it is retired, with no risk line;
- stopped waiting: `이 Mac의 core가 이전 그대로 실행 중이에요.` and that Mac mini's core stops once this Mac reaches it;
- failed: the step that failed with ×, `이 Mac의 core는 그대로예요.` with ✓, and Retry;
- done: `폰은 Mac mini의 QR을 한 번 다시 스캔하세요.` and `되돌리기`, which opens the same dialog for the move back.

The dialog only asks; the supervisor's `core_move` frame says where the move is, so a window that closed it and opens Settings again finds the same form, and a request another move holds is answered `다른 이동이 진행 중이에요`.
In Korean the particle after the machine's name follows its last sound, as Korean reads the name (`Mac mini로` but `집으로`, `web/src/i18n/koParticle.ts`).

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
The chip opens a popover with every feature of the kit's feature table, grouped under Herdr basics, session reading and multi-agent collaboration, with a heading, `✓ Works` or `– Not available` rather than color alone.
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
Antigravity has titles from agy's conversation index, Sleep, Fork (Hide opens the same conversation in a new pane with `agy --conversation <id>` and types `/fork` there, refused in a folder agy does not trust, where the new pane would open on agy's trust question; a read of agy's index that meets agy writing it is read again, up to 20 reads in a row inside the fork's 30 s wait; a fork that still fails closes its pane like any agent's failed fork, logged as `pane.fork_failed`, and leaves the conversation `/fork` made in agy's history, since agy 1.4.0 has no command that deletes one) and its conversation from its transcript, beside guidance, letters, Memory, subagents and the spawn guard from Hide's `hide` block; its row wears no Basic chip, so no feature popover opens for it, and the bell it does not ring yet is named in [agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block).
Its tool-permission question reaches Needs You as an approval, as Herdr 0.9.3 reports it (seen 2026-10-16); its folder-trust question is unmeasured ([agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block)).
Escape closes the popover and focus returns to the chip.
A part that failed, was removed or is outdated shows one line on that agent's row naming it (`Hook: Removed`, `Plugin: Outdated: …` for OpenCode's hook piece, `Herdr integration: Failed: …`) with Reinstall, only while the agent is on; a hook the operator removed stays removed until Reinstall or switching the agent off and on.
An agent that is on and found whose hook Hide could not put in says why in one muted line without Reinstall, since nothing is broken: its program has not created its configuration folder yet, a file of that name is not Hide's, or Hide does not write that hook on this system (`Plugin: OpenCode has not created ~/.config/opencode yet; …`).
A switch-off whose removal did not finish keeps the row from reading Off and says so in its own line.
There is no row or switch for Codex per pane, no per-agent CLI group (that is Hide AI's) and no hooks diagnosis list: the row statuses are the diagnosis.
Idle agents (`Sleep after`) and Starting work (`Link the issue in pull requests`) follow the list.

The first-run agent choice is the first step of the first-run wizard (see First run below), PRD agent-adapters-onboarding and PRD first-run-onboarding, shown when this Mac's kit has never run.
It lists the seven agents Hide supports, Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity in that order, as tiles in a grid, each with the agent's own mark (or a two-letter monogram where no official mark is bundled, `docs/BRAND.md`) and its state: the agents installed here are on and show a check, an agent not installed is dimmed, has no switch and links its official install page (`Install ↗`), an agent installed on a system where Hide has nothing to attach to yet (the kit's `unsupported_system`) is dimmed and says `Not on Windows yet` with that system's name, with no switch, no install link and no entry under `What changes` (D-25, B49), and a tile is a switch (`role="switch"`) pressed with Space or Enter.
Claude Code and Codex are on when they are installed; every other installed agent is on too, since the operator chose the full set, and `Continue` installs what is left on.
Under each name the tile says what Hide knows: `Checking…`, then `Ready` or `Not signed in` for the agents whose sign-in Hide checks (Claude Code, Codex, Grok, Pi and Antigravity; Antigravity says `Installed` until Hide AI's folder for it is made, because Hide reads agy's sign-in only once that folder is ready), and `Installed` for OpenCode and omp, whose sign-in Hide cannot read (`tileStatus` in `web/src/firstRun.ts`).
With no agent installed, one line says so and links Claude Code's and Codex's install pages, and Continue still goes on.
One sentence above the tiles says that Hide adds its own entries to each agent turned on and changes nothing before Continue, and `What changes` opens the list of what Continue writes for each agent left on (its hook file, skill, plugin or extension, and Herdr integration, at the places the kit reports, with the account home written as `~` on every system) and the `hide` command's link; the list follows each flip (B11, D-26).
`Continue` is the only button and the only way on: Escape, a click outside and a close button do nothing, so a stray key cannot finish a choice that leaves Claude Code and Codex off with no hooks; with every tile off, Continue installs nothing and moves on.
Continue holds, reading `Applying…`, from the press until this Mac's kit pass lands, and a second press meanwhile changes nothing; a chosen agent whose piece failed keeps the step on screen with the agent and the kit's reason in one line, and the button reads `Try again`.
The question belongs to the kit record (`awaiting_choice` in `~/.hide/kit/installed.json`), not to the window: closing the app, reloading the page or a failed save leaves it asked, and the next launch asks again until Continue has saved.
The tiles are drawn from the live availability plus the operator's own flips, so an agent that becomes installed while the step is open appears on, and Continue sends exactly what is shown.
A Mac whose kit has already run (an existing install) never sees it, and nothing is held back from it.
Until the choice is made the kit puts nothing for Claude Code or Codex on a fresh Mac either, so the hooks wait for Continue or for Settings, Agents.
A device whose own record waits gets the saved choice once per run, by what is installed there, as soon as it reports (also after it was not connected when Continue was pressed); with nothing chosen it is told that, so its record stops waiting, and nothing is installed for agents.
A device that still waits after it was sent the choice is logged once (`first_run_choice.unanswered`) and not sent it again until it reconnects.
Under `Continue`, one line names the agent Hide AI will use: the first agent in that fixed order that is switched on in the step and signed in (`firstRunAgent` in `web/src/hideAi.ts`, "Hide AI uses Claude Code"; in Korean the name takes 를, or 을 after Grok, whose reading ends in a consonant, `hideAiFirstRunKey` in `web/src/agentOnboardingRules.ts`), read from `status.background_ai.providers[].state`, which the step asks the core to probe while it is open; with no such agent the line is absent, and it follows each flip of a tile (PRD settings-cleanup D-18, B45, B46).
An agent installed later appears in these rows off and is never turned on by a pass.

### Conductor

Settings › Conductor, in the machine group after Hide AI, keeps what a Conductor plans with on this machine (PRD software-factory-conductor D-31, B48, B49).
`동시 실행 상한` is how many agents one mission runs at once, reviewers and verifiers included and the Conductor not counted, from 1 to 16; `기본 범위` is how far a mission goes when the operator does not say, up to ready pull requests or through the merge.
`작업자 후보` lists the agents a Conductor chooses workers from, at most 16: each row's agent, its model from the start panel's list (disabled with why when that list cannot be read), its effort automatic or fixed, and what it is for; an agent this machine has not installed is dimmed with `설치 안 됨`.
With no candidates the Conductor chooses among the installed agents; with some, Hide refuses a worker that is none of them (`mission_candidate_refused`, [delivery.md: Missions](delivery.md#missions)).
Every edit sends only the field it changes and the core checks the result; a refusal shows inline, and a mission already created keeps the cap, scope and candidates it was created with.

### Hide AI

Settings > Hide AI is the agent Hide runs behind the scenes (labels, worktree names, Project Memory analysis), separate from the agents the operator runs (PRD settings-cleanup D-14 to D-18, B33 to B47).
Every value is the core's `status.background_ai`; the tab sends `ai_settings` events, one per intent, and an edit is pending until the snapshot carries it, so a choice the core refused simply returns to what the snapshot says.

- **Use Hide AI** is a switch on its own at the top.
  Off, Hide makes no model call: agent titles show each session's own text, worktree names are typed, and Project Memory analysis waits while saved Memory search keeps working.
  Everything below it dims and is out of reach (`inert`), and the stored values stay as they were, so turning it on resumes with them.
- **Runs on** has an Agent select and a Model select.
  The Agent select lists only agents the core would accept (`selectable`: installed, signed in, a Hide AI backend, a call that cannot change files) with each official mark, and the chosen agent stays in it with its state even when it has a problem; the same row shows its state as symbol and words (Signed in, Not signed in, Out of usage until 3:10 PM).
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
- Hide AI runs the core machine's agents, so in a window on another machine every sign-in state and line above names that machine ("Not signed in on Mac mini", "Claude Code is signed out on Mac mini", "Sign in to Codex on Mac mini to use Hide AI"); the agents have no one sign-in command, so the fix is the machine in words, as the move's Hide AI check says it; on the core's own machine nothing names a machine (PRD core-host-node-move B15, B1).
- **Features** holds Agent summaries (the `agent_summary` switch of `ai_settings`: off, no agent label is asked for, the one being made is dropped, and every surface names each agent by its session's own title or its provider with no AI line or written question; on again, the kept labels return at once and each pane's current turn is asked for) and Worktree names (the same `issue_settings` value the Issues tab held).

## Mobile companion

### Settings > Mobile

Mobile is a page of Settings' machine group, after Hide AI; Tailscale runs on the core's machine, so a failing step and the serve entry line name that machine (`Tailscale isn't installed on Mac mini` on a window of another machine, `This Mac` on its own).
Its switch, 폰에서 hide 열기, is off at first, and its one-line description says it works over Tailscale and only inside the tailnet (PRD settings-cleanup B59).
While the switch is off hide runs no Tailscale command.
Turned on, the three Mac checks (Tailscale installed, logged in, MagicDNS and HTTPS on for the tailnet) fold into one line, `Tailscale is ready`, once they all pass (B57); a failing step shows only itself with a warning and its action (a download link, "Tailscale 앱에서 로그인", or the admin console's DNS page with a link) and nothing for the steps that passed or still wait.
The phone side, Tailscale on the phone with the same account, is guidance hide cannot check and is part of the `폰 연결하기` row's description.
The page rechecks every three seconds while it is open, so logging in or turning HTTPS on continues without reopening it.
Once every Mac step passes and hide has confirmed its serve entry, the `폰 연결하기` row offers `QR 보기`; opening the page makes no code and voids none (B58).
Pressing it shows the QR, 폰 카메라로 찍으세요, the ts.net address and the code's m:ss countdown, and the row's button becomes `QR 숨기기`, which takes the code out again; a pairing phone spends the code and the QR goes, so the next phone needs `QR 보기` again.
An HTTPS entry hide did not make shows its target in one line and no pairing row; a failed serve command shows the failed step and its message in one line, and so does a Funnel that would publish the address, which also disconnects every phone when it is turned on later.
A removal that fails when the switch goes off keeps that line under the switch until hide finishes it.
The phones group is titled 연결된 폰 · n / 4; each row shows the phone's name, when it was last seen, whether notifications are on or off, the days left before the seven-day revoke once it has been away a day, and 해지, which closes that phone at once.
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
The target names the core's machine as its system does in the phone's language (`이 Mac · 홈`, `내 PC · 홈`), and reads `홈` alone until the targets arrive.
시작 starts the agent there; once it appears in the list the phone opens its detail.
The desktop does not follow a phone start: the visible tab and the keyboard stay where the operator has them, and the new agent only appears in the sidebar and the lists like any other.
Pressing 시작 again for the same text and choice after a lost answer sends the same request id, and hided starts one agent for it.
While hided is out of reach the sheet shows the unreachable line and keeps what was written; a refused start shows its reason inside the sheet and keeps the text, which is cleared only by a start that went.
When hided is out of reach the last list stays dimmed under "연결 안 됨 · 맥의 hide가 꺼져 있거나 폰의 Tailscale이 꺼져 있어요. 다시 시도 중", the app retries on its own, and it shows the same line when opened without a network.
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
A revoked phone shows "이 폰의 연결이 해지됐어요. 맥에서 QR을 다시 여세요." and drops its own push subscription.
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
- The recent project cycle is Ctrl+Shift+` (previous Ctrl+Alt+Shift+`): Alt+Tab is the system's window switcher on Windows and Linux and never reaches an app, so the cycle moves to ` under the rule, the key macOS keeps for its own window cycle.
  Going back adds Alt to the Ctrl+Shift the cycle already holds, the one four-key desktop chord: a held cycle commits when its Ctrl is released, so the rule's Alt+Shift+` could not step back inside it, and GNOME keeps Alt+Shift+` for switching between an app's windows.
- In a browser tab, Reopen closed tab is Ctrl+Alt+Shift+T: Chrome keeps Alt+Shift+T to focus its toolbar, which keyboard users rely on, and the macOS browser chord ⌥⇧T is those same keys there.
- In a browser tab, Switch Projects and Tasks is Ctrl+Alt+Shift+B for the same reason: Chrome keeps Alt+Shift+B to focus its bookmarks bar.
- Not yet handled: a browser tab's area cycle is Alt+` and Alt+Shift+` on Windows and Linux (Chrome keeps Ctrl+Tab, as it keeps ⌃Tab on macOS), and stock GNOME and KDE take both to switch between an app's windows, so on those desktops the cycle reaches the page only once the desktop's keys are changed or the cycle is bound to another chord in Settings, Shortcuts.
- Move to Trash in the Explorer is Delete, as in Windows Explorer and the Linux file managers.
- Copy Relative Path is Alt+Shift+H: the rule would give it Copy Path's Alt+Shift+C, VS Code's own chord for it is a two-step sequence (Ctrl+K, then Ctrl+Shift+C) that the registry cannot hold, and Alt+Shift+R opens the PRs tool.
  A browser tab has neither path chord, because Chrome keeps ⌥⌘C (Inspect); the menu items work there.
- Cut, Copy and Paste in the Explorer are the system's own clipboard chords: ⌘X, ⌘C and ⌘V on a Mac, Ctrl+X, Ctrl+C and Ctrl+V on Windows and Linux.
  The tree answers them only while it holds the keyboard, so a terminal keeps ⌘C for its selection and Ctrl+C for the interrupt, and a text field keeps its own.
- Inside a text field, a palette, a board or a document, no terminal holds the keyboard, so the system's own command key applies: ⌘↵ is Ctrl+Enter (send a form, open a ⌘P result to the side, open a pull request on GitHub), ⌘-click is Ctrl-click (a link, a pull request, an issue), and Ctrl+S saves the document in front as well as Ctrl+Shift+S.
  A file or diff View's comment chord ⇧⌘. is Ctrl+Shift+. there for the same reason ([View openers and comments](#view-openers-and-comments)).
- The digits follow the rule: a device is ⌘1-9, so it is Ctrl+Shift+1-9 there, and an agent row stays ⌥1-9, Alt+1-9.
  A tab has no digit: ⌃1-9 would take the control bytes ⌃2 to ⌃8 send a terminal (NUL, Escape, ^\ to ^_), and ⌃⇥ already steps through the recent tabs.

### In a terminal

| Action | macOS | Windows and Linux |
| --- | --- | --- |
| Copy the selection | `⌘C` | `Ctrl+Shift+C` (the terminal's even with nothing selected, so Chrome's element inspector never opens), or `Ctrl+C` while text is selected |
| Interrupt the program | `⌃C` | `Ctrl+C` with nothing selected |
| Paste | `⌘V` | `Ctrl+Shift+V` |
| Delete to the line start, go to the line start or end | `⌘⌫`, `⌘←`, `⌘→` (sent as ^U, ^A, ^E) | `Ctrl+U`, `Home` and `End`, which the terminal sends as typed |
| A newline the agent keeps in its prompt | `⇧↩` | `Shift+Enter` |

Ctrl+C copies only while text is selected and clears the selection as it copies, as Windows Terminal does, so the next Ctrl+C interrupts; with nothing selected it reaches the program.
Every other plain Ctrl key reaches the program, apart from text size (Ctrl+=, Ctrl+- and Ctrl+0).
Settings, Shortcuts refuses a binding on the terminal's copy or paste chord.

While an input method composes (Hangul, kana, pinyin), the keys in this table are the input method's and none is sent; a ⇧↩ that ends a syllable reaches the program after that syllable, never before it (`web/src/terminals.ts`).
The syllable being composed is drawn in the terminal's own colours, underlined.
Anywhere in the window, an Escape pressed while a field composes is the input method's too: it never closes the dialog, sheet or palette the field is in (`web/src/keyboard.ts`).

### Every command

The hold hint follows the same table: holding Ctrl+Shift alone in the desktop app on Windows and Linux floats the tab numbers, and holding Alt alone the Agents-list numbers.
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
| Next panel (all projects) | none | none | none | none |
| Previous panel (all projects) | none | none | none | none |
| Next recent project (exception) | `⌥⇥` | `` Ctrl+Shift+` `` | `⌥⇥` | `` Ctrl+Shift+` `` |
| Previous recent project (exception) | `⌥⇧⇥` | `` Ctrl+Alt+Shift+` `` | `⌥⇧⇥` | `` Ctrl+Alt+Shift+` `` |
| Select agent 1-9 | `⌥1 … ⌥9` | `Alt+1 … Alt+9` | none | none |
| Select device 1-9 | `⌘1 … ⌘9` | `Ctrl+Shift+1 … Ctrl+Shift+9` | none | none |
| Search | `⌘K` | `Ctrl+Shift+K` | `⌘K` | `Ctrl+Shift+K` |
| Open file | `⌘P` | `Ctrl+Shift+P` | `⌘P` | `Ctrl+Shift+P` |
| Overview | `⇧⌘O` | `Alt+Shift+O` | `⇧⌘O` | `Alt+Shift+O` |
| Add to backlog | `⇧⌘M` | `Alt+Shift+M` | `⌥⇧M` | `Alt+Shift+M` |
| Focus sidebar | `⇧⌘P` | `Alt+Shift+P` | `⇧⌘P` | `Alt+Shift+P` |
| Toggle left sidebar | `⌘B` | `Ctrl+Shift+B` | `⌘B` | `Ctrl+Shift+B` |
| Switch Projects and Tasks (exception) | `⇧⌘B` | `Alt+Shift+B` | `⌥⇧B` | `Ctrl+Alt+Shift+B` |
| Toggle device rail | none | none | none | none |
| Toggle right lane | `⌘E` | `Ctrl+Shift+E` | `⌘E` | `Ctrl+Shift+E` |
| Toggle Memory tool | `⇧⌘L` | `Alt+Shift+L` | `⇧⌘L` | `Alt+Shift+L` |
| Toggle PRs tool | `⇧⌘R` | `Alt+Shift+R` | `⌥⇧R` | `Alt+Shift+R` |
| Toggle Files tool | none | none | none | none |
| Toggle Changes tool | `⇧⌘G` | `Alt+Shift+G` | `⇧⌘G` | `Alt+Shift+G` |
| Toggle File Tree | `⇧⌘E` | `Alt+Shift+E` | `⇧⌘E` | `Alt+Shift+E` |
| Find in pane | `⌘F` | `Ctrl+Shift+F` | `⌘F` | `Ctrl+Shift+F` |
| Save file | `⌘S` | `Ctrl+Shift+S` | `⌘S` | `Ctrl+Shift+S` |
| Keep open | `⇧⌘K` | `Alt+Shift+K` | `⇧⌘K` | `Alt+Shift+K` |
| Split right | `⌘D` | `Ctrl+Shift+D` | `⌘D` | `Ctrl+Shift+D` |
| Split down | `⇧⌘D` | `Alt+Shift+D` | `⇧⌘D` | `Alt+Shift+D` |
| Zoom pane | `⇧⌘↩` | `Alt+Shift+Enter` | `⇧⌘↩` | `Alt+Shift+Enter` |
| Show task list | `⇧⌘U` | `Alt+Shift+U` | `⇧⌘U` | `Alt+Shift+U` |
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
In the desktop app, holding ⌥ alone floats each Agents-list row's number at its top right, and holding ⌘ alone (Ctrl+Shift on Windows and Linux) floats each device tile's number at its top left, the corner a tile's marks leave free: a keycap in the popover colors with a border, a small shadow and one mono digit, positioned over the row or tile rather than in it, so a title, a row's time, its fold slot and a tile's marks never move.
The number is the screen order at that moment, first to ninth, top to bottom for the rows the Agents list draws (a folded parent's descendants are not rows) and for the rail's tiles, and an item past the ninth carries none.
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
