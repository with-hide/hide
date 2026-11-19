import { describe, expect, it } from "vitest";
import { addCall, draftOf, editCall, ownFolderLine, refusalField, refusalKey } from "./automationDraft";
import type { Automation } from "./snapshot";

// PRD automations B4, B5, B7, B26: what the form opens on and what a press
// sends the core, which reads every value; an edit sends only what changed.

function automation(fields: Partial<Automation> = {}): Automation {
  return {
    id: "a1", name: "아침 이슈 정리", task: "새 이슈 정리", schedule: { kind: "weekly", days: ["mon", "wed"], time: "10:00" }, reads_as: { kind: "weekly", days: ["mon", "wed"], time: "10:00" },
    next_run_at_unix_ms: null, agent: { kind: "codex", model: "gpt-5", effort: "high" }, folder: { path: "/home/u/hide/automations/a1", default: true, worktree: false }, running: false, unseen: 0, records: [], ...fields,
  };
}

describe("the automation form's values", () => {
  it("opens a new automation on the agent last chosen, in its own folder", () => {
    const draft = draftOf(null, { kind: "codex", model: "gpt-5", effort: "high" });
    expect(draft.agent).toEqual({ kind: "codex", profile: "default", model: "gpt-5", effort: "high" });
    expect(addCall({ ...draft, name: " 점검 ", task: "의존성 점검" }, false, true)).toEqual({
      call: "add", name: "점검", task: "의존성 점검", schedule: { kind: "daily", time: "09:00" }, agent: { kind: "codex", model: "gpt-5", effort: "high" }, worktree: false, run_now: true,
    });
  });

  it("asks for a new worktree only in a Git project's folder", () => {
    const draft = { ...draftOf(null, undefined), name: "n", task: "t", folder: "/p", worktree: true };
    expect(addCall(draft, true, false)).toMatchObject({ folder: "/p", worktree: true });
    expect(addCall(draft, false, false)).toMatchObject({ folder: "/p", worktree: false });
  });

  it("sends an edit with only what changed, and none when nothing did", () => {
    const before = automation();
    const draft = draftOf(before, undefined);
    expect(editCall(before, draft, false)).toBeNull();
    expect(editCall(before, { ...draft, schedule: { ...draft.schedule, days: ["mon"] } }, false)).toEqual({ call: "edit", id: "a1", schedule: { kind: "weekly", days: ["mon"], time: "10:00" } });
    const inProject = automation({ folder: { path: "/p", default: false, worktree: false } });
    expect(editCall(inProject, { ...draftOf(inProject, undefined), folder: null }, false)).toEqual({ call: "edit", id: "a1", folder: null });
  });

  it("puts each refusal under the field it names and words an unknown one as not now", () => {
    expect(refusalField("schedule_under_hour")).toBe("schedule");
    expect(refusalField("automation_name_taken")).toBe("name");
    expect(refusalField("automation_worktree_needs_git")).toBe("folder");
    expect(refusalField("automations_unavailable")).toBe("form");
    expect(refusalKey("schedule_under_hour")).toBe("automationScreen.refusal.schedule_under_hour");
    expect(refusalKey("socket_closed")).toBe("automationScreen.refusal.other");
  });

  it("names the own folder under the home on every system, its id left to the core", () => {
    expect(ownFolderLine("/Users/u/hide/automations", "아침 이슈 정리")).toBe("~/hide/automations/아침-이슈-정리-…");
    expect(ownFolderLine("C:\\Users\\u\\hide\\automations", "")).toBe("~/hide/automations/…");
  });
});
