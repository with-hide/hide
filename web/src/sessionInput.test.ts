import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { fileWords, pickWords } from "./deskComposer";
import { sessionBarSlots, sessionMentions, switchCommand } from "./sessionInput";
import type { DeskMentions } from "./snapshot";

const t = ((key: string, values?: Record<string, string>) => `${key}${values?.agent ? `:${values.agent}` : ""}`) as unknown as TFunction<"translation">;
const session = (id: string, parent: string | null = null) => ({ id, name: id, agent_kind: "claude", mark: "going" as const, detail: null, parent, project: "p", branch: null, last_prompt_at_unix_ms: null });

describe("a session's input", () => {
  it("names every session Desk's `@` lists but itself, and a child of itself stands at the top", () => {
    const section: DeskMentions = { sessions: [session("w1:p1"), session("w1:p2", "w1:p1"), session("w1:p3")], projects: [] };
    const named = sessionMentions(section, null, "w1:p1")!;
    expect(named.sessions.map((one) => [one.id, one.parent])).toEqual([["w1:p2", null], ["w1:p3", null]]);
  });

  it("writes a pick as plain words in place of its `@` word, and a file with a space quoted", () => {
    expect(pickWords("고쳐줘 @gr 그리고", { start: 4, query: "gr" }, fileWords({ relative_path: "docs/a b.md" }))).toEqual({ text: '고쳐줘 @"docs/a b.md" 그리고', caret: 19 });
    expect(pickWords("@gr", { start: 0, query: "gr" }, fileWords({ relative_path: "scripts/grader.ts" }))).toEqual({ text: "@scripts/grader.ts ", caret: 19 });
  });

  it("lets the bar change only what the agent's own line changes, and says why for the rest", () => {
    const claude = sessionBarSlots("claude", { model: "opus" }, { state: "ready", models: ["sonnet"] }, t);
    expect(claude.model).toMatchObject({ value: "opus", fixed: null });
    expect(claude.model.choices.map((one) => one.id)).toEqual(["sonnet", "opus"]);
    expect(claude.effort.choices.map((one) => one.id)).toEqual(["low", "medium", "high", "xhigh", "max"]);
    const opencode = sessionBarSlots("opencode", {}, null, t);
    expect(opencode.model.fixed).toBe("desk.bar.modelFixed:OpenCode");
    expect(opencode.effort.fixed).toBe("desk.bar.effortFixed:OpenCode");
    // omp changes its effort by a line, and its model only in its own picker.
    const omp = sessionBarSlots("omp", {}, null, t);
    expect([omp.model.fixed !== null, omp.effort.fixed]).toEqual([true, null]);
    // Grok's effort line names the model, so the effort waits for a known one.
    expect(sessionBarSlots("grok", {}, null, t).effort.fixed).toBe("desk.bar.effortNeedsModel:Grok");
    expect(sessionBarSlots("grok", { model: "grok-4.7" }, null, t).effort.fixed).toBeNull();
    expect(switchCommand("/model --reasoning={effort}")).toBe("/model");
  });
});
