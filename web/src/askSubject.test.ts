import { describe, expect, it } from "vitest";
import { askParts } from "./components/agent-tree";
import { lineText } from "./agentRow";
import { createInterfaceI18n } from "./i18n/instance";

const decision = { cause: "mission_decision", line: "뼈대의 check 줄을 건너뛸까요?" } as const;

describe("a mission decision laid on its Conductor's row (needs-you-cleanup B1, B9)", () => {
  it("leads with 판단 and says the decision's line, in every language", async () => {
    const { t: ko } = await createInterfaceI18n("ko");
    const { t: en } = await createInterfaceI18n("en");
    expect(askParts(ko, { verb: "answer", what: null, more: 0, subject: decision })).toEqual({ lead: "판단", text: decision.line, tail: null });
    expect(askParts(en, { verb: "answer", what: null, more: 0, subject: decision }).lead).toBe("Decision");
    const agent = { status_code: "question" as const };
    expect(lineText(ko, agent, { text: "", mode: "request", subject: decision })).toBe(`판단 · ${decision.line}`);
  });

  it("counts the other asks under the same root after the one that waited longest", async () => {
    const { t: ko } = await createInterfaceI18n("ko");
    expect(askParts(ko, { verb: "answer", what: "어느 쪽으로 갈까요?", more: 2 })).toEqual({ lead: "질문", text: "어느 쪽으로 갈까요?", tail: "외 2건" });
    expect(askParts(ko, { verb: "answer", what: null, more: 1, subject: { cause: "plan_review", steps: 3 } })).toEqual({ lead: "질문", text: "계획 검토 · 3단계", tail: "외 1건" });
  });
});
