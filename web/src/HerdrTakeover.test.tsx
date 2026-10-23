// @vitest-environment jsdom
// PRD herdr-takeover: the wizard's Herdr step, Help > Set up Hide's, and the
// screen that holds Hide while this machine's Herdr speaks another protocol,
// drawn from the core's `status.herdr_takeover` and `status.herdr`.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { FirstRunWizard, useSetupWalk } from "./FirstRunWizard";
import { HerdrGate, HerdrMovedLine, RESTART_HOLD_MS } from "./HerdrTakeover";
import { herdrGate } from "./herdrTakeoverRules";
import { AboutTab } from "./settings/AboutTab";
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

const state = (takeover: HerdrTakeover | null, step: string | null = "herdr", herdrState = "protocol_mismatch", received = { version: "0.8.2", protocol: 20 }) => ({
  connection: "live" as const,
  revision: 1,
  daemon: { core_state_path: "/srv/op/.hide/state/core-state.json" },
  rest: {
    ui_state: { agent_onboarding: "pending", first_run: step ? { version: 1, step } : null },
    navigator: { devices: [{ id: "local", kit: { agents: [{ id: "codex", label: "Codex", availability: "available", enabled: false, chosen: false, skill: { state: "off", reason: null, location: null }, hook: null, doc_url: "https://example.test" }], components: [], busy: false } }] },
    status: {
      herdr: { state: herdrState, received_version: received.version, received_protocol: received.protocol, expected_protocol: 22 },
      herdr_takeover: takeover ?? undefined,
      herdr_versions: { hide: "0.9.5", path_herdr: { version: "0.8.2", behind: true, update_command: "install -m 755 '/Applications/hide.app/Contents/Resources/herdr' '/srv/op/.local/bin/herdr'" } },
      herdr_profiles: { state: "ready", error: null, profiles: [], truncated: false },
    },
  },
});

const sent = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "herdr_takeover").map((event) => event.payload);
const button = (selector: string) => document.querySelector(selector) as HTMLButtonElement | null;

/** The shell's place for the screen that holds Hide, as `App` draws it. */
function Gate({ actions }: { actions: ReturnType<typeof createActions> }) {
  const gate = useShellStore((s) => herdrGate(s.rest));
  return gate ? <HerdrGate actions={actions} gate={gate} /> : null;
}

async function mount(next: ReturnType<typeof state>, what: "wizard" | "gate" = "wizard") {
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
    root.render(<TooltipProvider>{what === "wizard" ? <FirstRunWizard actions={actions} /> : <><HerdrMovedLine actions={actions} /><Gate actions={actions} /></>}</TooltipProvider>);
  });
  return { events, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const host = { quit: vi.fn(), revealPath: vi.fn(), check: vi.fn() };
const desktop = (restartHerdr = vi.fn(async () => "restarted")) => {
  window.hideHost = { kind: "electron", platform: "darwin", restartHerdr, quit: host.quit, revealPath: host.revealPath, notificationPermission: async () => "unsupported", updates: { check: host.check, onStatus: () => () => {} } } as never;
  return restartHerdr;
};

afterEach(() => {
  vi.clearAllMocks();
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

it("a moved server whose operator's herdr is behind shows the core's command to copy and what it does, and Continue closes it (B5)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "moved", path_herdr_behind: "0.8.2" }), "herdr", "connected"));
  const line = document.querySelector("[data-herdr-takeover-path-behind]")!;
  expect(line.textContent).toContain("install -m 755 '/Applications/hide.app/Contents/Resources/herdr' '/srv/op/.local/bin/herdr'");
  expect(line.textContent).toContain("Replaces it with Hide's herdr 0.9.5");
  expect(line.textContent).not.toContain("--handoff");
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

it("a look that could not read a server Hide cannot use says so with Try again and Not now, instead of skipping to the agents (B8, B9)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ kind: "look", stage: "failed", failure: "unreadable", server_version: "", version: "" }), "herdr", "malformed"));
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("herdr");
  expect(document.body.textContent).toContain("Couldn't read Herdr");
  await act(async () => button("[data-herdr-takeover-look]")!.click());
  await act(async () => button("[data-herdr-takeover-later]")!.click());
  expect(sent(events)).toEqual([{ action: "look" }, { action: "later" }]);
  await unmount();
});

it("an older protocol holds Hide on its move with no Not now, and offers View log and Quit beside it", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer(), "done"), "gate");
  expect(document.querySelector("[data-herdr-gate]")?.getAttribute("data-herdr-gate")).toBe("move");
  expect(document.querySelector("[data-herdr-takeover-versions]")?.getAttribute("data-herdr-takeover-versions")).toBe("0.8.2->0.9.5");
  expect(button("[data-herdr-takeover-later]")).toBeNull();
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(document.querySelector("[data-herdr-gate]")).not.toBeNull();
  await act(async () => button("[data-herdr-takeover-move]")!.click());
  expect(sent(events)).toEqual([{ action: "move" }]);
  await act(async () => button("[data-herdr-gate-log]")!.click());
  expect(host.revealPath).toHaveBeenCalledWith("/srv/op/.hide/state/Logs/core.jsonl");
  await act(async () => button("[data-herdr-gate-quit]")!.click());
  expect(host.quit).toHaveBeenCalledTimes(1);
  await unmount();
});

