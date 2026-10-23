// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { PaneConnectionChip } from "./PaneConnection";
import type { PaneConnection, PaneRow } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const pane = (connection: PaneConnection | null | undefined): PaneRow =>
  ({ id: "w1:p2", children: connection === undefined ? undefined : { instrumented: false, uninstrumented_reason: null, uninstrumented_label: null, chips: [], connection } }) as never;

const not = (over: Partial<PaneConnection> = {}): PaneConnection => ({ connected: false, can_reopen: true, reason: "started_before_hide", reopen: null, ...over });

const snapshot = (codexOff?: unknown) => ({
  connection: "live" as const,
  rest: { navigator: { devices: [{ id: "local", label: "This Mac", kind: "local", kit: { codex_daemon_off: codexOff ?? null } }] } },
});

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(row: PaneRow, next = snapshot()) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const render = async (nextRow: PaneRow) => {
    await act(async () => {
      root.render(<TooltipProvider><PaneConnectionChip pane={nextRow} actions={actions} local /></TooltipProvider>);
    });
  };
  await act(async () => { useShellStore.setState(next as never); });
  await render(row);
  return { events, render, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const chip = () => document.querySelector("[data-pane-connection]") as HTMLElement | null;
const popover = () => document.querySelector("[data-pane-connection-popover]") as HTMLElement | null;
const open = async () => { await act(async () => { chip()!.click(); }); };

it("draws no chip for a connected pane, a pane with nothing to judge, or a plain shell", async () => {
  for (const row of [pane({ connected: true, can_reopen: false, reason: null, reopen: null }), pane(null), pane(undefined)]) {
    const { unmount } = await mount(row);
    expect(chip()).toBeNull();
    await unmount();
  }
});

it("opens from the chip, closes with Not now and leaves the chip where it was", async () => {
  const { events, unmount } = await mount(pane(not()));
  expect(chip()?.getAttribute("data-pane-connection")).toBe("started_before_hide");
  expect(chip()?.getAttribute("aria-label")).toBe("Hide integration limited");
  expect(chip()?.textContent).toBe("");
  expect(popover()).toBeNull();
  await open();
  expect(popover()?.textContent).toContain("Reconnect this session to Hide");
  await act(async () => { (document.querySelector("[data-pane-connection-dismiss]") as HTMLElement).click(); });
  expect(popover()).toBeNull();
  expect(chip()).not.toBeNull();
  expect(events).toEqual([]);
  await unmount();
});

it("sends Reopen once however many times it is pressed before the core answers, and reads the core's pending and failed states", async () => {
  const { events, render, unmount } = await mount(pane(not()));
  await open();
  const reopen = () => document.querySelector("[data-pane-reopen]") as HTMLButtonElement;
  await act(async () => { reopen().click(); reopen().click(); reopen().click(); });
  expect(events).toEqual([{ schema_version: 2, kind: "pane_reopen", payload: { pane_id: "w1:p2" } }]);

  // The snapshot says it is running: the button is busy and cannot send again.
  await act(async () => { useShellStore.setState({ rest: { ...useShellStore.getState().rest! } }); });
  await render(pane(not({ reopen: { state: "pending" } })));
  expect(reopen().disabled).toBe(true);
  expect(reopen().getAttribute("aria-busy")).toBe("true");

  await act(async () => { useShellStore.setState({ rest: { ...useShellStore.getState().rest! } }); });
  await render(pane(not({ reopen: { state: "failed", reason: "agent_busy" } })));
  expect(reopen().disabled).toBe(false);
  expect(document.querySelector("[data-pane-reopen-failed]")?.getAttribute("data-pane-reopen-failed")).toBe("agent_busy");
  expect(document.querySelector("[role=alert]")?.textContent).toContain("working or waiting");

  // Pressing it again after the refusal is a new intent.
  await act(async () => { reopen().click(); });
  expect(events.length).toBe(2);
  await unmount();
});

it("offers no Reopen where it would change nothing: a setup problem, and a pane on another device", async () => {
  const setup = await mount(pane(not({ reason: "setup_needed", can_reopen: false })));
  await open();
  expect(popover()?.textContent).toContain("Hide isn't set up for this agent");
  expect(document.querySelector("[data-pane-reopen]")).toBeNull();
  expect(document.querySelector("[data-pane-connection-dismiss]")).not.toBeNull();
  await setup.unmount();

  const remote = await mount(pane(not({ can_reopen: false })));
  await open();
  expect(document.querySelector("[data-pane-reopen]")).toBeNull();
  await remote.unmount();
});

const offLink = () => document.querySelector("[data-codex-shared-server-off]") as HTMLElement | null;
const confirmation = () => document.querySelector("[data-codex-shared-server-confirm]") as HTMLElement | null;
const press = async (selector: string) => { await act(async () => { (document.querySelector(selector) as HTMLElement).click(); }); };

it("explains the shared server, asks before turning it off and stopping it, and shows the answer it reads back (PRD codex-daemon-apply B1-B3)", async () => {
  const { events, render, unmount } = await mount(pane(not({ reason: "codex_shared_server" })));
  await open();
  expect(popover()?.textContent).toContain("Hide can't follow this Codex");
  expect(popover()?.textContent).toContain("Reopen with Hide");
  expect(popover()?.textContent).toContain("Changes Codex everywhere on this Mac and stops its running shared server.");

  // The link asks first and sends nothing.
  await act(async () => { offLink()!.click(); });
  expect(events).toEqual([]);
  const dialog = confirmation();
  expect(dialog?.getAttribute("data-codex-shared-server-confirm")).toBe("local");
  expect(dialog?.textContent).toContain("Turn off Codex's shared server on this Mac?");
  expect(dialog?.textContent).toContain("the one running now stops");
  expect(dialog?.textContent).toContain("Every Codex attached to it disconnects. In its pane, run codex resume to continue.");
  expect(dialog?.textContent).toContain("Codex that Hide started doesn't use this server and keeps running.");
  // No button has the keyboard, and no count of Codex is claimed (B2).
  expect(document.activeElement?.tagName).not.toBe("BUTTON");
  expect(dialog?.textContent).not.toMatch(/\d/);

  await press("[data-codex-shared-server-go]");
  expect(events).toEqual([{ schema_version: 2, kind: "codex_daemon_disable", payload: { device_id: "local" } }]);
  expect(confirmation()).toBeNull();
  // The popover stays to show the answer (B3).
  expect(popover()).not.toBeNull();

  await act(async () => { useShellStore.setState(snapshot({ state: "pending" }) as never); });
  expect(document.querySelector("[data-codex-shared-server-outcome]")?.getAttribute("data-codex-shared-server-outcome")).toBe("pending");
  expect(offLink()?.hasAttribute("disabled")).toBe(true);

  // Done: the pane now reads "started before Hide was set up", and the answer stays on screen.
  await act(async () => { useShellStore.setState(snapshot({ state: "done" }) as never); });
  await render(pane(not({ reason: "started_before_hide" })));
  expect(document.querySelector("[data-codex-shared-server-outcome]")?.getAttribute("data-codex-shared-server-outcome")).toBe("done");
  expect(popover()?.textContent).toContain("Codex's shared server is off.");
  await unmount();
});

it("changes nothing when the confirmation is kept or dismissed (B2)", async () => {
  const { events, unmount } = await mount(pane(not({ reason: "codex_shared_server" })));
  await open();
  await act(async () => { offLink()!.click(); });
  await press("[data-codex-shared-server-keep]");
  expect(confirmation()).toBeNull();
  await act(async () => { offLink()!.click(); });
  await act(async () => {
    confirmation()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  expect(confirmation()).toBeNull();
  expect(events).toEqual([]);
  await unmount();
});

it("closes the confirmation by itself once its pane stops offering the turn-off", async () => {
  const { events, render, unmount } = await mount(pane(not({ reason: "codex_shared_server" })));
  await open();
  await act(async () => { offLink()!.click(); });
  expect(confirmation()).not.toBeNull();
  await render(pane(not({ reason: "started_before_hide" })));
  expect(confirmation()).toBeNull();
  expect(events).toEqual([]);
  await unmount();
});

it("says when autostart went off but the running server did not stop, and keeps the link to try again (B7)", async () => {
  const { events, unmount } = await mount(pane(not({ reason: "codex_shared_server" })));
  await open();
  await act(async () => { offLink()!.click(); });
  await press("[data-codex-shared-server-go]");
  await act(async () => { useShellStore.setState(snapshot({ state: "failed", reason: "stop_failed" }) as never); });
  const line = document.querySelector("[data-codex-shared-server-outcome]");
  expect(line?.getAttribute("data-codex-shared-server-outcome")).toBe("failed");
  expect(line?.textContent).toBe("Autostart is off, but the running shared server didn't stop. Try again.");
  expect(offLink()?.hasAttribute("disabled")).toBe(false);
  await act(async () => { offLink()!.click(); });
  await press("[data-codex-shared-server-go]");
  expect(events).toHaveLength(2);
  await unmount();
});

it("does not show the last request's answer as the next one's until the core moves on (B9: a second turn-off)", async () => {
  const { events, unmount } = await mount(pane(not({ reason: "codex_shared_server" })), snapshot({ state: "done" }));
  await open();
  await act(async () => { offLink()!.click(); });
  await press("[data-codex-shared-server-go]");
  expect(events).toHaveLength(1);
  expect(document.querySelector("[data-codex-shared-server-outcome]")).toBeNull();
  await act(async () => { useShellStore.setState(snapshot({ state: "pending" }) as never); });
  expect(document.querySelector("[data-codex-shared-server-outcome]")?.getAttribute("data-codex-shared-server-outcome")).toBe("pending");
  await act(async () => { useShellStore.setState(snapshot({ state: "done" }) as never); });
  expect(document.querySelector("[data-codex-shared-server-outcome]")?.getAttribute("data-codex-shared-server-outcome")).toBe("done");
  await unmount();
});

it("says only that the machine could not be reached when the answer is unreachable, which may come after autostart went off", async () => {
  const { unmount } = await mount(pane(not({ reason: "codex_shared_server" })));
  await open();
  await act(async () => { offLink()!.click(); });
  await press("[data-codex-shared-server-go]");
  await act(async () => { useShellStore.setState(snapshot({ state: "failed", reason: "unreachable" }) as never); });
  const line = document.querySelector("[data-codex-shared-server-outcome]");
  expect(line?.getAttribute("data-codex-shared-server-outcome")).toBe("failed");
  expect(line?.textContent).toBe("Couldn't reach that device. Try again.");
  expect(offLink()?.hasAttribute("disabled")).toBe(false);
  await unmount();
});

it("does not open on an old answer: a finished turn-off nobody asked for in this popover is not shown", async () => {
  const { unmount } = await mount(pane(not({ reason: "codex_shared_server" })), snapshot({ state: "failed", reason: "timed_out" }));
  await open();
  expect(document.querySelector("[data-codex-shared-server-outcome]")).toBeNull();
  await unmount();
});
