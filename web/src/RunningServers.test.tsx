import { emptyScope } from "../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { RunningServers } from "./RunningServers";
import type { Checkout, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import type { WorkspaceView } from "./workspace";
import type { DispatchFn } from "./ws";

// This flow does not render a terminal. Keep the real actions import while
// answering xterm's canvas capability probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const view: WorkspaceView = {
  device_id: "local", path: "/projects/studio", tool: null, last_tool: null, prs: { only_checkout: false }, pinned: false, tree: false,
  lane_width: null, panel_width: null, tree_width: null, lane_called: 0, lane_calls: 0,
};
function checkout(servers: { host: string; port: number }[]): Checkout {
  return { agent_scope: emptyScope(),
    id: "checkout:studio", workspace_id: "project:studio", label: "Studio", path: view.path,
    branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true,
    pull_request: null, active_tab_id: "tab:studio", strip: [], next_tab_label: "2",
    tabs: [{
      id: "tab:studio", workspace_id: "project:studio", checkout_id: "checkout:studio", label: "1", empty: false, delegated: false,
      panes: [{
        id: "pane:studio", herdr_label: null, terminal_title: null, cwd: view.path,
        status_code: "unknown", requires_close_confirmation: false, requires_close_status_check: false,
        identity_label: null, servers,
      }],
    }],
  };
}
function project(device: string, rows: Checkout[]): Workspace {
  return { agent_scope: emptyScope(),
    id: "project:studio", label: "Studio", path: view.path, device_id: device,
    registered: true, temporary: false, pinned: false, checkouts: rows,
    inactive_checkouts: { expanded: false, checkout_ids: [] },
  };
}
function catalog(current: Checkout, remote: Checkout): SnapshotRest {
  return {
    status: { server_discovery: { loading: false, failure: null } },
    navigator: {
      // A remote Project may have the same IDs and paths. It must not satisfy
      // the local endpoint check, even when it is listed first and focused.
      workspaces: [project("mini", [remote]), project("local", [current])],
      devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }] as never,
      focused_device_id: "mini", focused_checkout_id: remote.id,
    },
    workspace_view: { ...view, device_id: "mini", path: "/projects/other" },
  };
}

describe("Workspace server action", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(() => {
    saved = useShellStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    // jsdom has no layout observer. This substitutes only the browser API
    // used by the real Radix popover, not product state or interaction code.
    vi.stubGlobal("ResizeObserver", class BrowserResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    useShellStore.setState({ connection: "live", rest: null });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
  });
  async function render(displayed: Checkout) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><RunningServers checkout={displayed} view={view} actions={actions} /></TooltipProvider>));
  }
  async function openServer() {
    const button = container.querySelector<HTMLButtonElement>("button[data-open-server]");
    if (!button) throw new Error("Missing server action");
    await act(async () => button.click());
  }

  it("names how many servers are known, and leaves the count out while it is unknown", async () => {
    const name = () => container.querySelector("button[data-open-server]")?.getAttribute("aria-label");
    const listing = (count: number) => checkout(Array.from({ length: count }, (_, index) => ({ host: "127.0.0.1", port: 3000 + index })));
    for (const [count, expected] of [[0, "Open server, 0 running"], [1, "Open server, 1 running"], [2, "Open server, 2 running"]] as const) {
      const shown = listing(count);
      useShellStore.setState({ rest: catalog(shown, shown) });
      await render(shown);
      expect(name()).toBe(expected);
    }
    const reading = listing(2);
    useShellStore.setState({ rest: { ...catalog(reading, reading), status: { server_discovery: { loading: true, failure: null } } } });
    await render(reading);
    expect(name()).toBe("Open server");
  });

  it("shows actionable small feedback when the single direct server has disappeared before the click", async () => {
    const stale = checkout([{ host: "127.0.0.1", port: 3000 }]);
    useShellStore.setState({ rest: catalog(stale, stale) });
    await render(stale);
    await act(async () => useShellStore.setState({ rest: catalog(checkout([]), stale) }));
    await openServer();

    const picker = document.querySelector<HTMLElement>('[aria-label="Running servers"]');
    expect(picker).not.toBeNull();
    expect(picker?.querySelector('[role="status"]')?.textContent).toContain("This server is no longer listed. Choose a running server again.");
    expect(events.filter((event) => event.kind === "browser_open")).toHaveLength(0);
  });

  it("opens a single live server directly in its own Workspace despite a different front device and colliding catalog IDs", async () => {
    const local = checkout([{ host: "::1", port: 3000 }]);
    const remote = checkout([{ host: "127.0.0.1", port: 4000 }]);
    useShellStore.setState({ rest: catalog(local, remote) });
    await render(local);
    await openServer();

    expect(events.filter((event) => event.kind === "browser_open")).toEqual([{
      schema_version: 2, kind: "browser_open",
      payload: { url: "http://[::1]:3000", workspace: { device_id: "local", path: view.path }, request_id: expect.stringMatching(/^follow:/) },
    }]);
    expect(document.querySelector('[aria-label="Running servers"]')).toBeNull();
  });

  it("opens a server of the machine this screen runs on when the core runs on another, and of no other device", async () => {
    const remote = checkout([{ host: "127.0.0.1", port: 4000 }]);
    const remoteView = { ...view, device_id: "mini" };
    // A node that dials the core lists its projects in its own session, as every device does, never in the core's own list.
    const base = catalog(checkout([]), remote);
    useShellStore.setState({
      rest: {
        ...base,
        navigator: { ...base.navigator, workspaces: [project("local", [checkout([])])], devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote", dials_in: true }] as never },
        status: { ...base.status, remote: [{ target_id: "mini", state: "connected", session: { workspaces: [project("mini", [remote])] } }] as never },
      },
    });
    const actions = createActions((event) => { events.push(event); return true; });
    const show = async () => act(async () => root.render(<TooltipProvider><RunningServers checkout={remote} view={remoteView} actions={actions} /></TooltipProvider>));
    await show();
    expect(container.querySelector("button[data-open-server]")?.getAttribute("aria-label")).toBe("Open server");

    window.location.hash = "#token=t&node=mini";
    try {
      await show();
      await openServer();
    } finally {
      window.location.hash = "";
    }
    expect(events.filter((event) => event.kind === "browser_open")).toEqual([{
      schema_version: 2, kind: "browser_open",
      payload: { url: "http://127.0.0.1:4000", workspace: { device_id: "mini", path: view.path }, request_id: expect.stringMatching(/^follow:/) },
    }]);
  });
});
