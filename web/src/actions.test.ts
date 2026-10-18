// @vitest-environment jsdom
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { clearProbeCache } from "./terminalLinkProvider";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";
import type { DispatchFn } from "./ws";

// No terminal renders here. Keep the real actions import while answering
// xterm's canvas capability probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
afterEach(() => useUiStore.getState().setViewFocusRequest(null));

describe("which pages the keyboard follows", () => {
  const workspace = { device_id: "local", path: "/repo" };
  const opened = () => {
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    const requestIds = () => sent.filter((event) => event.kind === "browser_open").map((event) => (event.payload as { request_id?: string }).request_id);
    return { actions, requestIds };
  };

  it("names a page the operator opens and waits for the display the core answers with", () => {
    const { actions, requestIds } = opened();
    actions.openBrowser("https://a.test/", workspace, undefined, undefined, "pane-1");
    actions.openBrowser("https://pr.test/", workspace);
    const [fromLink, fromRow] = requestIds();
    expect(fromLink).toMatch(/^follow:/);
    expect(fromRow).toMatch(/^follow:/);
    expect(fromRow).not.toBe(fromLink);
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: workspaceKey(workspace), browserOpen: fromRow });
  });

  it("leaves the keyboard alone for a New tab and a page's own new tab", () => {
    const { actions, requestIds } = opened();
    actions.openBrowser("", workspace, "a1");
    actions.openBrowser("https://b.test/", workspace, undefined, "page-1");
    expect(requestIds()).toEqual([undefined, undefined]);
    expect(useUiStore.getState().viewFocusRequest).toBeNull();
  });
});

// The MacBook's window after the core moved to the Mac mini: its own checkout
// is in the node's session, and the path a pane prints is the MacBook's.
describe("a terminal link to a file of this window's own checkout", () => {
  const checkout = { id: "remote:mbp:checkout:1", workspace_id: "remote:mbp:project:1", path: "/work/proj-m", exists: true };
  const rest = {
    navigator: { devices: [{ id: "core", kind: "local" }, { id: "mbp", kind: "remote", dials_in: true }], workspaces: [] },
    status: { remote: [{ target_id: "mbp", session: { workspaces: [{ id: checkout.workspace_id, device_id: "mbp", path: checkout.path, checkouts: [checkout] }] } }] },
  } as never;
  const revealed = async (hash: string) => {
    window.location.hash = hash;
    const sent: Parameters<DispatchFn>[0][] = [];
    createActions((event) => { sent.push(event); return true; }).openTerminalPath({ real: "/work/proj-m/README.md", kind: "file" }, null, null, false);
    await vi.waitFor(() => expect(sent.some((event) => event.kind === "reveal_path")).toBe(true));
    return sent.find((event) => event.kind === "reveal_path")?.payload;
  };

  afterEach(() => {
    window.location.hash = "";
    clearProbeCache();
    Reflect.deleteProperty(window, "hideHost");
    useShellStore.setState({ rest: null });
  });

  it("names the node, so the core's machine never judges the path against its own checkouts", async () => {
    Object.assign(window, { hideHost: { kind: "electron", probePaths: async (paths: string[]) => paths.map((path) => ({ real: path, kind: "directory" })), openPath: vi.fn() } });
    useShellStore.setState({ rest });
    expect(await revealed("#token=t&node=mbp")).toMatchObject({ path: "/work/proj-m/README.md", workspace_id: checkout.workspace_id, checkout_id: checkout.id, device_id: "mbp" });
  });

  it("names no device for a checkout of the core's own machine", async () => {
    Object.assign(window, { hideHost: { kind: "electron", probePaths: async (paths: string[]) => paths.map((path) => ({ real: path, kind: "directory" })), openPath: vi.fn() } });
    const own = { ...checkout, id: "c1", workspace_id: "w1" };
    useShellStore.setState({ rest: { navigator: { devices: [{ id: "core", kind: "local" }], workspaces: [{ id: "w1", device_id: "core", path: own.path, checkouts: [own] }] }, status: { remote: [] } } as never });
    const payload = await revealed("#token=t");
    expect(payload).toMatchObject({ checkout_id: "c1" });
    expect(payload).not.toHaveProperty("device_id");
  });
});

describe("input after a confirmed View close", () => {
  const workspace = { device_id: "local", path: "/repo" };
  const frame = (ids: string[], selected: string) => ({ ...workspace, tool: "files", tree: true, layout: { active_area: "a1", display_count: ids.length, root: { area: { id: "a1", active: selected, displays: ids.map((id) => ({ id, kind: "browser", state: "open", label: id, url: "", tab_id: null })) } } } });
  const setup = async () => {
    vi.stubGlobal("CSS", { escape: (value: string) => value });
    const { installKeyboardOwner } = await import("./viewFocus");
    const remove = installKeyboardOwner();
    const checkout = { id: "c", path: "/repo" };
    const rest = { navigator: { focused_checkout_id: "c", focused_workspace_id: "w", workspaces: [{ id: "w", device_id: "local", checkouts: [checkout] }] }, workspace_view: frame(["d1", "d2"], "d1") } as never;
    useShellStore.setState({ rest, connection: "live", editor: null });
    document.body.innerHTML = '<div data-workspace-screen="c"><div data-view-area-id="a1"><button role="tab" aria-selected="true">next</button><div class="cm-content" tabindex="0" contenteditable="true"></div></div><input data-other /></div>';
    const content = document.querySelector<HTMLElement>(".cm-content")!;
    content.focus();
    const actions = createActions(() => true);
    const land = () => useShellStore.setState({ rest: { ...useShellStore.getState().rest, workspace_view: frame(["d2"], "d2") } as never });
    return { remove, actions, content, land };
  };
  afterEach(() => { useShellStore.setState({ rest: null }); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

  it("waits for removal, then follows the replacement content", async () => {
    const { remove, actions, content, land } = await setup();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return 1; });
    try {
      actions.closeView("d1");
      expect(frames).toHaveLength(0);
      content.blur();
      land();
      frames.forEach((callback) => callback(0));
      expect(document.activeElement).toBe(content);
    } finally { remove(); }
  });

  it("keeps newer input when a delayed close finally removes its View", async () => {
    const { remove, actions, land } = await setup();
    const other = document.querySelector<HTMLInputElement>("[data-other]")!;
    try {
      actions.closeView("d1");
      other.focus();
      land();
      expect(document.activeElement).toBe(other);
    } finally { remove(); }
  });
});
