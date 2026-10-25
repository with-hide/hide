import { useSyncExternalStore, type HTMLAttributes } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { formatUnit } from "../i18n/format";
import { requireInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";

// Time since a core-owned reference, counted on one clock for the
// whole window. Agent rows use the latest prompt; archives use archive time.
// The core publishes the moment
// once; time passing never republishes the snapshot. Each label subscribes on
// its own and re-renders only when its text changes, so a second passing
// touches no row and a minute passing touches one span per row.

const listeners = new Set<() => void>();
let timer: number | null = null;
let now = Date.now();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (timer === null) {
    now = Date.now();
    timer = window.setInterval(() => {
      now = Date.now();
      for (const notify of listeners) notify();
    }, 1000);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== null) {
      window.clearInterval(timer);
      timer = null;
    }
  };
}

/** `42s`, `3m`, `2h`, `1d` in English: the largest whole unit; a moment from the future is `0s`. */
export function formatElapsed(language: InterfaceLanguage, elapsedMs: number): string {
  const seconds = Math.floor(Math.max(0, elapsedMs) / 1000);
  if (seconds < 60) return formatUnit(language, seconds, "second");
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return formatUnit(language, minutes, "minute");
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return formatUnit(language, hours, "hour");
  return formatUnit(language, Math.floor(hours / 24), "day");
}

/** The elapsed text for `since` on the shared clock; `null` when the core never measured it. */
export function useElapsed(since: number | null | undefined): string | null {
  const language = requireInterfaceLanguage(useInterfaceTranslation().i18n.language);
  return useSyncExternalStore(subscribe, () => (since == null ? null : formatElapsed(language, (timer === null ? Date.now() : now) - since)));
}

/** Now on the same clock, to the minute, so a time axis re-renders once a minute rather than every second. */
export function useMinuteClock(): number {
  return useSyncExternalStore(subscribe, () => Math.floor((timer === null ? Date.now() : now) / 60_000) * 60_000);
}

/**
 * The time left before `deadline` on the same clock, in the largest whole
 * unit rounded up (a Factory question's 21h left); `past` once it passed and
 * `null` without a deadline.
 */
export function useRemaining(deadline: number | null | undefined): { text: string } | "past" | null {
  const language = requireInterfaceLanguage(useInterfaceTranslation().i18n.language);
  const text = useSyncExternalStore(subscribe, () => {
    if (deadline == null) return null;
    const left = deadline - (timer === null ? Date.now() : now);
    if (left <= 0) return "";
    const hours = Math.ceil(left / 3_600_000);
    return hours >= 1 && left >= 3_600_000 ? formatUnit(language, hours, "hour") : formatUnit(language, Math.ceil(left / 60_000), "minute");
  });
  return text === null ? null : text === "" ? "past" : { text };
}

/** A span holding the elapsed text; nothing at all when the core has no time for the agent. */
export function Elapsed({ since, ...props }: { since: number | null | undefined } & HTMLAttributes<HTMLSpanElement>) {
  const text = useElapsed(since);
  return text === null ? null : <span {...props}>{text}</span>;
}
