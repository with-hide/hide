import { describe, expect, it } from "vitest";
import { threadLines, type AcpSessionUpdate, type DeskSessionRef, type ThreadHidePart } from "../../desk";
import { conversationPhase, conversationRows, conversationTurns, runCounts, type ConversationRow } from "./rows";

const say = (kind: "user_message_chunk" | "agent_message_chunk", words: string, id = words): AcpSessionUpdate => ({ sessionUpdate: kind, messageId: id, content: { type: "text", text: words } });
const run = (id: string, command: string, status: "completed" | "failed" | "in_progress" = "completed"): AcpSessionUpdate => ({ sessionUpdate: "tool_call", toolCallId: id, title: command, kind: "execute", status, rawInput: { command } });
const read = (id: string, path: string): AcpSessionUpdate => ({ sessionUpdate: "tool_call", toolCallId: id, title: `Read ${path}`, kind: "read", status: "completed", locations: [{ path }] });
const edit = (id: string, path: string, newText: string): AcpSessionUpdate => ({ sessionUpdate: "tool_call", toolCallId: id, title: `Write ${path}`, kind: "edit", status: "completed", content: [{ type: "diff", path, newText }] });
const peer: DeskSessionRef = { name: "m110-mini-journey", mark: "going", agent_kind: "claude", pane_id: "w1:p2" };

/** What the rows draw, as a reader names them: the words, the gray lines' counts, the bands and the foot. */
function shape(rows: readonly ConversationRow[]): string[] {
  return rows.map((row) => {
    switch (row.kind) {
      case "user":
        return `user:${row.line.text}`;
      case "text":
        return `text:${row.line.text}`;
      case "run":
        return `run:${row.run.lines.map((line) => line.id).join(",")}${row.open ? ":open" : ""}`;
      case "work":
        return `  ${row.line.id}`;
      case "letter":
        return `letter:${row.letter.direction}:${row.letter.kind ?? "-"}:${row.letter.first}${row.open ? ":open" : ""}`;
      case "question":
        return `question:${row.line.questions.map((question) => `${question.text}=${question.answer ?? "?"}`).join("|")}`;
      case "foot":
        return `foot:${row.live?.id ?? "-"}`;
      default:
        return row.kind;
    }
  });
}

const rowsOf = (updates: AcpSessionUpdate[], parts: Record<string, ThreadHidePart> = {}, options: Partial<Parameters<typeof conversationRows>[1]> = {}) =>
  conversationRows(conversationTurns(threadLines(updates, parts)), { working: false, open: new Set(), handed: false, foot: false, ...options });

describe("a conversation's rows (mockup v3 section 2)", () => {
  it("shows every piece of the agent's words and folds only the tools between them, one gray line per run", () => {
    const rows = rowsOf([
      say("user_message_chunk", "교육 자료 계획 세워 줘"),
      say("agent_message_chunk", "repl로 Drive를 읽을게요."),
      run("c1", "aside repl a", "failed"),
      read("r1", "presentation/AGENTS.md"),
      say("agent_message_chunk", "입문반 파일을 찾았어요."),
      run("c2", "aside repl b"),
      run("c3", "aside repl c"),
      say("agent_message_chunk", "구성안 초안을 썼어요."),
    ]);
    expect(shape(rows)).toEqual(["user:교육 자료 계획 세워 줘", "text:repl로 Drive를 읽을게요.", "run:c1,r1", "text:입문반 파일을 찾았어요.", "run:c2,c3", "text:구성안 초안을 썼어요."]);
  });

  it("keeps a turn's summary in view when the turn ended by asking its own question, and the question with its answer", () => {
    const rows = rowsOf([
      say("user_message_chunk", "계획 세워 줘"),
      run("c1", "ls"),
      say("agent_message_chunk", "구성안 초안을 쓰고 열었어요. 먼저 정할 게 하나 있어요."),
      { sessionUpdate: "tool_call", toolCallId: "q1", title: "AskUserQuestion", kind: "other", status: "pending", _meta: { hide: { question: [{ text: "어느 폴더에 둘까요?", choices: ["새 폴더", "기존 폴더"] }] } } },
      { sessionUpdate: "tool_call_update", toolCallId: "q1", status: "completed", _meta: { hide: { answers: ["새 폴더"] } } },
    ]);
    expect(shape(rows)).toEqual(["user:계획 세워 줘", "run:c1", "text:구성안 초안을 쓰고 열었어요. 먼저 정할 게 하나 있어요.", "question:어느 폴더에 둘까요?=새 폴더"]);
  });

  it("opens a run's lines in place under its gray line, never above it", () => {
    const updates = [say("agent_message_chunk", "볼게요"), run("c1", "ls"), run("c2", "pwd"), say("agent_message_chunk", "끝")];
    const folded = rowsOf(updates);
    const key = folded.find((row) => row.kind === "run")!.key;
    expect(shape(rowsOf(updates, {}, { open: new Set([key]) }))).toEqual(["text:볼게요", "run:c1,c2:open", "  c1", "  c2", "text:끝"]);
  });
});

