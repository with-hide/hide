import { describe, expect, it } from "vitest";
import type { SessionThreadHistorySection, SessionThreadSection, SnapshotRest, ThreadPage } from "./snapshot";
import { threadConversation, threadInputHeld, threadLaunchModel } from "./threadView";

const said = (text: string) => ({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });

function page(texts: string[], parts: ThreadPage["parts"] = {}, sessions: ThreadPage["sessions"] = {}): ThreadPage {
  return { updates: texts.map(said), parts, sessions };
}

function thread(extra: Partial<SessionThreadSection> = {}): SessionThreadSection {
  return { generation: 2, state: "ready", pane_id: "w1:p1", machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: null, input: null, ...extra };
}

describe("a thread's conversation", () => {
  it("is the kept pages, oldest first, then the live page", () => {
    const history: SessionThreadHistorySection = {
      generation: 2,
      pages: [page(["처음"], { a: { kind: "plan" } }), page(["둘째"], { b: { kind: "letter", to: "agent-7", body: "hi" } }, { "agent-7": { pane_id: "w1:p2", machine: "mac" } })],
    };
    const spawned = { kind: "spawn" as const, name: "review", intent: null, agent_id: "agent-42" };
    const shown = threadConversation(thread({ tail: page(["지금"], { b: spawned }, { "agent-42": { pane_id: "w1:p3", machine: "mac" } }) }), history);
    expect(shown.updates.map((update) => (update.content as { text: string }).text)).toEqual(["처음", "둘째", "지금"]);
    expect(shown.parts).toEqual({ a: { kind: "plan" }, b: spawned });
    expect(shown.sessions).toEqual({ "agent-7": { pane_id: "w1:p2", machine: "mac" }, "agent-42": { pane_id: "w1:p3", machine: "mac" } });
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

describe("the head's model", () => {
  it("is the model Hide launched the pane's session with, on the thread's machine, and none when the launch chose none", () => {
    const session = (device_id: string, pane_id: string, launch_model: string | null) => ({ device_id, pane_id, session_digest: "d", launch_model, launch_effort: null });
    const rest = { overview_graph: { sessions: [session("mac", "w1:p1", "sonnet"), session("mini", "w1:p1", "haiku"), session("mini", "w2:p3", null)] } } as unknown as SnapshotRest;
    expect(threadLaunchModel(rest, thread())).toBe("sonnet");
    expect(threadLaunchModel(rest, thread({ machine: "mini", pane_id: "remote:mini:pane:w1:p1" }))).toBe("haiku");
    expect(threadLaunchModel(rest, thread({ machine: "mini", pane_id: "remote:mini:pane:w2:p3" }))).toBeNull();
    expect(threadLaunchModel(rest, thread({ pane_id: "w9:p9" }))).toBeNull();
  });
});
