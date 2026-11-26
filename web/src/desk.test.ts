import { describe, expect, it } from "vitest";
import { agentPickable, approvalButtons, deskFavorites, favoriteChosen, askedIn, askingCount, deskAgentWord, failureReason, deskPhase, holdsFoot, markedMention, mentionParts, namedText, outputKindOf, threadPhase, type ThreadTurn, sessionFacts, threadLines, threadTurns, type AcpSessionUpdate, type DeskAgent, type DeskJob, type DeskOption } from "./desk";
import type { AgentRow, DeskRecord } from "./snapshot";

const job = (id: string, state: DeskJob["state"], extra: Partial<DeskJob> = {}): DeskJob => ({
  id, agent_id: null, pane_id: `pane-${id}`, title: id, agent_kind: "claude", project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "w" },
  state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
  ...extra,
});

const text = (sessionUpdate: "user_message_chunk" | "agent_message_chunk", words: string): AcpSessionUpdate => ({ sessionUpdate, content: { type: "text", text: words } });

describe("the sidebar's Desk row (PRD desk-2-ui B46)", () => {
  it("counts the sessions Desk started that ask the operator", () => {
    expect(askingCount([job("a", "asking"), job("b", "asking"), job("c", "done"), job("d", "working")])).toBe(2);
  });
});

