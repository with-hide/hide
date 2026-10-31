import { describe, expect, it } from "vitest";
import { agentPickable, askedOnce, askingCount, deskAgentWord, markedMention, mentionParts, outputKindOf, threadGroups, threadLines, threadTurns, unseenOutputs, type AcpSessionUpdate, type DeskAgent, type DeskJob } from "./desk";

const job = (id: string, state: DeskJob["state"], extra: Partial<DeskJob> = {}): DeskJob => ({
  id, agent_id: null, pane_id: `pane-${id}`, title: id, agent_kind: "claude", project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "w" },
  state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
  ...extra,
});

const text = (sessionUpdate: "user_message_chunk" | "agent_message_chunk", words: string): AcpSessionUpdate => ({ sessionUpdate, content: { type: "text", text: words } });

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
    const groups = threadGroups([job("conductor", "working", { workers: ["pane-w1", "pane-w2"] }), job("w1", "done"), job("w2", "asking"), job("mine", "moved_out")]);
    expect(groups.map((group) => [group.key, group.jobs.map((one) => one.id)])).toEqual([["working", ["conductor"]]]);
  });

  it("counts the threads asking for ☰ and the outputs not seen for ▣", () => {
    const seen = { id: "o1", kind: "image" as const, label: "a.png", thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: null };
    const unseen = { id: "o2", kind: "doc" as const, label: "b.md", thumb: null, seen: false, pr_state: null, device_id: null, checkout: null, path: null, url: null };
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
    const peer = { name: "worker", mark: "going" as const, agent_kind: "codex" };
    const lines = threadLines(
      [{ sessionUpdate: "tool_call", toolCallId: "h", title: "Bash", kind: "execute", rawInput: { command: "hide request send worker --intent retry" } }, { sessionUpdate: "tool_call_update", toolCallId: "h", status: "completed" }],
      { h: { kind: "letter", direction: "sent", peer, body: "다시 올려줘" } },
    );
    expect(lines).toEqual([{ kind: "hide", id: "h", part: { kind: "letter", direction: "sent", peer, body: "다시 올려줘" }, command: "hide request send worker --intent retry" }]);
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

describe("a thread's turns (B25, B26)", () => {
  it("starts a turn at each message, folds the work before the answer into counts, and keeps the parts that lead somewhere", () => {
    const peer = { name: "worker", mark: "going" as const, agent_kind: "codex" };
    const lines = threadLines(
      [
        text("user_message_chunk", "고쳐줘"),
        text("agent_message_chunk", "볼게요"),
        { sessionUpdate: "tool_call", toolCallId: "e", title: "a.ts", kind: "edit", status: "completed" },
        { sessionUpdate: "tool_call", toolCallId: "x", title: "Bash", kind: "execute", status: "completed", rawInput: { command: "pnpm test" } },
        { sessionUpdate: "tool_call", toolCallId: "p", title: "Bash", kind: "execute", rawInput: { command: "hide plan set" } },
        { sessionUpdate: "tool_call", toolCallId: "l", title: "Bash", kind: "execute", rawInput: { command: "hide request send worker" } },
        text("agent_message_chunk", "고쳤어요"),
        text("user_message_chunk", "고마워"),
      ],
      { p: { kind: "plan", plan: {} as never }, l: { kind: "letter", direction: "sent", peer, body: "봐줘" } },
    );
    const turns = threadTurns(lines);
    expect(turns.map((turn) => turn.user?.text)).toEqual(["고쳐줘", "고마워"]);
    expect(turns[0]).toMatchObject({ files: 1, commands: 1, answer: [{ text: "고쳤어요" }], parts: [{ id: "l" }] });
    expect(turns[0]!.work.map((line) => line.id)).toEqual([lines[1]!.id, "e", "x", "p", "l"]);
    expect(turns[1]).toMatchObject({ work: [], answer: [], parts: [] });
  });
});

describe("a letter's sender and a command's start (B25, D-75)", () => {
  it("names who a letter came from as its chip draws them, and keeps when a call was made", () => {
    const chip = (name: string) => ({ name: `chip:${name}`, mark: "going" as const, agent_kind: "codex" });
    const lines = threadLines([
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "다시 " }, _meta: { hide: { origin: "letter", from: "lead" } } },
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "올려줘" }, _meta: { hide: { origin: "letter", from: "lead" } } },
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "고마워" }, _meta: { hide: { origin: "letter", from: "review" } } },
      text("user_message_chunk", "내 말"),
      { sessionUpdate: "tool_call", toolCallId: "c", title: "Bash", kind: "execute", status: "in_progress", _meta: { hide: { at_ms: 1_793_000_000_000 } } },
    ], {}, chip);
    expect(lines.map((line) => (line.kind === "user" ? [line.text, line.peer?.name ?? null] : [line.kind, line.kind === "tool" ? line.startedAtMs : null]))).toEqual([
      ["다시 올려줘", "chip:lead"],
      ["고마워", "chip:review"],
      ["내 말", null],
      ["tool", 1_793_000_000_000],
    ]);
  });
});

describe("an asking thread's question (D-75)", () => {
  const turns = (answer: string) => threadTurns(threadLines([text("user_message_chunk", "해줘"), text("agent_message_chunk", answer)]));

  it("is said once, at the foot: an answer that is the question goes, and one that ends with it keeps the rest", () => {
    expect(askedOnce(turns("다크 모드도 찍을까요?"), "다크 모드도  찍을까요?")[0]!.answer).toEqual([]);
    expect(askedOnce(turns("라이트는 끝났어요.\n다크 모드도 찍을까요?"), "다크 모드도 찍을까요?")[0]!.answer.map((line) => line.text)).toEqual(["라이트는 끝났어요."]);
    expect(askedOnce(turns("라이트는 끝났어요."), "다크 모드도 찍을까요?")[0]!.answer.map((line) => line.text)).toEqual(["라이트는 끝났어요."]);
    expect(askedOnce(turns("다크 모드도 찍을까요?"), null)[0]!.answer).toHaveLength(1);
  });
});

describe("an agent in Desk's bar (D-79, D-81, B44)", () => {
  const agent = (state: DeskAgent["state"], signed_in: boolean | null): DeskAgent => ({ id: "codex", label: "Codex", state, detail: null, signed_in, login_command: "codex login", models: [], efforts: [] });

  it("says a ready agent's sign-in as Settings reads it, and nothing while that is unread", () => {
    expect(deskAgentWord(agent("ready", true))).toBe("signed_in");
    expect(deskAgentWord(agent("ready", false))).toBe("signed_out");
    expect(deskAgentWord(agent("ready", null))).toBeNull();
  });

  it("says the state of one not ready, whatever its sign-in, so a downloaded adapter never reads as signed in", () => {
    expect(deskAgentWord(agent("preparing", true))).toBe("preparing");
    expect(deskAgentWord(agent("install_failed", true))).toBe("install_failed");
    expect(deskAgentWord(agent("no_ask_mode", true))).toBe("no_ask_mode");
  });

  it("cannot be given an agent Hide cannot hold to asking or one switched off, and can be given one signed out", () => {
    expect(agentPickable(agent("no_ask_mode", true))).toBe(false);
    expect(agentPickable(agent("off", true))).toBe(false);
    expect(agentPickable(agent("ready", false))).toBe(true);
    expect(agentPickable(agent("preparing", null))).toBe(true);
  });
});
