// This app's own updates, as the desktop host reports them (PRD auto-update):
// one subscription to the bridge for the page's life, read by the Help menu,
// its dot, the ↑ button and its card beside Help, and Settings > About. A browser tab and the phone have no bridge,
// so they read null and show none of it (B24).

import { useEffect, useSyncExternalStore } from "react";
import { hostBridge, type UpdateStatus } from "./host";
import { formatRelativeTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";

let current: UpdateStatus | null = null;
let subscribed = false;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  const updates = hostBridge()?.updates;
  if (!subscribed && updates) {
    subscribed = true;
    updates.onStatus((status) => {
      current = status;
      for (const notify of listeners) notify();
    });
  }
  return () => {
    listeners.delete(listener);
  };
}

/** The status, or null where this app's updates are not this page's to show. */
export function useUpdateStatus(): UpdateStatus | null {
  return useSyncExternalStore(subscribe, () => current, () => null);
}

/** The version a Restart would open, when one is ready. */
export function readyVersion(status: UpdateStatus | null): string | null {
  return status?.phase.kind === "ready" ? status.phase.version : null;
}

/**
 * The version the ready card offers: the ready one, unless the operator
 * closed its card; a newer ready version shows the card again. A required
 * update's card has no close, so it offers the ready version whatever was closed.
 */
export function cardVersion(status: UpdateStatus | null): string | null {
  const ready = readyVersion(status);
  return ready !== null && (status?.required === true || ready !== status?.dismissed) ? ready : null;
}

/** The release page of `version` on with-hide/hide, the repository the updater takes releases from (`desktop/src/main/updater.ts`). */
export function releaseNotesUrl(version: string): string {
  return `https://github.com/with-hide/hide/releases/tag/v${encodeURIComponent(version)}`;
}

/** How long ago a check finished, in the page's language: "now", "10 min. ago", "3 hr. ago". */
export function checkedAgo(language: InterfaceLanguage, checkedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - checkedAt) / 1000));
  if (seconds < 60) return formatRelativeTime(language, 0, "second");
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return formatRelativeTime(language, -minutes, "minute");
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return formatRelativeTime(language, -hours, "hour");
  return formatRelativeTime(language, -Math.floor(hours / 24), "day");
}

/** Installs the ready update and reopens the app; the core and its agents keep running. */
export function restartIntoUpdate(): void {
  hostBridge()?.updates.restart();
}

/**
 * Whether a press of Update now does something: a development build, or an
 * app that is not where it can replace itself, takes no update, and one
 * already looking or downloading has nothing more to start.
 */
export function offersUpdate(status: UpdateStatus | null): boolean {
  const phase = status?.phase.kind;
  return status !== null && phase !== "off" && phase !== "checking" && phase !== "downloading";
}

/** Updates this app now, as the operator pressed: restarts into the ready version, or looks for one. */
export function updateNow(status: UpdateStatus | null): void {
  if (readyVersion(status) !== null) restartIntoUpdate();
  else checkForUpdates();
}

/**
 * Another computer, which opens this one, asked it to update (PRD
 * device-connect-iroh B22): while `asked`, the app looks for the official
 * release at once and restarts into it once it is ready, with no press. A
 * restart that asks for the system password (Linux, D-18) is left to the
 * operator, whose notice stays. A browser tab and the phone have no
 * updates and do nothing.
 */
export function useAskedUpdate(asked: boolean): void {
  const status = useUpdateStatus();
  const known = status !== null;
  const ready = readyVersion(status) !== null;
  const unattended = status?.installOnQuit === true;
  useEffect(() => {
    if (!asked || !known) return;
    if (!ready) checkForUpdates();
    else if (unattended) restartIntoUpdate();
  }, [asked, known, ready, unattended]);
}

/** Closes the ready card for `version`; Help's dot and line and About keep the update. */
export function dismissUpdateCard(version: string): void {
  hostBridge()?.updates.dismiss(version);
}

/** Looks for an update now. */
export function checkForUpdates(): void {
  hostBridge()?.updates.check();
}

/** Takes preview releases from the next check on, or stops taking them. */
export function setPreviewUpdates(on: boolean): void {
  hostBridge()?.updates.setPrerelease(on);
}
