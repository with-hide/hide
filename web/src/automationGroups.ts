// How the automation screen and the phone's tab order a core's automations
// (mission-104 S-1, S-5): the repeating ones, the once ones still waiting,
// and the once ones done. The core says which is which (`schedule.kind`,
// `done`); this module only places them.

import type { AutomationSchedule } from "./snapshot";

type Placed = { schedule: AutomationSchedule; done?: boolean };

/** The screen's groups: `repeating` in the core's order, `waiting` nearest time first, `done` newest time first. */
export type AutomationGroups<T> = { repeating: T[]; waiting: T[]; done: T[] };

/** The wall time a once schedule was set for, as text that sorts by time (`2026-12-02 09:00`). */
function onceKey(item: Placed): string {
  return item.schedule.kind === "once" ? `${item.schedule.date} ${item.schedule.time}` : "";
}

export function automationGroups<T extends Placed>(items: readonly T[]): AutomationGroups<T> {
  const repeating = items.filter((item) => item.schedule.kind !== "once");
  const waiting = items.filter((item) => item.schedule.kind === "once" && !item.done).sort((a, b) => onceKey(a).localeCompare(onceKey(b)));
  const done = items.filter((item) => item.schedule.kind === "once" && item.done).sort((a, b) => onceKey(b).localeCompare(onceKey(a)));
  return { repeating, waiting, done };
}

/** The phone's list: the core's order with the done once automations after the rest (S-5). */
export function doneLast<T extends Placed>(items: readonly T[]): T[] {
  return [...items.filter((item) => !item.done), ...items.filter((item) => item.done)];
}
