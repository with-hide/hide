// What each released version brought, as the desktop host holds it
// (`desktop/src/main/whatsNew.ts`), and the shell's own state around it: the
// update history open or closed and at which version. One subscription to the
// bridge for the page's life. A browser tab and the phone have no bridge, so
// they read null and show none of it (PRD whats-new, Non-goals).

import { useSyncExternalStore } from "react";
import { hostBridge, type WhatsNewState } from "./host";
import type { RecordUpdate } from "./releases";
import { releaseNotesUrl } from "./updates";

let current: WhatsNewState | null = null;
let history: { open: boolean; version: string | null } = { open: false, version: null };
let subscribed = false;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  const bridge = hostBridge()?.whatsNew;
  if (!subscribed && bridge) {
    subscribed = true;
    bridge.onState((state) => {
      current = state;
      notify();
    });
  }
  return () => {
    listeners.delete(listener);
  };
}

/** The host's state, or null where this page has no host to ask. */
export function useWhatsNewState(): WhatsNewState | null {
  return useSyncExternalStore(subscribe, () => current, () => null);
}

/** Whether the update history is open, and the version it opened at (null: the version in use). */
export function useUpdateHistory(): { open: boolean; version: string | null } {
  return useSyncExternalStore(subscribe, () => history, () => history);
}

/** Whether this page can show the update history at all: only the desktop app's window. */
export function updateHistoryAvailable(): boolean {
  return hostBridge()?.whatsNew !== undefined;
}

/** Opens the update history at `version` (the version in use when null), fetching again when the kept copy is an hour old. */
export function openUpdateHistory(version: string | null = null): void {
  history = { open: true, version };
  hostBridge()?.whatsNew.refresh();
  notify();
}

/** Moves the open history to `version`. */
export function chooseUpdateHistoryVersion(version: string): void {
  history = { open: true, version };
  notify();
}

export function closeUpdateHistory(): void {
  history = { open: false, version: null };
  notify();
}

/** Fetches again now, whatever the age of the kept copy (the history's Try again). */
export function retryUpdateHistory(): void {
  hostBridge()?.whatsNew.refresh(true);
}

/** Moves the host's record forward: a version's window closed, or a version opened in the history. */
export function saveWhatsNew(update: RecordUpdate): void {
  hostBridge()?.whatsNew.save(update);
}

/** Whether the kept file has `version`, so a link can open the history there instead of GitHub. */
export function releaseKnown(state: WhatsNewState | null, version: string): boolean {
  return state?.feed?.versions.some((row) => row.version === version) ?? false;
}

/**
 * What a "What's new" link does for `version`: the update history at that
 * version when the kept file has it, else the release page on GitHub (B18).
 */
export function useReleaseNotes(openLink: (url: string, external: boolean) => void): (version: string) => void {
  const state = useWhatsNewState();
  return (version) => {
    if (releaseKnown(state, version)) openUpdateHistory(version);
    else openLink(releaseNotesUrl(version), true);
  };
}
