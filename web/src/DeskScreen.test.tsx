// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { DeskAgent, DeskItem } from "./desk";
import { DeskScreen, DeskSidebarEntry } from "./DeskScreen";
import { clientI18n } from "./i18n/translator";
import type { DeskSection, SnapshotRest } from "./snapshot";
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

const claude = (state: DeskAgent["state"]): DeskAgent => ({ id: "claude", label: "Claude", state, detail: null, login_command: "claude /login", models: [], efforts: [] });

const desk = (items: DeskItem[], state: DeskAgent["state"] = "ready"): DeskSection => ({
  selection: { agent: "claude", model: null, effort: null },
  agents: [claude(state)],
  host: { state: "ready", detail: null },
  conversation: { generation: 1, items, dropped: 0 },
  approvals_waiting: 0,
  settings: { enabled: true, default_agent: "claude", location: "worktree", cap: 8 },
  lane: { pinned: false, kept: [] },
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

  async function render(section: DeskSection) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, desk: section, deskLive: null, deskJobs: { jobs: [], workers: [] }, deskMentions: { sessions: [], projects: [] } });
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

  it("draws no sidebar entry while Desk is off, and leaves the room when it is switched off in front (B41)", async () => {
    const off = (section: DeskSection): DeskSection => ({ ...section, settings: { ...section.settings, enabled: false } });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, desk: off(desk([])), deskLive: null, deskJobs: { jobs: [], workers: [] }, deskMentions: { sessions: [], projects: [] } });
      root.render(<TooltipProvider><DeskSidebarEntry actions={actions} /></TooltipProvider>);
    });
    expect(document.body.querySelector("[data-desk-sidebar-row]")).toBeNull();
    await show(desk([]));
    expect(document.body.querySelector("[data-desk-sidebar-row]")).not.toBeNull();

    await act(async () => useUiStore.getState().setScreen({ kind: "desk" }));
    await render(desk([]));
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
    // No panel came before Desk here, so All projects takes its place.
    await show(off(desk([])));
    expect(useUiStore.getState().screen).toEqual({ kind: "main" });
  });

  it("keeps a sent message in the box until the conversation shows it, so a refused one stays", async () => {
    useUiStore.setState({ deskDraft: { text: "ask @fixer", mentions: [fixer] } });
    await render(desk([]));
    await press("[data-desk-composer-send]");
    expect(sent("desk_send")).toEqual([{ text: "ask @session(w1:p2 fixer)", mentions: [fixer] }]);
    expect(useUiStore.getState().deskDraft.text).toBe("ask @fixer");

    await show(desk([{ kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null }]));
    expect(useUiStore.getState().deskDraft.text).toBe("ask @fixer");

    await show(desk([
      { kind: "notice", id: 1, notice: "turn_failed", agent: "claude", detail: null },
      { kind: "user", id: 2, text: "ask @session(w1:p2 fixer)", at_ms: 0, mentions: [fixer] },
    ]));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });
  });

  it("puts a sent message back in the box when the agent turned out not to be signed in (B6)", async () => {
    useUiStore.setState({ deskDraft: { text: "ask @fixer", mentions: [fixer] } });
    await render(desk([], "login_needed"));
    await press("[data-desk-composer-send]");
    await show(desk([{ kind: "user", id: 1, text: "ask @session(w1:p2 fixer)", at_ms: 0, mentions: [fixer] }], "login_needed"));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "", mentions: [] });

    // The core takes the message out again and says how to sign in.
    await show(desk([{ kind: "notice", id: 2, notice: "login_needed", agent: "claude", detail: "claude /login" }], "login_needed"));
    expect(useUiStore.getState().deskDraft).toEqual({ text: "ask @fixer", mentions: [fixer] });
  });

  it("sends the operator's last message again as they wrote it on 다시 시도", async () => {
    await render(desk([
      { kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] },
      { kind: "notice", id: 2, notice: "turn_failed", agent: "claude", detail: null },
    ]));
    await press("[data-desk-notice-action='retry']");
    expect(sent("desk_send")).toEqual([{ text: "@project(/work/repo-home repo) fix the build", mentions: [repo] }]);
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

  it("opens the checkout a project chip names", async () => {
    await render(desk([{ kind: "user", id: 1, text: "@project(/work/repo-home repo) fix the build", at_ms: 0, mentions: [repo] }]));
    await press("[data-mention-chip='project']");
    expect(sent("focus_checkout")).toEqual([expect.objectContaining({ workspace_id: "w-repo", checkout_id: "c-home" })]);
  });
});
