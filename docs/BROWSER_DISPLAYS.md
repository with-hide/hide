# Browser displays

A web page in hide is a display in a View area, beside the files and diffs of the same Workspace (issue 155).
It is a tab like any other view: it splits, moves, closes, and comes back after a relaunch with the rest of the layout.
The page itself is drawn by the desktop app, a native Chromium view laid over the area's slot; a plain browser tab that serves the shell cannot draw one and says so.
There is no Herdr browser pane and no chromux profile behind it, and nothing is screencast.

## Who owns what

| Owner | Holds | Code |
| --- | --- | --- |
| The core | Browser View layout, requested address and load stamp, and the native page's reported loading state | `herdr-core/src/view_layout.rs`, `herdr-core/src/runtime/view_areas.rs`, `herdr-core/src/runtime/workspace_control.rs` |
| hided | The local `file:` event boundary, checkout-scoped Browser commands and gateway discovery, and native routes into consented SSH devices and, on a node whose core runs on another machine, into the core's machine and the node's own window's gateway | `hided/src/server.rs`, `hided/src/file_url.rs`, `hided/src/workspace_cli.rs`, `hided/src/browser_control.rs`, `hided/src/browser_relay.rs`, `hided/src/browser_routes.rs`, `hided/src/node_pages.rs`, `hided/src/node_browser.rs`, `hided/src/browser_assets.rs` |
| The web shell | Where each page sits, since only it has the geometry, the toolbar, the overlay freeze, and the notice in a plain browser tab | `web/src/BrowserDisplay.tsx`, `web/src/browserViews.ts`, `web/src/host.ts` |
| The desktop app | The pages: one `WebContentsView` per display it was asked to show, their navigation, route requests, and their lifetime, including when a hidden one is unloaded | `desktop/src/main/browser.ts`, `desktop/src/main/browserSync.ts`, `desktop/src/main/host.ts`, `desktop/src/preload/index.ts` |

The core never loads a page and the desktop app never decides which displays exist.
The core's Browser View inventory is sent as an empty array when the last page closes, so the native host removes that page even when no other Browser View remains.
A display's `load` stamp is how the core asks for a load: the host loads the display's address again whenever the stamp is newer than the one it last applied, so opening an address the Workspace already shows focuses that display and loads it again rather than adding a second one.
The stamp is not saved; a relaunched page loads its address once when it is first shown.
The retained inventory also carries the authoritative `area_id`, so native control never guesses a page's area from the front Workspace.
Only checkouts in the current connected catalog contribute to that inventory; a removed or disconnected checkout's saved layout grants no native page authority.

## Opening a page

