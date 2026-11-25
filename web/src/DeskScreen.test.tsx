// @vitest-environment jsdom
import { Blob as NodeBlob } from "node:buffer";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { configureAttachments } from "./attachments";
import { TooltipProvider } from "./components/ui/tooltip";
import type { DeskAgent, DeskItem, DeskJob } from "./desk";
import { DeskScreen, DeskSettingsPage } from "./DeskScreen";
import { clientI18n } from "./i18n/translator";
import type { DeskMentions, DeskSection, SnapshotRest } from "./snapshot";
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

const claude = (state: DeskAgent["state"], signedIn: boolean | null): DeskAgent => ({ id: "claude", label: "Claude Code", state, detail: null, signed_in: signedIn, always_runs: [], login_command: "claude /login", models: [], efforts: [] });

const desk = (items: DeskItem[], state: DeskAgent["state"] = "ready", signedIn: boolean | null = true, queued: DeskSection["conversation"]["queued"] = [], held = false): DeskSection => ({
  selection: { agent: "claude", model: null, effort: null },
  agents: [claude(state, signedIn)],
  host: { state: "ready", detail: null },
  conversation: { generation: 1, items, dropped: 0, queued, queued_max: 8, queued_full: queued.length >= 8, queued_held: held },
  approvals_waiting: 0,
  settings: { enabled: true, default_agent: "claude", location: "worktree", cap: 8, permissions: { hide: "run", read: "run", shell: "run", edit: "run" } },
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

  const dropFile = async (name: string, bytes: number[]) => {
    const file = new File([new Uint8Array(bytes)], name);
    file.stream = () => new NodeBlob([new Uint8Array(bytes)]).stream() as ReadableStream<Uint8Array<ArrayBuffer>>;
    const drop = new Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [file] } });
    await act(async () => { document.body.querySelector("[data-desk-composer-frame]")!.dispatchEvent(drop); });
    await vi.waitFor(() => expect(document.body.querySelector('[data-desk-attachment="reading"]')).toBeNull());
  };
  const tiles = () => [...document.body.querySelectorAll("[data-desk-composer-frame] [data-desk-attachment]")].map((tile) => [tile.getAttribute("data-desk-attachment"), tile.textContent]);

  it("sends a dropped file's stage with the words, keeps it in the box until the conversation shows the message, and draws it on the bubble (desk-2-ui B41)", async () => {
    configureAttachments({ dispatch: (event) => { events.push(event); return true; }, sendBinary: () => undefined });
    useUiStore.setState({ deskDraft: { text: "왜 끊기는지 봐 줘", mentions: [] } });
    await render(desk([]));
    await dropFile("install.log", [7, 8]);

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

  it("draws only a failed turn in the failed colour: a stopped host is neutral words, a blocked turn and a failed install keep the warning colour", async () => {
    const colour = (kind: string) => document.body.querySelector<HTMLElement>(`[data-desk-notice='${kind}'] span`)!.className;
    await render(desk([{ kind: "notice", id: 1, notice: "host_stopped", agent: "claude", detail: null }, { kind: "notice", id: 2, notice: "blocked", agent: "claude", detail: null }, { kind: "notice", id: 3, notice: "install_failed", agent: "claude", detail: null }]));
    expect(colour("host_stopped")).not.toMatch(/text-destructive|text-warning/);
    expect(colour("blocked")).toContain("text-warning");
    expect(colour("install_failed")).toContain("text-warning");
  });

  it("hides a failed turn's 다시 시도 while its message waits, so one more press does not send it twice", async () => {
    const failed: DeskItem[] = [user(1), { kind: "notice", id: 2, notice: "turn_failed", agent: "claude", detail: null }];
    await render(desk(failed, "ready", true, [{ id: 3, text: "실패해 보세요", at_ms: 0, mentions: [] }]));
    expect(document.body.querySelector("[data-desk-notice-action='retry']")).toBeNull();
    await show(desk(failed));
    expect(document.body.querySelector("[data-desk-notice-action='retry']")).not.toBeNull();
    // A blocked turn's 다시 시도 would queue a copy behind the waiting message, so it waits too.
    await show(desk([{ kind: "notice", id: 2, notice: "blocked", agent: "claude", detail: null }], "ready", true, [{ id: 3, text: "실패해 보세요", at_ms: 0, mentions: [] }]));
    expect(document.body.querySelector("[data-desk-notice-action='retry']")).toBeNull();
  });

  it("names the agent's own reason on a failed turn when it gave one", async () => {
    const notice = () => document.body.querySelector<HTMLElement>("[data-desk-notice='turn_failed']")!.textContent!;
    await render(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: "Rate limit reached" }]));
    expect(notice()).toContain("· Rate limit reached");
    await show(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null }]));
    expect(notice()).not.toContain("·");
  });

  const reply = (id: number, words: string): DeskItem => ({ kind: "reply", id, agent: "claude", text: words, at_ms: 0, state: "stopped", thought_ms: null, steps: { sessions: 0, files: 0, letters: 0, starts: 0, other: 0 }, mentions: [] });
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

  it("opens a file a connected device's session showed from that device, which no card lists (PRD desk-2-ui B27)", async () => {
    const remoteRest = {
      navigator: {
        focused_device_id: "local", focused_checkout_id: null, focused_workspace_id: null,
        devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }, { id: "deepbox", label: "deepbox", kind: "remote", state: "connected" }],
        workspaces: [{ id: "w-deep", label: "deep", device_id: "deepbox", agent_scope: emptyScope(), checkouts: [
          { id: "c-deep", workspace_id: "w-deep", path: "/home/me/repo", label: "main", active_tab_id: "t1", strip: [], tabs: [{ id: "t1", panes: [{ id: "d:p1" }] }] },
        ] }],
      },
    } as unknown as SnapshotRest;
    const row = { pane_id: "d:p1", identity_label: "remote worker", agent_kind: "claude", mark: "going", device_id: "deepbox" } as never;
    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    const thread = { generation: 1, state: "ready", pane_id: "d:p1", machine: "deepbox", failure: null, pane: "same", connected: true, has_older: false, older: "idle", input: null,
      tail: { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "문서 보여줘" } }, { sessionUpdate: "tool_call", toolCallId: "v1", title: "Bash", kind: "execute", rawInput: { command: "hide file open /home/me/repo/new.md" } }], parts: { v1: { kind: "view", view: "file", target: "/home/me/repo/new.md" } }, sessions: {} } };
    await act(async () => {
      const card: DeskJob = { id: "pane:d:p1", pane_id: "d:p1", title: "remote worker", agent_kind: "claude", project: { workspace_id: "w-deep", label: "deep" }, location: { kind: "checkout", label: "main" },
        state: "working", queue_position: null, now: "쓰는 중", plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: null,
        mark: "going", mark_read: false, machine: "deepbox", disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [], agent_id: null };
      useShellStore.setState({ rest: remoteRest, agents: [row], deskJobs: { jobs: [], workers: [], named: [card] } });
      useUiStore.getState().setDeskRoom((now) => ({ ...now, session: "d:p1" }));
    });
    // The core answers the open with the session's thread.
    await act(async () => useShellStore.setState({ sessionThread: thread as never, sessionThreadHistory: { generation: 1, pages: [] } }));
    // The tab strip reads its sizes from the design tokens, which jsdom has no stylesheet for.
    for (const [name, value] of [["--size-tab-preferred", "160"], ["--size-tab-title-min", "48"], ["--size-tab-selected-max", "240"], ["--size-tab-icon-identity", "16"], ["--size-control-sm", "24"]] as const) document.documentElement.style.setProperty(name, value);
    await press("[data-desk-output='/home/me/repo/new.md']");
    const front = useUiStore.getState().deskRoom.panel?.tabs.find((tab) => tab.kind === "output");
    expect(front).toMatchObject({ kind: "output", output: { device_id: "deepbox", checkout: "/home/me/repo", path: "/home/me/repo/new.md" } });
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
      // Whatever refused the line, its notice carries the restart that frees it: a no-ask agent has no box to send from.
      const noAsk: DeskItem = { kind: "notice", id: 4, notice: "no_ask_mode", agent: "grok", detail: null };
      await show(desk([noAsk], "no_ask_mode", false, [waiting(5, "묻지 못하는 에이전트")], true));
      expect(document.body.querySelector("[data-desk-notice='no_ask_mode'] [data-desk-notice-action='restart']")?.textContent).toBe("Restart");
      await press("[data-desk-notice-action='restart']");
      expect(sent("desk_restart")).toEqual([{}, {}]);
      await show(desk([noAsk], "no_ask_mode", false, [], false));
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

  describe("permissions (D-82, D-83, B46, B47)", () => {
    const archive: DeskItem = {
      kind: "approval", id: 7, card: "command", command: "hide agent archive agent-18285", folder: "~/repo", reason: null, state: "waiting",
      asks: [{ kind: "irreversible", value: "end_working_session" }], subject: "project panel results", refusal: null,
    };
    const answers = () => [...document.body.querySelectorAll<HTMLElement>("[data-desk-approval-answer]")].map((button) => button.textContent);
    const refusal = () => document.body.querySelector<HTMLInputElement>("[data-desk-approval-refusal]");
    const type = async (value: string) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      await act(async () => { setter.call(refusal()!, value); refusal()!.dispatchEvent(new Event("input", { bubbles: true })); });
    };
    const key = (name: string, composing = false) => act(async () => { refusal()!.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, isComposing: composing })); });

    it("names a hide line's card by its permission row, and any other command a shell command", async () => {
      await render(desk([{ ...archive, row: "hide" }]));
      expect(document.body.querySelector("[data-desk-item='approval']")?.textContent).toContain("hide commands");
      await show(desk([{ ...archive, row: "shell" }]));
      expect(document.body.querySelector("[data-desk-item='approval']")?.textContent).toContain("Shell command");
      expect(document.body.querySelector("[data-desk-item='approval']")?.textContent).toContain("~/repo");
    });

    it("asks about a working session's archive with three answers and no 'always', and says what it ends", async () => {
      await render(desk([archive]));
      expect(document.body.querySelector("[data-desk-item='approval']")?.textContent).toContain("Asking because this can't be undone");
      expect(document.body.querySelector("[data-desk-approval-why='end_working_session']")?.textContent).toBe("Archives a session that is working·project panel results");
      expect(answers()).toEqual(["Reject…", "Allow in this conversation", "Allow once"]);
      await press("[data-desk-approval-answer='conversation']");
      await press("[data-desk-approval-answer='once']");
      expect(sent("desk_approval")).toEqual([{ id: 7, answer: "conversation" }, { id: 7, answer: "once" }]);
    });

    it("sends a refusal's line to Desk on Enter, never mid-composition, and refuses plainly when the line is empty", async () => {
      await render(desk([archive]));
      await press("[data-desk-approval-answer='reject']");
      expect(sent("desk_approval")).toEqual([]);
      expect(answers()).toEqual(["Reject without a reason", "Reject and send"]);
      await type("wait until it ends, then archive it");
      await key("Enter", true);
      expect(sent("desk_approval")).toEqual([]);
      await act(async () => { refusal()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true })); });
      expect(sent("desk_approval")).toEqual([]);
      await key("Enter");
      expect(sent("desk_approval")).toEqual([{ id: 7, answer: "reject", text: "wait until it ends, then archive it" }]);

      // Without a reason the typed words stay behind.
      await press("[data-desk-approval-answer='reject_plain']");
      // Blank words are a plain refusal too.
      await type("   ");
      await press("[data-desk-approval-answer='reject_send']");
      expect(sent("desk_approval").slice(1)).toEqual([{ id: 7, answer: "reject" }, { id: 7, answer: "reject" }]);

      // Escape takes the line back to the three answers.
      await key("Escape");
      expect(refusal()).toBeNull();
      expect(answers()).toEqual(["Reject…", "Allow in this conversation", "Allow once"]);
    });

    it("folds an answered card to how it was answered", async () => {
      await render(desk([{ ...archive, state: "allowed_conversation" }, { ...archive, id: 8, state: "rejected", refusal: "wait for it" }, { ...archive, id: 9, state: "rejected" }]));
      const folded = [...document.body.querySelectorAll("[data-desk-approval-answered]")].map((line) => line.textContent);
      expect(folded).toEqual([
        "Allowed in this conversation·hide agent archive agent-18285",
        "Rejected · your line went to Desk·hide agent archive agent-18285",
        "Rejected·hide agent archive agent-18285",
      ]);
      expect(document.body.querySelector("[data-desk-approval-answer]")).toBeNull();
    });

    it("sets each row to run or ask as one whole table and keeps the irreversible row locked to always ask", async () => {
      const actions = createActions((event) => { events.push(event); return true; });
      await act(async () => {
        useShellStore.setState({ connection: "live", rest, desk: desk([]) });
        root.render(<TooltipProvider><DeskSettingsPage actions={actions} /></TooltipProvider>);
      });
      const chosen = () => Object.fromEntries([...document.body.querySelectorAll<HTMLElement>("[data-desk-permission]")].map((row) => [
        row.dataset.deskPermission, row.querySelector("[data-state='on']")?.textContent ?? row.querySelector("[data-desk-permission-locked]")?.textContent,
      ]));
      expect(chosen()).toEqual({ hide: "Run", read: "Run", shell: "Run", edit: "Run", irreversible: "Always ask" });
      expect(document.body.querySelector("[data-desk-permission='irreversible'] button")).toBeNull();

      await press("[data-desk-permission='edit'] [data-desk-permission-answer='ask']");
      expect(sent("desk_settings_set")).toEqual([{ enabled: true, default_agent: "claude", location: "worktree", cap: 8, permissions: { hide: "run", read: "run", shell: "run", edit: "ask" } }]);
      // The chosen answer pressed again changes nothing.
      await press("[data-desk-permission='hide'] [data-desk-permission-answer='run']");
      expect(sent("desk_settings_set")).toHaveLength(1);
    });
  });

  it("names, dimmed on a row, the agents whose own setup runs it without asking (D-82)", async () => {
    const codex: DeskAgent = { ...claude("ready", true), id: "codex", label: "Codex", always_runs: ["read"], login_command: "codex login" };
    const actions = createActions(() => true);
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, desk: { ...desk([]), agents: [claude("ready", true), codex] } });
      root.render(<TooltipProvider><DeskSettingsPage actions={actions} /></TooltipProvider>);
    });
    const unasked = [...document.body.querySelectorAll<HTMLElement>("[data-desk-permission-unasked]")].map((note) => [note.closest<HTMLElement>("[data-desk-permission]")?.dataset.deskPermission, note.textContent]);
    expect(unasked).toEqual([["read", "Always runs in Codex"]]);
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

    // The tab strip reads its sizes from the design tokens, which jsdom has no stylesheet for.
    for (const [name, value] of [["--size-tab-preferred", "160"], ["--size-tab-title-min", "48"], ["--size-tab-selected-max", "240"], ["--size-tab-icon-identity", "16"], ["--size-control-sm", "24"]] as const) document.documentElement.style.setProperty(name, value);
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
    // Until the screen is read, the card says what the card knows, with 직접 쓰기 only.
    expect(document.body.querySelector("[data-desk-question]")?.textContent).toContain("Delete the old captures?");

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
