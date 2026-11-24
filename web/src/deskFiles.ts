// The files Desk's room holds in the core (PRD desk-2-ui D-22, B19, B36,
// B37). The room's tabs are the window's (`deskRoom.ts`); each file tab's
// document is the core's. hided owns the hold for this window's connection:
// the window sends the whole set of files its room shows (`desk_files`)
// whenever that set changes and each time it connects, and hided lets the
// set go when the connection closes, however the window went away. The same
// file shown twice is one document, and a tab brought to the front sends
// nothing.

import { tabKey, type DeskRoomWindow, type DeskTab } from "./deskRoom";
import { checkoutAtRoot, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

type FileTab = Extract<DeskTab, { kind: "file" }>;

/** A file as hided holds it: its checkout's ids, the checkout's device, and its path. */
export type DeskFileRef = { workspace_id: string; checkout_id: string; device_id: string; path: string };

/**
 * The files `room` shows, each named by the checkout the catalog lists at
 * its root. A tab whose checkout is not listed is left out; its view says so.
 */
export function deskRoomFiles(room: DeskRoomWindow, rest: SnapshotRest): DeskFileRef[] {
  const seen = new Set<string>();
  const files: DeskFileRef[] = [];
  for (const tab of room.panel?.tabs ?? []) {
    if (tab.kind !== "file" || seen.has(tabKey(tab))) continue;
    seen.add(tabKey(tab));
    const at = checkoutAtRoot(rest, (tab as FileTab).device_id, tab.root);
    if (at) files.push({ workspace_id: at.workspace, checkout_id: at.checkout.id, device_id: tab.device_id, path: tab.path });
  }
  return files;
}

/**
 * Calls `send` with the room's whole file set each time it changes, and
 * again each time the connection comes live, once a snapshot names the
 * checkouts: sent before that, the set would be empty and let every file go.
 * A tab whose checkout appears later is sent then.
 */
export function watchDeskFiles(send: (files: DeskFileRef[]) => void): () => void {
  let sent: string | null = null;
  const check = () => {
    const shell = useShellStore.getState();
    if (shell.connection !== "live") {
      // The next connection holds nothing until this window says so.
      sent = null;
      return;
    }
    if (!shell.rest) return;
    const files = deskRoomFiles(useUiStore.getState().deskRoom, shell.rest);
    const key = JSON.stringify(files);
    if (key === sent) return;
    sent = key;
    send(files);
  };
  const unsubscribeRoom = useUiStore.subscribe((state, previous) => {
    if (state.deskRoom !== previous.deskRoom) check();
  });
  const unsubscribeShell = useShellStore.subscribe((state, previous) => {
    if (state.connection !== previous.connection || state.rest?.navigator !== previous.rest?.navigator || state.rest?.status?.remote !== previous.rest?.status?.remote) check();
  });
  check();
  return () => {
    unsubscribeRoom();
    unsubscribeShell();
  };
}
