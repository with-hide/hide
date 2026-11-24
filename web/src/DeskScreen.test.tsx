// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
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

const desk = (items: DeskItem[], state: DeskAgent["state"] = "ready", signedIn: boolean | null = true): DeskSection => ({
  selection: { agent: "claude", model: null, effort: null },
  agents: [claude(state, signedIn)],
  host: { state: "ready", detail: null },
  conversation: { generation: 1, items, dropped: 0 },
  approvals_waiting: 0,
  settings: { enabled: true, default_agent: "claude", location: "worktree", cap: 8, permissions: { hide: "run", read: "run", shell: "run", edit: "run" } },
  lane: { view_width: null, session_width: null },
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

  async function render(section: DeskSection, mentions: DeskMentions = { sessions: [], projects: [] }) {
    const actions = createActions((event) => { events.push(event); return true; });
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

  it("puts a sent message back in the box when the agent turned out not to be signed in (B6)", async () => {
    useUiStore.setState({ deskDraft: { text: "ask @session(w1:p2 fixer)", mentions: [fixer] } });
    await render(desk([], "ready", false));
    await press("[data-desk-composer-send]");
    await show(desk([{ kind: "user", id: 1, text: "ask @session(w1:p2 fixer)", at_ms: 0, mentions: [fixer] }], "ready", false));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });

    // The core takes the message out again and says how to sign in.
    await show(desk([{ kind: "notice", id: 2, notice: "login_needed", agent: "claude", detail: "claude /login" }], "ready", false));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "ask @session(w1:p2 fixer)", mentions: [fixer] });
  });

  it("sends the operator's last message again as they wrote it on 다시 시도", async () => {
    await render(desk([
      { kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] },
      { kind: "notice", id: 2, notice: "turn_failed", agent: "claude", detail: null },
    ]));
    await press("[data-desk-notice-action='retry']");
    expect(sent("desk_send")).toEqual([{ text: "@project(/work/repo-home repo) fix the build", mentions: [repo] }]);
  });

  it("names the agent's own reason on a failed turn when it gave one", async () => {
    const notice = () => document.body.querySelector<HTMLElement>("[data-desk-notice='turn_failed']")!.textContent!;
    await render(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: "Rate limit reached" }]));
    expect(notice()).toContain("· Rate limit reached");
    await show(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null }]));
    expect(notice()).not.toContain("·");
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

  it("opens the checkout a project chip names", async () => {
    await render(desk([{ kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] }]));
    await press("[data-mention-chip='project']");
    expect(sent("focus_checkout")).toEqual([expect.objectContaining({ workspace_id: "w-repo", checkout_id: "c-home" })]);
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
    await act(async () => useUiStore.getState().setDeskRoom((now) => ({ ...now, panel: { tabs: [{ kind: "mission", mission: "mission-39", title: "1.3.0" }], preview: null, active: "mission:mission-39", explorer: false } })));
    expect(document.body.querySelector("[data-desk-tab-panel]")?.getAttribute("data-desk-tab-panel")).toBe("beside_session");
    // Escape runs the innermost layer the shell's Escape owner holds, as keyboard.ts does.
    const escape = () => act(async () => { useUiStore.getState().escapeLayers.at(-1)!(); });
    await escape();
    expect([useUiStore.getState().deskRoom.panel, useUiStore.getState().deskRoom.session]).toEqual([null, "w1:p2"]);
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
    await act(async () => useUiStore.getState().setDeskRoom(() => ({ session: "w1:p2", panel: null, answer: 0 })));
    expect(sent("pane_ask")).toEqual([{ device_id: "local", pane_id: "w1:p2" }]);
    // Until the screen is read, the card says what the card knows, with 직접 쓰기 only.
    expect(document.body.querySelector("[data-desk-question]")?.textContent).toContain("Delete the old captures?");

    const ask = { kind: "question" as const, turn: "7", since_ms: null, text: "Delete the old captures?", choices: [{ id: "1", label: "Keep" }, { id: "2", label: "Delete" }], answerable: true };
    await act(async () => useShellStore.setState({ paneAsk: { device_id: "local", pane_id: "w1:p2", ask, reason: null } }));
    expect(document.body.querySelectorAll("[data-phone-ask-choice]")).toHaveLength(2);
    await press('[data-phone-ask-choice="2"]');
    await press("[data-phone-ask-send]");
    const [choice] = sent("pane_choice");
    expect(choice).toMatchObject({ device_id: "local", pane_id: "w1:p2", turn: "7", kind: "question", choice: "2" });
    // A second press while it is on its way sends nothing more.
    await press("[data-phone-ask-send]");
    expect(sent("pane_choice")).toHaveLength(1);
    await act(async () => useShellStore.setState({ paneChoice: { request_id: String(choice!.request_id), ok: true, reason: null } }));
    expect(document.body.querySelector("[data-phone-ask-sent]")).not.toBeNull();
  });
});