it("a move that failed under the hold keeps Try again and offers no Not now", async () => {
  desktop();
  const { unmount } = await mount(state(offer({ stage: "failed", failure: "refused" }), "done"), "gate");
  expect(button("[data-herdr-takeover-move]")!.textContent).toContain("Try again");
  expect(button("[data-herdr-takeover-later]")).toBeNull();
  await unmount();
});

it("a restart under the hold shows what ends; a browser tab names the desktop app and has no View log or Quit", async () => {
  const browser = await mount(state(offer({ kind: "restart", server_version: "0.6.2" }), "done", "protocol_mismatch", { version: "0.6.2", protocol: 10 }), "gate");
  expect(document.querySelector("[data-herdr-gate]")?.getAttribute("data-herdr-gate")).toBe("restart");
  expect(document.querySelector("[data-herdr-takeover-desktop-only]")?.textContent).toContain("desktop app");
  expect(button("[data-herdr-takeover-restart]")).toBeNull();
  expect(button("[data-herdr-gate-quit]")).toBeNull();
  await browser.unmount();

  const restartHerdr = desktop();
  const app = await mount(state(offer({ kind: "restart", server_version: "0.6.2" }), "done", "protocol_mismatch", { version: "0.6.2", protocol: 10 }), "gate");
  expect(document.body.textContent).toContain("Commands running in panes end");
  await act(async () => button("[data-herdr-takeover-restart]")!.click());
  expect(restartHerdr).toHaveBeenCalledTimes(1);
  await app.unmount();
});

it("a restart under the hold keeps its step while the old server is gone and Hide's is not yet read", async () => {
  const restartHerdr = desktop(vi.fn(async () => "restarted"));
  const old = state(offer({ kind: "restart", server_version: "0.6.2" }), "done", "protocol_mismatch", { version: "0.6.2", protocol: 10 });
  const { unmount } = await mount(old, "gate");
  await act(async () => button("[data-herdr-takeover-restart]")!.click());
  expect(restartHerdr).toHaveBeenCalledTimes(1);
  // The core's look found no older server; the connection has not read Hide's Herdr yet.
  await act(async () => { useShellStore.setState(state(null, "done", "protocol_mismatch", { version: "0.6.2", protocol: 10 }) as never); });
  expect(document.querySelector("[data-herdr-gate]")?.getAttribute("data-herdr-gate")).toBe("older");
  expect(button("[data-herdr-takeover-restart]")!.textContent).toContain("Restarting…");
  expect(document.body.textContent).not.toContain("isn't compatible");
  await unmount();
});

it("a newer protocol holds Hide with the wait for a Hide update, the two versions read, and Check for updates in the desktop app", async () => {
  const newer = offer({ kind: "update_hide", server_version: "0.10.0" });
  const received = { version: "0.10.0", protocol: 23 };
  const browser = await mount(state(newer, "done", "protocol_mismatch", received), "gate");
  expect(document.body.textContent).toContain("Herdr is newer than Hide");
  expect(document.querySelector("[data-herdr-gate-versions]")?.getAttribute("data-herdr-gate-versions")).toBe("0.10.0>0.9.5");
  expect(button("[data-herdr-takeover-check-updates]")).toBeNull();
  await browser.unmount();

  desktop();
  const app = await mount(state(newer, "done", "protocol_mismatch", received), "gate");
  await act(async () => button("[data-herdr-takeover-check-updates]")!.click());
  expect(host.check).toHaveBeenCalledTimes(1);
  await act(async () => button("[data-herdr-gate-look]")!.click());
  expect(sent(app.events)).toEqual([{ action: "look" }]);
  await app.unmount();
});

