import { describe, expect, it } from "vitest";
import { settledPlace } from "./automationPlace";
import type { Automation, AutomationsSection } from "./snapshot";

// PRD automations B8: [만들고 지금 실행] opens the new automation's run even
// when the core's answer reaches the window before the section that holds it.

function automation(id: string, runs: number[]): Automation {
  return {
    id, name: id, task: "t", schedule: { kind: "daily", time: "09:00" }, reads_as: { kind: "daily", time: "09:00" }, next_run_at_unix_ms: null,
    agent: { kind: "claude" }, folder: { path: "/home/u/hide/automations/a", default: true, worktree: false }, running: false, unseen: 0,
    records: runs.map((number) => ({ kind: "run", number, trigger: "manual", started_at_unix_ms: 1, state: "running", attempt: 1, opened: false, resumable: false, conversation_missing: false, seen: true })),
  };
}

const section = (...items: Automation[]): AutomationsSection => ({ items, unseen: 0 });

describe("where the automation screen stands", () => {
  it("waits for an answered automation and its run when the answer comes before the section", () => {
    const answered = { selected: "a1", run: 1, form: null, arriving: true, example: null };
    expect(settledPlace(answered, section())).toBeNull();
    expect(settledPlace(answered, section(automation("a1", [])))).toBeNull();
    expect(settledPlace(answered, section(automation("a1", [1])))).toEqual({ ...answered, arriving: false, example: null });
  });

  it("steps back from an automation or a run the section no longer holds", () => {
    expect(settledPlace({ selected: "a1", run: null, form: "edit", arriving: false, example: null }, section())).toEqual({ selected: null, run: null, form: null, arriving: false, example: null });
    expect(settledPlace({ selected: "a1", run: 2, form: null, arriving: false, example: null }, section(automation("a1", [1])))).toEqual({ selected: "a1", run: null, form: null, arriving: false, example: null });
    expect(settledPlace({ selected: "a1", run: 1, form: null, arriving: false, example: null }, section(automation("a1", [1])))).toBeNull();
  });
});