describe("a failed turn's reason (B5)", () => {
  it("drops the protocol's error name, and is none when the reply above already says it", () => {
    expect(failureReason("Internal error: API Error: 400 거부", null)).toBe("API Error: 400 거부");
    expect(failureReason("Internal error: API Error: 400 거부", "API Error: 400 거부")).toBeNull();
    expect(failureReason("Rate limit reached", "다른 말")).toBe("Rate limit reached");
    expect(failureReason(null, null)).toBeNull();
    expect(failureReason("Internal error:", null)).toBeNull();
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

describe("what the working line says (feedback 1 c3)", () => {
  it("says Desk's running step, else answering once it wrote, else thinking", () => {
    expect(deskPhase("files", "찾아보겠습니다.")).toBe("files");
    expect(deskPhase(null, "찾아보겠습니다.")).toBe("answering");
    expect(deskPhase(undefined, null)).toBe("thinking");
  });

  it("says a session's newest line: its words, its call's kind, a Hide part, or thinking", () => {
    const turn = (work: ThreadTurn["work"], answer: ThreadTurn["answer"] = []): ThreadTurn => ({ key: "t", user: null, work, answer, parts: [], files: 0, commands: 0 });
    const tool = (toolKind: "read" | "edit" | "execute" | "think") => ({ kind: "tool" as const, id: toolKind, title: toolKind, toolKind, status: "in_progress" as const, input: null, output: null, diffs: [], images: [], startedAtMs: null });
    expect(threadPhase(undefined)).toBe("thinking");
    expect(threadPhase(turn([tool("read")]))).toBe("files");
    expect(threadPhase(turn([tool("edit")]))).toBe("editing");
    expect(threadPhase(turn([tool("execute")]))).toBe("other");
    expect(threadPhase(turn([tool("think")]))).toBe("thinking");
    expect(threadPhase(turn([tool("read")], [{ kind: "agent", id: "a", message: null, text: "다 읽었어요", atMs: null }]))).toBe("answering");
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
        { sessionUpdate: "tool_call", toolCallId: "p", title: "Bash", kind: "execute", rawInput: { command: "hide task set" } },
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

describe("the answer a question Hide found was asked in (B4 question 나)", () => {
  const answer = (words: string, at: number | null): AcpSessionUpdate => ({ sessionUpdate: "agent_message_chunk", messageId: words, content: { type: "text", text: words }, ...(at === null ? {} : { _meta: { hide: { at_ms: at } } }) });
  const lines = threadLines([
    text("user_message_chunk", "스크린샷 찍어 줘"),
    answer("라이트를 찍었어요. 다크도 찍을까요?", 1_000),
    text("user_message_chunk", "결제해놨어"),
    answer("본인 확인은 직접 해 줄 수 있어?", 2_000),
    text("user_message_chunk", "응"),
    answer("끝났어요.", 3_000),
  ]);
  const turns = threadTurns(lines);
  const said = (id: string | null) => {
    const line = lines.find((one) => one.id === id);
    return line?.kind === "agent" ? line.text : null;
  };

  it("is the last answer written no later than the question was found", () => {
    expect(said(askedIn(turns, 2_500))).toBe("본인 확인은 직접 해 줄 수 있어?");
    expect(said(askedIn(turns, 2_000))).toBe("본인 확인은 직접 해 줄 수 있어?");
    expect(said(askedIn(turns, 1_200))).toBe("라이트를 찍었어요. 다크도 찍을까요?");
  });

  it("is none when no answer read is that old or says when it was written", () => {
    expect(askedIn(turns, 500)).toBeNull();
    expect(askedIn(threadTurns(threadLines([text("user_message_chunk", "해줘"), answer("할까요?", null)])), 9_000)).toBeNull();
  });
});

describe("a waiting card's buttons (D-92, frame A2 가)", () => {
  const option = (id: string, kind: DeskOption["kind"], name = id): DeskOption => ({ id, kind, name });
  const read = (options: DeskOption[]) => approvalButtons(options).map(({ option: one, word }) => [one?.id ?? null, word]);

  it("keeps every answer the agent offers, reject first, then its own others, 항상 허용 and 한 번 허용 last, in the agent's order within each", () => {
    expect(read([option("yes", "allow_once"), option("always", "allow_always"), option("no", "reject_once")])).toEqual([["no", "reject"], ["always", "allow_always"], ["yes", "allow_once"]]);
    expect(read([option("yes", "allow_once"), option("ask", "other"), option("never", "reject_always")])).toEqual([["never", "reject"], ["ask", null], ["yes", "allow_once"]]);
  });

  it("adds Hide's own 거절… to a card that offers no rejecting option, so it ends without 중지", () => {
    expect(read([option("plan", "other"), option("go", "allow_once")])).toEqual([[null, "reject"], ["plan", null], ["go", "allow_once"]]);
    expect(read([])).toEqual([[null, "reject"]]);
  });

  it("labels answers that would read the same word by the agent's own names instead", () => {
    expect(read([option("auto", "allow_always"), option("edits", "allow_always"), option("once", "allow_once"), option("no", "reject_once"), option("never", "reject_always")])).toEqual([
      ["no", null], ["never", null], ["auto", null], ["edits", null], ["once", "allow_once"],
    ]);
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
  });

  it("cannot be given an agent switched off, and can be given one signed out", () => {
    expect(agentPickable(agent("off", true))).toBe(false);
    expect(agentPickable(agent("ready", false))).toBe(true);
    expect(agentPickable(agent("preparing", null))).toBe(true);
  });
});

describe("a chip's session (D-86, B50)", () => {
  const row = (pane: string, label: string, extra: Partial<AgentRow> = {}) => ({ pane_id: pane, identity_label: label, mark: "going", agent_kind: "codex", last_prompt_at_unix_ms: 1_000, ...extra }) as AgentRow;
  const rows = new Map([row("w1:p1", "agent-journey", { lineage_parent_pane_id: "w1:p0" }), row("w1:p0", "agent-14586", { mark: "turn", agent_kind: "claude" })].map((one) => [one.pane_id, one]));
  const cards = [job("named", "working", { pane_id: "w1:p1", title: "agent-journey" })];

  it("reads a listed session's name, mark, card, time and parent from its row", () => {
    const facts = sessionFacts(rows, cards, { name: "agent-47656", mark: null, agent_kind: null, pane_id: "w1:p1" });
    expect(facts).toEqual({ listed: true, name: "agent-journey", mark: "going", agent_kind: "codex", card: cards[0], since: 1_000, parent: { name: "agent-14586", mark: "turn", agent_kind: "claude", pane_id: "w1:p0" }, record: null });
  });

  it("reads a session the sidebar no longer lists as gone, with no mark and no card", () => {
    expect(sessionFacts(rows, cards, { name: "m349", mark: "done", agent_kind: "claude", pane_id: "w1:gone" })).toEqual({ listed: false, name: null, mark: null, agent_kind: null, card: null, since: null, parent: null, record: null });
  });

  // Frame K: a session whose pane is gone reads as Hide's record of it, its card the app's SessionCard of that record.
  it("reads a session the sidebar no longer lists from its record when Hide keeps one: its title, agent, end and card", () => {
    const record: DeskRecord = {
      pane_id: "w1:gone", session: { device_id: "mini", agent: "claude", session_id: "s1" }, agent_kind: "claude", title: "1.3.0 후보 빌드 만들기",
      started_at_unix_ms: 100, ended_at_unix_ms: 900, line: "서명까지 확인했어요", pull_requests: [{ number: 2930, state: "merged" }],
      cwd: "/repo/.worktrees/release-1.3.0-rc9", path: null, project: "/repo/herdr-ide", thread: true,
    };
    const facts = sessionFacts(rows, cards, { name: "m349", mark: "done", agent_kind: "claude", pane_id: "w1:gone" }, [record]);
    expect([facts?.listed, facts?.name, facts?.mark, facts?.agent_kind, facts?.since, facts?.record]).toEqual([false, "1.3.0 후보 빌드 만들기", null, "claude", 900, record]);
    expect(facts?.card).toMatchObject({ title: "1.3.0 후보 빌드 만들기", state: "done", now: "서명까지 확인했어요", pull_requests: [{ number: 2930, state: "merged" }], project: { label: "herdr-ide" }, location: { label: "release-1.3.0-rc9" } });
    // Another pane's record is not this one's.
    expect(sessionFacts(rows, cards, { name: "m350", mark: null, agent_kind: null, pane_id: "w1:other" }, [record])?.record).toBeNull();
  });

  it("finds a session named only by its name, and leaves one no row carries to its own values", () => {
    expect(sessionFacts(rows, cards, { name: "agent-14586", mark: null, agent_kind: null })?.card).toBeNull();
    expect(sessionFacts(rows, cards, { name: "agent-14586", mark: null, agent_kind: null })?.listed).toBe(true);
    expect(sessionFacts(rows, cards, { name: "nobody", mark: null, agent_kind: null })).toBeNull();
  });
});

describe("namedText", () => {
  it("says each held mention by its name, as the operator reads the message", () => {
    const fixer = { kind: "session" as const, id: "w1:p3", name: "fixer" };
    const repo = { kind: "project" as const, id: "/Users/me/repo", name: "repo name" };
    const text = `${markedMention(fixer)} 결과를 ${markedMention(repo)}에 정리해 줘, ${markedMention(fixer)}`;
    expect(namedText(text, [fixer, repo])).toBe("@fixer 결과를 @repo name에 정리해 줘, @fixer");
  });
});

describe("holdsFoot", () => {
  it("keeps the foot when the browser nudges the view down while the content grows below it", () => {
    // Scroll anchoring moved 3065 to 3066 as a reply grew by 314 px; the resize that pins the foot comes after.
    expect(holdsFoot(true, 3065, 3066, 314, 48)).toBe(true);
  });

  it("lets go once the operator scrolls up, and holds again when they reach the foot", () => {
    expect(holdsFoot(true, 3386, 2486, 900, 48)).toBe(false);
    expect(holdsFoot(false, 2486, 2600, 786, 48)).toBe(false);
    expect(holdsFoot(false, 2600, 3380, 6, 48)).toBe(true);
  });

  it("stays held when content shrinking under the foot pulls the view up", () => {
    expect(holdsFoot(true, 3386, 3350, 0, 48)).toBe(true);
  });
});

describe("the favorite models the agent menu lists first", () => {
  const agent = (id: DeskAgent["id"], models: string[] = [], efforts: string[] = [], state: DeskAgent["state"] = "ready"): DeskAgent => ({ id, label: id, state, detail: null, signed_in: true, login_command: "", models: models.map((model) => ({ id: model, label: model.toUpperCase() })), efforts: efforts.map((effort) => ({ id: effort, label: effort })) });
  const selection = (agentId: DeskAgent["id"] | null) => ({ agent: agentId, model: null, effort: null });

  it("lists a favorite whose agent the bar offers and whose model and effort that agent's last session offered", () => {
    const agents = [agent("claude", ["opus", "sonnet"], ["high"]), agent("codex")];
    const list = deskFavorites([{ agent: "claude", model: "opus", effort: "high", purpose: "General" }, { agent: "claude", model: "haiku", purpose: "" }], agents, selection("claude"));
    expect(list.map((favorite) => [favorite.agent.id, favorite.model, favorite.effort, favorite.modelLabel, favorite.purpose])).toEqual([["claude", "opus", "high", "OPUS", "General"]]);
  });

  it("leaves out a favorite that names a profile or an agent that is off, which Desk cannot start", () => {
    const agents = [agent("claude", ["opus"]), agent("codex", [], [], "off")];
    expect(deskFavorites([{ agent: "claude", profile: "p-work", model: "opus", purpose: "" }, { agent: "codex", purpose: "" }], agents, selection("claude"))).toEqual([]);
  });

  it("lists a favorite of an agent that has offered no models yet only while another agent is chosen, since a switch applies it as the session opens", () => {
    const agents = [agent("claude"), agent("codex")];
    const candidates = [{ agent: "claude", model: "opus", purpose: "" }];
    expect(deskFavorites(candidates, agents, selection("codex"))).toHaveLength(1);
    expect(deskFavorites(candidates, agents, selection("claude"))).toEqual([]);
  });

  it("reads a favorite as chosen when the agent, model and effort it fixes are the ones Desk goes to", () => {
    const favorite = deskFavorites([{ agent: "claude", model: "opus", purpose: "" }], [agent("claude", ["opus"])], selection("claude"))[0]!;
    expect(favoriteChosen(favorite, { agent: "claude", model: "opus", effort: "high" })).toBe(true);
    expect(favoriteChosen(favorite, { agent: "claude", model: "sonnet", effort: null })).toBe(false);
  });
});
