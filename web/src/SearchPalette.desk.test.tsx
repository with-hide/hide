// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { DESK_ROOM_CLOSED, activeTab } from "./deskRoom";
import { clientI18n } from "./i18n/translator";
import { Palette } from "./Palette";
import type { DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-2-ui D-24, B39: on Desk ⌘K lists sessions, PRs
// and missions; ↵ opens a session beside Desk and ⌘↵ goes to its tab; a PR or a
// mission opens in the tab panel and opens no session.

Element.prototype.scrollIntoView = () => {};

const rest = {
  navigator: {
    focused_device_id: "local", focused_checkout_id: "c1", focused_workspace_id: "w1",
    devices: [{ id: "local", kind: "local" }],
    agents: [legacyAgentRow({ id: "a1", pane_id: "p1", identity_label: "Login fix", agent_kind: "claude", mark: "going", group: "working", status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false })],
    workspaces: [{
      agent_scope: emptyScope(), id: "w1", label: "fixture", path: "/tmp/fixture", device_id: "local", registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] },
      pull_requests: [{ number: 12345, title: "Login retry", url: "https://github.com/o/r/pull/12345", badge: "open", review: null, is_draft: false, state: "mergeable", head_branch: "fix" }],
      checkouts: [{ agent_scope: emptyScope(), id: "c1", workspace_id: "w1", label: "main", path: "/tmp/fixture", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, active_tab_id: "t1", strip: [], tabs: [{ id: "t1", workspace_id: "w1", checkout_id: "c1", label: "Tab 1", empty: false, delegated: false, panes: [{ id: "p1", device_id: "local", device_pane_id: "p1", cwd: "/tmp/fixture", engine_label: null, terminal_title: null, identity_label: null, status_code: "working", requires_close_confirmation: false, requires_close_status_check: false }] }] }],
    }],
  },
  missions: { items: [{ id: "mission-a", title: "Login mission", project: "fixture", device_id: "local", nodes: [{ status: "done" }, { status: "running" }] }] },
  ui_state: {},
} as unknown as SnapshotRest;

describe("⌘K on Desk", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let sent: Parameters<DispatchFn>[0][];

  beforeEach(async () => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    sent = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("en"); });
    useShellStore.setState({ rest, desk: { settings: { enabled: true } } as unknown as DeskSection, agents: [] });
    useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, overlay: "search" });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
  });

  const open = async (query: string) => {
    const actions = createActions((event) => { sent.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><Palette actions={actions} /></TooltipProvider>));
    const input = document.querySelector<HTMLInputElement>("[data-palette-input]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, query);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const press = async (init: KeyboardEventInit) => act(async () => {
    document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...init }));
  });
  const groups = () => [...document.querySelectorAll("[data-palette-group]")].map((group) => group.getAttribute("data-palette-group"));

  const system = (platform: "MacIntel" | "Win32") => Object.defineProperty(navigator, "platform", { value: platform, configurable: true });
  /** Moves the highlight with the arrow keys onto the row of `group`, the row Enter then acts on. */
  const highlight = async (group: string) => {
    for (let step = 0; step < 12; step += 1) {
      if (document.querySelector(`[data-palette-group="${group}"] [data-selected="true"]`)) return;
      await act(async () => document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    }
    throw new Error(`no row of ${group} could be highlighted`);
  };

  it("lists the sessions, PRs and missions before anything is typed, and no Recent checkouts", async () => {
    await open("");
    expect(groups()).toEqual(["agents", "pull-requests", "missions"]);
  });

  it("opens a session beside Desk on ↵ and goes to its tab on ⌘↵", async () => {
    system("MacIntel");
    await open("login");
    expect(groups()).toEqual(expect.arrayContaining(["agents", "pull-requests", "missions"]));
    await highlight("agents");
    await press({});
    expect(useUiStore.getState().deskRoom.session).toBe("p1");
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });

    useUiStore.setState({ overlay: "search", deskRoom: DESK_ROOM_CLOSED });
    await open("login");
    await highlight("agents");
    await press({ metaKey: true });
    expect(useUiStore.getState().deskReturn).toBe("p1");
    // The session's tab is asked of the core, which moves the screen, and Desk takes no session beside it.
    expect(sent.some((event) => JSON.stringify(event.payload).includes('"p1"'))).toBe(true);
    expect(useUiStore.getState().deskRoom.session).toBeNull();
  });

  it("goes to a session's tab on the system's own command key: ⌘ on macOS, Ctrl elsewhere", async () => {
    for (const [platform, held, other] of [["MacIntel", { metaKey: true }, { ctrlKey: true }], ["Win32", { ctrlKey: true }, { metaKey: true }]] as const) {
      system(platform);
      for (const [init, tab] of [[held, true], [other, false]] as const) {
        useUiStore.setState({ overlay: "search", deskRoom: DESK_ROOM_CLOSED, screen: { kind: "desk" }, deskReturn: null });
        await open("login");
        await highlight("agents");
        await press(init);
        // A plain ↵ opens the session beside Desk; the command key goes to its tab.
        expect(useUiStore.getState().deskReturn === "p1").toBe(tab);
        expect(useUiStore.getState().deskRoom.session === "p1").toBe(!tab);
        await act(async () => root.unmount());
        root = createRoot(container);
      }
    }
  });

  it("opens a PR and a mission in the tab panel, opens no session for them, and keeps ⌘↵ the same as ↵ on them", async () => {
    system("MacIntel");
    await open("login mission");
    const mission = document.querySelector<HTMLElement>('[data-palette-row="mission:mission-a"]')!;
    expect(mission.textContent).toContain("1/2 · fixture");
    await act(async () => mission.click());
    let room = useUiStore.getState().deskRoom;
    expect(room.session).toBeNull();
    expect(activeTab(room.panel)).toEqual({ kind: "mission", mission: "mission-a", title: "Login mission" });

    useUiStore.setState({ overlay: "search", deskRoom: DESK_ROOM_CLOSED });
    await open("12345");
    await press({ metaKey: true });
    room = useUiStore.getState().deskRoom;
    expect(room.session).toBeNull();
    expect(activeTab(room.panel)).toMatchObject({ kind: "output", output: { kind: "pr", label: `#${12345}` } });
  });

  it("lists no mission on any other screen", async () => {
    useUiStore.setState({ screen: { kind: "workspace" } });
    await open("login");
    expect(groups()).not.toContain("missions");
  });
});
