import { describe, expect, it } from "vitest";
import { askingCount, markedMention, mentionParts, outputKindOf, threadGroups, threadLines, unseenOutputs, type AcpSessionUpdate, type DeskJob } from "./desk";

const job = (id: string, state: DeskJob["state"], extra: Partial<DeskJob> = {}): DeskJob => ({
  id, title: id, project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "w" },
  state, pull_requests: [], outputs: [], joined: false, disconnected: false, ...extra,
});

describe("the Threads list (B28)", () => {
  it("lists 내 차례, 작업 중 and 끝남 in that order, each only while it holds a thread", () => {
    const groups = threadGroups([job("a", "done"), job("b", "working"), job("c", "asking"), job("d", "queued"), job("e", "failed")]);
    expect(groups.map((group) => [group.key, group.jobs.map((one) => one.id)])).toEqual([
      ["turn", ["c", "e"]],
      ["working", ["b", "d"]],
      ["done", ["a"]],
    ]);
  });

  it("keeps a mission's workers under their Conductor and leaves out a thread pulled into the operator's tabs (B37)", () => {
    const groups = threadGroups([job("conductor", "working", { workers: ["w1", "w2"] }), job("w1", "done"), job("w2", "asking"), job("mine", "moved_out")]);
    expect(groups.map((group) => [group.key, group.jobs.map((one) => one.id)])).toEqual([["working", ["conductor"]]]);
  });

  it("counts the threads asking for ☰ and the outputs not seen for ▣", () => {
    const seen = { id: "o1", kind: "image" as const, label: "a.png", seen: true };
    const unseen = { id: "o2", kind: "doc" as const, label: "b.md", seen: false };
    const jobs = [job("a", "asking", { outputs: [seen, unseen] }), job("b", "asking"), job("c", "done", { outputs: [unseen] })];
    expect([askingCount(jobs), unseenOutputs(jobs)]).toEqual([2, 2]);
  });
});

describe("what counts as an output (D-65, B30)", () => {
  it("keeps images, pages, PRs and readable documents and leaves out code, diffs, logs and Hide AI's names", () => {
    expect((["image", "page", "pr", "markdown", "pdf", "html", "code", "diff", "log", "hide_ai"] as const).map(outputKindOf)).toEqual(["image", "page", "pr", "doc", "doc", "doc", null, null, null, null]);
  });
});

describe("a thread's lines (B25, B26)", () => {
  const text = (sessionUpdate: "user_message_chunk" | "agent_message_chunk", words: string): AcpSessionUpdate => ({ sessionUpdate, content: { type: "text", text: words } });

  it("joins one speaker's chunks and starts a new line where the speaker changes", () => {
    const lines = threadLines([text("user_message_chunk", "하나"), text("agent_message_chunk", "둘"), text("agent_message_chunk", " 셋"), text("user_message_chunk", "넷")]);
    expect(lines.map((line) => [line.kind, "text" in line ? line.text : null])).toEqual([["user", "하나"], ["agent", "둘 셋"], ["user", "넷"]]);
  });

  it("folds a call's updates into the call: its status, a shell command as its input, and the update's content replacing the call's", () => {
    const [line] = threadLines([
      { sessionUpdate: "tool_call", toolCallId: "t", title: "Bash", kind: "execute", status: "pending", rawInput: { command: "pnpm test" }, content: [{ type: "content", content: { type: "text", text: "old" } }] },
      { sessionUpdate: "tool_call_update", toolCallId: "t", status: "failed", content: [{ type: "content", content: { type: "text", text: "1 failed" } }, { type: "diff", path: "a.ts", oldText: "a", newText: "b" }] },
    ]);
    expect(line).toMatchObject({ kind: "tool", status: "failed", input: "pnpm test", output: "1 failed", diffs: [{ path: "a.ts", newText: "b" }] });
  });

  it("draws a call the reader turned into a Hide part as that part, and an update for it changes nothing", () => {
    const lines = threadLines(
      [{ sessionUpdate: "tool_call", toolCallId: "h", title: "Bash", kind: "execute", rawInput: { command: "hide request send worker --intent retry" } }, { sessionUpdate: "tool_call_update", toolCallId: "h", status: "completed" }],
      { h: { kind: "letter", to: "worker", body: "다시 올려줘" } },
    );
    expect(lines).toEqual([{ kind: "hide", id: "h", part: { kind: "letter", to: "worker", body: "다시 올려줘" }, command: "hide request send worker --intent retry" }]);
  });

  it("starts a new line where one speaker's message ends and the next begins", () => {
    const chunk = (messageId: string, words: string): AcpSessionUpdate => ({ sessionUpdate: "agent_message_chunk", content: { type: "text", text: words }, messageId });
    const lines = threadLines([chunk("m1", "하나"), chunk("m1", " 둘"), chunk("m2", "셋")]);
    expect(lines.map((line) => "text" in line && line.text)).toEqual(["하나 둘", "셋"]);
  });

  it("shows a shell call's command as its input and no other call's input", () => {
    const [line] = threadLines([{ sessionUpdate: "tool_call", toolCallId: "e", title: "Edit a.ts", kind: "edit", rawInput: { path: "a.ts", content: "x".repeat(10_000) } }]);
    expect(line).toMatchObject({ kind: "tool", input: null });
  });

  it("keeps a letter apart from the operator's own words by its origin", () => {
    const lines = threadLines([text("user_message_chunk", "내 말"), { sessionUpdate: "user_message_chunk", content: { type: "text", text: "편지" }, _meta: { hide: { origin: "letter" } } }]);
    expect(lines.map((line) => line.kind === "user" && line.origin)).toEqual(["operator", "letter"]);
  });
});

describe("a Desk message's mentions (B18)", () => {
  it("reads each mention by its own marked token, so a path or a name with spaces stays whole", () => {
    const project = { kind: "project" as const, id: "~/work/my app", name: "my app" };
    const session = { kind: "session" as const, id: "agent-7", name: "mention 버튼 정리" };
    const parts = mentionParts("@project(~/work/my app my app) 정리하고 @session(agent-7 mention 버튼 정리)에 알려줘 @me", [project, session], markedMention);
    expect(parts).toEqual([project, " 정리하고 ", session, "에 알려줘 @me"]);
  });
});
