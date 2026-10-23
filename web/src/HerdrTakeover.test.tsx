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
import type { UpdateStatus } from "./host";
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
  path_herdr: null,
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

const host = { quit: vi.fn(), revealPath: vi.fn(), check: vi.fn(), restart: vi.fn() };
// The page subscribes to the host's update status once for its life, so every stand-in hands its listener here.
let updateStatus: ((status: UpdateStatus) => void) | null = null;
const desktop = (restartHerdr = vi.fn(async () => "restarted")) => {
  window.hideHost = {
    kind: "electron", platform: "darwin", restartHerdr, quit: host.quit, revealPath: host.revealPath, notificationPermission: async () => "unsupported",
    updates: { check: host.check, restart: host.restart, onStatus: (listener: (status: UpdateStatus) => void) => { updateStatus = listener; return () => {}; } },
  } as never;
  return restartHerdr;
};
const phase = (next: UpdateStatus["phase"]) => act(async () => updateStatus?.({ phase: next, prerelease: false, installOnQuit: true }));

afterEach(() => {
  vi.clearAllMocks();
  useSetupWalk.getState().close();
  delete window.hideHost;
  document.body.innerHTML = "";
});

const enter = () => act(async () => { document.querySelector("[data-first-run]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true })); });
const text = () => document.body.textContent ?? "";

it("opens the first run on the move Hide makes on its own: a title, the two versions, two lines, no button and no count (B1, B2, B3)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "moving" })));
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("herdr");
  expect(document.querySelector("[data-first-run-progress]")?.getAttribute("data-first-run-progress")).toBe("1");
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("Moving Herdr to 0.9.5");
  expect(document.querySelector("[data-herdr-takeover-busy]")).not.toBeNull();
  expect(document.querySelector("[data-slot=dialog-description]")).toBeNull();
  expect(document.querySelector("[data-herdr-takeover-versions]")?.getAttribute("data-herdr-takeover-versions")).toBe("0.8.2->0.9.5");
  expect(text()).toContain("✓Tabs and agents keep running");
  expect(text()).toContain("!A Herdr screen open in a terminal disconnects once");
  expect(text()).not.toMatch(/\d+ (workspaces|tabs|agents)/);
  expect(document.querySelectorAll("[data-herdr-takeover-move], [data-herdr-takeover-later]")).toHaveLength(0);
  await enter();
  expect(sent(events)).toEqual([]);
  await unmount();
});

it("a move Hide does not make on its own again waits on its Move, which ⌘↵ presses too", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer()));
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("Move Herdr to 0.9.5");
  expect(button("[data-herdr-takeover-later]")).toBeNull();
  await act(async () => button("[data-herdr-takeover-move]")!.click());
  await enter();
  expect(sent(events)).toEqual([{ action: "move" }, { action: "move" }]);
  await unmount();
});

it("a failed move says what came of it in one line, offers Try again, and only then Not now (B8, B9)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ stage: "failed", failure: "refused" })));
  expect(document.querySelector("[data-herdr-takeover-failure]")?.textContent).toBe("Herdr declined. Nothing changed.");
  expect(button("[data-herdr-takeover-move]")!.textContent).toContain("Try again");
  expect(button("[data-herdr-takeover-restart]")).toBeNull();
  await act(async () => button("[data-herdr-takeover-later]")!.click());
  expect(sent(events)).toEqual([{ action: "later" }]);
  await unmount();
});

it("a server Hide reads as it is can be left from the start (B6)", async () => {
  desktop();
  const { unmount } = await mount(state(offer({ server_version: "0.9.1", same_protocol: true }), "herdr", "connected"));
  expect(button("[data-herdr-takeover-later]")).not.toBeNull();
  await unmount();
});

it("a moved server's result says the work keeps running and how the terminal's herdr was brought up, and Continue closes it (B4, B5)", async () => {
  desktop();
  const moved = (path: HerdrTakeover["path_herdr"]) => state(offer({ stage: "moved", path_herdr: path }), "herdr", "connected");
  const { events, unmount } = await mount(moved(null));
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("Moved");
  expect(document.querySelector("[data-slot=dialog-description]")?.textContent).toBe("Your work keeps running.");
  expect(document.querySelector("[data-herdr-takeover-path]")).toBeNull();

  await act(async () => { useShellStore.setState(moved({ stage: "updating", version: "0.8.2" }) as never); });
  expect(document.querySelector('[data-herdr-takeover-path="updating"]')?.textContent).toBe("Updating the herdr in your terminal");

  await act(async () => { useShellStore.setState(moved({ stage: "updated", version: "0.9.5" }) as never); });
  expect(document.querySelector('[data-herdr-takeover-path="updated"]')?.textContent).toBe("✓The herdr in your terminal is now 0.9.5 too");
  expect(button("[data-herdr-takeover-copy]")).toBeNull();

  await act(async () => { useShellStore.setState(moved({ stage: "behind", version: "0.8.2" }) as never); });
  const line = document.querySelector('[data-herdr-takeover-path="behind"]')!;
  expect(line.textContent).toContain("Update the herdr in your terminal (0.8.2) too. Until then its commands are refused.");
  expect(line.querySelector("[data-herdr-update-command]")?.getAttribute("data-herdr-update-command")).toBe("install -m 755 '/Applications/hide.app/Contents/Resources/herdr' '/srv/op/.local/bin/herdr'");
  expect(line.textContent).not.toContain("Replaces it with");
  expect(line.textContent).not.toContain("--handoff");
  await act(async () => button("[data-herdr-takeover-continue]")!.click());
  expect(sent(events)).toEqual([{ action: "continue" }]);
  await unmount();
});

