// @vitest-environment jsdom
// PRD herdr-takeover: the wizard's Herdr step, Help > Set up Hide's, and
// Overview's Herdr place, drawn from the core's `status.herdr_takeover`.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { FirstRunWizard, useSetupWalk } from "./FirstRunWizard";
import { HerdrMovedLine, HerdrOverview, RESTART_HOLD_MS } from "./HerdrTakeover";
import { TooltipProvider } from "./components/ui/tooltip";
import type { HerdrTakeover } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const offer = (overrides: Partial<HerdrTakeover> = {}): HerdrTakeover => ({
  kind: "move",
  stage: "offered",
  server_version: "0.8.2",
  version: "0.9.5",
  same_protocol: false,
  failure: null,
  path_herdr_behind: null,
  ...overrides,
});

const state = (takeover: HerdrTakeover | null, step: string | null = "herdr", herdrState = "protocol_mismatch") => ({
  connection: "live" as const,
  revision: 1,
  rest: {
    ui_state: { agent_onboarding: "pending", first_run: step ? { version: 1, step } : null },
    navigator: { devices: [{ id: "local", kit: { agents: [{ id: "codex", label: "Codex", availability: "available", enabled: false, chosen: false, skill: { state: "off", reason: null, location: null }, hook: null, doc_url: "https://example.test" }], components: [], busy: false } }] },
    status: { herdr: { state: herdrState, received_version: "0.8.2" }, herdr_takeover: takeover ?? undefined, herdr_profiles: { state: "ready", error: null, profiles: [], truncated: false } },
  },
});

const sent = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "herdr_takeover").map((event) => event.payload);
const button = (selector: string) => document.querySelector(selector) as HTMLButtonElement | null;

