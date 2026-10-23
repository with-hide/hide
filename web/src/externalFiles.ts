// Files outside every checkout that the operator clicked in this desktop
// window, shown in Hide read-only (PRD outside-path-in-hide). The desktop
// host grants the file to this window; the core holds a tab with the grant's
// id, the file's name and its viewer, never the path (D-05). This window asks
// its host for the bytes, and for the path its badge shows; any other window,
// a browser tab, the phone or another machine's window, sees the tab by name
// alone (B19).
//
// The host lets go of a grant no tab stands on, so every sync names the
// grants the core's tabs hold and the opens still on their way to the core;
// a sync before the shell has the core's tabs names nothing, so a reload of
// this window lets go of nothing (B15). An open the core never answered is
// named for `PENDING_GRANT_MS` and then let go.

import { useEffect, useState } from "react";
import { create } from "zustand";
import { hostBridge, type ExternalBridge, type ExternalGrant, type ExternalKind } from "./host";
import { windowDeviceId } from "./screenMachine";
import { localDeviceId, type SnapshotRest } from "./snapshot";
import type { BrowserOpenReceipt } from "./viewLayout";

/** The bridge, only in a desktop window whose host has it. */
export function externalBridge(): ExternalBridge | null {
  return hostBridge()?.external ?? null;
}

/**
 * Whether this window shows an outside file in Hide: a desktop window on the
 * core's own machine. Any other window hands it to its system (D-09).
 */
export function outsideFilesShown(rest: SnapshotRest | null): boolean {
  return externalBridge() !== null && rest !== null && windowDeviceId(rest.navigator?.devices) === localDeviceId(rest);
}

/** How long an open the core has not answered keeps its grant named. */
export const PENDING_GRANT_MS = 10_000;

type ExternalStore = {
  /** Grants this window opened that the core has not answered yet: the open's request id and when it was sent. */
  pending: Record<string, { request: string; at: number }>;
  /** What this window's host says of each grant asked about: its path and kind, or null when it does not hold it. */
  described: Record<string, { path: string; kind: ExternalKind } | null>;
};

export const useExternalStore = create<ExternalStore>(() => ({ pending: {}, described: {} }));

/** Notes an open on its way to the core, so no sync lets its grant go first. */
export function notePendingGrant(grant: string, requestId: string, now = Date.now()): void {
  useExternalStore.setState((state) => ({ pending: { ...state.pending, [grant]: { request: requestId, at: now } } }));
}

/**
 * Settles the opens on their way: one the core now shows, answered (shown or
 * refused), or sent `PENDING_GRANT_MS` ago, is no longer pending. What the
 * host said of a grant no tab and no open stands on is forgotten, so a grant
 * handed out again is asked about again.
 */
export function settlePendingGrants(shown: readonly string[], receipts: readonly BrowserOpenReceipt[], now = Date.now()): void {
  const { pending, described } = useExternalStore.getState();
  const answered = new Set(receipts.map((receipt) => receipt.request_id));
  const kept = Object.entries(pending).filter(([grant, open]) => !shown.includes(grant) && !answered.has(open.request) && now - open.at < PENDING_GRANT_MS);
  const standing = new Set([...shown, ...kept.map(([grant]) => grant)]);
  const known = Object.entries(described).filter(([grant]) => standing.has(grant));
  if (kept.length !== Object.keys(pending).length) useExternalStore.setState({ pending: Object.fromEntries(kept) });
  if (known.length !== Object.keys(described).length) useExternalStore.setState({ described: Object.fromEntries(known) });
}

/** When the oldest open on its way stops being pending, or null with none. */
export function pendingDeadline(): number | null {
  const sent = Object.values(useExternalStore.getState().pending).map((open) => open.at);
  return sent.length === 0 ? null : Math.min(...sent) + PENDING_GRANT_MS;
}

/** The grants a sync names: the core's tabs' and the opens on their way; null until the shell has the core's tabs. */
export function syncedGrants(shown: readonly string[] | null): string[] | null {
  if (shown === null) return null;
  return [...new Set([...shown, ...Object.keys(useExternalStore.getState().pending)])].sort();
}

/**
 * What this window's host holds of `grant`: its path and kind, null when it
 * holds nothing (another window's tab, or a grant gone with the app), and
 * undefined while the host is asked.
 */
export function useExternalGrant(grant: string, asked = true): { path: string; kind: ExternalKind } | null | undefined {
  const described = useExternalStore((state) => (grant in state.described ? state.described[grant] : undefined));
  useEffect(() => {
    if (!asked || grant in useExternalStore.getState().described) return;
    const bridge = externalBridge();
    const answer = (value: { path: string; kind: ExternalKind } | null) =>
      useExternalStore.setState((state) => ({ described: { ...state.described, [grant]: value } }));
    if (!bridge) {
      answer(null);
      return;
    }
    void bridge.describe(grant).then((found) => answer(found ? { path: found.path, kind: found.kind } : null), () => answer(null));
  }, [grant, asked]);
  return described;
}

/** A granted file's read as the viewers take it, read again whenever `load` moves (a second click, B10). */
export type ExternalBytes =
  | { status: "loading" }
  | { status: "ready"; bytes: Uint8Array }
  | { status: "preview_only" }
  | { status: "failed"; reason: string };

export function useExternalBytes(grant: string, load: number): ExternalBytes {
  const [state, setState] = useState<ExternalBytes>({ status: "loading" });
  useEffect(() => {
    let live = true;
    setState({ status: "loading" });
    const bridge = externalBridge();
    if (!bridge) {
      setState({ status: "failed", reason: "grant_gone" });
      return undefined;
    }
    bridge.read(grant).then(
      (read) => {
        if (!live) return;
        if ("bytes" in read) setState({ status: "ready", bytes: read.bytes });
        else if ("preview_only" in read) setState({ status: "preview_only" });
        else setState({ status: "failed", reason: read.failure });
      },
      () => {
        if (live) setState({ status: "failed", reason: "read_failed" });
      },
    );
    return () => {
      live = false;
    };
  }, [grant, load]);
  return state;
}

/** The key a text view of an outside file takes a terminal link's line by (`lineRequest.ts`). */
export function externalLineKey(grant: string): string {
  return `external:${grant}`;
}

export type { ExternalGrant };
