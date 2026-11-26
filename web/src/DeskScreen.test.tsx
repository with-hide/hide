// @vitest-environment jsdom
import { Blob as NodeBlob } from "node:buffer";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { configureAttachments } from "./attachments";
import { configureFileBytes, receiveBytes, receiveBytesError } from "./fileBytes";
import { clearPreviews } from "./filePreview";
import { TooltipProvider } from "./components/ui/tooltip";
import type { DeskAgent, DeskItem, DeskJob, DeskOption, DeskOutput } from "./desk";
import { DeskScreen, DeskSettingsPage } from "./DeskScreen";
import { clientI18n } from "./i18n/translator";
import type { DeskCommands, DeskMentions, DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import type { DispatchFn } from "./ws";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
Element.prototype.scrollIntoView = () => {};

// Project `repo` is named in the `@` menu by its primary checkout's path, as the core sends it.
const rest = {
  navigator: {
    focused_device_id: "local", focused_checkout_id: null, focused_workspace_id: null,
    devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
    workspaces: [{ id: "w-repo", label: "repo", device_id: "local", agent_scope: emptyScope(), checkouts: [
      { id: "c-fix", workspace_id: "w-repo", path: "/work/repo-fix", label: "fix", tabs: [], active_tab_id: null, strip: [] },
      { id: "c-home", workspace_id: "w-repo", path: "/work/repo-home", label: "main", tabs: [], active_tab_id: null, strip: [] },
    ] }],
  },
} as unknown as SnapshotRest;

const claude = (state: DeskAgent["state"], signedIn: boolean | null): DeskAgent => ({ id: "claude", label: "Claude Code", state, detail: null, signed_in: signedIn, login_command: "claude /login", models: [], efforts: [] });

const desk = (items: DeskItem[], state: DeskAgent["state"] = "ready", signedIn: boolean | null = true, queued: DeskSection["conversation"]["queued"] = [], held = false): DeskSection => ({
  selection: { agent: "claude", model: null, effort: null },
  agents: [claude(state, signedIn)],
  host: { state: "ready", detail: null },
  conversation: { generation: 1, items, dropped: 0, queued, queued_max: 8, queued_full: queued.length >= 8, queued_held: held },
  approvals_waiting: 0,
  settings: { enabled: true, default_agent: "claude", location: "worktree", cap: 8, permission: "allow_all" },
  lane: { view_width: null, session_width: null }, opens: [],
});

const fixer = { kind: "session" as const, id: "w1:p2", name: "fixer" };
const repo = { kind: "project" as const, id: "/work/repo-home", name: "repo" };

describe("Desk's room", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(async () => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    // The `@` menu's list measures itself, which jsdom cannot.
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("en"); });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
    clearPreviews();
  });

  let actions = createActions(() => true);
  async function render(section: DeskSection, mentions: DeskMentions = { sessions: [], projects: [] }) {
    actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, desk: section, deskLive: null, deskJobs: { jobs: [], workers: [], named: [] }, deskMentions: mentions });
      root.render(<TooltipProvider><DeskScreen actions={actions} /></TooltipProvider>);
    });
  }
  const show = async (section: DeskSection) => act(async () => { useShellStore.setState({ desk: section }); });
  const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
  const press = async (selector: string) => {
    const element = document.body.querySelector<HTMLElement>(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => element.click());
  };

  it("leaves the room when Desk is switched off in front (B41, B44)", async () => {
    const off = (section: DeskSection): DeskSection => ({ ...section, settings: { ...section.settings, enabled: false } });
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
    // No panel came before Desk here, so All projects takes its place.
    await show(off(desk([])));
    expect(useUiStore.getState().screen).toEqual({ kind: "main" });
  });

  it("leaves the room on its head's 나가기 ✕ (feedback 1 D2)", async () => {
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    await press("[data-desk-leave]");
    expect(useUiStore.getState().screen).toEqual({ kind: "main" });
  });

  it("lands at the conversation's foot when Desk opens again, though the browser nudges the view as the replies fill in", async () => {
    const resized: (() => void)[] = [];
    vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resized.push(callback); } observe() {} unobserve() {} disconnect() {} });
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([{ kind: "user", id: 1, text: "what runs now?", at_ms: 0, mentions: [] }]));
    const box = document.body.querySelector<HTMLElement>("[data-desk-chat-scroll]")!;
    // The replies' text fills in after Desk's layout pass: the content grows, and scroll anchoring moves the view down 1 px first.
    Object.defineProperty(box, "scrollHeight", { configurable: true, value: 3072 });
    Object.defineProperty(box, "clientHeight", { configurable: true, value: 600 });
    await act(async () => { box.scrollTop = 1; box.dispatchEvent(new Event("scroll")); });
    await act(async () => { for (const callback of resized) callback(); });
    expect(box.scrollTop).toBe(3072);
  });

  it("keeps a sent message in the box until the conversation shows it, so a refused one stays", async () => {
    useUiStore.setState({ deskDraft: { text: "ask @session(w1:p2 fixer)", mentions: [fixer] } });
    await render(desk([]));
    await press("[data-desk-composer-send]");
    expect(sent("desk_send")).toEqual([{ text: "ask @session(w1:p2 fixer)", mentions: [fixer] }]);
    expect(useUiStore.getState().deskDraft.text).toBe("ask @session(w1:p2 fixer)");

    await show(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null }]));
    expect(useUiStore.getState().deskDraft.text).toBe("ask @session(w1:p2 fixer)");

    await show(desk([
      { kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null },
      { kind: "user", id: 2, text: "ask @session(w1:p2 fixer)", at_ms: 0, mentions: [fixer] },
    ]));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
  });

  it("lets a message the agent could not take go from the box, since the core keeps it as the first waiting one (B6)", async () => {
    useUiStore.setState({ deskDraft: { text: "ask @session(w1:p2 fixer)", mentions: [fixer] } });
    await render(desk([], "ready", false));
    await press("[data-desk-composer-send]");
    // The core refused the open, said how to sign in and put the message first in the waiting line.
    await show(desk([{ kind: "notice", id: 2, notice: "login_needed", agent: "claude", detail: "claude /login" }], "ready", false, [{ id: 3, text: "ask @session(w1:p2 fixer)", at_ms: 0, mentions: [fixer] }]));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
    expect(document.body.querySelectorAll("[data-desk-queued]")).toHaveLength(1);
  });

  it("sends the operator's last message again as they wrote it on 다시 시도", async () => {
    await render(desk([
      { kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] },
      { kind: "notice", id: 2, notice: "turn_failed", agent: "claude", detail: null },
    ]));
    await press("[data-desk-notice-action='retry']");
    // The core sends the files of message 1 with it again, when it had any.
    expect(sent("desk_send")).toEqual([{ text: "@project(/work/repo-home repo) fix the build", mentions: [repo], again: 1 }]);
  });

  const dropFile = async (name: string, bytes: number[], at = "[data-desk-composer-frame]") => {
    const file = new File([new Uint8Array(bytes)], name);
    file.stream = () => new NodeBlob([new Uint8Array(bytes)]).stream() as ReadableStream<Uint8Array<ArrayBuffer>>;
    const drop = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [file] } });
    await act(async () => { document.body.querySelector(at)!.dispatchEvent(drop); });
    await vi.waitFor(() => expect(document.body.querySelector('[data-desk-attachment="reading"]')).toBeNull());
  };
  const tiles = () => [...document.body.querySelectorAll("[data-desk-composer-frame] [data-desk-attachment]")].map((tile) => [tile.getAttribute("data-desk-attachment"), tile.textContent]);

  it("sends a dropped file's stage with the words, keeps it in the box until the conversation shows the message, and draws it on the bubble (desk-2-ui B41)", async () => {
    configureAttachments({ dispatch: (event) => { events.push(event); return true; }, sendBinary: () => undefined });
    useUiStore.setState({ deskDraft: { text: "왜 끊기는지 봐 줘", mentions: [] } });
    await render(desk([]));
    // Dropped on the column's head, far from the box, the file still lands in it (Doop L 가).
    await dropFile("install.log", [7, 8], "[data-desk-chat-head]");

    await press("[data-desk-composer-send]");
    const [message] = sent("desk_send") as { request_id: string; stages: string[] }[];
    expect(message).toEqual({ text: "왜 끊기는지 봐 줘", mentions: [], request_id: expect.stringMatching(/^[0-9a-f-]{36}$/), stages: [`${message!.request_id}-0`] });
    expect(sent("attachment_stage")).toEqual([{ request_id: `${message!.request_id}-0`, name: "install.log", size: 2, clipboard: false }]);
    expect(tiles()).toHaveLength(1);
    // A file dropped while the message is on its way stays for the next one.
    await dropFile("later.txt", [1]);

    await show(desk([{ kind: "user", id: 1, text: "왜 끊기는지 봐 줘", at_ms: 0, mentions: [], attachments: [{ name: "install.log", bytes: 2, path: `/state/desk-attachments/${message!.request_id}-0-install.log` }] }]));
    expect(tiles()).toEqual([["ready", expect.stringContaining("later.txt")]]);
    expect(document.body.querySelector("[data-desk-sent-attachments]")?.textContent).toContain("install.log");
  });

  // letter-47579: the line is the one way back, so a refused message never also returns to the box, where the next send carried its words twice.
  it("leaves the box empty after a refused send and shows the message held first in line with its file (B6)", async () => {
    configureAttachments({ dispatch: (event) => { events.push(event); return true; }, sendBinary: () => undefined });
    useUiStore.setState({ deskDraft: { text: "이거 봐 줘", mentions: [] } });
    await render(desk([]));
    await dropFile("shot.png", [1, 2]);
    await press("[data-desk-composer-send]");
    const [message] = sent("desk_send") as { request_id: string }[];
    const file = { name: "shot.png", bytes: 2, path: `/state/desk-attachments/${message!.request_id}-0-shot.png` };
    await show(desk([{ kind: "user", id: 1, text: "이거 봐 줘", at_ms: 0, mentions: [], attachments: [file] }]));
    expect(useUiStore.getState().deskDraft.text).toBe("");

    // The agent was not signed in: the core moved the message, file and all, to the head of the line and holds the line.
    await show(desk([{ kind: "notice", id: 2, notice: "login_needed", agent: "claude", detail: "claude /login" }], "ready", true, [{ id: 3, text: "이거 봐 줘", at_ms: 0, mentions: [], attachments: [file] }], true));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
    expect(tiles()).toEqual([]);
    const held = document.body.querySelector("[data-desk-queued='3']");
    expect(held?.textContent).toContain("이거 봐 줘");
    expect(held?.textContent).toContain("Sent after you restart");
    expect(held?.querySelector("[data-desk-sent-attachments]")?.textContent).toContain("shot.png");
  });

  // letter-47596: a message whose file is gone stays in the conversation, unsent, and is not offered again.
  it("keeps a message whose file is gone in the conversation with its line and leaves the box empty", async () => {
    configureAttachments({ dispatch: (event) => { events.push(event); return true; }, sendBinary: () => undefined });
    useUiStore.setState({ deskDraft: { text: "이거 봐 줘", mentions: [] } });
    await render(desk([]));
    await dropFile("shot.png", [1, 2]);
    await press("[data-desk-composer-send]");
    const [message] = sent("desk_send") as { request_id: string }[];
    const user: DeskItem = { kind: "user", id: 1, text: "이거 봐 줘", at_ms: 0, mentions: [], attachments: [{ name: "shot.png", bytes: 2, path: `/state/desk-attachments/${message!.request_id}-0-shot.png` }] };
    await show(desk([user]));

    await show(desk([user, { kind: "notice", id: 2, notice: "attachments_gone", agent: "claude", detail: null }]));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
    expect(tiles()).toEqual([]);
    expect(document.body.querySelector("[data-desk-notice='attachments_gone']")?.textContent).toContain("The file is gone, so this wasn't sent");
    expect(document.body.querySelector("[data-desk-notice='attachments_gone'] [data-desk-notice-action]")).toBeNull();
    expect(document.body.querySelectorAll("[data-desk-queued]")).toHaveLength(0);
  });

  it("draws only a failed turn in the failed colour: a stopped host is neutral words, a failed install keeps the warning colour", async () => {
    const colour = (kind: string) => document.body.querySelector<HTMLElement>(`[data-desk-notice='${kind}'] span`)!.className;
    await render(desk([{ kind: "notice", id: 1, notice: "host_stopped", agent: "claude", detail: null }, { kind: "notice", id: 3, notice: "install_failed", agent: "claude", detail: null }]));
    expect(colour("host_stopped")).not.toMatch(/text-destructive|text-warning/);
    expect(colour("install_failed")).toContain("text-warning");
  });

  it("hides a failed turn's 다시 시도 while its message waits, so one more press does not send it twice", async () => {
    const failed: DeskItem[] = [user(1), { kind: "notice", id: 2, notice: "turn_failed", agent: "claude", detail: null }];
    await render(desk(failed, "ready", true, [{ id: 3, text: "실패해 보세요", at_ms: 0, mentions: [] }]));
    expect(document.body.querySelector("[data-desk-notice-action='retry']")).toBeNull();
    await show(desk(failed));
    expect(document.body.querySelector("[data-desk-notice-action='retry']")).not.toBeNull();
  });

  it("names the agent's own reason on a failed turn when it gave one", async () => {
    const notice = () => document.body.querySelector<HTMLElement>("[data-desk-notice='turn_failed']")!.textContent!;
    await render(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: "Rate limit reached" }]));
    expect(notice()).toContain("· Rate limit reached");
    await show(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null }]));
    expect(notice()).not.toContain("·");
  });

  const reply = (id: number, words: string): DeskItem => ({ kind: "reply", id, agent: "claude", text: words, at_ms: 0, state: "stopped", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 }, mentions: [], missions: [] });
  const user = (id: number): DeskItem => ({ kind: "user", id, text: "실패해 보세요", at_ms: 0, mentions: [] });

  it("says a failed turn's reason once, in plain words and the failed colour, and drops an older 다시 시도 once a newer turn ran", async () => {
    const refusal = "API Error: 400 이 모델은 지금 쓸 수 없어요";
    await render(desk([
      user(1), reply(2, refusal), { kind: "notice", id: 3, notice: "turn_failed", agent: "claude", detail: `Internal error: ${refusal}` },
      user(4), reply(5, refusal), { kind: "notice", id: 6, notice: "turn_failed", agent: "claude", detail: `Internal error: ${refusal}` },
    ]));
    const text = document.body.textContent!;
    // The agent's words stand in its reply alone: no wire prefix, no second copy in the notice, no `stopped` label beside them.
    expect(text).not.toContain("Internal error");
    expect(text.split("이 모델은 지금 쓸 수 없어요")).toHaveLength(3);
    expect(document.body.querySelector("[data-desk-reply='stopped']")).toBeNull();
    const notices = Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-notice='turn_failed']"));
    expect(notices.map((one) => one.querySelector("span")!.className.includes("text-destructive"))).toEqual([true, true]);
    // Only the newest failed turn offers `다시 시도`.
    expect(notices.map((one) => one.querySelector("[data-desk-notice-action]") !== null)).toEqual([false, true]);
  });

  it("names a reason the reply did not say, without the protocol's error name", async () => {
    await render(desk([user(1), reply(2, ""), { kind: "notice", id: 3, notice: "turn_failed", agent: "claude", detail: "Internal error: Rate limit reached" }]));
    const notice = document.body.querySelector<HTMLElement>("[data-desk-notice='turn_failed']")!.textContent!;
    expect(notice).toContain("· Rate limit reached");
    expect(notice).not.toContain("Internal error");
  });

  it("marks the head with Desk's own mark, as the sidebar's row draws it (Doop 1)", async () => {
    await render(desk([]));
    expect(document.body.querySelector("[data-desk-chat-head] [data-desk-mark]")).not.toBeNull();
  });

  it("sends an example line at once, and only writes it in the box while the agent cannot take it", async () => {
    await render(desk([]));
    const example = document.body.querySelector<HTMLElement>("[data-desk-example]")!.textContent!;
    await press("[data-desk-example]");
    expect(sent("desk_send")).toEqual([{ text: example, mentions: [] }]);

    events = [];
    await show(desk([], "preparing"));
    await press("[data-desk-example]");
    expect(sent("desk_send")).toEqual([]);
    expect(useUiStore.getState().deskDraft).toEqual({ text: example, mentions: [] });
  });

  const session = (id: string, name: string, parent: string | null = null): DeskMentions["sessions"][number] => ({
    id, name, agent_kind: "claude", mark: "going", detail: "테스트 고치는 중", parent, project: "repo", branch: "main", last_prompt_at_unix_ms: null,
  });
  const tree: DeskMentions = { sessions: [session("w1:lead", "lead"), session("w1:child", "child", "w1:lead"), session("w1:other", "other")], projects: [{ id: "/work/repo-home", name: "repo", sessions: [] }] };
  const box = () => document.body.querySelector<HTMLElement>("[data-desk-composer-input]")!;
  const key = async (name: string) => act(async () => { box().dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true })); });
  const listed = () => Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-mention-session]")).map((row) => row.dataset.deskMentionSession);

  const commands: DeskCommands = { agent: "claude", commands: [
    { name: "/compact", description: "Clear history but keep a summary", hint: "what to keep" },
    { name: "/context", description: "Show the context use" },
    { name: "/code-review", description: "Review the diff" },
  ] };
  // Each draft mounts the room anew, so the caret starts at the draft's end as it does when Desk opens.
  const withCommands = async (text: string, section: DeskCommands | null = commands) => {
    await act(async () => root.unmount());
    root = createRoot(container);
    useUiStore.setState({ deskDraft: { text, mentions: [] } });
    await render(desk([]));
    await act(async () => { useShellStore.setState({ deskCommands: section }); });
  };
  const commandRowsShown = () => Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-command]")).map((row) => row.dataset.deskCommand);

  it("opens the agent's commands on a / at the box's start, filtered as typed, each with its hint and what it does", async () => {
    await withCommands("/");
    expect(commandRowsShown()).toEqual(["/compact", "/context", "/code-review"]);
    const compact = document.body.querySelector("[data-desk-command='/compact']")!;
    expect(compact.querySelector("[data-desk-command-hint]")?.textContent).toBe("what to keep");
    expect(compact.querySelector("[data-desk-command-description]")?.textContent).toBe("Clear history but keep a summary");
    expect(document.body.querySelector("[data-desk-command-menu]")?.textContent).toContain("Claude Code commands");

    await withCommands("/con");
    expect(commandRowsShown()).toEqual(["/context"]);
    // A name that holds the words further in comes after the ones it starts.
    await withCommands("/co");
    expect(commandRowsShown()).toEqual(["/compact", "/context", "/code-review"]);
    await withCommands("/review");
    expect(commandRowsShown()).toEqual(["/code-review"]);
  });

  it("writes the picked command and one space with ↵, sends nothing, and closes with Esc", async () => {
    await withCommands("/con");
    await key("Enter");
    expect(useUiStore.getState().deskDraft.text).toBe("/context ");
    expect(sent("desk_send")).toEqual([]);
    expect(commandRowsShown()).toEqual([]);

    await withCommands("/co");
    await key("ArrowDown");
    await press("[data-desk-command='/code-review']");
    expect(useUiStore.getState().deskDraft.text).toBe("/code-review ");

    await withCommands("/co");
    await key("Escape");
    expect(commandRowsShown()).toEqual([]);
    await key("Enter");
    expect(sent("desk_send")).toEqual([{ text: "/co", mentions: [] }]);
  });

  it("opens no / menu for an agent that listed nothing, for another agent's list, past the first word or for a / inside the words", async () => {
    const cases: [string, DeskCommands | null][] = [
      ["/", { agent: "claude", commands: [] }],
      ["/", { agent: "codex", commands: commands.commands }],
      ["/", null],
      ["/zzz", commands],
      ["/compact now", commands],
      ["see /compact", commands],
    ];
    for (const [text, section] of cases) {
      await withCommands(text, section);
      expect(document.body.querySelector("[data-desk-command-menu]"), text).toBeNull();
    }
    // With no menu ↵ sends the words as they are.
    await withCommands("/", { agent: "claude", commands: [] });
    await key("Enter");
    expect(sent("desk_send")).toEqual([{ text: "/", mentions: [] }]);
  });

  it("shows a compaction the agent told outside its words as the answer's one line, and a left-out command's notice", async () => {
    const reply = (compaction: { done: boolean; reason: string | null }, id: number): DeskItem => ({ kind: "reply", id, agent: "opencode", text: "", at_ms: 0, state: "done", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 }, mentions: [], missions: [], compaction });
    await render(desk([
      reply({ done: false, reason: "Nothing to compact yet" }, 1),
      reply({ done: true, reason: null }, 2),
      { kind: "notice", id: 3, notice: "command_left_out", agent: "codex", detail: null },
    ]));
    const lines = Array.from(document.body.querySelectorAll("[data-desk-reply-compaction]")).map((line) => line.textContent);
    expect(lines).toEqual(["Couldn't compact the conversation · Nothing to compact yet", "Compacted the conversation"]);
    expect(document.body.querySelector("[data-desk-notice='command_left_out']")?.textContent).toBe("Desk doesn't send this command to Codex");
  });

  it("lists parents only in the @ menu, opens one with → or its fold and closes it with ← (B48)", async () => {
    useUiStore.setState({ deskDraft: { text: "@", mentions: [] } });
    await render(desk([]), tree);
    expect(listed()).toEqual(["w1:lead", "w1:other"]);
    expect(document.body.querySelector("[data-desk-mention-line]")?.textContent).toBe("repo · main · 테스트 고치는 중");

    await key("ArrowRight");
    expect(listed()).toEqual(["w1:lead", "w1:child", "w1:other"]);
    await key("ArrowLeft");
    expect(listed()).toEqual(["w1:lead", "w1:other"]);
    await press("[data-desk-mention-fold]");
    expect(listed()).toEqual(["w1:lead", "w1:child", "w1:other"]);
  });

  it("says in one row that nothing matches what follows @ and still sends the words with ↵ (Doop MentionMenu empty)", async () => {
    useUiStore.setState({ deskDraft: { text: "@zzzz", mentions: [] } });
    await render(desk([]), tree);
    expect(document.body.querySelector("[data-desk-mention-empty]")?.textContent).toBe("No session or project matches");
    expect(listed()).toEqual([]);
    await key("Enter");
    expect(sent("desk_send")).toEqual([{ text: "@zzzz", mentions: [] }]);
  });

  it("closes the empty @ row with Esc", async () => {
    useUiStore.setState({ deskDraft: { text: "@zzzz", mentions: [] } });
    await render(desk([]), tree);
    expect(document.body.querySelector("[data-desk-mention-empty]")).not.toBeNull();
    await key("Escape");
    expect(document.body.querySelector("[data-desk-mention-empty]")).toBeNull();
  });

  it("puts a picked session in the box as one chip, sends its token, and takes it out with one ⌫ (B49)", async () => {
    useUiStore.setState({ deskDraft: { text: "@", mentions: [] } });
    await render(desk([]), tree);
    await press("[data-desk-mention-session='w1:other']");
    const other = { kind: "session" as const, id: "w1:other", name: "other", mark: "going" as const, agent_kind: "claude" };
    expect(useUiStore.getState().deskDraft).toEqual({ text: "@session(w1:other other) ", mentions: [other] });
    expect(box().querySelectorAll("[data-desk-token]")).toHaveLength(1);
    expect(box().querySelector("[data-desk-session-chip]")?.textContent).toBe("other");

    await press("[data-desk-composer-send]");
    expect(sent("desk_send")).toEqual([{ text: "@session(w1:other other)", mentions: [{ kind: "session", id: "w1:other", name: "other" }] }]);

    // The caret right after the chip: one ⌫ takes the whole chip.
    await act(async () => {
      const range = document.createRange();
      range.setStartAfter(box().querySelector("[data-desk-token]")!);
      document.getSelection()!.removeAllRanges();
      document.getSelection()!.addRange(range);
    });
    await key("Backspace");
    expect(useUiStore.getState().deskDraft.text).toBe(" ");
    expect(box().querySelectorAll("[data-desk-token]")).toHaveLength(0);
  });

  it("leaves a chip alone when ⌫ ends a Korean syllable's composition, which Safari reports as keyCode 229", async () => {
    useUiStore.setState({ deskDraft: { text: "@", mentions: [] } });
    await render(desk([]), tree);
    await press("[data-desk-mention-session='w1:other']");
    await act(async () => {
      const range = document.createRange();
      range.setStartAfter(box().querySelector("[data-desk-token]")!);
      document.getSelection()!.removeAllRanges();
      document.getSelection()!.addRange(range);
      box().dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", keyCode: 229, bubbles: true, cancelable: true }));
    });
    expect(box().querySelectorAll("[data-desk-token]")).toHaveLength(1);
  });

  it("leaves the box, the `@` menu and a refusal's line alone on the Enter that ends a Korean syllable, which Safari reports as keyCode 229", async () => {
    const enter = (target: HTMLElement) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true, cancelable: true })); });
    useUiStore.setState({ deskDraft: { text: "안녕", mentions: [] } });
    await render(desk([]), tree);
    await enter(box());
    expect(sent("desk_send")).toEqual([]);
    expect(useUiStore.getState().deskDraft.text).toBe("안녕");
    // The `@` menu does not take it as a choice.
    useUiStore.setState({ deskDraft: { text: "@", mentions: [] } });
    await show(desk([]));
    await enter(box());
    expect(useUiStore.getState().deskDraft.mentions).toEqual([]);
    expect(document.body.querySelector("[data-desk-mention-session='w1:other']")).not.toBeNull();
  });

  /** A connected device's session beside Desk whose conversation showed `shown` with `hide file open`, its card listing `outputs`; the file reads the panel asked for. */
  async function remoteSessionShowed(shown: readonly string[], outputs: DeskOutput[]): Promise<Record<string, unknown>[]> {
    const reads: Record<string, unknown>[] = [];
    configureFileBytes((event) => { reads.push(event.payload as Record<string, unknown>); });
    const remoteRest = {
      navigator: {
        focused_device_id: "local", focused_checkout_id: null, focused_workspace_id: null,
        devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }, { id: "deepbox", label: "deepbox", kind: "remote", state: "connected" }],
        workspaces: [{ id: "w-deep", label: "deep", device_id: "deepbox", agent_scope: emptyScope(), checkouts: [
          { id: "c-deep", workspace_id: "w-deep", path: "/home/me/repo", label: "main", active_tab_id: "t1", strip: [], tabs: [{ id: "t1", panes: [{ id: "d:p1" }] }] },
        ] }],
      },
      status: { remote: [{ target_id: "deepbox", state: "connected" }] },
    } as unknown as SnapshotRest;
    const row = { pane_id: "d:p1", identity_label: "remote worker", agent_kind: "claude", mark: "going", device_id: "deepbox" } as never;
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    const calls = shown.map((path, index) => ({ sessionUpdate: "tool_call", toolCallId: `v${index}`, title: "Bash", kind: "execute", rawInput: { command: `hide file open ${path}` } }));
    const parts = Object.fromEntries(shown.map((path, index) => [`v${index}`, { kind: "view", view: "file", target: path }]));
    const thread = { generation: 1, state: "ready", pane_id: "d:p1", machine: "deepbox", failure: null, pane: "same", connected: true, has_older: false, older: "idle", input: null,
      tail: { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "보여줘" } }, ...calls], parts, sessions: {} } };
    await act(async () => {
      const card: DeskJob = { id: "pane:d:p1", pane_id: "d:p1", title: "remote worker", agent_kind: "claude", project: { workspace_id: "w-deep", label: "deep" }, location: { kind: "checkout", label: "main" },
        state: "working", queue_position: null, now: "쓰는 중", plan: null, question: null, summary: null, pull_requests: [], outputs, joined: false, reason: null, failure: null, ask: null,
        mark: "going", mark_read: false, machine: "deepbox", disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [], agent_id: null };
      useShellStore.setState({ rest: remoteRest, agents: [row], deskJobs: { jobs: [], workers: [], named: [card] } });
      useUiStore.getState().setDeskRoom((now) => ({ ...now, session: "d:p1" }));
    });
    // The core answers the open with the session's thread.
    await act(async () => useShellStore.setState({ sessionThread: thread as never, sessionThreadHistory: { generation: 1, pages: [] } }));
    stubTabStripTokens();
    return reads;
  }

  const frontOutput = () => useUiStore.getState().deskRoom.panel?.tabs.find((tab) => tab.kind === "output");

  it("draws and opens a file a connected device's session showed from that device, which no card lists (PRD desk-2-ui B27)", async () => {
    const reads = await remoteSessionShowed(["/home/me/repo/new.md", "/home/me/repo/shot.png"], []);
    // Each tile's preview is read from the file on that device, as the open below reads it: the text's head, the image's size first.
    expect(reads).toMatchObject([{ path: "/home/me/repo/new.md", length: 4096, device_id: "deepbox", root: "/home/me/repo" }, { path: "/home/me/repo/shot.png", length: 1, device_id: "deepbox", root: "/home/me/repo" }]);
    await press("[data-desk-output='/home/me/repo/new.md'] [data-desk-tile-open]");
    expect(frontOutput()).toMatchObject({ kind: "output", output: { device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/new.md" } });
  });

  it("draws and opens an image the card lists from the card's own checkout, not the session's", async () => {
    const listed: DeskOutput = { id: "view-9", kind: "image", label: "shot.png", thumb: null, seen: true, pr_state: null, device_id: "deepbox", checkout: "/home/me/wt", path: "/home/me/wt/shot.png", url: null };
    const reads = await remoteSessionShowed(["/home/me/wt/shot.png"], [listed]);
    // The shelf and the conversation draw the same file, read once.
    expect(reads).toMatchObject([{ path: "/home/me/wt/shot.png", length: 1, device_id: "deepbox", root: "/home/me/wt" }]);
    await press("[data-desk-hide-part='view'] [data-desk-output='view-9'] [data-desk-tile-open]");
    expect(frontOutput()).toMatchObject({ kind: "output", output: listed });
  });

  it("keeps one working line from the send to the turn's end, saying its step, and drops it while an approval waits (feedback 1 c3)", async () => {
    await render(desk([]));
    const line = () => document.body.querySelector("[data-working-line]")?.getAttribute("data-working-line") ?? null;
    expect(line()).toBeNull();
    await act(async () => useShellStore.setState({ deskLive: { text: null, started_at_ms: 1 } }));
    expect(line()).toBe("thinking");
    await act(async () => useShellStore.setState({ deskLive: { text: "찾아보겠습니다.", started_at_ms: 1, step: null } }));
    expect(line()).toBe("answering");
    await act(async () => useShellStore.setState({ deskLive: { text: "찾아보겠습니다.", started_at_ms: 1, step: "files" } }));
    expect(line()).toBe("files");
    expect(document.body.querySelector("[data-working-line]")?.textContent).toContain("Reading files");
    await show({ ...desk([]), approvals_waiting: 1 });
    expect(line()).toBeNull();
    await show(desk([]));
    await act(async () => useShellStore.setState({ deskLive: { text: null, started_at_ms: null } }));
    expect(line()).toBeNull();
  });

  it("draws a file an answer names inside a project as a chip that opens it in the tab panel, and leaves a missing path and code as words (feedback 2 frame M [2])", async () => {
    const asked: Record<string, unknown>[] = [];
    configureFileBytes((event) => {
      const read = event.payload as Record<string, unknown>;
      asked.push(read);
      queueMicrotask(() => (read.path === "/work/repo-home/docs/report.md" ? answerBytes(read, [35], 40) : receiveBytesError({ request_id: read.request_id as string, reason: "not_found" })));
    });
    await render(desk([{ kind: "reply", id: 1, text: "정리는 /work/repo-home/docs/report.md 에, 옛 메모는 /work/repo-home/old/notes.md 에 있어요.\n```\ncat /work/repo-home/docs/report.md\n```", agent: "claude", at_ms: 0, state: "done", mentions: [], missions: [], thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 } }]));
    await vi.waitFor(() => expect(document.body.querySelectorAll("[data-desk-file-chip]")).toHaveLength(1));
    const chip = document.body.querySelector<HTMLElement>("[data-desk-file-chip='project']")!;
    expect(chip.textContent).toBe("report.md");
    expect(document.body.textContent).toContain("/work/repo-home/old/notes.md");
    expect(asked.map((read) => read.path).sort()).toEqual(["/work/repo-home/docs/report.md", "/work/repo-home/old/notes.md"]);
    stubTabStripTokens();
    await act(async () => chip.click());
    expect(useUiStore.getState().deskRoom.panel?.tabs).toEqual([expect.objectContaining({ kind: "file", device_id: "local", root: "/work/repo-home", path: "/work/repo-home/docs/report.md" })]);
  });

  it("reveals a path outside every project from its ↗ chip in the file manager, and never reads or opens it (feedback 2 frame M [2])", async () => {
    const outside = "/Users/me/Downloads/crash.log";
    const revealPath = vi.fn();
    const probePaths = vi.fn(async (paths: string[]) => paths.map((path) => (path === outside ? { real: path, kind: "file" as const } : null)));
    window.hideHost = { kind: "electron", platform: "darwin", probePaths, revealPath } as never;
    const asked: unknown[] = [];
    configureFileBytes((event) => asked.push(event.payload));
    try {
      await render(desk([{ kind: "reply", id: 1, text: `크래시 로그는 ${outside} 에 있어요.`, agent: "claude", at_ms: 0, state: "done", mentions: [], missions: [], thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 } }]));
      await vi.waitFor(() => expect(document.body.querySelector("[data-desk-file-chip='outside']")).not.toBeNull());
      await act(async () => document.body.querySelector<HTMLElement>("[data-desk-file-chip='outside']")!.click());
      expect(revealPath).toHaveBeenCalledWith(outside);
      expect(asked).toEqual([]);
      expect(useUiStore.getState().deskRoom.panel?.tabs ?? []).toEqual([]);
    } finally {
      delete (window as { hideHost?: unknown }).hideHost;
    }
  });

  it("opens the checkout a project chip names", async () => {
    await render(desk([{ kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] }]));
    await press("[data-mention-chip='project']");
    expect(sent("focus_checkout")).toEqual([expect.objectContaining({ workspace_id: "w-repo", checkout_id: "c-home" })]);
  });

  describe("a message sent while a reply is being written", () => {
    const writing = () => act(async () => useShellStore.setState({ deskLive: { text: "답을 쓰는 중", started_at_ms: 1 } }));
    const waiting = (id: number, text: string) => ({ id, text, at_ms: 0, mentions: [] });

    it("goes with ↵ and waits as a bubble with 대기 중, the stop button staying", async () => {
      useUiStore.setState({ deskDraft: { text: "끝나면 알려줘", mentions: [] } });
      await render(desk([]));
      await writing();
      expect(document.body.querySelector("[data-desk-composer-stop]")).not.toBeNull();
      await key("Enter");
      expect(sent("desk_send")).toEqual([{ text: "끝나면 알려줘", mentions: [] }]);
      // The box keeps the words until the core shows them waiting, then lets them go.
      expect(useUiStore.getState().deskDraft.text).toBe("끝나면 알려줘");
      await show(desk([], "ready", true, [waiting(5, "끝나면 알려줘")]));
      expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
      expect(document.body.querySelector("[data-desk-queued='5']")?.textContent).toContain("Waiting for the reply");
      expect(document.body.textContent).not.toContain("The waiting line is full");
    });

    it("draws a line stopped on its first message as waiting for a restart, and lets 다시 시작 on the sign-in notice send it, however full the line", async () => {
      const login: DeskItem = { kind: "notice", id: 2, notice: "login_needed", agent: "claude", detail: "claude /login" };
      const full = Array.from({ length: 9 }, (_, index) => waiting(10 + index, `말 ${index}`));
      await render(desk([login], "ready", false, [waiting(5, "로그인 뒤에 보낼 말")], true));
      expect(document.body.querySelector("[data-desk-queued='5']")?.textContent).toContain("Sent after you restart");
      expect(document.body.querySelector("[data-desk-queued='5']")?.textContent).not.toContain("Waiting for the reply");
      expect(document.body.querySelector("[data-desk-notice-action='restart']")?.textContent).toBe("Signed in · Restart");
      await press("[data-desk-notice-action='restart']");
      expect(sent("desk_restart")).toEqual([{}]);
      // A full line stopped the same way is freed by the same button.
      await show(desk([login], "ready", false, full, true));
      expect(document.body.querySelector("[data-desk-notice-action='restart']")).not.toBeNull();
      // Whatever refused the line, its notice carries the restart that frees it, even a notice with no action of its own.
      const resume: DeskItem = { kind: "notice", id: 4, notice: "resume_failed", agent: "claude", detail: null };
      await show(desk([resume], "ready", true, [waiting(5, "다시 열지 못한 대화")], true));
      expect(document.body.querySelector("[data-desk-notice='resume_failed'] [data-desk-notice-action='restart']")?.textContent).toBe("Restart");
      await press("[data-desk-notice-action='restart']");
      expect(sent("desk_restart")).toEqual([{}, {}]);
      await show(desk([resume], "ready", true, [], false));
      expect(document.body.querySelector("[data-desk-notice-action]")).toBeNull();
      // A line that is not stopped keeps its 대기 중.
      await show(desk([], "ready", true, [waiting(5, "그냥 기다려요")], false));
      expect(document.body.querySelector("[data-desk-queued='5']")?.textContent).toContain("Waiting for the reply");
    });

    it("keeps the words in the box and sends nothing while eight messages already wait", async () => {
      useUiStore.setState({ deskDraft: { text: "九番目", mentions: [] } });
      const eight = Array.from({ length: 8 }, (_, index) => waiting(index + 1, `대기 ${index + 1}`));
      await render(desk([], "ready", true, eight));
      await writing();
      await key("Enter");
      expect(sent("desk_send")).toEqual([]);
      expect(useUiStore.getState().deskDraft.text).toBe("九番目");
      expect(document.body.querySelector("[data-desk-composer-notice]")?.textContent).toBe("The waiting line is full");
      // One leaves the line, and the same press goes.
      await show(desk([], "ready", true, eight.slice(1)));
      await key("Enter");
      expect(sent("desk_send")).toEqual([{ text: "九番目", mentions: [] }]);
    });

    it("is taken out with its ✕, which sends the core's one event", async () => {
      await render(desk([], "ready", true, [waiting(5, "하나"), waiting(6, "둘")]));
      await press("[data-desk-queued='6'] [data-desk-queued-remove]");
      expect(sent("desk_queued_remove")).toEqual([{ id: 6 }]);
    });
  });

  describe("permissions (D-92)", () => {
    /** Claude's own answers to a shell command, as its ACP adapter names them. */
    const claudeOptions: DeskOption[] = [
      { id: "allow-once", kind: "allow_once", name: "Yes" },
      { id: "allow-with-updates", kind: "allow_always", name: "Yes, and don't ask again for npm run commands" },
      { id: "reject", kind: "reject_once", name: "No" },
    ];
    const build: DeskItem = {
      kind: "approval", id: 7, card: "command", command: "npm run build", folder: "~/repo/web", reason: null, state: "waiting",
      options: claudeOptions, chosen: null, refusal: null,
    };
    const buttons = () => [...document.body.querySelectorAll<HTMLElement>("[data-desk-approval-option]")].map((button) => [button.dataset.deskApprovalOption, button.textContent]);
    const answers = () => [...document.body.querySelectorAll<HTMLElement>("[data-desk-approval-answer]")].map((button) => button.textContent);
    const refusal = () => document.body.querySelector<HTMLInputElement>("[data-desk-approval-refusal]");
    const type = async (value: string) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      await act(async () => { setter.call(refusal()!, value); refusal()!.dispatchEvent(new Event("input", { bubbles: true })); });
    };
    const key = (name: string, composing = false) => act(async () => { refusal()!.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, isComposing: composing })); });

    it("shows the agent's own answers as 거절… · 항상 허용 · 한 번 허용 and sends the one pressed as the agent's option", async () => {
      await render(desk([build]));
      const card = document.body.querySelector("[data-desk-item='approval']")!.textContent!;
      expect(card).toContain("Shell command");
      expect(card).toContain("npm run build");
      expect(card).toContain("~/repo/web");
      expect(buttons()).toEqual([["reject", "Reject…"], ["allow-with-updates", "Always allow"], ["allow-once", "Allow once"]]);
      await press("[data-desk-approval-option='allow-with-updates']");
      await press("[data-desk-approval-option='allow-once']");
      expect(sent("desk_approval")).toEqual([{ id: 7, option: "allow-with-updates" }, { id: 7, option: "allow-once" }]);
    });

    it("names answers of one kind by the agent's own names, so Claude's ways out of plan mode read apart", async () => {
      const planExit: DeskOption[] = [
        { id: "exit-plan-auto", kind: "allow_always", name: "Yes, and use auto mode" },
        { id: "exit-plan-accept-edits", kind: "allow_always", name: "Yes, auto-accept edits" },
        { id: "exit-plan-default", kind: "allow_once", name: "Yes, manually approve edits" },
        { id: "reject", kind: "reject_once", name: "No" },
      ];
      await render(desk([{ ...build, card: "other", command: "ExitPlanMode", options: planExit }]));
      expect(buttons()).toEqual([
        ["reject", "Reject…"],
        ["exit-plan-auto", "Yes, and use auto mode"],
        ["exit-plan-accept-edits", "Yes, auto-accept edits"],
        ["exit-plan-default", "Allow once"],
      ]);
      await press("[data-desk-approval-option='exit-plan-accept-edits']");
      expect(sent("desk_approval")).toEqual([{ id: 7, option: "exit-plan-accept-edits" }]);
    });

    it("chooses 다 허용 or 에이전트대로 under the box in one event, the second naming the chosen agent", async () => {
      const open = async () => {
        const trigger = document.body.querySelector<HTMLElement>("[data-desk-bar='permissions']")!;
        await act(async () => { trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 })); });
      };
      const choices = () => [...document.body.querySelectorAll<HTMLElement>("[data-desk-permission-choice]")].map((item) => [item.dataset.deskPermissionChoice, item.getAttribute("data-state"), item.textContent]);
      await render(desk([]));
      expect(document.body.querySelector("[data-desk-bar='permissions']")?.textContent).toBe("Allow all");
      await open();
      expect(document.body.textContent).toContain("How Desk asks while it works");
      expect(choices()).toEqual([
        ["allow_all", "checked", "Allow allDefaultDoes it right away without asking."],
        ["agent", "unchecked", "As the agentAsks each time Claude Code asks, with its own answers."],
      ]);
      await press("[data-desk-permission-choice='agent']");
      expect(sent("desk_settings_set")).toEqual([{ permission: "agent" }]);

      await show({ ...desk([]), settings: { ...desk([]).settings, permission: "agent" } });
      expect(document.body.querySelector("[data-desk-bar='permissions']")?.textContent).toBe("As the agent");
      await open();
      // The chosen one pressed again sends nothing.
      await press("[data-desk-permission-choice='agent']");
      expect(sent("desk_settings_set")).toHaveLength(1);
    });

    it("sends a refusal's line to Desk with the agent's reject on Enter, never mid-composition, and refuses plainly when the line is empty", async () => {
      await render(desk([build]));
      await press("[data-desk-approval-option='reject']");
      expect(sent("desk_approval")).toEqual([]);
      expect(answers()).toEqual(["Reject without a reason", "Reject and send"]);
      await type("run the type check instead");
      await key("Enter", true);
      expect(sent("desk_approval")).toEqual([]);
      await act(async () => { refusal()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true })); });
      expect(sent("desk_approval")).toEqual([]);
      await key("Enter");
      expect(sent("desk_approval")).toEqual([{ id: 7, option: "reject", text: "run the type check instead" }]);

      // Without a reason the typed words stay behind.
      await press("[data-desk-approval-answer='reject_plain']");
      // Blank words are a plain refusal too.
      await type("   ");
      await press("[data-desk-approval-answer='reject_send']");
      expect(sent("desk_approval").slice(1)).toEqual([{ id: 7, option: "reject" }, { id: 7, option: "reject" }]);

      // Escape takes the line back to the card's answers.
      await key("Escape");
      expect(refusal()).toBeNull();
      expect(buttons().map(([id]) => id)).toEqual(["reject", "allow-with-updates", "allow-once"]);
    });

    it("ends a card that offers no refusal with Hide's own 거절…, sent as no option", async () => {
      const plan: DeskOption[] = [{ id: "plan", kind: "other", name: "Keep planning" }, { id: "go", kind: "allow_once", name: "Yes" }];
      await render(desk([{ ...build, card: "other", command: "ExitPlanMode", options: plan }]));
      expect(buttons()).toEqual([["plan", "Keep planning"], ["go", "Allow once"]]);
      await press("[data-desk-approval-kind='reject']");
      await type("keep planning instead");
      await key("Enter");
      await press("[data-desk-approval-answer='reject_plain']");
      expect(sent("desk_approval")).toEqual([{ id: 7, option: null, text: "keep planning instead" }, { id: 7, option: null }]);
    });

    it("folds an answered card to the kind of answer pressed", async () => {
      await render(desk([
        { ...build, state: "allowed", chosen: "allow_always" },
        { ...build, id: 8, state: "allowed", chosen: "allow_once" },
        { ...build, id: 9, state: "allowed", chosen: null },
        { ...build, id: 10, state: "rejected", chosen: "reject_once", refusal: "wait for it" },
        { ...build, id: 11, state: "rejected", chosen: "reject_once" },
        { ...build, id: 12, state: "closed" },
        { ...build, id: 13, state: "allowed", chosen: "other", answer: "Keep planning" },
      ]));
      const folded = [...document.body.querySelectorAll("[data-desk-approval-answered]")].map((line) => line.textContent);
      expect(folded).toEqual([
        "Always allowed·npm run build",
        "Allowed once·npm run build",
        "Allowed·npm run build",
        "Rejected · your line went to Desk·npm run build",
        "Rejected·npm run build",
        "Not run · closed by a new conversation or restart·npm run build",
        "Keep planning·npm run build",
      ]);
      expect(document.body.querySelector("[data-desk-approval-option]")).toBeNull();
    });

    it("has no permission table in Settings › Labs › Desk, whose changes leave the permission under the box alone", async () => {
      const actions = createActions((event) => { events.push(event); return true; });
      await act(async () => {
        useShellStore.setState({ connection: "live", rest, desk: desk([]) });
        root.render(<TooltipProvider><DeskSettingsPage actions={actions} /></TooltipProvider>);
      });
      expect(document.body.querySelector("[data-desk-settings]")).not.toBeNull();
      expect(document.body.textContent).not.toContain("Permissions");
      await press("[data-desk-use]");
      expect(sent("desk_settings_set")).toEqual([{ enabled: false, default_agent: "claude", location: "worktree", cap: 8 }]);
    });
  });

  it("opens a session a chip names beside Desk, stays in Desk, and closes the tab panel before the session on Escape (D-09, B2, B21)", async () => {
    // `fixer` runs in the sidebar but Desk did not start it: it has no card until its conversation opens.
    const row = { pane_id: "w1:p2", identity_label: "fixer", agent_kind: "claude", mark: "going" } as never;
    const card: DeskJob = { id: "pane:w1:p2", pane_id: "w1:p2", title: "fixer", agent_kind: "claude", project: { workspace_id: "w-repo", label: "repo" }, location: { kind: "checkout", label: "main" },
      state: "working", queue_position: null, now: "고치는 중", plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: null,
      mark: "going", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [], agent_id: null };
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([{ kind: "user", id: 1, text: "@session(w1:p2 fixer) 어디까지 왔어?", at_ms: 0, mentions: [fixer] }]));
    await act(async () => useShellStore.setState({ agents: [row] }));
    expect(document.body.querySelector("[data-desk-room]")?.getAttribute("data-desk-room")).toBe("closed");

    await press("[data-desk-session-chip]");
    expect(useUiStore.getState().deskRoom.session).toBe("w1:p2");
    expect(sent("session_thread_open")).toEqual([{ pane_id: "w1:p2" }]);
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
    expect(document.body.querySelector("[data-desk-room]")?.getAttribute("data-desk-room")).toBe("session");

    // The core sends the session's card with the open conversation; the head reads it.
    await act(async () => useShellStore.setState({ deskJobs: { jobs: [], workers: [], named: [card] } }));
    expect(document.body.querySelector("[data-desk-session-head]")?.textContent).toContain("fixer");
    expect(sent("session_thread_open")).toHaveLength(1);

    stubTabStripTokens();
    await act(async () => useUiStore.getState().setDeskRoom((now) => ({ ...now, panel: { tabs: [{ kind: "mission", mission: "mission-39", title: "1.3.0" }], preview: null, active: "mission:mission-39", explorer: false, beside: null } })));
    expect(document.body.querySelector("[data-desk-tab-panel]")?.getAttribute("data-desk-tab-panel")).toBe("beside_session");
    // Escape runs the innermost layer the shell's Escape owner holds, and with none open closes the session beside Desk,
    // as keyboard.ts does (its order is keyboard.desk.test.ts's).
    const escape = () => act(async () => {
      const layer = useUiStore.getState().escapeLayers.at(-1);
      if (layer) layer();
      else actions.deskCloseSession();
    });
    expect(useUiStore.getState().escapeLayers).toHaveLength(1);
    await escape();
    expect([useUiStore.getState().deskRoom.panel, useUiStore.getState().deskRoom.session]).toEqual([null, "w1:p2"]);
    expect(useUiStore.getState().escapeLayers).toHaveLength(0);
    await escape();
    expect(useUiStore.getState().deskRoom.session).toBeNull();
    expect(sent("session_thread_close")).toHaveLength(1);
  });

  it("reads what the session beside Desk asks and answers it as the phone does, once (B14)", async () => {
    const row = { pane_id: "w1:p2", device_id: "local", identity_label: "fixer", agent_kind: "claude", mark: "turn" } as never;
    const card: DeskJob = { id: "pane:w1:p2", pane_id: "w1:p2", title: "fixer", agent_kind: "claude", project: { workspace_id: "w-repo", label: "repo" }, location: { kind: "checkout", label: "main" },
      state: "asking", queue_position: null, now: null, plan: null, question: "Delete the old captures?", summary: null, pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: "question",
      mark: "turn", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 5, read: true, mission: null, workers: [], agent_id: null };
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    await act(async () => useShellStore.setState({ agents: [row], deskJobs: { jobs: [], workers: [], named: [card] } }));
    await act(async () => useUiStore.getState().setDeskRoom(() => ({ session: "w1:p2", panel: null, closed: null, answer: 0 })));
    expect(sent("pane_ask")).toEqual([{ device_id: "local", pane_id: "w1:p2" }]);
    // Until the screen is read, no card stands at the conversation's end: what the agent asked stays in its
    // words, and the card's question is not said again in a card of its own.
    expect(document.body.querySelector("[data-desk-ask], [data-desk-question]")).toBeNull();
    expect(document.body.textContent).not.toContain("Delete the old captures?");

    const ask = { kind: "question" as const, turn: "7", since_ms: null, text: "Delete the old captures?", choices: [{ id: "1", label: "Keep" }, { id: "2", label: "Delete" }], answerable: true };
    await act(async () => useShellStore.setState({ paneAsk: { device_id: "local", pane_id: "w1:p2", ask, reason: null } }));
    expect(document.body.querySelectorAll("[data-phone-ask-choice]")).toHaveLength(2);
    await press('[data-phone-ask-choice="2"]');
    await press("[data-phone-ask-send]");
    const [choice] = sent("pane_choice");
    // The ask goes back as the frame gave it, so the daemon answers only the ask the card drew.
    expect(choice).toMatchObject({ device_id: "local", pane_id: "w1:p2", choice: "2", ask });
    // A second press while it is on its way sends nothing more.
    await press("[data-phone-ask-send]");
    expect(sent("pane_choice")).toHaveLength(1);
    await act(async () => useShellStore.setState({ paneChoice: { request_id: String(choice!.request_id), ok: true, reason: null } }));
    expect(document.body.querySelector("[data-phone-ask-sent]")).not.toBeNull();

    // A choice refused stale reads the card again.
    await act(async () => useShellStore.setState({ paneAsk: { device_id: "local", pane_id: "w1:p2", ask: { ...ask, turn: "8" }, reason: null } }));
    await press('[data-phone-ask-choice="1"]');
    await press("[data-phone-ask-send]");
    const second = sent("pane_choice")[1]!;
    const asks = sent("pane_ask").length;
    await act(async () => useShellStore.setState({ paneChoice: { request_id: String(second.request_id), ok: false, reason: "stale" } }));
    expect(sent("pane_ask")).toHaveLength(asks + 1);
  });
});

/** The file channel's answer to `read`: one frame with `bytes` of a file `total` long. */
function answerBytes(read: Record<string, unknown>, bytes: number[], total: number) {
  const header = new TextEncoder().encode(JSON.stringify({ type: "file_bytes", request_id: read.request_id, path: read.path, offset: 0, total, eof: true }));
  const frame = new Uint8Array(4 + header.length + bytes.length);
  new DataView(frame.buffer).setUint32(0, header.length, false);
  frame.set(header, 4);
  frame.set(bytes, 4 + header.length);
  receiveBytes(frame.buffer);
}

/** The tab strip reads its sizes from the design tokens, which jsdom has no stylesheet for. */
function stubTabStripTokens() {
  for (const [name, value] of [["--size-tab-preferred", "160"], ["--size-tab-title-min", "48"], ["--size-tab-selected-max", "240"], ["--size-tab-icon-identity", "16"], ["--size-control-sm", "24"]] as const) document.documentElement.style.setProperty(name, value);
}