it("a restart shows what ends, takes no Enter, and runs only on its own button (B10, B11, B12)", async () => {
  const restartHerdr = desktop(vi.fn(async () => "stop_failed"));
  const { events, unmount } = await mount(state(offer({ kind: "restart" })));
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("Restart Herdr");
  expect(document.querySelector("[data-slot=dialog-description]")).toBeNull();
  expect(document.querySelector("[data-herdr-takeover-versions]")?.textContent).toBe("Herdr 0.8.2NowHerdr 0.9.5After the restart");
  expect(text()).toContain("✓Tabs and folders come back");
  expect(text()).toContain("!Running commands end. Press after your builds finish");
  expect(button("[data-herdr-takeover-restart]")!.textContent).toBe("Restart");
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

it("a restart whose stop failed says so, keeps the restart, and only then offers Not now (B12, D-29)", async () => {
  desktop();
  const { events, unmount } = await mount(state(offer({ kind: "restart", stage: "failed", failure: "stop_failed" })));
  expect(document.querySelector('[data-herdr-takeover-failure="stop_failed"]')?.textContent).toBe("Herdr didn't stop. Nothing changed.");
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
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("Couldn't read Herdr");
  expect(document.querySelector("[data-slot=dialog-description]")?.textContent).toBe("Your Herdr is unchanged.");
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
  expect(text()).toContain("Running commands end");
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

it("a newer protocol holds Hide while the desktop app looks for a Hide update at once and opens it once downloaded (B18)", async () => {
  const newer = offer({ kind: "update_hide", server_version: "0.10.0" });
  const received = { version: "0.10.0", protocol: 23 };
  const title = () => document.querySelector("[data-slot=dialog-title]")?.textContent;
  const browser = await mount(state(newer, "done", "protocol_mismatch", received), "gate");
  expect(title()).toBe("Hide needs an update");
  expect(text()).toContain("✓Your Herdr and its work are unchanged");
  expect(button("[data-herdr-takeover-check-updates]")).toBeNull();
  await browser.unmount();

  desktop();
  const app = await mount(state(newer, "done", "protocol_mismatch", received), "gate");
  expect(host.check).toHaveBeenCalledTimes(1);
  await phase({ kind: "idle", checkedAt: 1 });
  expect(title()).toBe("Hide needs an update");
  expect(document.querySelector("[data-slot=dialog-description]")?.textContent).toBe("A Hide for Herdr 0.10.0 is coming soon.");
  await phase({ kind: "downloading", version: "0.4.3", percent: 40 });
  expect(title()).toBe("Downloading the Hide update");
  expect(document.querySelector("[data-slot=dialog-description]")).toBeNull();
  expect(host.restart).not.toHaveBeenCalled();
  await phase({ kind: "ready", version: "0.4.3" });
  expect(host.restart).toHaveBeenCalledTimes(1);
  // An install that failed keeps the update ready; the screen does not restart into it again.
  await phase({ kind: "ready", version: "0.4.3" });
  expect(host.restart).toHaveBeenCalledTimes(1);
  await act(async () => button("[data-herdr-takeover-check-updates]")!.click());
  expect(host.check).toHaveBeenCalledTimes(2);
  await act(async () => button("[data-herdr-gate-look]")!.click());
  expect(sent(app.events)).toEqual([{ action: "look" }]);
  await phase({ kind: "idle", checkedAt: null });
  await app.unmount();
});

it("a mismatch the core offers nothing for still holds Hide, with one line and Try again", async () => {
  const { events, unmount } = await mount(state(null, "done"), "gate");
  expect(document.querySelector("[data-herdr-gate]")?.getAttribute("data-herdr-gate")).toBe("older");
  expect(document.querySelector("[data-slot=dialog-title]")?.textContent).toBe("This Herdr isn't compatible with Hide");
  expect(document.querySelector("[data-slot=dialog-description]")?.textContent).toBe("When your work is done, stop Herdr and open Hide again.");
  await act(async () => button("[data-herdr-gate-look]")!.click());
  expect(sent(events)).toEqual([{ action: "look" }]);
  await unmount();
});

it("a server on Hide's protocol never holds Hide, whatever its release (D-16)", async () => {
  const { unmount } = await mount(state(offer({ server_version: "0.9.4", same_protocol: true }), "done", "connected", { version: "0.9.4", protocol: 22 }), "gate");
  expect(document.querySelector("[data-herdr-gate]")).toBeNull();
  await unmount();
});

it("a move made outside the wizard leaves a line only for a terminal herdr Hide could not bring up, until OK (B5)", async () => {
  const updating = await mount(state(offer({ stage: "moved", path_herdr: { stage: "updating", version: "0.8.2" } }), "done", "connected"), "gate");
  expect(document.querySelector("[data-herdr-takeover-moved]")).toBeNull();
  await updating.unmount();
  const behind = await mount(state(offer({ stage: "moved", path_herdr: { stage: "behind", version: "0.8.2" } }), "done", "connected"), "gate");
  expect(document.querySelector("[data-herdr-takeover-moved]")?.textContent).toContain("Update the herdr in your terminal (0.8.2) too.");
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
