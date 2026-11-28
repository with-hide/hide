# hide browser

Read and drive a browser display in hide, the one the operator is looking at.
Every command works on one display of your own checkout, by its View id from `hide view list` (for example `browser-3`).
Open a page with `hide browser open <url-or-path>`; the display stays when a command ends.
When its answer carries `browser_control` (no CDP URLs for the page), do what its `next_action` says: the commands below when they drive the page, its `reason` when you may not drive it, and `hide browser connect --display <id>` when it says to retry; never open the page again, which reloads it.
A web page, a blank one, and an HTML file are driven alike: a file of your checkout, and an HTML file outside every checkout that you opened yourself.

## Commands

    hide browser snapshot <display> [--interactive] [--diff | --grep <pattern>]
    hide browser click <display> (@ref | --text <label> | --xy <x> <y> [--space css|image])
    hide browser fill <display> @ref <text>
    hide browser type <display> <text>
    hide browser press <display> <key>
    hide browser hover <display> (@ref | --xy <x> <y> [--space css|image])
    hide browser drag <display> (@ref | <x>,<y>) (@ref | <x>,<y>) [--mode auto|pointer|html5] [--space css|image]
    hide browser scroll <display> (up | down | @ref)
    hide browser wait <display> (--text <text> | --selector <selector|@ref> [--gone]) [--timeout <ms>]
    hide browser screenshot <display> <path> [--ref @ref]
    hide browser eval <display> <expression>
    hide browser console <display>
    hide browser network <display>

click, fill, type, press, hover, drag and scroll also take `--verify <ms>` (how long to wait before checking what changed, default 300, at most 10000) or `--no-verify`.
`wait --timeout` defaults to 5000 and is at most 60000.
`press` knows Enter, Tab, Escape, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp and PageDown.
Put `--` before a value that starts with `--`.

## Reading a page

`snapshot` prints the title, the address, a blank line, and one indented line per element:

    # Sign in
    # http://127.0.0.1:3000/login

    @1 textbox "Email" [email]
    @2 textbox "Password" [password]
    @3 textbox "Remember me" [checkbox checked]
    @4 combobox "Plan" = "Monthly"
    @5 button "Sign in" (disabled)
    @6 link "Forgot?" -> /reset

Each line carries the element's current state: checked, disabled, a select's chosen value, a link's target.
A field with no label of its own shows its value, or its placeholder while empty.
Password, card number, CVC, one-time code and similar values are never shown; you see the placeholder or label instead.
`overlay (covers page; interact or dismiss first)` means a modal or cookie wall covers the page: act on it before anything under it.
`(clickable)` marks a control with no role that still takes clicks.
`--interactive` keeps only the lines you can act on.
`--grep <pattern>` keeps the matching lines and the lines above them; a pattern that is not a valid regular expression is matched literally.

## Refs

`@N` names an element of the top document, including open shadow roots and same-origin frames.
A ref stays the same across snapshots of the same document, new elements get the next numbers, and a navigation starts again at `@1`.
A cross-origin frame shows as its own section, `# OOPIF <tag> origin=<origin>`, and its elements are `@<tag>:N`; its values and links show only their origin.
A frame that answers nothing in time shows as `# OOPIF unresponsive origin=<origin>` with no refs, and the rest of the page is read; its script may never yield, or a dialog it opened is waiting for the operator.
A frame that did not answer is asked again by the next read, so one that was only slow shows up in the next snapshot or poll.
After two silent rounds in a row a command leaves it alone until the command ends, so a frame that never answers costs a step or two, not one on every read; the next command asks it afresh.
`wait` ends at its own timeout and names the frames it could not read, and it never reports a frame that did not answer as one that lacks the text.
An action that could not read a frame says so instead of reporting that nothing changed.
`type` or `press` that cannot tell which frame holds the focus, because a frame did not answer, sends nothing and fails `page_unresponsive`.
A frame that is read but gives no answer to the look for the frames inside it keeps its section and refs, and `# OOPIF unlisted origin=<origin>` follows the top document: what is inside it is not shown, and not known to be absent.
A ref from an older document, or from a frame that navigated, fails `ref_stale` instead of touching another element: take a fresh snapshot.
While a frame that did not answer may be the ref's frame, the ref cannot be told stale, so it fails `page_unresponsive` with the same advice to take a fresh snapshot.

## Acting and checking

An action answers with one line of JSON: what it did, `changed`, and `next`, the command to look again.
`changed` lists the snapshot lines the action added (`+`) or removed (`-`), checked again for about two seconds on a slow page.
"no visible change within ~2s" means the action was sent but nothing showed yet: confirm with `wait` or `snapshot --diff` before you repeat it, or you may submit twice.
After three actions in a row that changed nothing, the answer says you are likely stuck: try another element, dismiss what covers the page, or hand off to the operator.
`snapshot --diff` shows only the lines added and removed since your last snapshot of the same kind, says when there is no earlier snapshot or the address changed, and says so in one line when nothing changed.

