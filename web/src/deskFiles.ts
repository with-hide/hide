// The files Desk's room holds in the core (PRD desk-2-ui D-22, B19, B36,
// B37). The room's tabs are the window's (`deskRoom.ts`); each file tab's
// document is the core's, held by one `desk_file` event per change of the
// room: a file tab added opens its document, letting go the preview it
// replaced in the same event, and file tabs gone (a tab's `✕`, the panel
// closed, another session, the room left) let theirs go together. The same
// file shown twice is one document, and a tab brought to the front sends
// nothing.

import { tabKey, type DeskRoomWindow, type DeskTab } from "./deskRoom";
import { useUiStore } from "./ui";

type FileTab = Extract<DeskTab, { kind: "file" }>;

/** A file named as the core holds it: its checkout's ids and its path. */
export type DeskFileRef = { workspace_id: string; checkout_id: string; path: string };

export type DeskFilePayload =
  | ({ action: "open"; device_id?: string; replace?: DeskFileRef } & DeskFileRef)
  | { action: "close"; files: DeskFileRef[] };

function fileTabs(room: DeskRoomWindow): Map<string, FileTab> {
  return new Map((room.panel?.tabs ?? []).filter((tab): tab is FileTab => tab.kind === "file").map((tab) => [tabKey(tab), tab]));
}

/**
 * The one `desk_file` payload a room change sends, or null when its file
 * tabs did not change. `place` names a tab's checkout, null when it is no
 * longer listed: such a file opens nothing, and the core forgets it with
 * its checkout. `remote` gives the device id an open names for a device's
 * file, absent for this machine's.
 */
export function deskFileChange(
  before: DeskRoomWindow,
  after: DeskRoomWindow,
  place: (tab: FileTab) => { workspace: string; checkout: string } | null,
  remote: (tab: FileTab) => string | undefined,
): DeskFilePayload | null {
  const was = fileTabs(before);
  const now = fileTabs(after);
  if (was.size === 0 && now.size === 0) return null;
  const ref = (tab: FileTab): DeskFileRef | null => {
    const at = place(tab);
    return at ? { workspace_id: at.workspace, checkout_id: at.checkout, path: tab.path } : null;
  };
  const gone = [...was].filter(([key]) => !now.has(key)).flatMap(([, tab]) => ref(tab) ?? []);
  const added = [...now].filter(([key]) => !was.has(key)).map(([, tab]) => tab);
  const opened = added.length === 1 ? ref(added[0]!) : null;
  if (opened) {
    const device = remote(added[0]!);
    // An open replaces at most the preview tab (B36).
    return { action: "open", ...opened, ...(device ? { device_id: device } : {}), ...(gone.length === 1 ? { replace: gone[0] } : {}) };
  }
  return gone.length > 0 ? { action: "close", files: gone } : null;
}

/**
 * Calls `changed` once for each room this window moves to, with the room it
 * left. It keeps the last room it saw itself: a store change made while the
 * store announces another (the room closing as the screen leaves Desk, in
 * `ui.ts`) would otherwise reach a listener twice.
 */
export function watchDeskRoom(changed: (before: DeskRoomWindow, after: DeskRoomWindow) => void): () => void {
  let last = useUiStore.getState().deskRoom;
  return useUiStore.subscribe((state) => {
    if (state.deskRoom === last) return;
    const before = last;
    last = state.deskRoom;
    changed(before, last);
  });
}
