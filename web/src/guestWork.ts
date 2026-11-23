// Another machine's work as this window draws it (PRD device-connect-iroh
// D-50, D-74, B10, B49, B68): the panes a paired machine's work runs in on
// this computer (`navigator.guest_panes`), and what a connected machine this
// one opens told of its own Needs You and Done (`navigator.peer_work`). The
// core and hided own every value; this only places and words them, so each
// rule is tested without a browser.
//
// A pane of another machine's work is none of this core's: it has no tab,
// row or state here. A window reads it, types into it, sizes it and closes
// it, and everything else is done on the machine that asked for the work.

import { tileMonogram } from "./devices";
import type { MessageKey } from "./i18n/catalogs";
import type { GuestPanes, PeerWork, PeerWorkRow, SnapshotRest, Workspace } from "./snapshot";

/** One pane of another machine's work on this computer, as a row and a tab draw it. */
export type GuestPane = {
  /** Its address here, `guest:<device>:pane:<pane>`: what the terminal is drawn and typed into by. */
  id: string;
  /** The machine that asked for the work. */
  deviceId: string;
  key: string;
  machine: string;
  /** The machine's letters, as its rail tile would draw them (`MB`). */
  mark: string;
  folder: string | null;
  /** What the row is called: the name its machine shows it by, else the folder's own name, else the pane's id there. */
  label: string;
  /** This computer's project that holds the folder, when one does. */
  projectId: string | null;
  /** That project's checkout that holds the folder, by its longest path: the row the work stands under. */
  checkoutId: string | null;
};

const NO_GUESTS: GuestPanes[] = [];
const NO_WORK: PeerWork[] = [];

/** `path` is `root` itself or below it, by whole names. */
function within(path: string, root: string): boolean {
  const trimmed = root.replace(/\/+$/, "");
  return trimmed !== "" && (path === trimmed || path.startsWith(`${trimmed}/`));
}

function folderName(folder: string): string {
  const names = folder.split("/").filter(Boolean);
  return names[names.length - 1] ?? folder;
}

/** The name a paired machine goes by: the one its pairing recorded, else its device id. */
export function peerName(rest: SnapshotRest | null | undefined, deviceId: string): string {
  return rest?.navigator?.peers?.find((peer) => peer.device_id === deviceId)?.name ?? deviceId;
}

/**
 * Every pane of another machine's work on this computer, oldest first within
 * its machine. `projects` are this computer's own: a pane started in a folder
 * one of them holds stands under that project, by the longest checkout path
 * that holds it.
 */
export function guestPanes(rest: SnapshotRest | null | undefined, projects: readonly Workspace[]): GuestPane[] {
  const guests = rest?.navigator?.guest_panes ?? NO_GUESTS;
  if (guests.length === 0) return [];
  const placeOf = (folder: string | null): { projectId: string | null; checkoutId: string | null } => {
    let best: { projectId: string; checkoutId: string; length: number } | null = null;
    if (folder === null) return { projectId: null, checkoutId: null };
    for (const project of projects) {
      for (const checkout of project.checkouts) {
        if (!within(folder, checkout.path)) continue;
        if (best === null || checkout.path.length > best.length) best = { projectId: project.id, checkoutId: checkout.id, length: checkout.path.length };
      }
    }
    return { projectId: best?.projectId ?? null, checkoutId: best?.checkoutId ?? null };
  };
  return guests.flatMap((guest) => {
    const machine = peerName(rest, guest.device_id);
    return [...guest.panes]
      .sort((left, right) => left.since_unix_ms - right.since_unix_ms)
      .map((pane) => {
        const folder = pane.folder ?? null;
        const label = pane.name?.trim() || (folder ? folderName(folder) : pane.pane);
        return { id: pane.id, deviceId: guest.device_id, key: guest.key, machine, mark: tileMonogram(machine), folder, label, ...placeOf(folder) };
      });
  });
}

/**
 * Where `projectId`'s panes of other machines' work stand in a list that
 * draws `rows`, the project's checkouts it shows as rows: each under the
 * checkout that holds its folder, after that checkout's own rows, and the
 * rest (a checkout folded away, or the project's own row standing for it)
 * at the project's end, so none is hidden by a fold (B10).
 */
