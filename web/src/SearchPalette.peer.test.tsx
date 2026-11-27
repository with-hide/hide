// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { NOTHING_UNDER_WAY } from "./pairing";
import { Palette } from "./Palette";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Another computer's agent is found by ⌘K as an SSH device's agent is, and
// opens on that computer's screen with its pane chosen, as its row in the
// lists and its notice do (PRD device-connect-iroh D-50, D-78).

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

Element.prototype.scrollIntoView = () => {};

const key = "ab".repeat(32);
const rest = {
  navigator: {
    focused_device_id: "local",
    devices: [{ id: "local", kind: "local", agent_scope: { raised: [] } }, { id: "mini", kind: "remote", label: "Mac mini", state: "ready", peer_key: key, agent_scope: { raised: [] } }],
    agents: [],
    workspaces: [],
    peers: [{ key, device_id: "mini", name: "Mac mini", connected: true, opens: "active" }],
    peer_work: [{ device_id: "mini", key, needs_you: 1, done: 1, rows: [
      { id: "w1:p2", group: "needs_you", title: "Fix login redirect", agent_kind: "claude", place: "shop-api", mark: "turn", status_code: "question", detail: "Which branch?" },
      { id: "w1:p5", group: "done", title: "Tidy deploy scripts", agent_kind: "codex", place: "infra", mark: "unread", status_code: "done" },
    ] }],
  },
  ui_state: {},
} as unknown as SnapshotRest;

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
  useShellStore.setState({ rest, connection: "live", pairing: NOTHING_UNDER_WAY, agents: [] });
  useUiStore.setState({ screen: { kind: "main" }, overlay: "search", remoteScreen: null });
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  delete window.hideHost;
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
const enter = () => act(async () => {
  document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
});
const row = (id: string) => document.querySelector(`[data-palette-row="${id}"]`);

it("finds another computer's agent by its title and opens it on that computer's screen with its pane chosen", async () => {
  const go = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", remoteScreens: { show: vi.fn(), go, onState: () => () => {}, onWorkResult: () => () => {} } } as never;
  await open("login redirect");
  expect(row("peer-agent:mini:w1:p2")?.textContent).toContain("Fix login redirect");
  expect(row("peer-agent:mini:w1:p2")?.textContent).toContain("Mac mini");
  await enter();
  expect(go.mock.calls).toEqual([[{ device: "mini", key, work: { op: "show", pane: "w1:p2" } }]]);
  expect(useUiStore.getState().remoteScreen).toBe("mini");
  expect(sent.filter((event) => event.kind === "focus_pane" || event.kind === "focus_device")).toEqual([]);
});

it("selects that computer's tile from a window that draws no screen of it", async () => {
  await open("deploy scripts");
  expect(row("peer-agent:mini:w1:p5")).not.toBeNull();
  await enter();
  expect(sent.filter((event) => event.kind === "focus_device")).toEqual([{ schema_version: 2, kind: "focus_device", payload: { device_id: "mini" } }]);
});

it("lists no other computer's agent on a screen another computer shows of this one", async () => {
  useShellStore.setState({ pairing: null });
  await open("login redirect");
  expect(row("peer-agent:mini:w1:p2")).toBeNull();
});
