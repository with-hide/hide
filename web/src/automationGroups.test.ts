import { describe, expect, it } from "vitest";
import { automationGroups, doneLast } from "./automationGroups";
import type { AutomationSchedule } from "./snapshot";

// mission-104 S-1, S-5: the screen draws the repeating automations in the
// core's order, the once ones still waiting nearest first, and the done ones
// newest first under their fold; the phone lists the done ones last.

const item = (id: string, schedule: AutomationSchedule, done = false) => ({ id, schedule, done });
const once = (date: string, time: string) => ({ kind: "once" as const, date, time });

describe("the automation screen's groups", () => {
  it("keeps the core's order for the repeating ones and orders the once ones by their date and time", () => {
    const items = [
      item("week", once("2026-12-02", "09:00")),
      item("daily", { kind: "daily", time: "09:00" }),
      item("old", once("2026-11-01", "09:00"), true),
      item("tomorrow-late", once("2026-11-26", "18:00")),
      item("cron", { kind: "cron", expr: "0 9 * * *" }),
      item("tomorrow", once("2026-11-26", "09:00")),
      item("yesterday", once("2026-11-24", "09:00"), true),
    ];
    const groups = automationGroups(items);
    expect(groups.repeating.map((a) => a.id)).toEqual(["daily", "cron"]);
    expect(groups.waiting.map((a) => a.id)).toEqual(["tomorrow", "tomorrow-late", "week"]);
    expect(groups.done.map((a) => a.id)).toEqual(["yesterday", "old"]);
  });

  it("leaves a group empty when nothing is in it, so the screen draws none", () => {
    expect(automationGroups([item("daily", { kind: "daily", time: "09:00" })])).toEqual({ repeating: [item("daily", { kind: "daily", time: "09:00" })], waiting: [], done: [] });
  });

  it("puts the phone's done ones after the rest in the core's order", () => {
    const items = [item("a", once("2026-11-01", "09:00"), true), item("b", { kind: "daily", time: "09:00" }), item("c", once("2026-12-02", "09:00"))];
    expect(doneLast(items).map((a) => a.id)).toEqual(["b", "c", "a"]);
  });
});
