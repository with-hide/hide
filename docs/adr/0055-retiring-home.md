# 0055. Retiring Home

- Status: Accepted
- Date: 2026-11-09
- Code: `hide-host/src/home.rs`, `hide-core/src/runtime/home_retire.rs`

## Decision

### Retiring Home

Hide keeps no Home: an agent or a tab starts in a registered project's checkout, and a start with no project offers Add a project (PRD retire-hide-home D-01, D-02).
Older releases kept `~/hide` on each device, with one link per registered project, a Hide `AGENTS.md`, a `CLAUDE.md` link to it and the marker `.hide-home.json` naming every link Hide made; this release removes what that marker lists, once per device (`runtime/home_retire.rs`, `hide-host/src/home.rs`).
The store the release before wrote flags one registration per device as Home; `persistence` reads the flag once and records a `HomeRetirement` in `ui_state.home_retire` (not serialized to the shell), leaving the registration, an ordinary pinned project, exactly as stored until the helper has looked at the folder (B8, B13).
For each pending device the core asks its helper once, on a worker with the runtime lock released, for `home_retire` (`Call::HomeRetire { keep_empty, settle }`, helper protocol 87; this machine's runs in process) when the helper is ready and the core has seen its projects; a helper older than 87 is not asked and is asked when its updated helper becomes ready (D-06), because connecting replaces a helper whose digest differs.
`keep_empty` is true when the core recorded a session or an open pane in that registration (`home_kept_open`), which keeps the folder as a project even when nothing else is there (D-04, D-08).
The helper removes only what the marker lists: a link while it is still a symlink to the target the marker recorded, `AGENTS.md` only while it is exactly the guide an earlier build wrote (its head, one of two introductions, one bullet per project in the order of the names, its tail; on Windows the marker lists no link and the bullets stand alone), `CLAUDE.md` while it is still Hide's link or import, and the marker last (D-11, B10).
A folder with no marker, a link the operator changed, a guide or a file they touched in any way stay where they are.
A folder that stays keeps its marker through the first call; its answer (`kept`, `marked`) makes the core rename a registration still named `Home` to `hide` and save it, and only then does the core send the second call (`settle`), which removes the marker; a stop between the two asks the first call again with the same answer, so the name and the marker converge (B10).
A cleanup a helper tried and could not finish (an unreadable marker, a refused permission, a file or folder it cannot read, a session store it cannot read) leaves the project named `hide` as well (B11).
When only the marker is left and nothing keeps the folder, Hide first records, in its own store (`~/.hide/home-retiring.json`, written durably), the name it will set the folder aside under (`~/.hide-home-retiring-<token>`), the folder's identity and the marker's text, and then renames `~/hide` to that name without replacing anything; the marker and that folder are removed there. The record, and the folder still being the recorded one, is the only claim Hide has on a folder of that name: a stop after the rename is finished by the next run, which finds no `~/hide`, while a folder that merely has such a name, or took the name before the rename, is the operator's and stays (the stale record is dropped). A leftover that is empty or holds only the marker goes; one that also holds a file that arrived, even after the marker was removed, goes back to `~/hide` whole with its marker (written whole to Hide's own file beside the record and renamed into the empty slot without replacing; a link or any non-file in that slot is never followed or replaced, and fails the call), so the call still answers marked and kept until the core has saved the new name. A `~/hide` that exists again is never replaced: the folder and the record stay, the call fails and a later run finishes the return.
A `~/hide` with no marker, empty or not, is never removed.
The automations' own folder `~/hide/automations` is Hide's, not the operator's: a folder whose only other entry is the marker loses the marker, stays where it is and is answered not kept, so its registration goes and no `hide` project appears (PRD automations D-09, B31).
A marker-less `~/hide` that holds only that folder is answered the same and left untouched, so a stop after the marker went converges.
The answer (`HomeRetired`: the folder, whether it is `kept`, whether it was `marked`, and the `removed` and `changed` entries) settles the registration: a folder that is gone takes its registration with it, and a folder with no marker changes nothing (B8).
Each retirement holds an exclusive lock on the account's home folder, waits at most 20 s for another and then answers `busy`; the folder is owner-only.
A failure (an unreadable marker, a refused permission, a file or folder it cannot read, a session store it cannot read, a timeout) keeps the project, reaches only the diagnostic log (`home.retire_failed`, design principle 13) and is asked again at the next launch; a refusal is not asked again in the same launch.
Removing a device, or losing its connection, leaves its `~/hide` and its agents on it; registering it again continues them.
The consent contract does not change: the helper's scope already includes `~/hide`, and the add-device wording no longer names it.