async function mount(next: ReturnType<typeof state>, what: "wizard" | "overview" = "wizard") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    root.render(<TooltipProvider>{what === "wizard" ? <FirstRunWizard actions={actions} /> : <><HerdrMovedLine actions={actions} /><HerdrOverview actions={actions} /></>}</TooltipProvider>);
  });
  return { events, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const desktop = (restartHerdr = vi.fn(async () => "restarted")) => {
  window.hideHost = { kind: "electron", platform: "darwin", restartHerdr, notificationPermission: async () => "unsupported", updates: { check: vi.fn(), onStatus: () => () => {} } } as never;
  return restartHerdr;
};

afterEach(() => {
  useSetupWalk.getState().close();
  delete window.hideHost;
  document.body.innerHTML = "";
});

it("opens the first run on moving an older Herdr Hide cannot read, with no Not now and no count, and moves on one press (B1, B2, B3, B7)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer()));
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("herdr");
  expect(document.querySelector("[data-first-run-progress]")?.getAttribute("data-first-run-progress")).toBe("1");
  expect(document.querySelector("[data-herdr-takeover-versions]")?.getAttribute("data-herdr-takeover-versions")).toBe("0.8.2->0.9.5");
  expect(document.body.textContent).toContain("Workspaces and tabs");
  expect(document.body.textContent).not.toMatch(/\d+ (workspaces|tabs|agents)/);
  expect(button("[data-herdr-takeover-later]")).toBeNull();
  await act(async () => button("[data-herdr-takeover-move]")!.click());
  expect(sent(events)).toEqual([{ action: "move" }]);
  // ⌘↵ anywhere in the step presses the move too.
  await act(async () => { document.querySelector("[data-first-run]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true })); });
  expect(sent(events)).toEqual([{ action: "move" }, { action: "move" }]);
  await unmount();
});

it("holds the button while the core moves the server, so no second move is sent (B3)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "moving" })));
  const move = button("[data-herdr-takeover-move]")!;
  expect(move.disabled).toBe(true);
  expect(move.textContent).toContain("Moving…");
  await act(async () => { document.querySelector("[data-first-run]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true })); });
  expect(sent(events)).toEqual([]);
  await unmount();
});

it("a failed move names its cause, offers Try again, and only then Not now with the step to stop Herdr yourself (B8, B9)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "failed", failure: "refused" })));
  expect(document.querySelector("[data-herdr-takeover-failure]")?.getAttribute("data-herdr-takeover-failure")).toBe("refused");
  expect(document.querySelector("[data-herdr-takeover-stop-yourself]")).not.toBeNull();
  expect(button("[data-herdr-takeover-move]")!.textContent).toContain("Try again");
  expect(button("[data-herdr-takeover-restart]")).toBeNull();
  await act(async () => button("[data-herdr-takeover-later]")!.click());
  expect(sent(events)).toEqual([{ action: "later" }]);
  await unmount();
});

it("a server Hide reads as it is can be left from the start, without the hint to stop it (B6)", async () => {
  desktop();
  const { unmount } = await mount(state(offer({ server_version: "0.9.1", same_protocol: true }), "herdr", "connected"));
  expect(button("[data-herdr-takeover-later]")).not.toBeNull();
  expect(document.body.textContent).not.toContain("Hide cannot read this Herdr");
  await unmount();
});

it("a moved server whose operator's herdr is behind says so with a copyable herdr update, and Continue closes it (B5)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "moved", path_herdr_behind: "0.8.2" }), "herdr", "connected"));
  expect(document.querySelector("[data-herdr-takeover-path-behind]")?.textContent).toContain("herdr update --handoff");
  expect(document.body.textContent).toContain("0.8.2");
  await act(async () => button("[data-herdr-takeover-continue]")!.click());
  expect(sent(events)).toEqual([{ action: "continue" }]);
  await unmount();
});

it("a restart shows what ends, takes no Enter, and runs only on its own button (B10, B11, B12)", async () => {
  const restartHerdr = desktop(vi.fn(async () => "stop_failed"));
  const { events, unmount } = await mount(state(offer({ kind: "restart" })));
  expect(document.body.textContent).toContain("Commands running in panes end");
  expect(document.body.textContent).toContain("Workspaces and panes come back");
  expect(document.querySelector("[data-herdr-takeover-restart]")?.closest("span")?.querySelector("kbd")).toBeFalsy();
  await act(async () => { document.querySelector("[data-first-run]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true })); });
  expect(restartHerdr).not.toHaveBeenCalled();
  expect(document.activeElement?.hasAttribute("data-herdr-takeover-restart")).toBe(false);
  expect(button("[data-herdr-takeover-later]")).toBeNull();
  await act(async () => button("[data-herdr-takeover-restart]")!.click());
  expect(restartHerdr).toHaveBeenCalledTimes(1);
  // The stop that did not end the server is the core's to hold, as a failed move is.
  expect(sent(events)).toEqual([{ action: "stop_failed" }]);
  expect(button("[data-herdr-takeover-restart]")!.disabled).toBe(false);
  await unmount();
});

it("a restart whose stop failed names it, keeps the restart, and only then offers Not now with the step to stop Herdr yourself (B12, D-29)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ kind: "restart", stage: "failed", failure: "stop_failed" })));
  expect(document.querySelector('[data-herdr-takeover-failure="stop_failed"]')?.textContent).toContain("Herdr did not stop");
  expect(document.querySelector('[data-herdr-takeover-failure="stop_failed"] [data-herdr-takeover-stop-yourself]')).not.toBeNull();
  expect(button("[data-herdr-takeover-restart]")!.disabled).toBe(false);
  await act(async () => button("[data-herdr-takeover-later]")!.click());
  expect(sent(events)).toEqual([{ action: "later" }]);
  await unmount();
});

it("a restart that went through asks the core to look and holds its button for a bounded time", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    const restartHerdr = desktop();
    const { events, unmount } = await mount(state(offer({ kind: "restart", server_version: "0.9.1", same_protocol: true })));
    await act(async () => button("[data-herdr-takeover-restart]")!.click());
    expect(restartHerdr).toHaveBeenCalledTimes(1);
    expect(sent(events)).toEqual([{ action: "look" }]);
    expect(button("[data-herdr-takeover-restart]")!.disabled).toBe(true);
    await act(async () => button("[data-herdr-takeover-restart]")!.click());
    expect(restartHerdr).toHaveBeenCalledTimes(1);
    // An offer the look did not take away never strands the step.
    await act(async () => { vi.advanceTimersByTime(RESTART_HOLD_MS); });
    expect(button("[data-herdr-takeover-restart]")!.disabled).toBe(false);
    await unmount();
  } finally {
    vi.useRealTimers();
  }
});

it("a browser tab's wizard is the agent step alone, whatever the core's step (B19, D-24)", async () => {
  const { unmount } = await mount(state(offer()));
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("agents");
  await unmount();
});

it("Help > Set up Hide opens on the move only while there is one, and goes on to the agents once it is gone (B17)", async () => {
  desktop();
  const { unmount } = await mount(state(offer({ server_version: "0.9.1", same_protocol: true }), null, "connected"));
  await act(async () => { useSetupWalk.getState().open(); });
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("herdr");
  await act(async () => { useShellStore.setState(state(null, null, "connected") as never); });
  expect(useSetupWalk.getState().step).toBe("agents");
  await act(async () => { useSetupWalk.getState().close(); useSetupWalk.getState().open(); });
  expect(useSetupWalk.getState().step).toBe("agents");
  await unmount();
});

it("Overview puts the version, one line and the move in place of not answering; a server Hide reads adds nothing (B16)", async () => {
  const first = await mount(state(offer(), "done"), "overview");
  expect(document.querySelector("[data-herdr-takeover-overview]")?.getAttribute("data-herdr-takeover-overview")).toBe("move");
  expect(document.body.textContent).toContain("Herdr 0.8.2");
  expect(document.body.textContent).toContain("This Herdr isn't compatible with Hide");
  await act(async () => button("[data-herdr-takeover-move]")!.click());
  expect(sent(first.events)).toEqual([{ action: "move" }]);
  await first.unmount();

  const same = await mount(state(offer({ same_protocol: true }), "done", "connected"), "overview");
  expect(document.querySelector("[data-herdr-takeover-overview]")).toBeNull();
  await same.unmount();
});

it("Overview's restart confirms what ends first on the desktop and names the desktop app in a browser tab (B16, B19)", async () => {
  const browser = await mount(state(offer({ kind: "restart" }), "done"), "overview");
  expect(document.querySelector("[data-herdr-takeover-desktop-only]")?.textContent).toContain("desktop app");
  expect(button("[data-herdr-takeover-restart-open]")).toBeNull();
  await browser.unmount();

  const restartHerdr = desktop();
  const app = await mount(state(offer({ kind: "restart" }), "done"), "overview");
  await act(async () => button("[data-herdr-takeover-restart-open]")!.click());
  expect(restartHerdr).not.toHaveBeenCalled();
  expect(document.querySelector('[data-herdr-takeover-confirm="restart"]')?.textContent).toContain("Commands running in panes end");
  await act(async () => (document.querySelector('[data-herdr-takeover-confirm="restart"] [data-herdr-takeover-restart]') as HTMLButtonElement).click());
  expect(restartHerdr).toHaveBeenCalledTimes(1);
  await app.unmount();
});

it("a newer protocol asks for a Hide update, with the update check only in the desktop app (B18, B19)", async () => {
  const browser = await mount(state(offer({ kind: "update_hide", server_version: "0.10.0" }), "done"), "overview");
  expect(document.querySelector('[data-herdr-takeover-overview="update_hide"]')?.textContent).toContain("Update Hide");
  expect(button("[data-herdr-takeover-check-updates]")).toBeNull();
  await browser.unmount();
  desktop();
  const app = await mount(state(offer({ kind: "update_hide", server_version: "0.10.0" }), "done"), "overview");
  expect(button("[data-herdr-takeover-check-updates]")).not.toBeNull();
  await app.unmount();
});

it("a move made outside the wizard leaves its herdr update line until OK, and none when the operator's herdr keeps up (B5)", async () => {
  const behind = await mount(state(offer({ stage: "moved", path_herdr_behind: "0.8.2" }), "done", "connected"), "overview");
  expect(document.querySelector("[data-herdr-takeover-moved]")).not.toBeNull();
  await act(async () => button("[data-herdr-takeover-done]")!.click());
  expect(sent(behind.events)).toEqual([{ action: "continue" }]);
  await behind.unmount();
});