describe("a running turn (mockup v3 section 1)", () => {
  const updates = [say("user_message_chunk", "찾아 줘"), say("agent_message_chunk", "상위 폴더로 들어가 볼게요."), run("c1", "ls"), run("c2", "cat a"), run("c3", "aside repl x", "in_progress")];

  it("keeps the finished calls in the gray line and moves the call still running under the owl, once", () => {
    expect(shape(rowsOf(updates, {}, { working: true, foot: true }))).toEqual(["user:찾아 줘", "text:상위 폴더로 들어가 볼게요.", "run:c1,c2", "foot:c3"]);
  });

  it("draws no gray line while the only call of the run still runs, and no live line once the session rests", () => {
    expect(shape(rowsOf([say("agent_message_chunk", "볼게요"), run("c1", "ls", "in_progress")], {}, { working: true, foot: true }))).toEqual(["text:볼게요", "foot:c1"]);
    expect(shape(rowsOf(updates, {}, { working: false, foot: true }))).toEqual(["user:찾아 줘", "text:상위 폴더로 들어가 볼게요.", "run:c1,c2,c3", "foot:-"]);
  });

  it("says what the session does now from its newest line", () => {
    const phase = (more: AcpSessionUpdate[]) => conversationPhase(conversationTurns(threadLines([say("user_message_chunk", "해 줘"), ...more])).at(-1));
    expect(phase([])).toBe("thinking");
    expect(phase([read("r", "a.ts")])).toBe("files");
    expect(phase([edit("e", "a.ts", "x")])).toBe("editing");
    expect(phase([run("c", "ls", "in_progress")])).toBe("other");
    expect(phase([run("c", "ls"), say("agent_message_chunk", "다 했어요")])).toBe("answering");
  });
});

describe("letters (mockup v3 section 5)", () => {
  it("draws every letter as its own band, sent and received alike, with its kind and first line", () => {
    const sent = (body: string): ThreadHidePart => ({ kind: "letter", peer, body, letterKind: "request", target: `letter:${peer.pane_id}` });
    const rows = rowsOf(
      [
        say("agent_message_chunk", "한 곳만 더 막으라고 할게요."),
        run("l1", "hide letter send m110 --kind request"),
        run("l2", "hide letter send m110 --kind request"),
        run("c1", "ls"),
        { sessionUpdate: "user_message_chunk", messageId: "b", content: { type: "text", text: "🦉 m110-mini-journey 답장: 1) 지웠어요 · hide inbox" }, _meta: { hide: { origin: "letter", from: "m110-mini-journey", kind: "reply", summary: "1) 지웠어요" } } },
        say("agent_message_chunk", "확인했어요."),
      ],
      { l1: sent("실행 부분 맨 앞에서도 확인해 주세요.\n둘째 줄"), l2: sent("하나 더요") },
    );
    expect(shape(rows)).toEqual(["text:한 곳만 더 막으라고 할게요.", "letter:sent:request:실행 부분 맨 앞에서도 확인해 주세요.", "letter:sent:request:하나 더요", "run:c1", "letter:received:reply:1) 지웠어요", "text:확인했어요."]);
    const band = rows.find((row) => row.kind === "letter" && row.letter.direction === "sent")!;
    expect(band.kind === "letter" && band.letter.text).toBe("실행 부분 맨 앞에서도 확인해 주세요.\n둘째 줄");
  });

  it("opens a letter in place by its key", () => {
    const updates: AcpSessionUpdate[] = [{ sessionUpdate: "user_message_chunk", messageId: "b", content: { type: "text", text: "끝났어요\n자세히는" }, _meta: { hide: { origin: "letter", from: "lead", kind: "report" } } }];
    const key = rowsOf(updates)[0]!.key;
    expect(shape(rowsOf(updates, {}, { open: new Set([key]) }))).toEqual(["letter:received:report:끝났어요:open"]);
  });
});

describe("a run's counts by kind (mockup v3 section 4)", () => {
  it("counts commands, failures, files read and changed each once, with the lines changed", () => {
    const lines = threadLines([run("c1", "ls"), run("c2", "x", "failed"), read("r1", "a.md"), read("r2", "a.md"), edit("e1", "plan.md", "a\nb\nc"), edit("e2", "plan.md", "a\nb\nc\nd"), { sessionUpdate: "tool_call", toolCallId: "s", title: "grep x", kind: "search", status: "completed" }]);
    expect(runCounts(lines)).toEqual({ commands: 2, failed: 1, read: 1, searched: 1, fetched: 0, edited: 1, added: 7, removed: 0, other: 0, thoughts: 0 });
  });
});
