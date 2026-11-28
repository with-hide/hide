# 0051. The shortcut registry

- Status: Accepted
- Date: 2026-09-22
- Code: `web/src/shortcuts.ts`, `web/src/keyboard.ts`, `desktop/src/main/menu.ts`

## Decision

### The shortcut registry

`web/src/shortcuts.ts` is one table, command to chord per host, matched on `KeyboardEvent.code` at the window capture phase ahead of xterm and Chrome's defaults and never during IME composition (`web/src/keyboard.ts`).
The `⌘/` sheet is generated from the table.
The table is written in macOS chords; `modChord` gives Windows and Linux theirs (⌘ is Ctrl+Shift, and ⌘ with a ⇧ or ⌥ is Alt+Shift) and `PC_KEYS` the few exceptions, each with its reason, so every reader resolves `systemRegistry(system)` for the system the operator types on (`keySystem` in `web/src/host.ts`, `keySystemOf(process.platform)` in `desktop/src/main`).
A stored set keeps macOS chords (`macChord`), so one set means the same keys on either system; [UI_BEHAVIOR.md: Keyboard shortcuts per system](../UI_BEHAVIOR.md#keyboard-shortcuts-per-system) owns the table on both systems and the terminal copy and interrupt keys (`web/src/keys.ts`).
The close chord uses the page's recorded keyboard owner: the focused View display first, otherwise the owned visible pane, with tools, absent or retired owners producing a diagnostic and no close.
The new tab chord reads the same owner (`newTabPolicy`): a drawn View area gets its New tab, an owned pane's Agent area gets an agent tab, and anything else the Agent active area.
The recorder ignores the focus the shell's last element regains when the host hands the keyboard back from a native page to deliver a menu command (`installKeyboardOwner`), so the command runs with the page's owner; once it has run, `noteCommandDelivered` gives the owner to the shell element that now holds the keyboard.
It never closes a whole tab; tab controls and their menus name that intent explicitly.
The explicit Close pane command still closes the core's focused pane through the same confirmation path.
⌘F follows where the operator works: inside a View area it opens the document's find, anywhere else the focused terminal pane's find bar, even with an editor open beside it; Escape and × end the pane search and give the keyboard back to that pane.
A pane whose agent searches its own conversation (`own_find` on its row, from the one table in `hide-core/src/agent_find.rs`: Claude Code's transcript search and Codex's full-transcript find) sends `pane_find_open` instead, because a full-screen agent keeps its conversation inside itself and the pane's scrollback holds none of it, so the find bar would search only the rows on screen.
The core asks the engine off the lock whether the pane has scrollback: with none it sends the agent's own keys with `Engine::send_keys` and the agent's prompt, steps and count take over, and with some (an agent drawing inline, such as Claude Code's default renderer, which has no search of its own) it answers that the find bar opens; a failure opens the bar with the reason.
The find bar searches up to the pane's last 10,000 lines; a pane holder answers at most 2,000 rows (`hide_holder_protocol::MAX_READ_LINES`), so on the core's own engine the node reads that many (`Holders::read`) and the count says the history was cut.
The scrollback is the only witness the core asks, because it does not ask whether a pane is on the alternate screen: an inline agent whose output has not yet passed one screen has no history either and gets its own keys, which for Claude Code's default renderer opens its transcript without a search while the whole conversation is still on screen.
Claude Code's transcript toggles on the key that opens it, so a screen that already shows its transcript footer gets only the search key; the footer is the one witness the agent offers, and a footer that changed sends the full keys as before.
The answer names the request that asked (`find.opened`), so the shell acts on it once, and only while the page's keyboard owner is the one ⌘F was pressed with: an answer that lands after the operator moved the keyboard (a click into a document or onto another pane) opens no bar and takes no keyboard, where it used to give the keyboard back to the agent's pane and the next keys went there.
Settings > Shortcuts rebinds the editable commands (`EDITABLE_PANE_COMMANDS`: split right and down, directional pane focus, zoom, close pane, larger, smaller and reset text, Overview, the direct sidebar command and the device rail toggle, the next/previous Agent pane or focused View tab and global next/previous Recent Panels cycles, and the eight chordless area focus and resize commands `AREA_COMMANDS`) in the running host's own set.
The core and the desktop menu refuse a stored or reported set past `BINDINGS_CAP` (48, in `hide-core/src/runtime/events.rs` and `desktop/src/main/menu.ts`, changed together); `menu.test.ts` fails when the editable list outgrows it.
Directional pane focus reads the drawn pane rectangles only when its chord or menu command is answered, within the keyboard owner's one canvas, and shares the area's edge-neighbour policy (`areaLayout.ts`), allowing one physical pixel for DOM edge rounding at page zoom.
This is O(visible panes) work per invocation, with no resident task, timer or queue; an actual neighbour dispatches one existing pane-focus action, and an outer edge or a zoomed canvas dispatches none.
The area commands have no default chord on any host, so they appear in Settings and the desktop Pane menu without an accelerator until the operator binds one.
Each host keeps its set in the core apart from the other's, because the hosts reserve different keys: a browser's in `ui_state.browser_shortcut_bindings`, and the desktop app's in `ui_state.shortcut_bindings`, the macOS chord set, in the removed native app's text form (`command+shift+return`) and command names (`increase_text_size`), with `toggle_conversation` kept but never run here; a save that omits `browser_shortcut_bindings` keeps it.
The desktop app honours the operator's existing macOS chord set (user decision 2026-09-26): hided passes the state file the removed native app wrote (`~/Library/Application Support/hide/state.json`) as `shortcut_import_path`, and the core reads only its `shortcut_bindings` field, once, while its own set is empty, then records `shortcut_bindings_imported` so a later reset to defaults stays a reset; a missing file imports nothing and a later launch may still import, and an unreadable one is a diagnostic.
The core owns that import rather than any shell (S10), which is why the mechanism survived the removal of the native app that used to write the file.
The window listener, the sheet and the editor all read one effective registry (`effectiveRegistry`), and a chord is refused in its row before it is saved when it breaks the host's rules or is another command's: in a browser it needs ⌘, ⌥ or ⌃ and may not be one Chrome or macOS keeps; in the desktop app a pane command needs ⌘ and one printable key, Return or an arrow, while a navigation cycle also accepts ⌃ or ⌥ and Tab, and may not be one macOS or the app menu keeps; on Windows and Linux the same rules read ⌘ as Ctrl+Shift, refuse the Windows or Super key, and refuse the chords Chrome and the system keep there; a stored set that fails them is dropped whole and the defaults run with a diagnostic.
While a row records, the listener runs no command, and IME composition never records.
The Electron column is the macOS chord set the desktop app uses, with ⌘/ kept from the browser because the macOS set has none: the desktop app has no browser keeping chords, so the moved ones return to ⌘.
Each surface reads the running host's column (`web/src/host.ts`: `window.hideHost` means the desktop app): the window listener, the ⌘/ sheet, which drops the "moved for Chrome" note there, and every hint that names a chord.
The desktop app menu is built from the same column for the running system with the stored set applied (`desktop/src/main/menu.ts`), and carries macOS's Services and Hide items only there: the shell reports the set over the bridge when it changes and the menu is rebuilt only when its chords do; a click reaches the same `run` path as a chord, and a chord the listener answers calls `preventDefault` in the page.
The Agent pane or View tab cycles use ⌃Tab in Electron and ⌥Backquote in a browser, with ⇧ for backward; global Recent Panels has distinct command IDs and no default chord, and Recent Projects retains ⌥Tab on macOS.
`areaCycle.ts` routes a View keyboard owner to that area's tabs; every other owner, including sidebar, tools and screens without a drawn Agent area, gets the existing bounded recent Agent pane list across devices, projects and checkouts.
Only an actually focused drawn Agent pane supplies the cycle origin and records a pane visit; a focused View with no other tab stays a no-op rather than falling through to Agent panes.
Main and Overview ignore a View owner left recorded when a shortcut unmounted the Workspace, because no file holds the keyboard on those screens.
One frozen cycle is held in `ui.ts`; membership changes prune candidates, a disappeared View scope cancels, and release dispatches one pane-focus action for an Agent or one `view_layout focus` for a View.
`viewFocus.ts` publishes only owner changes to a bounded set of mounted area subscribers; Agent Views and the lane derive their foreground-colored tab underline from that one owner.
Native browser `before-input-event` matches the same effective registry before the page or menu consumes a cycle key and forwards input through the existing trusted browser bridge.
The host retains at most one held page/release modifier with a cycle identifier, clears it on matching shell completion, window blur, page failure or destruction, and rejects starts from hidden or unfocused pages.
Release from another native page keeps the initiating scope, and a late completion cannot clear a newer hold.
Committed View cycles request keyboard follow after the core confirms selection; a native-page focus intent is bounded to one slot and delivered after visible-slot sync, only in an already-focused window.
Cycle menu items carry no accelerator, so native page and renderer input have one owner; a menu click selects immediately through the same command path.
This adds no per-key subprocess, file I/O, second history, notifier fan-out or unbounded queue; the held preview changes page-local state, and only a committed selection reaches the core.
The numbered chords (`select_tab_1..9`, `select_agent_1..9`, `select_device_1..9`) exist only in the Electron column: their target is the screen order the page knows (`web/src/numbering.ts`: the tabs of the Agent area in use, ⌘9 its last, read from the frame the Agent column last drew, the single sidebar's physical pane occurrences, with raised duplicates assigned only at their first occurrence, and the device rail's tiles, the window's machine first, whether or not the rail shows), so the page answers them and the app menu carries no item or accelerator for them (`KEYBOARD_ONLY`); a pane chord rebound onto ⌘n or ⌥⌘n is refused as that command's, and a stored one already there is dropped alone.
The hold hint is a pure state machine (`web/src/hints.ts`, ported from the removed native app's `HideHintState`: one modifier set, one deadline, revealed after 150 ms of the exact hold) fed by the same window listener from keydown, keyup, blur and visibilitychange, and cleared by any key during the hold or by a layer opening in the ui store; the listener publishes only the family the state reveals (`ui.hint`: `tabs`, `agents`, `devices` or null), computed from the registry's chords for the running host, so a browser reveals nothing and an unrevealed hold renders nothing.
The keycap is `web/src/components/ui/keycap.tsx`, an absolutely positioned overlay the Agent tabs, the sidebar agent row and the device rail's tiles draw only while their family is revealed.
The ⌘/ sheet folds each family into one row with its range (`sheetRows`), and Settings > Shortcuts lists all three read-only.
The design relies on Electron handing a key to the page first and to the menu only when the page leaves it unhandled, so one press runs one command; that order is an assumption, not a verified fact here.
The e2e covers each path alone (a Playwright chord, a main-process menu click, one `create_tab` each), neither reaches NSMenu, and a native keystroke against the app has not been checked: if one press runs twice, the listener stops answering the chords the menu owns in Electron.

The table below is written in macOS chords; Windows and Linux press each one through the rule and its exceptions ([UI_BEHAVIOR.md: Keyboard shortcuts per system](../UI_BEHAVIOR.md#keyboard-shortcuts-per-system)).

| Command | macOS | Browser | Electron |
| --- | --- | --- | --- |
| New tab | ⌘T | ⌥T (moved: Chrome reserves ⌘T) | ⌘T |
| Select agent 1-9 (the nth row the Agents list draws) | ⌥1-⌥9 | none | ⌥1-⌥9 |
| Select tab 1-9 (the nth tab of the Agent area in use, ⌘9 the last) | ⌘1-⌘9 | none (Chrome keeps ⌘1-9) | ⌘1-⌘9 |
| Select device 1-9 (the nth tile of the device rail) | ⌥⌘1-⌥⌘9 | none | ⌥⌘1-⌥⌘9 |
| Close focused View or pane | ⌘W | ⌥W (moved) | ⌘W |
| Reopen closed tab | ⌘⇧T | ⌥⇧T (moved) | ⌘⇧T |
| Add project | ⌘⇧N | none (Chrome keeps ⌘⇧N); the Add project row under Overview and the empty Home's button open it | ⌘⇧N |
| Start agent (the start panel) | - | none (Chrome keeps ⌘N); ⌘K `에이전트 시작…`, found by typing, opens it | ⌘N, also File › Start agent |
| Next / previous recent tab (the Agent pane or focused View tab) | ⌃Tab / ⌃⇧Tab | ⌥` / ⌥⇧` | ⌃Tab / ⌃⇧Tab, committed on releasing ⌃ |
| Next / previous panel (all projects) | none by default | none by default; bindable in Settings | none by default; the menu selects immediately |
| Next / previous recent project (Recent Projects) | ⌥Tab / ⌥⇧Tab | ⌥Tab / ⌥⇧Tab | ⌥Tab / ⌥⇧Tab |
| Search, Open file | ⌘K, ⌘P | same chords, answered by the palettes | same chords |
| Overview | ⌘⇧O | shared modal or page | same chord |
| Add to backlog (the quick add) | ⌘⇧M | ⌥⇧M (moved: Chrome keeps ⌘⇧M for its profiles) | ⌘⇧M |
| Sidebar | ⌘⇧P | show the single sidebar and focus it | same chord |
| Save file | ⌘S | ⌘S | ⌘S |
| Toggle left sidebar, Find in pane, Keep open | ⌘B, ⌘F, ⌘⇧K | same chords | same chords |
| Toggle right lane | - | ⌘E | ⌘E |
| Toggle Explorer tool | - | ⌘⇧E | ⌘⇧E |
| Split right / down | ⌘D / ⌘⇧D | ⌘D / ⌘⇧D | ⌘D / ⌘⇧D |
| Zoom pane (Expand while the keyboard is in the right lane) | ⌘⌥↩ | ⇧⌘↩ | ⇧⌘↩ |
| Close pane | ⌘⇧W | ⌥⇧W (moved: Chrome reserves ⌘⇧W) | ⌘⇧W |
| Larger / smaller / reset text | ⌘= / ⌘- / ⌘0 | same chords; while an image or PDF display holds the keyboard they zoom it in Chrome's steps and ⌘0 returns it to its fit | same chords, and the same for an image or PDF; while a browser display's page holds the keyboard they zoom that page in Chrome's steps and it keeps the keyboard, and ⌘+ (⌘⇧=) zooms it in too (`docs/BROWSER_DISPLAYS.md`) |
| Move to Trash | ⌘⌫ (Explorer tree only) | not intercepted; a terminal gets ^U | same as the browser |
| Settings | ⌘, | ⌥, (moved: Chrome keeps ⌘,) | ⌘, (in the app menu) |
| Keyboard shortcuts | - | ⌘/ | ⌘/ |