The usual loop:

    hide browser snapshot browser-3 --interactive
    hide browser fill browser-3 @1 "a@b.c"
    hide browser click browser-3 @5
    hide browser wait browser-3 --text "Welcome"
    hide browser snapshot browser-3 --diff

`click --xy` and `hover --xy` take CSS pixels of the viewport; with `--space image` they take pixels of a screenshot taken now (`screenshot` reports the mapping as `css_to_image`).
`drag --mode auto` uses a native HTML5 drag when the source is draggable and pointer events otherwise; both ends must be in the same frame.

## What the operator sees

The display stays on the operator's screen while you work, and an arrow cursor in hide's accent color shows each action: it glides to the target, a click ripples, a drag leaves a line, filled fields flash, pressed keys are named, scrolls show an arrow.
It fades two seconds after the last action, lets clicks through, and never appears in a snapshot.
No command moves the operator's mouse, changes which View is in front, or takes keyboard focus.
Input and screenshots need the display to be its area's selected View on a desk, or a page on no desk (one you opened while the operator watched another pane), which takes input as it is: one behind another tab of its area on a desk fails `display_hidden`, and reading commands still work there. A page on no desk the operator has never shown has no pixels yet, so `screenshot` on it fails `display_hidden`.
A page you open from a pane the operator is not watching waits in your pane's Views list, still loaded, and every command here works on it; only the operator brings it onto their screen.
A selected View of a Workspace that is not in front takes input and screenshots like any other, though the operator does not see the action.

## When something fails

A failure prints one line of JSON, `{"ok":false,"reason":...,"display":...,"next_action":...}`, and exits non-zero.
Follow `next_action`; the reasons you will meet:

| reason | what to do |
| --- | --- |
| `ref_stale` | Take a fresh snapshot and use its refs. |
| `target_hidden` | The element is not visible; find a visible one in a snapshot. |
| `target_covered` | Another element (named in `detail`) covers it; act on that first. |
| `target_outside_viewport`, `outside_viewport` | Scroll the element into view, or use a point inside the viewport. |
| `text_not_found`, `text_ambiguous` | Use `snapshot --grep`, then click by `@ref`. |
| `target_not_fillable`, `option_missing` | Fill a field that takes a value, with a listed option. |
| `fill_rejected` | The page shows something other than the text; look with `snapshot --diff` before inserting it again. |
| `key_unsupported` | Press one of the listed keys. |
| `invalid_selector` | Use a valid CSS selector or an `@ref`. |
| `drag_across_frames` | Drag between two points of the same frame. |
| `drag_carries_files` | The drag carries local files, which `hide browser` never drops on a page; do not retry it (`--mode pointer` would start a native drag the gateway cannot see), and ask the operator if the page needs them. |
| `drag_not_started` | The source did not start a native drag; retry with `--mode pointer`. |
| `timeout` | The condition did not hold in time; look at a snapshot. |
| `eval_error` | Fix the expression. |
| `cdp_error`, `page_script_failed` | The page changed under the command; take a fresh snapshot and retry. |
| `screenshot_unwritable` | Choose a path in a writable folder. |
| `screenshot_failed` | Retry; if it keeps failing, check the display with `hide view show`. |
| `dialog_open` | A JavaScript dialog, or a script that never yields, holds the page; the operator must answer it in hide. Dialogs are never accepted for you. |
| `display_hidden` | Ask the operator to show the display. |
| `display_busy` | Another debugger (agent-browser, Playwright) or another `hide browser` command holds the display; let it finish. |
| `display_unsupported` | Only http(s), blank and HTML file pages can be driven; open the page with `hide browser open <url-or-path>`, and read other outside files with `hide file open`. |
| `file_page_pane_only` | Only an agent or a shell in a Hide pane drives a file page; serve its folder over http and open that address to drive it from here. |
| `outside_page_not_opened` | Only the agent that opened an outside page drives it, and Hide knows that agent only when it reads the agent's sessions, never for a plain shell; serve the file's folder over http and open that address to drive it. |
| `file_page_elsewhere` | A file page is driven only from a pane on the computer whose Hide window loads it; serve its folder over http and open that address. |
| `display_missing`, `display_closed` | Run `hide view list` and choose a current browser display. |
| `page_unresponsive` | The page did not answer in time; retry or ask the operator. For a ref whose frame may be one that did not answer, take a fresh snapshot (`hide browser snapshot <display>`) and use its refs, since retrying a stale ref repeats the failure. After an input (click, type, press, drag, scroll) the event was sent and may already have taken effect: look with `snapshot --diff` before repeating it. |
| `browser_relay_limit`, `browser_control_busy` | Other `hide browser` commands or CDP clients are running; let one finish. |
| `browser_relay_message_limit`, `browser_limit` | A message crossed a size or rate limit; capture a region with `screenshot --ref`, narrow the request, or wait a minute. |
| `browser_control_unavailable` | Reconnect the Hide desktop app. |
| `browser_window_unreachable` | The machine whose window shows the display is not reachable; retry once it is back. |

Network rows come from the page's resource timing: no request or response headers or bodies.
`console` shows the messages the current document logged, newest first.
