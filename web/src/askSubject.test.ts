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

describe("a mission node raised to the operator, laid on its Conductor's row (PRD mission-node-simple B14)", () => {
  it("leads with 확인 필요 and says the node and why, in every language", async () => {
    const { t: ko } = await createInterfaceI18n("ko");
    const { t: en } = await createInterfaceI18n("en");
    const raised = { cause: "mission_attention", node: "device-row-simple", reason: "merged_unproven" } as const;
    expect(askParts(ko, { verb: "answer", what: null, more: 0, subject: raised })).toEqual({ lead: "확인 필요", text: "device-row-simple · PR 머지됨, 기준 기록이 없거나 실패", tail: null });
    expect(askParts(en, { verb: "answer", what: null, more: 0, subject: { ...raised, reason: "worker_ended" } })).toEqual({ lead: "Needs a look", text: "device-row-simple · Worker ended with work left", tail: null });
  });
});

describe("a brief or a letter the agent has not confirmed, laid on its row", () => {
  it("says which input in plain words, not received only when its composer held it, and when its typings are used up, in every language", async () => {
    const { t: ko } = await createInterfaceI18n("ko");
    const { t: en } = await createInterfaceI18n("en");
    const brief = { cause: "input_unconfirmed", input: "brief", spent: false, held: false } as const;
    expect(askParts(ko, { verb: "answer", what: null, more: 0, subject: brief })).toEqual({ lead: "질문", text: "브리프 확인 안 됨", tail: null });
    expect(askParts(ko, { verb: "answer", what: null, more: 0, subject: { ...brief, held: true } }).text).toBe("브리프 못 받음");
    expect(askParts(ko, { verb: "answer", what: null, more: 0, subject: { ...brief, input: "letter", spent: true } }).text).toBe("편지 확인 안 됨 · 재시도 다 씀");
    expect(askParts(en, { verb: "answer", what: null, more: 0, subject: brief }).text).toBe("Brief not confirmed");
    expect(askParts(en, { verb: "answer", what: null, more: 0, subject: { ...brief, held: true } }).text).toBe("Brief not received");
  });
});