it("a mismatch the core offers nothing for still holds Hide, with its version and Try again", async () => {
  const { events, unmount } = await mount(state(null, "done"), "gate");
  expect(document.querySelector("[data-herdr-gate]")?.getAttribute("data-herdr-gate")).toBe("older");
  expect(document.body.textContent).toContain("This Herdr isn't compatible with Hide");
  expect(document.body.textContent).toContain("Herdr 0.8.2");
  await act(async () => button("[data-herdr-gate-look]")!.click());
  expect(sent(events)).toEqual([{ action: "look" }]);
  await unmount();
});

it("a server on Hide's protocol never holds Hide, whatever its release (D-16)", async () => {
  const { unmount } = await mount(state(offer({ server_version: "0.9.4", same_protocol: true }), "done", "connected", { version: "0.9.4", protocol: 22 }), "gate");
  expect(document.querySelector("[data-herdr-gate]")).toBeNull();
  await unmount();
});

it("a move made outside the wizard leaves its herdr update line until OK, and none when the operator's herdr keeps up (B5)", async () => {
  const behind = await mount(state(offer({ stage: "moved", path_herdr_behind: "0.8.2" }), "done", "connected"), "gate");
  expect(document.querySelector("[data-herdr-takeover-moved]")).not.toBeNull();
  await act(async () => button("[data-herdr-takeover-done]")!.click());
  expect(sent(behind.events)).toEqual([{ action: "continue" }]);
  await behind.unmount();
});

it("Settings > About names the server and the terminal's herdr, with a button only where there is something to do", async () => {
  const versions = (path: { version: string; behind: boolean; update_command: string } | null) => ({ hide: "0.9.5", path_herdr: path });
  const about = async (herdr: Record<string, unknown>, takeover: HerdrTakeover | null, herdrVersions: ReturnType<typeof versions>) => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const events: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { events.push(event); return true; });
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: { status: { herdr, herdr_takeover: takeover ?? undefined, herdr_versions: herdrVersions } } } as never);
      root.render(<TooltipProvider><AboutTab actions={actions} /></TooltipProvider>);
    });
    return { events, unmount: () => act(async () => root.unmount()) };
  };
  const quiet = await about({ state: "connected", received_version: "0.9.5" }, null, versions({ version: "0.9.5", behind: false, update_command: "install" }));
  expect(document.querySelector("[data-herdr-server]")?.textContent).toContain("0.9.5 · same as Hide");
  expect(document.querySelector('[data-path-herdr="current"]')?.textContent).toContain("0.9.5");
  expect(document.querySelectorAll("[data-herdr-takeover-move], [data-herdr-takeover-copy]")).toHaveLength(0);
  await quiet.unmount();

  const brew = "brew upgrade herdr";
  const older = await about({ state: "connected", received_version: "0.9.4" }, offer({ server_version: "0.9.4", same_protocol: true }), versions({ version: "0.8.2", behind: true, update_command: brew }));
  expect(document.querySelector("[data-herdr-server]")?.textContent).toContain("0.9.4 · Hide has 0.9.5");
  expect(document.querySelector('[data-path-herdr="behind"]')?.textContent).toContain("0.8.2 · its commands are refused");
  expect(document.querySelector("[data-herdr-update-command]")?.getAttribute("data-herdr-update-command")).toBe(brew);
  expect(document.body.textContent).toContain("Updates it with Homebrew");
  await act(async () => button("[data-herdr-takeover-move]")!.click());
  // Opening About looks again, so a `herdr` updated since reads as it is now.
  expect(sent(older.events)).toEqual([{ action: "look" }, { action: "move" }]);
  await older.unmount();

  const none = await about({ state: "connected", received_version: "0.9.5" }, null, versions(null));
  expect(document.querySelector("[data-path-herdr]")).toBeNull();
  await none.unmount();
});