- `hide browser open <url-or-path> [--beside] [--wait] [--request-id <id>]` from a shell the daemon can bind to a checkout: a connected Herdr pane, or a local process whose working directory is inside a registered checkout (`docs/ARCHITECTURE.md`, hided and the WebSocket boundary).
  The CLI resolves a relative file on the calling machine, gives loopback hosts `http` and other hosts `https`, and submits one action scoped to the caller's checkout with no Workspace override.
  The core checks an HTML file against that checkout on its own device and reads it outside the core lock before placing its Browser View.
  The action result confirms View placement, while `hide view status <view-id>` returns `pending`, `loading`, `loaded`, `failed`, `unloaded`, `disconnected`, or `unsupported` for its native page.
  `unloaded` means the desktop app closed that load's page to give its memory back ([Unloading hidden pages](#unloading-hidden-pages)); it loads again when shown or when an agent drives it.
  `disconnected` means no desktop renderer is attached for a page that had loaded; `unsupported` means no desktop renderer was available for a pending page.
  `--wait` ends with `page_unloaded` for an unloaded page, which a `hide browser` command on it loads again.
  On the checkout on screen, only an open from the pane the operator is in, the selected pane of the front Workspace, brings the page to the front of its area and calls the lane.
  Another pane there, of the same tab or another, adds its page to the checkout's one list, in the area it lands in and behind the one in front, and moves no tab, checkout or keyboard; the same address opened from two tabs is one page, so the operator's page and load stay unchanged.
  On a checkout not on screen, a pane of its active tab brings its page to the front of that checkout, where it loads and can be driven, and a pane of another tab adds it behind; neither calls the lane or moves the screen.
  A shell bound only to the checkout counts as another pane on the checkout on screen and as its active tab on one that is not.
  `--beside` places the page as a file's Open to the side does ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#opening-preview-and-open-to-the-side)): in the area next to the active one, trying right, left, below and above, or a new area to its right; a page of the same address already in that area is brought forward and loaded again rather than opened twice, and with no room for another area the open is refused with the next action to close a View or open without `--beside`.
  The desktop window's gateway, acting for a CDP client that creates a page, brings that page forward, since a page loads only in front; it acts as `gateway:<key>:<path>`, one key per window, so the core tells it from a checkout shell (`gateway_caller_id`).
  A new page that is not in front can stay pending until it first appears.
  An agent reads the checkout's Views (`hide view list`, `hide view status`, select, close, `--wait`) from any of its tabs, even when its tab is not the Workspace's active tab and the Workspace was never in front: every caller sees the same list, so such a page is listed as `pending` rather than missing.
  Each row names the sessions that opened it (`openers`, `here` on the caller's own) and whether the operator did, and `hide view list --session <id|here>` keeps the rows one session opened (PRD session-files D-31); the Files strip on screen shows only the keyboard session's and the operator's ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#file-views-per-checkout)).
  `--wait` polls that state for at most ten seconds and returns a failure if the page fails, disconnects, or remains pending; it does not reveal a hidden View.
  If a later status query fails, the refusal still carries the applied open request ID and View ID for reconciliation.
  The action receipt and page status carry the load stamp, so a concurrent reopen that supersedes the requested load returns `page_superseded` instead of the newer load's result.
  The CLI returns one JSON line, exits nonzero on refusal, and never starts Hide.
  Its successful result includes `cdp_http_url` and `browser_ws_url` when the native gateway can issue a capability for that display.
  If discovery fails after placement, the result retains its applied View receipt and carries `browser_control.state: unavailable` with a retry command; discovery does not undo or repeat the open.
- Click, double-click or press Enter on an HTML file (`.html`, `.htm` or `.xhtml`, case-insensitive) in the Explorer of a local or connected device checkout, or choose Open in Browser in its menu.
  Open source in the same menu opens the file as an ordinary editor tab; Open to the side keeps its editor behavior.
  The device host must have file access consent for a remote page to load.
- Click an HTML path (`.html`, `.htm` or `.xhtml`, case-insensitive) in a local desktop terminal, or choose Open in browser from that path's right-click menu.
  The shell sends one `browser_open` event with the owning registered checkout and `reveal: true`, so the page and its checkout come forward together; the existing local `file:` boundary checks the address.
  An HTML file of another checkout than the one in front opens in the front Workspace instead, with no reveal, so no checkout moves; the same boundary admits a file of any registered checkout.
  Open source in the same menu opens the file in the editor at the link's line instead.
  An HTML path outside every registered checkout, clicked in the desktop app's window on the core's machine, opens as an outside page instead (The file boundary, Outside pages); anywhere else it follows the desktop host's default-application boundary.
- A page that opens a tab, a `target=_blank` link (an image a page wraps in one), a shift-click or `window.open` without window features, gets another browser display in the same Workspace, beside that page so the page that asked stays in view.
  The shell names the page (`beside_display`), and the core places the new display in the View area next to the page's area in Open to the side's order (right, left, down, up), else, the page's area being the only one, in a new area to its right; with the page gone it opens where any page would.
  An address the Workspace already shows is focused and loaded again there instead.
  A sized popup, `window.open` with window features the way a sign-in button opens one, is not a display; see Popups below.
  A remote page's routed loopback address is translated back to its source device address before that request reaches the core.
- The address field in the display's toolbar loads what was typed into that display.

An empty address is a new-tab page rendered by the shell, with a focused address field and Open choices for File (⌘P) and Diff when the checkout has changes.
File and Diff use the existing palettes and replace the empty display, preserving its ID and position; a file read that fails leaves it intact, and one whose original tab was closed or navigated meanwhile cannot overwrite another tab.
Each + or View tab menu New tab action creates its own empty display in that area, without creating a native page.
Closing an untouched empty display adds nothing to Recent Closed or draft recovery.

A nonempty display holds only an `http`, `https` or `file` address with something after the `//`, or `about:blank`, within 8 KiB; anything else is refused with its reason and nothing changes.
A remote `file:` address is loaded through a checkout-scoped route, never as a file on this Mac.

## Scoped browser control

`hide browser connect [--display <id>]` uses the caller's existing Workspace credential and prints a JSON result with `cdp_http_url` and `browser_ws_url`.
It accepts no Workspace override and requires a connected desktop renderer.
An explicit display must be a browser in that credential's checkout; an omitted display uses that checkout's active View area for new targets.
An explicit display whose web page the desktop app unloaded loads its address again first, hidden, and the connection waits up to five seconds for that load; a page command (`hide browser snapshot` and the rest) connects the same way.
Discovery does not reveal the Workspace, activate the app, or start a daemon.

The desktop registers one private numeric-loopback gateway address and random control token with hided through daemon-authenticated `/browser-control`.
The registration is tied to the app PID and process birth, has a four-host cap, and is removed on release or discarded when a later request finds the app gone.
An ambiguous multi-window registration refuses discovery instead of choosing another window.
At most eight discovery and browser action requests are in flight, and discovery has an eight-second deadline with a 16 KiB response cap, no proxy and no redirects.
The CLI receives only the public capability URLs; it never receives the desktop registration token or the shell's daemon token.

CDP target creation uses authenticated `/browser-control/action` and the existing Workspace prepare/read/commit contract.
The core inserts a distinct browser display into the capability's named area even when another area is active or the same address is already open.
An unknown area refuses before placement.
CDP exposes only HTTP(S) and blank pages; native `file:` displays, filesystem paths and other schemes return `browser_address_unsupported` for attachment or creation.
The one exception is the operator's Comment mode, whose comment-only capability hided alone holds ([Comment mode](#comment-mode)); an agent's `/connect` and `hide browser` commands never reach a `file:` page.
Every HTTP(S) or blank native page generation blocks `file:` requests before its first load, independently of debugger attachment, and retains that restriction until the generation ends.
The native guard cancels file main-frame and child-frame navigation, redirects, resources and popup requests, including addresses assembled by `Runtime.evaluate`; it never attempts to recognize JavaScript source text.
Manual local-file displays retain the file policy below and are never CDP targets of an agent or an external client; only Comment mode's own lease reaches one.
An explicit manual navigation to a validated local file replaces the restricted WebContents with the existing separate file partition instead of relaxing an in-flight generation.
Downloads and filesystem writes are disabled once a generation has held a debugger lease, including after disconnect.
Chromium still opens an unnamed intermediate file in the download folder, and may write a download's first bytes into it, before `will-download` refuses it; the cancellation deletes that file on Chromium's download thread, so no download is left behind.
Each native generation logs its first file request, navigation and popup refusal separately, and its first controlled-download refusal once; repeated attempts remain canceled without more diagnostic output.
Close and select bind the authenticated area into the core action and recheck the display's area and browser kind under the commit lock after retry lookup.
A page moved after the query is refused without changing its selection or closing it; shell renderers and terminal displays cannot be selected or closed by this route.
The app has one retry identity per launch registration, so retransmitting a completed close returns its receipt after the page is gone.
The core also publishes positive area scopes separately from its browser inventory, including empty areas of connected catalog checkouts.
Each scope has an incarnation that remains stable during ordinary layout changes and changes after authority is revoked and granted again.
The shell forwards these scopes through the existing native sync; a missing scope grants no CDP authority.
Capabilities pin that incarnation, so a coalesced snapshot that hides the intermediate revocation cannot revive an old connection URL.
The core records loss of authority when a session or checkout catalog is updated, before another worker update or snapshot read can hide it.
Unregistering a checkout, disconnecting its device or removing its area permanently revokes existing capabilities, so registering it again requires a fresh connection URL.
Closing an area's final page retains its area scope and allows a fresh target there without reviving an expired checkout capability.

The gateway creates a new private control token and random capability path for each app launch; restarting the app or replacing its daemon connection invalidates every old URL and debugger session.
It binds only numeric loopback, checks Host and Origin, and serves discovery only under the issued capability path.
It does not enable Electron's process-wide debugging port, and its target provider contains only the BrowserViews inventory, never the shell renderer or another area's pages.
The tab's attachment mark follows an actual debugger lease; disconnect detaches the client while preserving the display, and closing the display ends its sessions.
Browser-wide cookie, cache, permission, download, context and shutdown commands are refused; page-origin cookie reads are scoped to the current page.
Flattened descendant iframe sessions and one level of legacy session envelopes are supported; deeper legacy envelopes are refused before a mutation.
Limits are fixed in the host: 64 capabilities, 32 sockets, eight clients, 16 HTTP requests, 32 pending commands per client, 64 sessions per client, 4 MiB messages, 8 MiB buffered output and 600 commands per minute per client.
A command has a ten-second deadline; each client keeps at most 128 mutation receipts, accepts a replay for at most nine minutes, and retries an ambiguous core action at most three times with its original request identity.

### Client invocations

Use the complete `browser_ws_url` from `hide browser connect` as `browser_ws_url` below.
It is a capability: preserve its path, keep it out of shared logs, and request a fresh URL after its authority ends.
The supported profiles are agent-browser 0.38.2, chrome-devtools-mcp 1.10.1, Playwright 1.63.0 and the Browser Use 0.13.10 low-level session API.
Each client connects to an existing HTTP(S) or blank display; an area capability also permits a new display in that same area.

```sh
agent-browser --cdp "$browser_ws_url" open http://127.0.0.1:3000
agent-browser --cdp "$browser_ws_url" snapshot -i
```

```sh
chrome-devtools-mcp --wsEndpoint "$browser_ws_url" --no-usage-statistics --no-performance-crux
```

MCP's literal `--browser-url` mode resolves discovery at `/json/version` and loses the capability prefix; use its official `--wsEndpoint` option.
Anonymous root discovery remains unavailable.

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP(browser_ws_url, { noDefaults: true });
try {
  const context = browser.contexts()[0];
  const page = context.pages()[0];
  console.log(await page.title());
} finally {
  await browser.close(); // Disconnects this external client; preserves the display.
}
```

Playwright uses the existing context with `noDefaults: true`; creating another context or applying its default global download configuration is unsupported.
`Target.attachToBrowserTarget` supplies an opaque protocol parent for Playwright's independent page sessions, with only scoped discovery, version, and explicit child attach/detach commands.
It never attaches a native browser debugger or creates a context; virtual parents count toward the same 64-session limit and release their admitted children when detached.
Once the core confirms a selected-display or direct-page close, the gateway revokes that authority and drains the successful reply before the normal WebSocket close handshake, even when native retirement arrives later.
Page, tab and iframe target metadata carries one opaque context identifier per launch so clients can associate those targets with the existing context; it does not grant native context authority.
The gateway attests each leased debugger's native main frame before forwarding client commands and translates protocol frame references to that display's public page target ID.
Iframe target IDs retain their native frame identity only after attachment from that owned debugger; their session IDs remain opaque, and parent-frame references use the public page identity.
This translation applies to protocol frame metadata and commands, never to values returned by page evaluation.

```sh
BU_CDP_URL="$browser_ws_url" python - <<'PY'
import asyncio
import os
from browser_use import BrowserSession

async def main():
    session = BrowserSession(
        cdp_url=os.environ["BU_CDP_URL"], is_local=False, keep_alive=True,
        use_cloud=False, enable_default_extensions=False, accept_downloads=False,
        permissions=[], captcha_solver=False, auto_download_pdfs=False,
    )
    try:
        await session.connect()
        print(await session.get_tabs())
        page = await session.get_current_page()
        if page is None:
            raise RuntimeError("No eligible browser display is attached")
        print(await page.evaluate("() => document.title"))
    finally:
        await session.stop()

asyncio.run(main())
PY
```

This Browser Use example explicitly passes `BU_CDP_URL` to the Python API and uses `connect`, page queries and `stop`.
Default `Agent` or `start` watchdogs request browser-wide download behavior and are unsupported; this profile does not establish Browser Use CLI support.
Native file displays and unrestricted browser-context administration are unsupported for every client.

## Agent page commands

`hide browser snapshot|click|fill|type|press|hover|drag|scroll|wait|screenshot|eval|console|network <display> ...` read and drive one browser display of the caller's checkout, by its View id.
`hide browser help` prints the agent guide for them, the commands, the ref format, checking with `--diff` and `changed`, and each failure reason with its recovery; `hide guide`, which the `hide` agent skill points at, sends browser work to it, and the session guidance names the commands and points at it (`hide-agent-hooks/src/workspace_context.rs`).
A snapshot of a device's loopback page names the address the core keeps for the display (`http://localhost:8765/...`) on its address line, not the loopback route that page loaded from on the core's machine; console and network lines still print the addresses the page used.
A snapshot is text and exits zero; an action prints one JSON line with what it did, `changed` and `next`; every failure prints `{"ok":false,"reason","display","next_action"}` and exits non-zero.

Each command is one connection and one debugger lease: the CLI asks hided for a relay to the display ([ARCHITECTURE.md](ARCHITECTURE.md#hided-and-the-websocket-boundary)), attaches to its page, does its work and disconnects, which releases the lease and turns the tab's attachment mark off.
A display another debugger holds, an external client, another `hide browser` command or the operator's Comment mode ([Comment mode](#comment-mode)), fails `display_busy` at once; nothing queues.
A display that is not HTTP(S) or blank fails `display_unsupported`, a missing one `display_missing`, and one that closes during the command `display_closed`.
Nothing outlives the command in hided or on disk, except the file `screenshot` writes: refs are `data-ct-ref` attributes of the page's elements, so they survive a re-snapshot of the same document and start again at `@1` after a navigation, and the `--diff` baselines and the no-change streak live in a non-enumerable property of each frame's document.
Every CDP step has an eight-second deadline, below the gateway's ten-second command deadline, which would otherwise end the whole lease, and a whole command ends with `page_unresponsive` after two minutes.

Cross-origin frames are auto-attached, up to 24 per page, and each frame's document carries a tag of four random characters; a snapshot shows it as `# OOPIF <tag> origin=<origin>` with refs `@<tag>:N`, and its field values and link targets reduced to their origin.
A frame has its own renderer, so one that answers nothing within a step (its `Page.enable`, a read of its document or its own frame lookup) does not fail the command: the snapshot notes it as `# OOPIF unresponsive origin=<origin> - no answer in time; ...` right after the top document, with the origin its `location.origin` would have, reads the rest of the page, and gives it no tag, refs or baseline of its own.
Only an unanswered call counts as silence: a closed connection, including the relay's idle close, still fails the command.
The note is part of the top document's `--diff` baseline, so a diff names a frame once, when it goes silent or answers again; the lines a frame showed before it went silent cannot be read back and are not reported as removed.
Silence is what one read observed, not a state of the frame: every frame stays known, and a read of the page (every snapshot, the capture before an action and each one after it, every poll of a `wait`) asks again a frame that gave no answer before.
That covers a frame that missed its `Page.enable`, its tag, its look for its own frames or an earlier read.
The budget is for silence, not for a frame's history: a frame is given up in a command after `MISS_LIMIT` (two) unanswered rounds in a row, the round that found it silent and one more in case it was only slow.
It is counted per kind of request: its `Page.enable`, tag and reads together, and its look for its own frames apart.
Only an answer of the same kind starts a count again, so a frame that answers reads but never the look for its own frames is given up for that look after two, and one whose reads hang is given up for reads even if its look answers.
A given-up frame is noted and not asked again for the rest of that command, so a frame that never answers costs at most two steps (its rounds run together with the other frames', one step each) and not one on every read.
A read that would have asked it gets the silence it already showed (`Page::eval` and `Page::eval_all` answer a frame the command has given up on without sending), so the frame stays unread and named, never counted as a change or as absent.
The next command starts with a clean budget.
A look for a frame's own frames that went unanswered waits for the next read of the page; it is not asked again in the same call.
Each such round sends its calls together: `Page.enable` for the frames not yet enabled, then their tags, then the reads and the tag checks that follow them and the baseline swaps.
It reads the answers under one deadline, so the frames a page holds cost one step between them per round.
Looking inside several frames for their own frames is one such round too, not one frame after another.
The gateway handles each command as it arrives, not in sequence, and each frame has its own renderer, so a frame's answer is due one step after the round's calls were sent.
The gateway holds at most 32 commands per client, and it keeps one that nobody answers until its ten-second deadline, when it answers with an error.
`Cdp` therefore keeps every command it sent until a message of the gateway's ends it, an answer or its deadline error, and never from the time alone, since a relay in between delays the gateway's clock by an amount no client can know.
The calls of a batch of frame reads go in rounds that bring the commands held to at most 28, the top document's first and always.
A session whose last command has not been answered is asked again only while no more than 16 are held, so a renderer that is not answering cannot take the room a session not yet asked needs.
A call that does not fit is not sent, and every other call goes on: a frame that answers is read beside the frames that do not.
A call that was never sent (`CdpError::NotSent`, also when no time is left) reads as unread this round but is not a miss, since the frame was not asked; a later round asks it.
The top document's calls, a single call and an input event among them, are never held back by what frames hold, so a drag's release is always sent.
The gateway's pending cap, unlike its 600-commands-a-minute limit, is therefore never passed because of frames that are silent.
Nothing is sent when no time is left.
A frame that goes silent while its baseline is swapped leaves the snapshot for a note, so a diff shows the note and not every line of the frame as new.
A silent frame is never evidence of absence: a ref whose frame tag no readable frame carries, while a frame that gave no document may be the one, is neither stale nor gone, but fails an action with `page_unresponsive` and leaves `wait` to its next poll, so `wait --gone @tag:N` never succeeds on a frame that has only not answered yet.
That failure's `next_action` says to take a fresh snapshot: a ref from an earlier snapshot is the likelier cause, and the remedy is the same whether or not the silent frame was ever the ref's.
`wait` asks the top document and every frame together at each poll, inside the budget above, and a wait that times out says that frames were not read and names their origins.
A wait's first poll is a whole read, each call under a step, so a short timeout still finds what is already there.
Every read of a later poll waits no longer than the wait has left (and no longer than a step), so `wait` reports `timeout` at its deadline, not a step after it.
A poll interval counts only the sessions actually asked, not the frames the command has given up on.
A read cut short by the deadline is a wait that ran out of time, not `page_unresponsive`.
When it was the top document's read that was cut short, the `timeout` says the last poll ended before the page answered, so the text is not known to be absent.
An action's change check that could not read a frame says so, with the frame's origin, instead of reporting that nothing changed, and adds no stall to the streak.
The change check compares like with like: a frame that one of the two reads (before the action, after it) could not read is left out of both texts, so a frame that answered in one read and not the other is never a content change, and it is named: with no other change the answer says the frame did not answer and adds no stall, and with a change it ends with an `# unread:` line naming the frames that could not be compared.
A frame whose own frames could not be looked for (its look for child frames got no answer) is as unread as one that gave no document: a ref that no readable frame carries may be a child of it, so it is not stale or gone either.
That frame has been read, so it keeps its section and its refs and is compared normally.
The frames inside it are the unread ones: a snapshot notes them as `# OOPIF unlisted origin=<origin>` after the top document, a change check leaves them out of both texts and names the frame, and a `wait` that times out says the frames inside it were not listed.
The note is not the unresponsive note of a frame that gave no document, and the frame is not called silent.
The frame that holds the keyboard focus is found by asking every frame together. When none reports the focus and a frame did not answer, `type` and `press` fail with `page_unresponsive` and send nothing, since the focus may be in the frame that did not answer; the top document is the answer only when every frame answered and none holds the focus.
An action on such a ref goes to that frame's own session in its own coordinates, so a ref from a frame that navigated fails `ref_stale` instead of touching another element.
Same-origin frames and open shadow roots are read and acted on through the top document.
`changed` compares a snapshot of every frame taken before the action with one after it, read again after 0.7 and 1.2 seconds when nothing changed yet; clickable detection is capped in document order so a scroll alone never reads as a change.

Input and screenshots go only to a display on screen, so the operator sees what an action does.
A display that is not its area's selected View fails them at once with `display_hidden`, whose next step is to ask the operator to show it, since an agent that is not in the operator's pane cannot bring it forward on the checkout on screen; hided reads that from the core's view list when it opens the relay.
That is hided's own rule, not Chromium's: a View the host hides still answers.
A selected View of a Workspace that is not in front is not refused: the host hides it, yet the page still reports its document visible and Chromium answers input and screenshots within the step, so no input times out there and none waits to be delivered when the Workspace returns, and the operator does not see the action drawn.
The page's own visibility is not the test either: a selected View in a window another app covers reads `hidden` and still takes input and screenshots.
An input the page does not answer within the step, because a script holds it, fails `page_unresponsive` and is not undone: the event was sent and takes effect when the script yields, so the failure's `next_action` says it may already have happened and to look with `snapshot --diff` before repeating it, as it does when the relay ends the connection before the answer.
A call that times out and then finds the connection closed or a dialog open reports that cause, not silence.
A press that held the page this way ran its handler to the end, and no release or click arrives later, since the command sends none after the failure and its session ending delivers none.
A drag whose move or drop the page does not answer sends no release either, for the same reason: a release a held page would take when its script yields lands long after the command reported its failure, and it does not switch drag interception off, which ends with the session.
Reading commands work on a hidden display.
No command calls `Page.bringToFront`, moves the operator's mouse, changes the View in front or takes keyboard focus.
A JavaScript dialog is never answered for the operator: an action that opens one reports its type and message, and while it is open the next command's `Page.enable` on the top document's session gets no answer within the step deadline and fails `dialog_open`; Electron shows the dialog as a sheet on the window, where the operator answers it.
A step that gets no answer after the command received a dialog event from any frame also fails `dialog_open`.
A dialog that a cross-origin frame opened before the command started cannot be told from a frame whose script never yields, since Chromium sends no event again to a new client, so that frame is noted as unresponsive and the rest of the page is read.
Such a dialog does not hold the top document: a click on a top-document button was answered and ran its handler while a frame's `confirm` was open, which is why the frame is noted and not the whole page refused.
`console` relies on Chromium replaying the messages the current document logged to a session that enables `Runtime`, and shows the newest 50; `network` reads the document's resource timing, the newest 100 rows, and has no request or response headers or bodies.
`wait --timeout` is at most 60 seconds and `--verify` at most 10.
There is no file upload and no download: `DOM.setFileInputFiles` stays refused by the gateway and downloads stay disabled for a page that has held a debugger lease.
`Input.dispatchDragEvent` with a `data.files` list that is not empty is refused for the same reason, since dropping files hands the page local files as an upload does; a drag that carries no files passes.
A native HTML5 drag whose intercepted data names files fails `drag_carries_files` before any replay, so the gateway's refusal is never the first the agent hears of it; its `next_action` says not to retry, since a pointer drag would start the native drag where the gateway sees neither it nor its files.

Each action draws on the operator's view of the page: an arrow cursor in the accent color glides to the target, a click ripples, a drag leaves a line, filled fields flash, a pressed key is named beside its field and a scroll shows an arrow.
It is one `<hide-agent-overlay>` element, a direct child of the document element with a closed shadow root, `aria-hidden` and `pointer-events: none`, so it never enters a snapshot and lets the operator's clicks through; it removes itself two seconds after the last action.
Drawing it has a one-second deadline and never fails a command; reading commands draw nothing.

The page-side code is chromux's (MIT, `modakbul-gongbang/chromux` `93f770f`), kept as embedded assets that carry its notice: `snapshot.js`, `dom.js` (deep query, target checks, fill and the wait probes), `render.js` (`--diff`, `--grep`, `changed`, evaluated in an isolated world) and `overlay.js`.

## Comment mode

A page is a native view the shell cannot draw over, so the pointing half of the operator's Comment mode on a page ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#view-openers-and-comments), PRD view-opener-feedback D-07) is drawn by hided inside the page (`hided/src/browser_page/comment.rs`).
The window's `browser_comment` event turns it on: hided takes the display's debugger lease, as a `hide browser` command does, and draws `hided/assets/browser/comment.js` in an isolated world, `hide-comment`, with the page's comments for its current address as numbered pins.
The overlay only points: it outlines the element under the pointer, takes a click or a drag, and reports the place through a binding that exists only in that world, acting only on trusted input, so the page's own scripts can neither see the binding nor pick a place.
hided takes the place's screenshot at once, with the overlay hidden for that frame, keeps it as the session's one pick, and sends the window a `browser_comment` frame with `pick` (its id, the anchor, and where it shows as fractions of the page's viewport); a pin pressed sends `open` with the comment's id instead.
The window writes the note in its own layer over the page's slot (`PageNoteLayer` in `web/src/ViewComments.tsx`), which freezes the page to its still as any overlay does ([Overlays](#overlays)) and takes the keyboard from it, so the page's scripts can neither read nor change the note (D-17).
Add sends `browser_comment` with `add` {pick, note}; hided takes the pick it holds, once, and hands the add with its screenshot to the window's feedback queue, which stages the screenshot and gives the core the same `view_comment` event the shell sends, in order with that window's other comments and Sends.
A pick another pick replaced, or one whose session ended, adds nothing and is logged (`pick.gone`); a page that navigates, to a new document or within the same one (a route, a fragment, back or forward), drops the pick it held and sends `dropped`, which closes the note, and a window whose socket closes clears every page's Comment state; an edit or a delete from the card a pin opens is the shell's own `view_comment` event.
While a pick's screenshot is taken the overlay is drawn clear but still takes every press, and the operator's pointer and mouse events on it never bubble to the page's own listeners; a page's capture-phase listener still sees where a press landed, as it sees any input, and nothing it does there reaches a comment.
A comment's screenshot is the page clipped to the element or area (scaled down past a pixel cap), staged in this machine's `view-feedback` folder; only its name reaches the core, and a Send carries the bytes to the recipient's machine ([delivery.md](delivery.md#the-operators-view-feedback)).
Turning it on gives the page the keyboard, so Esc reaches the overlay without a click in the page first.
The window shows Comment on only when hided's `browser_comment` frame says on, which it sends once the overlay is in the page; until then the turn-on is `starting`, a press on the button takes it back, and leaving the View turns it off as it would a running one.
While it is on, the window sends the pins again as an `update`, which redraws a running session and never starts one, so pins that cross the page's own Esc cannot turn Comment back on.
A session is the View's in its checkout, keyed by device, checkout path and display id, because display ids repeat across checkouts; every `browser_comment` frame names the `workspace` and the `display_id`.
A session belongs to the window that turned it on and ends when Comment or Esc turns it off, when the page navigates away from a document it can draw on, when the lease ends, and when the window goes; ending never waits behind the pins the session still has to draw, so a lost message cannot leave the page held.
hided answers each end with a `browser_comment` frame whose `reason` is null for the operator's own end.
At most four sessions run at once (`comment_limit`), a display another debugger holds cannot start one (`display_busy`), and while a session holds the lease an agent's `hide browser` command on that page fails `display_busy`, the known cost of drawing inside the page.
A screen whose daemon relays a linked node's pages cannot reach their gateway, so Comment there answers `comment_unavailable_here` ([INSTALL.md](INSTALL.md#what-each-system-supports)).

A local HTML page, a `file:` display inside a checkout, takes Comment as a web page does, through a lease that exists for Comment alone.
hided asks the gateway's `/connect` with `purpose: "comment"` for one display (`BrowserControl::comment_capability`); no other caller names a purpose, a pane's scope is built without one (`scope_of`), a linked node's daemon refuses a scope that carries one (`scope_valid`), and the capability goes to the comment session and nowhere else, never into a result for a pane or a `hide browser` path.
For that purpose the gateway lists the display's page even when it is a `file:` page of the file partition (`Browser.cdpPages` marks it `local`), and for no other capability does such a page exist: an agent's `/connect` on it keeps the 409 `Display has no eligible HTTP(S) browser target`, its area listing, `Target.getTargets` and attach never name it, and `hide browser` on it answers `display_unsupported`.
A comment capability carries only the methods the session sends, `Target.getTargets`, `Target.attachToTarget`, `Page.enable`, `Page.getFrameTree`, `Page.createIsolatedWorld`, `Runtime.enable`, `Runtime.addBinding`, `Runtime.evaluate` (only in a world the same lease made with `Page.createIsolatedWorld`) and `Page.captureScreenshot`, each with only the parameters the session sends (no `grantUniveralAccess`, `userGesture` or `includeCommandLineAPI`), and passes only the events it reads, `Runtime.bindingCalled`, `Page.frameNavigated`, `Page.navigatedWithinDocument` and `Inspector.detached`.
Navigation, `Fetch`, `Network`, `Input`, `DOM`, new or child targets, `Page.close`, downloads and every other command are refused for it before they reach the page, and it has no discovery listing.
The lease ends as any does: when the page stops being a `file:` page (or a web page, for a web display) it is revoked, so a document that replaced the page ends the session, and the session answers its reason.
The same capability is what the session asks for on a web page, so Comment holds a web page to the same list.
A session that cannot start, or ends on its own, answers a reason the window turns into a hint on the Comment button: `browser_control_unavailable` (the desktop window's gateway did not answer or no longer authorizes the area), `display_missing` and `display_closed` (the page closed or was replaced), `page_unresponsive` and `dialog_open`, beside `display_busy`, `comment_limit` and `comment_unavailable_here`; another reason shows as `Comment stopped · <reason>`.

## The file boundary

A client names a local file through a `file:` URL, so hided checks it like any path a client sends, on `browser_open`, `browser_state` and the `view_layout` `navigate` action.
The shell and daemon spell a Windows drive path as `file:///C:/...`, preserving the drive as part of the path rather than a hostname.
It decodes the path, refuses one that names another host or does not decode (`invalid_path`), runs it through the same checkout boundary the Explorer uses (`outside_checkout` for a file outside every registered checkout), and writes the checked path back as the one spelling the shell and the CLI also produce.
A refusal is a `path_refused` frame to that client and never reaches the core.
For a native `browser_state` report, a registered checkout's pinned physical root can be mapped back to its registered spelling before that same boundary checks the file.
This preserves a completed load's stamp when the checkout was registered through an alias, while a replaced root, a symlink escape or an unrelated file is still refused; it does not extend file-open authority.

A local HTML page is held to what a browser allows a local page, and no more.
It loads the display's own address, and its stylesheets, scripts, images, fonts, media and frames only from its own checkout, symbolic links resolved, so a link out of the checkout carries nothing out.
A script reads no file: a `fetch`, XHR, WebSocket or beacon of a `file:` address is refused, and because the app turns off Electron's `GrantFileProtocolExtraPrivileges` fuse (`desktop/scripts/fuses.mjs`, flipped at packaging before the bundle is signed and read back from the built app), a local page is an origin of its own, so no page of the checkout reads another's frame, nor the pixels of an image it drew.
It reaches no address of this machine or a private network: loopback, link-local, private and shared ranges, the names a resolver answers from the local network (`localhost`, `.local`, a name with no dot), and a public name that resolves to one of those, so Hide's daemon and the operator's dev servers stay out of reach.
Public internet addresses stay open for the page's resources, since a page of generated HTML often loads a font or a script from a CDN; what makes that safe is that no file of this machine can be read to send there.
A name is resolved once per page generation and the answer kept, so a name that changes its answer afterwards is not seen; the page's own address, a navigation or a popup it opens keep their rules above.
`localPageRequest` and `isLocalNetworkHost` in `desktop/src/main/browserSync.ts` decide, and `BrowserViews.localPageRequest` applies the decision to a request of every local file page, whoever opened it.
Each kind of refusal (a file, the network) is logged once per page generation as `browser.request_refused` with `reason: local_page_boundary` and the kind, and shows nothing on screen.
A page of a connected device's checkout is served through its own route, which refuses every request that is not that route's.

### Outside pages

An HTML file outside every registered checkout that the operator clicked in a terminal of the desktop app (PRD outside-path-in-hide) is an outside page, and its address never reaches hided or the core.
The desktop host grants the clicked file and its folder to the window that clicked it (`desktop/src/main/externalFiles.ts`), and the core holds a display of kind `external` with the grant's 32-hex id, the file's name and `html`, never a path or an address; the browser sync names it by `grant` with an empty `url`, and the host loads the granted file itself, with no `/browser-route`.
Its page follows every rule of a local page above with the grant's folder, links resolved, as its checkout: its own files load from that folder and below, and nothing outside it.
A navigation to a `file:` address outside the folder, by a link or by `window.open`, is refused and logged as `browser.navigation_refused` (or `browser.window_open_refused`) with `reason: external_folder_boundary` and the grant, and a `window.open` of a file inside the folder loads in the same tab rather than opening another display.
Each HTML grant holds one of 32 in-memory partitions, `hide-browser-external-<slot>`, so an outside page shares storage with no other live outside page and no checkout page; its storage is cleared when the grant ends, and the slot is handed out again only after that clear succeeds.
The page reports its state to the shell marked `external`, and the shell sends no `browser_state` for it, so its `file:` address is never written anywhere hided reads.
It is never a gateway target: `/connect` and `hide browser` on its display answer as on a `file:` page of a checkout (`display_unsupported`), and Comment does not offer it.
The grant ends when no tab stands on it: every browser sync names the grants of the core's outside displays and of opens still on their way (`external_grants`), the host lets go of the rest and destroys their pages, and a sync before the reloaded window has the core's tabs names nothing so a reload lets go of nothing.
A grant no sync has named since it was handed out is kept for 10 s (`UNNAMED_GRANT_GRACE_MS`), because the shell learns of it only from the click's answer, and an open the core never answers stops being named after 10 s (`PENDING_GRANT_MS`), so a lost open frees its grant.
When a window attaches it tells the core which grants its host still holds (`external_grants`), so a tab whose grant went with a quit app closes.
What the fuse changes beyond these pages: any `file:` page the app itself loads (the status page) is an origin of its own and loads only its own script and style, which it does; the shell, the preload and the main process load over loopback or from the asar and are unchanged. The `file:` loader no longer reads inside the asar, so the status page ships unpacked beside it (`ARCHITECTURE.md`, The desktop host).
`desktop/e2e` runs on a copy of Electron flipped the same way (`fused-electron.ts`), so what its tests prove is what ships; an unpackaged run of the app (`pnpm --dir desktop dev`) is stock Electron, where the request rules hold and the fuse does not.
What Comment's world can read on a local page is what the page itself can read, and no more.
The comment session evaluates in an isolated world of the page's own document: the same origin, session and request rules as the page's scripts, the file partition's `webSecurity`, and the `onBeforeRequest` rules of `desktop/src/main/browser.ts`.
It holds no `Network`, `Fetch`, `Page.navigate` or `Page.setBypassCSP`, which are the commands that could widen a world, and `hided` evaluates only the shipped `comment.js` and fixed expressions; the gateway refuses an evaluation that does not name a world.
`desktop/e2e/view-comment-file.spec.ts` measures it on a local page: a `fetch`, an XHR, an `<img>` drawn to a canvas, an `<iframe>` document and a request to a loopback server, started from the page's world, from an isolated world made through the debugger, and from `executeJavaScript`, answer the same in all three.
Those answers are the local page rules above, so the comment world adds nothing to what the page can reach, and the lease adds nothing for an agent.

A device file keeps its remote path through the UI boundary.
The core confirms that the current View belongs to the connected device and checkout, and hided reads the HTML and its declared relative assets through that device's consented `hide-host` channel.
The channel pins the checkout root, refuses traversal and links outside that root, and caps each response at 16 MiB.
The route serves the opened HTML plus at most 128 declared relative stylesheets, images, scripts, and CSS image or font URLs.
It refuses undeclared checkout files, including same-directory secrets, and sends a restrictive content security policy that prevents a remote HTML preview from contacting another origin.
The native page receives a random loopback route for its own View and load stamp; the route URL does not replace the remote address stored in the core or shown in the toolbar.
Remote `localhost`, `localhost.`, IPv4 `127/8`, IPv6 loopback, and IPv4-mapped IPv6 loopback HTTP, HTTPS, and WebSocket traffic instead uses a dedicated SSH local forward to that device's loopback port, whose channels open on the device's one SSH connection.
The native route binds a local loopback IP and uses its actual local port.
For `localhost`, the forward reserves both IPv4 and IPv6 loopback at that port before publishing the route, so either resolver choice reaches the SSH device.
HTTPS keeps the source hostname or numeric loopback IP in the browser URL for certificate checks.
If this Mac cannot bind a numeric source IP for an HTTPS forward, the route fails explicitly.
The wildcard spelling `0.0.0.0` uses `127.0.0.1` as its safe local destination and a certificate for the wildcard name may fail validation.
The remote source address remains in the Browser toolbar.
Absolute loopback subrequests from a forwarded page use that View's forward when their scheme and source port match; other loopback requests are refused instead of reaching this Mac.
If the SSH route fails, the page shows the failure; it never tries the same port on this Mac.
Each native route is bounded, belongs to the desktop process that requested it, and closes when its View closes, its device disconnects, or that process exits.
Closing a route cancels its pending SSH channel open and its active transfer tasks before releasing both local listeners.
An individual SSH channel open has a 15-second limit, including the alternate localhost address attempt.
Creating a route reserves its View and load briefly, then connects to SSH outside the route registry lock.
Other Views can close while that connection is pending, and a canceled or superseded reservation cannot publish a late forward.
The complete SSH connection and authentication attempt has a 15-second limit.
Closing a pending View cancels its SSH connection, and at most four route builds may be in flight even when Views are repeatedly opened and closed.
Cancellation during SSH key exchange also shuts down the TCP socket before a session handle exists, so the connection cannot outlive its build permit.

### A screen whose core runs on another machine

When this Mac's hided runs in the node role ([ARCHITECTURE.md](ARCHITECTURE.md#a-core-on-another-machine)), the desktop host asks it for routes exactly as it asks a core's daemon, and the same route registry, caps, reservation and reaper answer (`hided/src/node_pages.rs`).
The View's source comes from the core: the node asks `POST /relay/browser-source` on the link's relay port with its live relay grant, and the core answers the View's current address and load or none.
A page of this Mac loads as it is, except a `file:` page, which loads only inside a checkout the core opened on this Mac over the live link, at the real path the boundary resolved, and is refused with `file_page_outside_checkout` anywhere else, as a core's own `file:` pages are judged.
A loopback page of the core's machine, such as one an agent there opened with `hide browser open http://localhost:5173`, goes through a local forward on the link's own SSH connection, with the same address rules as a device's.
A `file:` page of the core's machine is refused with `file_page_unavailable`, and a page of any other device with `host_unavailable`: the node reaches neither, and it never loads the same address on this Mac instead.
While the link is down every route request answers `core_unavailable`, and the reaper closes each route whose View the core no longer names.
A Browser View opens on the core only while some window is attached to it, the node's windows counting through their relays; with none, `hide browser open` answers `renderer_unavailable`.

The node's desktop window registers its CDP gateway with the node's daemon through the same `/browser-control`, and the registration (its address and control token) never leaves this Mac (`hided/src/node_browser.rs`).
The node tells the core only which windows are here, as owner pids, over `POST /relay/browser-control` on its live relay grant, again whenever a window registers or leaves, when a window that exited without leaving is noticed (it looks every five seconds), and on each new link; an announce the core refuses is sent again after two seconds, doubling to a minute.
The core keeps the windows for as long as that link lasts; it cannot check an announced pid, which belongs to another machine, so it trusts the node's account for it as it trusts that account's screens, and a node that announces a window is chosen for the callers whose only window it is.
Authority stays with the core: it checks the caller's credential, decides the Workspace, area and display, and asks the node over the link for the gateway's capability in that scope (`browser_gateway`); a node that does not answer is `browser_window_unreachable`.
The core chooses the window on the caller's own machine first, then its own window, then the one node window there is; several of a kind refuse with `browser_control_ambiguous`, as several windows on one daemon do.
A caller on this Mac gets this Mac's own capability URLs from `hide browser connect` and `hide browser open`, and for a page command a one-shot relay URL on this Mac's daemon (`ws://127.0.0.1:<port>/browser-relay/<ticket>`, valid ten seconds) that its CLI dials after the claim; the daemon pumps that CDP to its gateway, so none of it crosses the link.
The relay listens on a loopback port of its own that serves nothing else, so a ticket is no key to the daemon's screen routes, and it holds at most four relays, counting tickets not yet dialed; one past that is refused with `browser_relay_limit` before the gateway is asked.
A caller on a device gets no capability URL from the core's own window either (`hide browser open` and `connect` answer `browser_control_elsewhere` at once, without asking the gateway), because those URLs are the core machine's loopback; its page commands reach the window through the core.
A caller on the core's machine, or on a device, whose only window is this Mac's gets no capability URL (`browser_control_elsewhere`, since those URLs are this Mac's loopback); its page commands are relayed by the core through a link stream to the same relay, at most four per link, and a stream past that cap is refused with `browser_relay_limit` and a `link_stream.cap_reached` diagnostic.
The gateway's own limits stay the ones that hold: its client cap answers a relay's upgrade with a refusal the caller reads as `browser_control_busy`.
A page action the gateway asks for (open, close, select) goes to this Mac's daemon, which passes it to the core's `POST /relay/browser-control/action`; the core runs it as that window, through the same prepare/read/commit contract as its own window's, and refuses a pid the node never announced (`browser_control_unavailable`).
A node's window shows every checkout the core knows, as the core's own window does, so it holds the same authority: its page actions may name any of them.
Revocation follows the link: when it ends, the node drops its unused tickets, ends every relay it runs with `browser_relay_gateway_lost` (close code 4011), as a relay through the link ends, and asks the gateway's `POST /revoke` to drop every capability it issued, three times before it logs `revoke.failed` (the desktop host's own lease, which revokes everything when its daemon changes or goes, is the backstop); the gateway stays available for the next link, whose core is told about the window again.
The next link's browser questions wait for that revocation, so a gateway that does not answer it delays them by up to about seven seconds, and they read `browser_control_unavailable` meanwhile; nothing is handed out that the revocation could miss.
The core hides an ended link's windows rather than forgetting them, so a page action retried across a reconnect is recognized as the same window's (at most 16 nodes; ended ones go first).
Each relay logs one `relay.ended` row with where its CDP ran (`core`, `link` or `node`) and the bytes it carried.

A page the operator loaded can still follow its own links.
A `file:` page that moves to a file outside the checkouts keeps showing it in its view, but its report is refused at the boundary, so the core keeps the last address it accepted and a relaunch opens that one.

## The page and its limits

Web pages in every Workspace share one persistent session for logins, cookies, origin storage and zoom.
Remote loopback pages use a persistent session per device, and HTML file previews keep a separate session per Workspace; none uses the shell's session.
This keeps localhost cookies of this Mac, each SSH device, and remote HTML previews apart even when their host names are equal.
Pages run in a sandboxed renderer with context isolation, no Node and no preload, so nothing in a page reaches the `hideHost` bridge or the daemon's token.
Existing Workspace-specific web sessions are not copied into the shared session, so an upgrade may require one new login per site.
A page gets no permission but writing the clipboard, because a prompt it would raise has nowhere to show; a download follows Chromium's default and is logged as `browser.download`.
A page may navigate to `http`, `https`, `file` and `about:blank`.
A link to another app's scheme (`slack:`, `zoommtg:`) from a page on screen leaves only after the operator agrees, the way Chrome asks first: whether it arrives as a navigation, a server redirect, a frame or a new window, a sheet on the window names the app macOS would open it with, the asking origin and the link, and Open hands the link to that app.
Cancel is the default button, so a stray Return from a question the operator did not expect opens nothing, and after a Cancel the same page or popup asks nothing more until it navigates again.
A frame's link reaches Chromium's external protocol handler rather than a navigation event, so the session's `openExternal` permission request asks the same question, and Chromium's own opener never runs.
A `mailto:` link from a page on screen goes to the default mail app without asking.
Refused and logged with their scheme only: a scheme no app claims; a scheme Chromium answers itself (`data:`, `blob:`, `javascript:`, every `chrome` one and the like); a scheme macOS hands to a file share, a shell, a script or a remote session (`smb:`, `afp:`, `ftp:`, `ssh:`, `telnet:`, `vnc:`, `news:`, `applescript:`, `shortcuts:`, every `x-apple` one and the like), so one click on an unexpected question never mounts a share or runs anything (`appScheme` in `browserSync.ts`); a link from a hidden page or an HTML file preview; and a link that arrives while a question is open.
The link never reaches the log, since an app link often carries a sign-in code.

The shell tells the host, in one sync, every browser display of the Workspace in front, the rectangle its slot occupies now, and the core's retained Browser View inventory across all Workspaces and their checkout lists.
A display the inventory no longer names is closed, which ends its renderer process even when its Workspace is in the background.
A retained display absent from the current front placements, including a page behind the front, is hidden and keeps its page, so moving a page between areas or Workspaces never reloads it.
At most 12 pages live at once (`MAX_LIVE_VIEWS`); past that, the page shown least recently among the hidden ones is unloaded, whatever holds it ([Unloading hidden pages](#unloading-hidden-pages)).
The host checks every field of a sync before it places anything and drops a message that fails whole; only the shell the daemon serves may send one.
Native page reports carry the display's current load stamp, so a late report from a previous reload cannot mark the new request loaded.

### Unloading hidden pages

A page's renderer holds its memory for as long as it lives, so the desktop app closes a hidden page nobody needs and loads it again when it is wanted (PRD browser-tab-idle-unload).
A page is unloaded when it has been hidden and idle for 12 hours (`IDLE_UNLOAD_MS`), or when it is the least recently shown hidden page past the 12-page cap.
Hidden means not drawn in the Hide window: a page in the front Workspace's slot is seen even while the window is minimized or covered, and a page behind a still for an overlay is seen too.
Idle means nothing that would break if it closed: it plays no media, heard or not, has no download in progress, holds no CDP session (an agent's or Comment's), and has no popup open.
The 12 hours count from the last time the page was seen or held, so a page whose playback, download, session or popup ends gets 12 more hours from then; the cap ignores what holds a page.
One check runs each minute while the window is open (`IDLE_CHECK_MS`), reading at most the 12 live pages, and stops with the window; nothing else runs for it.

An unloaded page closes like an evicted one: its renderer ends, its tab and title stay in the View list looking as they did, and the host reports it `gone`, which the core reports as `unloaded` for that load.
Shown again, it loads its address in place behind the ordinary loading state; its session, so a sign-in, stays, while its scroll position and anything typed into it are gone, and a load that fails shows the ordinary failure with the toolbar's reload.
An agent's connection to an unloaded web page (`/connect` naming its display) loads it again hidden, without moving the operator's View, area, tab or keyboard (`BrowserViews.cdpRevive`): the host keeps the address, load stamp and size of each unloaded web page whose display the core still retains, and drops them with the display.
A page loaded again this way counts as just seen, so the next check does not close it again.
When that load fails, the connection is refused with `Display page failed to load`, the View reports `failed`, and the next connection loads it again.
A file page or an outside file is not loaded again for an agent, since neither enters CDP; it loads when shown.
A page the core has not yet seen since the app started stays `pending`, as before, until it is shown.
Each unload is logged as `browser.unloaded {id, reason: idle | cap}` and each load for an agent as `browser.reloaded_for_agent {id}`; neither names the address.

## Keyboard ownership and cycling

Only a visible native page can report keyboard focus to the shell; the shell resolves its complete Workspace identity and current View area.
When the lane closes or its documents fold, the shell retires their keyboard owner with that drawing and returns it to the focused pane, even if a native focus report arrived after the preceding menu command and the terminal was already its active DOM element.
Input moved to another live column or a popup is left in place.
The host also reports desktop window activation separately from page focus, so an embedded page taking input does not make the app appear inactive.
An inactive window retains its selection and logical keyboard destination while hiding canvas input emphasis.
The page's `before-input-event` matches the shell's effective Electron shortcut registry for focused-area, global panel and project cycle commands before the page or menu sees them.
One host slot holds the initiating page, actual release modifier and cycle identifier, including input received after the overlay hides that page.
The first native cycle chord gives keyboard response to the shell in the same focused window, even when the overlay leaves the initiating page visible.
Held repeats, release and Escape received by the shell follow the same host IPC route as the first chord, so a quick release cannot arrive before cycle initialization or also run through the DOM listener.
After hiding that held page, the host reasserts shell keyboard response, because hiding a WebContentsView removes its native keyboard response; any page hidden while it holds the keyboard in the focused window does the same, so a note, a dialog or a palette drawn over the page's still takes the keys.
The shell retains the initiating page's logical ownership until release commits or Escape restores its input destination.
When a first start is rejected or has zero or one eligible item, or a release commits nothing because the cycle came back to its origin, or the hold ends because its area shrank to one tab or the daemon went away, it restores the initiating visible page; a release that commits instead follows the chosen page.
The trusted bridge carries only cycle keydown/keyup and cancellation; the shell keeps the frozen scope and preview and owns the single commit.
The shell also reports the matching identifier when release arrived in its renderer, so a late completion cannot erase a newer hold.
Release or Escape received by another native page still ends the frozen initiating cycle.
Window blur, page renderer failure and destruction of the held page cancel the host slot, and a hidden or unfocused page cannot begin a cycle.
After a blur cancels a hold, the host owes the initiating page the keyboard and pays it once the window is back and that page is shown, as Escape does.
The window is back at its own `focus` event, not when it reads as key: on macOS that event is the window becoming main again, and Electron restores the focus it stored on `blur` (the shell, which the hold had given the keyboard) just before it, so a page paid while the window is key but not yet back loses the keyboard to that restore.
The shell's sync that uncovers the page can land after the window returns, so the debt waits for the page instead of expiring at the first focus event.
Another page taking the keyboard, a new hold, leaving the page's Workspace, or the page closing ends the debt.
The host logs the window's blur, focus and visibility (macOS reports it hidden while nothing of it is drawn, and the shell then sends no sync), each page's focus, blur and shown state with its display id and whether it is the owed page, and the shell's focus beside `browser.window_return`, so a failed return shows whether the keyboard was never paid or was taken after it, and by what.
None of these carries an address or page content, and each fires at the rate the operator focuses, covers or switches a window or page.
After the core confirms a selected display, the existing keyboard-follow request focuses its document or diff, or schedules one trusted native-page focus command after visible-slot sync.
A page the operator opens (a terminal link's URL or HTML file, Open in Browser, Open server, or a row's, a card's or ⌘K's link) follows the same way: its `browser_open` names itself (`request_id`, `follow:` and a fresh id), and the keyboard goes to the display the core's receipt in `status.browser_opens` names once it is the active area's view; a refused open (`browser.open_refused`) ends the wait.
A View area's New tab sends no name, since its address field takes the keyboard as it mounts, and neither does a page's own new tab, which leaves the keyboard with the page that asked.
The host focuses only a visible page in its already-focused candidate window; it never brings a window forward for this command.
Cycle menu items are immediate command clicks without accelerators, so one physical key cannot also dispatch a menu selection.
The keyboard area has a strong tab accent and a content boundary outside the native slot; other selected tabs remain readable, with no page blur or recurring capture for focus styling.

## Popups

A sized popup a page on screen opens (`window.open` with window features: Chromium's `new-window` disposition with features, `isPopup` in `browserSync.ts`) is a real window above hide that keeps its opener, so a sign-in popup can post its result to the page and close itself; a display would have no opener, and the page would wait forever.
A shift-click on a link is `new-window` too but has no features, so it is a tab like `target=_blank`.
The host builds the window from its own options, and of its geometry and chrome takes only the size the page asked for, held between 320 x 240 and the work area and centred over hide's window (`popupBounds`); no window feature makes it frameless, always on top, unclosable, modal or off screen.
Electron gives it the opener's web preferences with Node off and the sandbox and isolation forced, so a feature can at most change the popup's own page (turn its script off, say), never what it can reach.
It takes the keyboard only when the page that opened it held the keyboard; otherwise it is ordered in without activating the app, so a page cannot pull focus from what the operator is typing.
It shares its page's session and request rules, holds no bridge, follows the same navigation rules as its page, and its own popups belong to the same page.
Electron closes it with its opener, so it ends when its page's display closes, is evicted, or the window closes.
At most 4 are open at once (`MAX_POPUPS`); a hidden page asking for one, or any page asking for a fifth, is refused and logged.
While a popup holds the keyboard, Close (⌘W) and Close pane close the popup and the text-size commands zoom it; no other app command reaches the shell behind it.

## Zoom

While a page holds the keyboard, the text-size commands zoom the page instead of sizing text, the way ⌘= / ⌘- / ⌘0 do in Chrome.
The desktop app routes them by command, not by key, so a rebound text-size chord zooms the page too: an app-menu command for Larger, Smaller or Reset text that arrives while a page holds the keyboard steps that page through Chrome's zoom levels (25% to 500%) or back to 100%, and the page keeps the keyboard (`BrowserViews.zoomFocused`).
Chrome's second zoom-in chord, ⌘+ (⌘⇧= on a US keyboard), reaches no menu item, so the host zooms the focused page on that key before the page or the menu sees it, unless the operator bound that chord to a command.
With the shell holding the keyboard the same commands size the focused terminal or document text as before, or zoom the image or PDF in front in the same steps ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#file-document-toolbar-and-markdown)).
Chromium keeps one zoom level per host within a page's session partition, so web pages of the same host in different Workspaces zoom together.
The shell's own zoom, which places every page on its slot, is a separate session and never moves with a page's zoom.

Each page reports its zoom factor with its state (`BrowserPageState.zoom`): after every committed navigation, which takes the new host's zoom, and after every zoom the host makes.
No Chromium event says that a zoom moved another page of the same host, so after any zoom, a text-size command, ⌘+, the toolbar's popover or a popup's, the host reads the zoom of every page of that session partition again and reports those whose zoom moved (`BrowserViews.zoomMoved`); that is at most `MAX_LIVE_VIEWS` reads, only on a zoom.
The shell draws the zoom as a badge inside the address field's right end while it is not 100%, and its popover sends `zoom_in`, `zoom_out` and `zoom_reset` (`BrowserCommand`), which step that page the way the text-size commands do without moving the keyboard ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#browser-displays)).
The popover is a shell layer over the page, so the page stands behind its still while it is open; each zoom it reports asks the host for a fresh capture, which the host takes of a page it hides for an overlay of the front Workspace, so every step shows (`zoomMoved` in `web/src/browserViews.ts`).
A popup has no badge; the text-size commands still zoom it.

## Device size

A display can lay its page out at a device's width instead of its slot's: Fit, mobile 390, tablet 768 or desktop 1280 CSS pixels (PRD browser-width-zoom D-10..D-12, `web/src/browserDevice.ts`).
The choice is the display's own and page-local, kept by host key in `ui.ts` (`browserDevices`) for as long as the core keeps the display; it is never sent to the core or saved, so a reload of the shell or a restart returns every page to Fit.
The shell draws a frame for the page in its area, at the device's width centered at the top when that is narrower than the area, and at the area's width when it is wider; the frame is the slot it registers, so the placement, the still and the page share one rectangle.
The placement carries `device: { width, mobile }` beside that rectangle (`BrowserPlacement.device`), and the host lays the page out with `webContents.enableDeviceEmulation` (`deviceEmulation` in `browserSync.ts`): a viewport of the device's width at the scale that fits it into the rectangle, as tall as the rectangle at that scale, with a mobile screen for mobile and tablet so the page's meta viewport applies, and a desktop screen for desktop; the user agent is not changed.
Chromium lays a desktop screen out at the page's zoom and a mobile one at the device's own width, so the desktop viewport is the device's width times the zoom, and `window.innerWidth` is the device's width at any zoom.
A new document starts without the emulation, so the host applies it again after every committed navigation, and again whenever the page's bounds or zoom change; Fit turns it off.
While a page has a device size, its zoom does not move: the text-size commands and ⌘+ on it are taken and do nothing, neither the page nor the shell's text, and the shell hides its zoom badge.
An agent that drives the page's viewport through CDP `Emulation` overrides it until the display's size, bounds or document next change.
Pages of a connected device and of a node's window are placed by the same sync, so a device size applies to them the same way.

The idle still is taken and drawn at the page's frame, so a page at a device size freezes and comes back at the same centered or scaled rectangle.

A trackpad pinch magnifies the page up to 3x without reflowing it, as in Chrome.
Electron turns pinch off by default, and the limit lives in the page's renderer, so the host sets it again on every committed navigation, including one that moves the page to a new renderer.

## Overlays

A native view is drawn above the page's HTML, so the shell cannot draw over it.
When the palette, a menu, a dialog, a popover, a focused-area, Global Recent Panels or Recent Projects list meets a page's rectangle, the shell hides the page in the same sync that first sees the overlay, so no frame shows the page over it, and draws the page's idle still in its place (below); with no still of the page's address and slot size yet, the place stays blank until one arrives.
In that same flush, ahead of the sync that hides the page, the shell asks the host for a fresh capture; the host answers only for a page it still shows (`browser.capture_refused` otherwise), and IPC from one renderer is handled in order, so the capture is taken of the page as it is now and replaces the still and the cached one when it arrives.
A capture that fails or comes back empty leaves the place blank or the older still, draws no message, and is logged (`browser.capture_failed`, `browser.capture_empty`).
The page comes back live when the overlay is gone, and its still goes a frame later, so nothing flashes between the two.
A shell drag (a tab, a divider, a lane divider, a tree item) marks the document root with `data-view-drag` or `data-agent-drag` while it runs (`web/src/shellDrag.ts`), and every page of the front Workspace freezes the same way until the mark is gone, so the drag's guide or preview draws over the pages and its drop never reaches one.
A new shell drag sets that mark rather than adding its guide to the overlay selector; a drag inside a page never touches the shell and is the page's own.
Radix layers and the Recent cycle mount as children of the body, where the sync watches for them.
While a layer that can move with its anchor (a popover, a menu) is drawn over a page the sync follows it every frame.
Pages sit only among the documents of the right lane's viewing column, and the lane is drawn over Agent Views or beside them, never under another part of the shell (PRD right-lane D-02), so a page needs no still for the lane; a lane closed, or the documents folded, behind the 보기 slot or behind a stacked tool column, hides its slots before they go inert, which hides their pages without closing them (PRD lane-two-columns B3).

The idle still is one capture per shown page of the front Workspace, kept with the page's address and its slot's size; a still of another address or size is never drawn.
It is taken once a page has been quiet for half a second (`STILL_QUIET_MS`) after its load finished, its address changed, its slot changed size, or it came into view, and only while the page is shown, uncovered, loaded and not failed; nothing else asks, so an idle page is not captured again, and there is no timer per frame or per tick.
At most one capture is in flight per page, the stills are dropped with their display, when their page leaves the screen, and when the front Workspace changes, so the cache never holds more stills than pages on screen.
Each capture is one `capturePage` and PNG encode in the main process.

Tooltips never freeze a page, since they open and close too often.
A tooltip measures itself as it mounts, before Radix places it, and opens on the first side that fits the window and meets no shown page, trying its own side, the opposite one, then the two across (`web/src/tooltipSide.ts`), the shell's rectangles of the pages the last sync showed being the ones it avoids (`visiblePageRects`).
When every side that fits meets a page it keeps its own side and is drawn under the page; with no page shown, as in a plain browser tab, it keeps its own side.

## States

The toolbar holds Back, Forward, Reload (Stop while the page loads), the address, shown without a web scheme until it is focused, so a narrow area still shows the host, with the zoom badge at its right end while the page is not at 100%, the device size button and Comment.
A page that cannot load says so in its slot with the address and Chromium's reason, and Reload loads its address again.
A page whose renderer stopped says the same with the reason.
In a plain browser tab a browser display shows its address on the toolbar row and reads `Pages open in the Hide desktop app.` below it, and a web address gets Open in browser, which opens it in a new tab.

## Verification

```sh
bash scripts/verify-cargo.sh test
pnpm --dir web test
pnpm --dir desktop test
pnpm --dir desktop e2e
```

`desktop/e2e/browser.spec.ts` drives a desktop app it launched itself against a private hided and an isolated Herdr, never the operator's.
It opens a page with `hide browser open`, checks the native view sits on its slot, posts a sign-in popup's result back to its opener, holds a hostile popup's window features to a framed, closable window on the work area, caps popups and closes them with their page, asks before a navigation, a redirect and a frame hand a link to another app (with macOS's app lookup, sheet and opener stood in for), opens it only on Open and asks nothing more after a Cancel until the page navigates, opens a shift-clicked link as a display beside its page, which stays in view, refuses a popup and an app link from a hidden page, zooms a focused page with the text-size commands and ⌘+ without moving any text size, pinches a page before and after it moves to another renderer, types Hangul into the page, splits and resizes without a reload, freezes the pages under the palette, the Recent Panels list and a divider drag, hides a page with the palette's first frames while the host's capture is held back and turns its still to what the page shows now, hides a page when the lane switches to Sessions and puts it back on its slot when Files shows again, opens a page tab's wrapped hint beside the tab rather than over the page, navigates and goes back, shows a failed load, opens an HTML file from the Explorer, refuses a file outside the checkout, ends a closed page's renderer, restores the page after a relaunch, and shows the notice in a plain browser tab.
The test window never activates the app or takes the keyboard, because the e2e fixture launches every app with `--hide-show-inactive` and `--disable-backgrounding-occluded-windows` (see [BUILD.md](BUILD.md#the-desktop-app)); it is shown behind the operator's windows, keeps painting there, and captures of it are taken by window id.
The zoom test needs the key window: it is tagged `@needs-focus` and brings its window to the front itself.
Keep screenshots and logs under local-only `agents/runs/`.
`desktop/e2e/new-tab.spec.ts` preserves the native menu's target and its resulting core and drawn state in the `menu-target-transitions` attachment, through the existing opt-in probe and WebSocket observation.
It records command and region identifiers, pane presence and close confirmation, with no page address or content; its behavior assertions, waits and retry policy are unchanged.
`desktop/e2e/local-page-rules.spec.ts` opens a local page from an isolated pane on the fused Electron and checks that its folder's image and script load, that reads of files and requests to a loopback server and this machine's own network address fail, that an image drawn from its own folder cannot be read back, and that each refusal is logged once.
`desktop/src/main/browserSync.test.ts` tests the request decision and the address ranges, the device size field and zoom commands a sync may carry, and the emulation a device size asks for at a zoom.
`desktop/e2e/browser.spec.ts`'s `page width` tests open a page into a 520px lane and see it widen to 900px, a width the operator narrows stay through a file tab and back, the widening stop where Agent Views keeps 480px over the agents and docked, and an expanded lane stay; its `page zoom` test zooms a focused page with ⌘=, steps it in the popover with the keyboard left in the shell, sees another page of the host show the zoom and the badge go at 100%, and sees ⌘= take nothing at a device size; its `device size` test reads the page's `window.innerWidth` at 390, 768 and 1280 with the host at 110%, the scaled label in a narrow lane, the size kept through a navigation, the user agent unchanged and Fit back at the zoom.
`desktop/e2e/view-comment-file.spec.ts` turns Comment on for a local HTML page, picks an element and drags an area, Sends both with their screenshots, sees an agent's page command on that page still answer `display_unsupported`, and measures what the comment world reads against the page's own.
`desktop/src/main/browserCdp.test.ts` proves the comment capability's allowlist (every method outside it refused before the page, an evaluation in the page's own world refused, only the four events passed, no discovery), that a capability without the purpose never sees a local file page, and that the purpose is accepted only as `comment` for one display; `hided/src/browser_control.rs` proves that only the comment path names a purpose to the gateway.
`desktop/e2e/browser-cdp.spec.ts` exercises the scoped native gateway against its own candidate, including pre-attachment native file-request cancellation and persistent download cancellation after client disconnect.
Its iframe proof obtains the cross-site iframe's frame-owner ID directly from the candidate WebContents' native DOM, independently of the scoped gateway, and requires both the scoped child frame ID and attached target ID to equal it.
Chromium's `Page.getFrameTree` includes local children only, so an out-of-process child's identity cannot be inferred from the parent session's frame tree.
The proof retains exact parent identity, native frame parentage, distinct renderer processes and child-session runtime execution, then retires the child and requires stale commands to fail while the parent remains usable.
A same-renderer cross-site frame fails that OOPIF proof instead of substituting an in-process interaction or forcing launch flags.
JavaScript and HTTP redirect file attempts must leave native file content unreadable and permit the authorized HTTP page to remain usable; a redirect must reject its navigation caller.
Each JavaScript attempt must return its completion marker without an exception, so an empty result or unrelated script failure cannot satisfy the denial.
The redirect proof independently requires the fixture's single permitted redirect response to finish with status 302 and the forbidden file location, so an unrelated transport failure cannot satisfy the denial.
Chromium may preempt these attempts before the host receives them, so each diagnostic category is capped at one rather than required for those attempts.
The separate app-owned pre-first-load, post-disconnect and native iframe file attempts must each prove guard-caused cancellation with exactly one refusal diagnostic per generation and no file content.
Each run stores its iframe provenance in a new private evidence directory, preserving earlier results.
Its download witness confines the candidate app and owned page session to a private sink before every attempt, observes production cancellation on every native callback, and requires one refusal diagnostic and a sink that is empty once Chromium has deleted its intermediate files.
`web/e2e/browser-agent-assets.spec.ts` runs the agent page assets in plain Chromium on chromux's own cases: masking, state suffixes, stable refs, `--diff`, `--grep`, clickable divs, covering overlays, frame and shadow reach, fill, the refusals of hidden, covered and stale targets, `click --text`, and `changed` ignoring a scroll.
`desktop/e2e/browser-agent.spec.ts` runs `hide browser` from an isolated pane against its own candidate: snapshot, fill and click in a page and its cross-site frame, `--diff`, the overlay, console, network, eval and screenshot, a dialog that holds the display until it is answered, a cross-site frame that never answers, a selected View of a Workspace behind another that still takes input and screenshots, a press that holds the page past a step, and busy, hidden, file and missing displays.
`hided/tests/it/remote_core.rs` (`a_node_window_s_page_is_driven_from_its_machine_off_the_link_and_from_the_core_through_it`, external lane) proves the node's window from a pane on each machine against a scripted gateway on the node's machine: the node's pane is handed that gateway's own capability and snapshots and clicks with the node's daemon relaying it, the core opens no browser relay stream (it logs `link_stream.opened` for each) and the node's relays carried every byte the gateway saw; a page action of the window selects a page another one had replaced, and one from a process that is no window is refused; the core's pane opens, snapshots and clicks the same window through the link; and the link ending makes the gateway revoke the node pane's capability, which worked until then, refuses the node's pane with `hide_unavailable`, and lets the core's pane drive the page again once the link and the window are back.
`desktop/src/main/browserCdp.test.ts` tests the gateway's `/revoke`.

`hided/src/browser_page/fake_gateway.rs` is the scripted gateway the frame, drag release and wait rules of `page.rs` and `actions.rs` are tested against, with a step and a hold for unanswered commands they take as values rather than the production eight and ten seconds; `recording` models the gateway's pending cap, its error answer to a command nobody answers and the requests it saw, so the silence budget and the cap are asserted by counts (how many `Page.enable` a frame got, the most commands pending at once, that a drag's release was sent), never by elapsed time; a frame there can answer a kind of request only once every frame has asked, which proves requests go out together by order, not by time, and `desktop/src/main/browserCdp.test.ts` tests the drag refusal.
The spec also replays a native HTML5 drag through the gateway and sees the page receive its data and no files.
Playwright dismisses a dialog no listener handles, so that test listens and answers it the way the operator would.
`desktop/e2e/remote-browser-agent.spec.ts` runs the same snapshot and click from a device pane over the isolated SSH server, and the Workspace commands' `renderer_unavailable` once the desktop is gone.
`hided/tests/it/remote_core.rs` opens a page on the core's machine from its checkout with no window and with the node's desktop window attached, resolves its route on the node's daemon, reads the core machine's loopback page through the routed address, and sees the route close on release.
`desktop/e2e/remote-workspace.spec.ts` additionally uses an isolated SSH server, whose sessions get the private HOME `desktop/e2e/device-home.ts` proves because connecting installs Hide's kit there, and two private Herdr servers to prove remote CLI origin, HTTP and WebSocket forwarding, absolute loopback subrequests, local and remote cookie separation, remote popup address ownership, relative HTML assets, refusal of undeclared files and external requests, a device agent's open staying behind until the operator is on the device, background View cleanup, route cleanup on close and forced candidate exit, and a return route that comes back after the device's helper connection ends and reconnects, with every remote command run through the `hide` Hide installed and linked on the device.

## Running Workspace servers

The top row’s globe, `Open server`, right of the Overview icon, reuses the existing cwd-attributed pane listener projection.
The projection retains each bind address as well as its port, mapping wildcard listeners to loopback of the same address family.
The current device, checkout ID, path and endpoint are checked again when a choice is made; identical paths on a remote Project never authorize a local listener.
One endpoint uses the existing explicit Workspace `browser_open` route directly, and multiple endpoints use the shared popover picker.
Discovery identifies TCP listeners, so successful discovery does not prove an HTTP page loads; native verification must observe the actual Browser View loading the chosen IPv4/IPv6 URL and its connection-error recovery.
The action starts no server and adds no duplicate context footer.
