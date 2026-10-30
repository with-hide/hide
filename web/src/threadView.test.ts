import { describe, expect, it } from "vitest";
import type { SessionThreadHistorySection, SessionThreadSection, ThreadPage } from "./snapshot";
import { threadConversation, threadInputHeld } from "./threadView";

const said = (text: string) => ({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });

function page(texts: string[], parts: ThreadPage["parts"] = {}): ThreadPage {
  return { updates: texts.map(said), parts };
}

function thread(extra: Partial<SessionThreadSection> = {}): SessionThreadSection {
  return { generation: 2, state: "ready", pane_id: "w1:p1", machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: null, input: null, ...extra };
}

describe("a thread's conversation", () => {
  it("is the kept pages, oldest first, then the live page", () => {
    const history: SessionThreadHistorySection = {
      generation: 2,
      pages: [page(["처음"], { a: { kind: "plan" } }), page(["둘째"], { b: { kind: "letter", to: "agent-7", body: "hi" } })],
    };
    const spawned = { kind: "spawn" as const, name: "review", intent: null, agent_id: "agent-42" };
    const shown = threadConversation(thread({ tail: page(["지금"], { b: spawned }) }), history);
    expect(shown.updates.map((update) => (update.content as { text: string }).text)).toEqual(["처음", "둘째", "지금"]);
    expect(shown.parts).toEqual({ a: { kind: "plan" }, b: spawned });
  });

  it("leaves out the kept pages of a thread that closed", () => {
    const stale: SessionThreadHistorySection = { generation: 1, pages: [page(["다른 스레드"])] };
    expect(threadConversation(thread({ tail: page(["지금"]) }), stale).updates).toHaveLength(1);
    expect(threadConversation(thread({ state: "closed" }), stale).updates).toEqual([]);
    expect(threadConversation(null, null).updates).toEqual([]);
  });
});

describe("the input box", () => {
  it("sends only while the pane runs the thread's session and nothing else is on its way", () => {
    expect(threadInputHeld(thread())).toBeNull();
    expect(threadInputHeld(thread({ pane: "changed" }))).toBe("pane_changed");
    expect(threadInputHeld(thread({ pane: "gone" }))).toBe("pane_gone");
    expect(threadInputHeld(thread({ input: { request_id: "r", state: "sending" } }))).toBe("in_flight");
    expect(threadInputHeld(thread({ input: { request_id: "r", state: "refused", reason: "asleep" } }))).toBeNull();
    expect(threadInputHeld(null)).toBe("closed");
  });
});
