import { describe, expect, it } from "vitest";
import type { AgentRow, Mission, PlanView, SessionThreadHistorySection, SessionThreadSection, SnapshotRest, ThreadPage } from "./snapshot";
import { threadConversation, threadInputHeld, threadLaunchModel, threadScreen } from "./threadView";

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

describe("a thread's screen", () => {
  const row = (pane_id: string, identity_label: string, mark: AgentRow["mark"]) => ({ pane_id, identity_label, mark, agent_kind: "codex" }) as AgentRow;
  const call = (toolCallId: string, title: string) => ({ sessionUpdate: "tool_call", toolCallId, kind: "execute", title, status: "completed" });
  const plan = { pane_id: "w1:p9" } as PlanView;
  const mission = { id: "mission-3", title: "Desk 1차", status: "running", decision: null, nodes: [{ status: "done" }, { status: "working" }, { status: "dropped" }] } as unknown as Mission;
  const tail: ThreadPage = {
    updates: [
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "다음 단계\n둘째 줄" }, _meta: { hide: { origin: "letter", from: "lead" } } },
      call("a", "hide request send agent-7 --intent m:ask"),
      call("b", "hide agent spawn --name review"),
      call("p1", "hide plan set"),
      call("p2", "hide plan node 2 --status done"),
      call("d", "hide diff open notes.md"),
      call("m", "hide mission show --mission mission-3"),
      call("gone", "hide mission show --mission mission-9"),
      call("f", "hide file open shots/dark.png"),
    ],
    parts: {
      a: { kind: "letter", to: "agent-7", body: "봐줘\n자세한 건 아래" },
      b: { kind: "spawn", name: "review", intent: null, agent_id: "agent-42" },
      p1: { kind: "plan" },
      p2: { kind: "plan" },
      d: { kind: "view", view: "diff", target: "notes.md" },
      m: { kind: "mission", mission: "mission-3" },
      gone: { kind: "mission", mission: "mission-9" },
      f: { kind: "view", view: "file", target: "shots/dark.png" },
    },
    sessions: { "agent-7": { pane_id: "w1:p2", machine: "mac" }, lead: { pane_id: "w1:p1", machine: "mac" } },
  };

  it("draws each hide call as the part it made, a named session as its row, and what Hide cannot draw truly as the command", () => {
    const screen = threadScreen(thread({ pane_id: "w1:p9", tail }), null, { plans: { items: [plan] }, missions: { items: [mission] } }, [row("w1:p2", "리뷰어", "going"), row("w1:p1", "리드", "turn")]);
    const drawn = Object.fromEntries(screen.lines.map((line) => [line.id, line.kind === "hide" ? line.part : line.kind]));
    expect(screen.lines[0]).toMatchObject({ kind: "user", origin: "letter", peer: { name: "리드", mark: "turn", agent_kind: "codex", pane_id: "w1:p1" } });
    expect(drawn).toMatchObject({
      a: { kind: "letter", direction: "sent", peer: { name: "리뷰어", mark: "going", pane_id: "w1:p2" }, body: "봐줘" },
      // agent-42 has no live session: its chip is the name it was given.
      b: { kind: "spawn", session: { name: "review", mark: null, pane_id: null } },
      // Only the last plan call draws the plan as it is now.
      p1: "tool",
      p2: { kind: "plan", plan },
      d: "tool",
      m: { kind: "mission", mission: { id: "mission-3", title: "Desk 1차", status: "running", done: 1, total: 2, deciding: false } },
      gone: "tool",
      f: { kind: "view", output: { id: "shots/dark.png", kind: "image", label: "dark.png", thumb: null, seen: true, pr_state: null } },
    });
  });

  it("is being read until the record answers, and says why when it cannot be read", () => {
    expect(threadScreen(thread({ state: "loading" }), null, {}, [])).toMatchObject({ reading: true, failure: null, lines: [] });
    expect(threadScreen(thread({ state: "failed", failure: "no_record" }), null, {}, [])).toMatchObject({ reading: false, failure: "no_record" });
  });
});