export function guestPlaces(panes: readonly GuestPane[], projectId: string, rows: ReadonlySet<string>): { byCheckout: ReadonlyMap<string, GuestPane[]>; atEnd: GuestPane[] } {
  const byCheckout = new Map<string, GuestPane[]>();
  const atEnd: GuestPane[] = [];
  for (const pane of panes) {
    if (pane.projectId !== projectId) continue;
    if (pane.checkoutId !== null && rows.has(pane.checkoutId)) byCheckout.set(pane.checkoutId, [...(byCheckout.get(pane.checkoutId) ?? []), pane]);
    else atEnd.push(pane);
  }
  return { byCheckout, atEnd };
}

/** The panes that stand under no project of this computer, by the machine that asked for them (B10). */
export function guestGroups(panes: readonly GuestPane[]): { deviceId: string; machine: string; panes: GuestPane[] }[] {
  const groups = new Map<string, { deviceId: string; machine: string; panes: GuestPane[] }>();
  for (const pane of panes) {
    if (pane.projectId !== null) continue;
    const group = groups.get(pane.deviceId) ?? { deviceId: pane.deviceId, machine: pane.machine, panes: [] };
    group.panes.push(pane);
    groups.set(pane.deviceId, group);
  }
  return [...groups.values()];
}

/** The panes a tab strip draws beside `pane`: the same machine's in the same place, the project or the machine's own group. */
export function guestTabs(panes: readonly GuestPane[], pane: GuestPane): GuestPane[] {
  return panes.filter((other) => other.deviceId === pane.deviceId && other.projectId === pane.projectId);
}

/**
 * What stands in a closed pane's place (B10): the next tab of that work in
 * the same place, else the one before it, else the project row it stood
 * under, else nothing of that work. Never the Overview.
 */
export type AfterGuestClose = { kind: "pane"; paneId: string } | { kind: "checkout"; projectId: string; checkoutId: string } | { kind: "workspace" };

export function afterGuestClose(panes: readonly GuestPane[], pane: GuestPane): AfterGuestClose {
  const tabs = guestTabs(panes, pane);
  const at = tabs.findIndex((tab) => tab.id === pane.id);
  const next = tabs[at + 1] ?? (at > 0 ? tabs[at - 1] : undefined);
  if (next) return { kind: "pane", paneId: next.id };
  if (pane.projectId !== null && pane.checkoutId !== null) return { kind: "checkout", projectId: pane.projectId, checkoutId: pane.checkoutId };
  return { kind: "workspace" };
}

/** Whether `id` is the address of another machine's pane on this computer. */
export function isGuestPaneId(id: string): boolean {
  return id.startsWith("guest:");
}

/**
 * What a window does to another machine's pane (D-74): the closed set. The
 * rest of a pane's menu is drawn dimmed with the machine to do it on, whether
 * or not that machine is connected.
 */
export const GUEST_PANE_OFF = ["split_right", "split_down", "new_tab", "rename_tab", "start_agent"] as const;
export type GuestPaneOff = (typeof GUEST_PANE_OFF)[number];

/** Whether a Workspace command is one a window does not do to another machine's pane. */
export function guestPaneRefuses(command: string): command is GuestPaneOff {
  return (GUEST_PANE_OFF as readonly string[]).includes(command);
}

/** One row another machine told of, with the machine it is on. */
export type PeerRow = { row: PeerWorkRow; deviceId: string; machine: string };

/**
 * A connected machine's own Needs You or Done rows, as it told them (D-50,
 * B49): listed after this computer's, each named for its machine. A machine
 * that is no longer connected tells nothing, so its rows are gone with it.
 */
export function peerRows(rest: SnapshotRest | null | undefined, group: "needs_you" | "done"): PeerRow[] {
  const work = rest?.navigator?.peer_work ?? NO_WORK;
  return work.flatMap((peer) => {
    const machine = peerName(rest, peer.device_id);
    return (peer.rows ?? []).filter((row) => row.group === group).map((row) => ({ row, deviceId: peer.device_id, machine }));
  });
}

/**
 * Why another machine's row waits, in this window's words, when that
 * machine's core sent no sentence of its own: the same words this
 * computer's own Needs You row says for the ask (`agentSessions.ask.*`).
 * A Done row, or a code this build does not know, has none.
 */
export function peerReasonKey(row: Pick<PeerWorkRow, "group" | "status_code">): MessageKey | null {
  if (row.group !== "needs_you") return null;
  if (row.status_code === "approval") return "agentSessions.ask.approval";
  if (row.status_code === "question") return "agentSessions.ask.answer";
  return null;
}
