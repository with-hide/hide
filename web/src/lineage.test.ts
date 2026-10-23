import { beforeAll, describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { createInterfaceI18n } from "./i18n/instance";
import { relationEntries, relationState, type Relation } from "./lineage";
import type { AgentChip, PaneRow } from "./snapshot";

let t: TFunction<"translation">;
beforeAll(async () => {
  t = (await createInterfaceI18n("en")).t;
});

const relation: Relation = { requestId: "r1", sourcePaneId: "parent", targetPaneId: "child", label: "child task" };

function chip(pane_id: string): AgentChip {
  return { tone: { kind: "working", read: false }, pane_id, label: pane_id, detail: null, status_word_visible: false, agent_kind: "codex", demand: "none", activity: "working", emphasized: false, mark: "going", status_code: "working", delegated: true };
}

describe("a relationship focus", () => {
  it("is in flight until the core's receipt names it, and needs no mark once it landed", () => {
    expect(relationState(relation, null, t)).toEqual({ phase: "pending" });
    expect(relationState(relation, { request_id: "older", target_pane_id: "child", phase: "failed", message: "gone", retryable: true }, t)).toEqual({ phase: "pending" });
    expect(relationState(relation, { request_id: "r1", target_pane_id: "child", phase: "pending", message: null, retryable: false }, t)).toEqual({ phase: "pending" });
    expect(relationState(relation, { request_id: "r1", target_pane_id: "child", phase: "succeeded", message: null, retryable: false }, t)).toBeNull();
  });

  it("carries the core's reason and whether it can be retried when it failed", () => {
    expect(relationState(relation, { request_id: "r1", target_pane_id: "child", phase: "failed", message: "Pane child is no longer available.", retryable: true }, t)).toEqual({
      phase: "failed",
      message: "Pane child is no longer available.",
      retryable: true,
    });
  });

  it("names the target in the selected language when the core gave no reason", async () => {
    const ko = (await createInterfaceI18n("ko")).t;
    expect(relationState({ ...relation, timedOut: true }, null, ko)).toEqual({ phase: "failed", message: "child task 에이전트를 열지 못했어요. 시간 안에 응답이 없었어요.", retryable: true });
    expect(relationState(relation, { request_id: "r1", target_pane_id: "child", phase: "failed", message: null, retryable: false }, ko)).toEqual({ phase: "failed", message: "child task 에이전트를 열지 못했어요", retryable: false });
  });

  it("reads as a retryable failure when no answer arrived in time, unless the core answered", () => {
    const late = { ...relation, timedOut: true };
    expect(relationState(late, null, t)).toEqual({ phase: "failed", message: "child task did not open: Hide did not answer in time.", retryable: true });
    expect(relationState(late, { request_id: "r1", target_pane_id: "child", phase: "pending", message: null, retryable: false }, t)?.phase).toBe("failed");
    expect(relationState(late, { request_id: "r1", target_pane_id: "child", phase: "succeeded", message: null, retryable: false }, t)).toBeNull();
  });
});

describe("the relationship menu", () => {
  it("lists the parent and the other siblings, never the pane's own children", () => {
    const pane = {
      id: "me",
      children: { instrumented: true, uninstrumented_reason: null, uninstrumented_label: null, chips: [chip("kid")] },
      lineage_path: [
        { pane_id: "root", label: "root", siblings: [] },
        { pane_id: "me", label: "me", siblings: [chip("me"), chip("sister")] },
      ],
    } as unknown as PaneRow;
    expect(relationEntries(pane).map((entry) => [entry.relation, entry.paneId])).toEqual([
      ["parent", "root"],
      ["sibling", "sister"],
    ]);
  });
});
