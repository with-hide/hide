// Presentation rules for the typed limit cells. Declarations decide support;
// only the core node's reported values decide account availability.
import type { TFunction } from "i18next";
import { AGENT_ADAPTERS } from "./agentAdapters";
import type { AgentUsage, LimitCell, UsagePreferences, WindowKind, WindowReading } from "./generated/hided-ws";


export type UsageAdapter = (typeof AGENT_ADAPTERS)[number];
export const WINDOW_ORDER: readonly WindowKind[] = ["five_hour", "weekly", "monthly"];

export function hasLimits(adapter: UsageAdapter): boolean {
  return WINDOW_ORDER.some((kind) => adapter.usage.limits[kind].support === "supported");
}

export function accountWindows(adapter: UsageAdapter, cell: LimitCell | undefined): WindowKind[] {
  return WINDOW_ORDER.filter((kind) => adapter.usage.limits[kind].support === "supported" && !["not_offered", "unreadable"].includes(limitWindow(cell, kind)?.outcome.state ?? ""));
}

export function limitWindow(cell: LimitCell | undefined, kind: WindowKind): WindowReading | undefined {
  return cell?.state === "reading" ? cell.value.windows.find((row) => row.kind === kind && row.scope.kind === "all") : undefined;
}

export function windowLabel(kind: WindowKind, t: TFunction<"translation">): string {
  return t(`usage.window.${kind}`);
}

export function limitValue(cell: LimitCell | undefined, kind: WindowKind, now: number, t: TFunction<"translation">): string {
  if (!cell || cell.state === "loading") return "…";
  const row = limitWindow(cell, kind);
  if (!row) return t("common.unavailable");
  switch (row.outcome.state) {
    case "open": return row.outcome.resets_at_s * 1000 <= now ? t("usage.expired") : `${percentLabel(row.outcome.used_percent)} · ${resetCountdown(row.outcome.resets_at_s, now, t)}`;
    case "not_started": return t("usage.notStarted");
    case "expired": return t("usage.expired");
    case "not_offered": return "";
    case "unreadable": return t("common.unavailable");
  }
}

/** Unsupported wording follows the declared absence, never a guessed login. */
export function absenceReason(adapter: UsageAdapter, t: TFunction<"translation">): string {
  const weekly = adapter.usage.limits.weekly;
  if (weekly.support !== "unsupported" || typeof weekly.detail !== "object") return "";
  switch (weekly.detail.kind.kind) {
    case "not_built": return t("usage.reason.notBuilt", { agent: adapter.label });
    case "no_such_data": return t("usage.reason.account");
    case "excluded": return t("usage.reason.opencode");
  }
  return weekly.detail.reason;
}

export type UsageTone = "success" | "warning" | "danger" | "muted";

/** From 70% the window is getting tight and from 90% it is nearly spent; no reading is muted. */
export function usageTone(percent: number | null): UsageTone {
  if (percent === null) return "muted";
  if (percent >= 90) return "danger";
  if (percent >= 70) return "warning";
  return "success";
}

export function percentLabel(percent: number): string {
  return `${Math.round(percent)}%`;
}

/** `in 5d 4h`, `in 3h 12m` or `in 7m`: the time left before the window resets, never less than a minute. */
export function resetCountdown(resetsAtUnixSeconds: number, nowMs: number, t: TFunction<"translation">): string {
  const seconds = Math.max(0, Math.floor(resetsAtUnixSeconds - nowMs / 1000));
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return t("shell.usage.resetDaysHours", { days, hours });
  if (hours > 0) return t("shell.usage.resetHoursMinutes", { hours, minutes });
  return t("shell.usage.resetMinutes", { minutes: Math.max(1, minutes) });
}


export function windowPercent(window: WindowReading | undefined, now: number): number | null {
  return window?.outcome.state === "open" && window.outcome.resets_at_s * 1000 > now ? window.outcome.used_percent : null;
}
export const SHORT_WINDOW: Record<WindowKind, string> = { five_hour: "5h", weekly: "7d", monthly: "mo" };
export type UsageChip = { agent: string; window?: WindowKind; named: boolean; percent: number | null; state: string };
/** Pins are shared core state. A missing agent reading collapses its pins into one dim mark. */
export function usageChips(rows: readonly AgentUsage[], prefs: UsagePreferences, now: number): UsageChip[] {
  return AGENT_ADAPTERS.flatMap<UsageChip>((adapter) => {
    if (!hasLimits(adapter) || prefs.hidden.includes(adapter.herdr_kind)) return [];
    const pins = prefs.pinned.filter((pin) => pin.agent === adapter.herdr_kind)
      .sort((a, b) => WINDOW_ORDER.indexOf(a.window) - WINDOW_ORDER.indexOf(b.window));
    if (!pins.length) return [];
    const cell = rows.find((row) => row.agent === adapter.herdr_kind)?.limits;
    if (!cell || cell.state !== "reading") return [{ agent: adapter.herdr_kind, named: false, percent: null, state: cell?.state ?? "loading" }];
    const windows = pins.flatMap((pin) => {
      const row = limitWindow(cell, pin.window);
      return row && !["not_offered", "unreadable"].includes(row.outcome.state) ? [row] : [];
    });
    return windows.map((window) => ({ agent: adapter.herdr_kind, window: window.kind,
      named: windows.length > 1 || windowPercent(window, now) === null,
      percent: windowPercent(window, now), state: window.outcome.state }));
  });
}
