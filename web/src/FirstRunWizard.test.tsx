// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { FirstRunWizard, useSetupWalk } from "./FirstRunWizard";
import { TooltipProvider } from "./components/ui/tooltip";
import type { KitAgent } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const agent = (id: string, label: string, availability: KitAgent["availability"]): KitAgent => ({
  id,
  label,
  availability,
  enabled: false,
  chosen: false,
  skill: { state: "off", reason: null, location: null },
  hook: null,
  doc_url: "https://example.test",
});

const provider = (id: string, label: string, agentId: string, ready: boolean) => ({ id, label, agent: agentId, state: ready ? "ready" : "needs_login", selectable: ready });

/** `pending`: a first run on its agent step; otherwise an install whose wizard ended. */
const state = (pending: boolean, agents: KitAgent[], providers: ReturnType<typeof provider>[] = [], kit: { busy?: boolean } = {}, revision = 1) => ({
  connection: "live" as const,
  revision,
  rest: {
    ui_state: { agent_onboarding: pending ? "pending" : "done", first_run: { version: 1, step: pending ? "agents" : "done" } },
    navigator: { devices: [{ id: "local", kit: { agents, components: [{ id: "cli", label: "hide", state: "off", reason: null, location: "/Users/me/.local/bin/hide" }], busy: kit.busy ?? false } }] },
    status: { background_ai: { provider: null, chosen: false, providers } },
  },
});

/** What the gate sent for the operator's choice: the sign-in probe it asks for while open is not part of it. */
const choices = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind !== "ai_settings");

const tile = (id: string, on: string) => document.querySelector(`[data-onboarding-tile="${id}:${on}"]`) as HTMLElement | null;
const continueButton = () => document.querySelector("[data-onboarding-apply]") as HTMLButtonElement;

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(next: ReturnType<typeof state>) {
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
    root.render(<TooltipProvider><FirstRunWizard actions={actions} /></TooltipProvider>);
  });
  return { events, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const AGENTS = [agent("claude-code", "Claude Code", "available"), agent("cursor", "Cursor", "not_installed"), agent("codex", "Codex", "available")];

it("shows every agent as a tile, the set-up ones on and the others dimmed with an install link, and continues with what is left on", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  expect(tile("claude-code", "on")).not.toBeNull();
  expect(tile("codex", "on")).not.toBeNull();
  expect(tile("cursor", "unavailable")).not.toBeNull();
  expect(tile("cursor", "unavailable")?.getAttribute("role")).toBeNull();
  // B9: the official install page of an agent that is not installed.
  expect(document.querySelector('[data-onboarding-install="cursor"]')?.getAttribute("href")).toMatch(/^https:\/\//);
  // A logo or a monogram, never nothing: Cursor has a bundled mark, an agent without one a monogram.
  expect(document.querySelectorAll("[data-agent-logo], [data-agent-monogram]").length).toBe(3);

  await act(async () => { tile("codex", "on")!.click(); });
  expect(tile("codex", "off")?.getAttribute("aria-checked")).toBe("false");
  await act(async () => { continueButton().click(); });

  // A browser tab's wizard is this step alone, so Continue ends it there (B26).
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: ["claude-code"], finish: true } }]);
  await unmount();
});

it("holds Continue while its pass runs, and names a failed agent with its reason after it (B10, B12)", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  await act(async () => { continueButton().click(); });
  // Held until the snapshot that takes the press, then while the kit is busy.
  expect(continueButton().disabled).toBe(true);
  await act(async () => { continueButton().click(); });
  await act(async () => { useShellStore.setState(state(true, AGENTS, [], { busy: true }, 2) as never); });
  expect(continueButton().disabled).toBe(true);
  const failed = AGENTS.map((entry) => (entry.id === "codex" ? { ...entry, enabled: true, skill: { state: "failed" as const, reason: "permission denied", location: "/x" } } : { ...entry, enabled: entry.availability === "available" }));
  await act(async () => { useShellStore.setState(state(true, failed, [], { busy: false }, 3) as never); });
  expect(continueButton().disabled).toBe(false);
  expect(document.querySelector('[data-first-run-failed="codex"]')?.textContent).toContain("Codex was not set up: permission denied");
  expect(continueButton().textContent).toBe("Try again");
  expect(choices(events).filter((event) => event.kind === "agent_onboarding_apply")).toHaveLength(1);
  await unmount();
});

it("lists under What changes where Hide writes for each agent left on, and the hide command's link (B11)", async () => {
  const located = AGENTS.map((entry) => ({ ...entry, skill: { state: "off" as const, reason: null, location: `/Users/me/.${entry.id}/skills/hide` } }));
  const { unmount } = await mount(state(true, located));
  await act(async () => { (document.querySelector("[data-first-run-what-changes]") as HTMLElement).click(); });
  const rows = () => [...document.querySelectorAll("[data-first-run-change]")].map((row) => row.getAttribute("data-first-run-change"));
  expect(rows()).toEqual(["/Users/me/.claude-code/skills/hide", "/Users/me/.codex/skills/hide", "cli"]);
  expect(document.querySelector('[data-first-run-change="/Users/me/.codex/skills/hide"]')?.textContent).toContain("Codex · Skill");
  await act(async () => { tile("codex", "on")!.click(); });
  expect(rows()).toEqual(["/Users/me/.claude-code/skills/hide", "cli"]);
  await unmount();
});

it("reopened from Help, starts from the agents switched on now, closes with Escape, and sends only a Continue (B45, B46)", async () => {
  const enabled = AGENTS.map((entry) => ({ ...entry, enabled: entry.id === "codex" }));
  const { events, unmount } = await mount(state(false, enabled));
  expect(document.querySelector("[data-first-run]")).toBeNull();
  await act(async () => { useSetupWalk.getState().open(); });
  expect(document.querySelector("[data-first-run-mode]")?.getAttribute("data-first-run-mode")).toBe("setup");
  expect(tile("codex", "on")).not.toBeNull();
  expect(tile("claude-code", "off")).not.toBeNull();
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(document.querySelector("[data-first-run]")).toBeNull();
  expect(choices(events)).toEqual([]);

  await act(async () => { useSetupWalk.getState().open(); });
  await act(async () => { continueButton().click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: ["codex"], finish: false } }]);
  expect(useSetupWalk.getState().step).toBe("notifications");
  await act(async () => { useSetupWalk.getState().close(); });
  await unmount();
});

it("reopened from Help while the kit cannot run, stays on its first step with the reason and only has the agents read again (B12, B44)", async () => {
  const next = state(false, []);
  (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "timed out";
  const { events, unmount } = await mount(next);
  await act(async () => { useSetupWalk.getState().open(); });
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("timed out");
  await act(async () => { continueButton().click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: [], finish: false } }]);
  expect(useSetupWalk.getState().step).toBe("agents");
  expect(document.querySelector("[data-first-run]")?.getAttribute("data-first-run")).toBe("agents");
  await act(async () => { useSetupWalk.getState().close(); });
  await unmount();
});

it("has Continue as its only way out: Escape and a second button do nothing (the outside click is the e2e's), and it is not shown once decided", async () => {
  const shown = await mount(state(true, AGENTS));
  expect(document.querySelector("[data-onboarding-later]")).toBeNull();
  expect(document.querySelectorAll("[data-first-run] button:not([role=switch]):not([data-first-run-what-changes])").length).toBe(1);
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  expect(document.querySelector("[data-first-run]")).not.toBeNull();
  expect(choices(shown.events)).toEqual([]);
  await shown.unmount();

  const decided = await mount(state(false, AGENTS));
  expect(document.querySelector("[data-first-run]")).toBeNull();
  await decided.unmount();

  const empty = await mount(state(true, []));
  expect(document.querySelector("[data-first-run]")).toBeNull();
  await empty.unmount();
});

it("draws what Continue sends: an agent that becomes available while it is open shows on, and the operator's flips survive the refresh", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  await act(async () => { tile("codex", "on")!.click(); });
  // A later poll finds Cursor set up on the machine.
  await act(async () => {
    useShellStore.setState(state(true, AGENTS.map((entry) => (entry.id === "cursor" ? { ...entry, availability: "available" as const } : entry))) as never);
  });
  expect(tile("cursor", "on")).not.toBeNull();
  expect(tile("codex", "off")).not.toBeNull();
  await act(async () => { continueButton().click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: ["claude-code", "cursor"], finish: true } }]);
  await unmount();
});

it("names under Continue the agent Hide AI will use, and says on each checked tile whether it is signed in (B8, B13)", async () => {
  const providers = [provider("claude", "Claude Code", "claude-code", true), provider("codex", "Codex", "codex", true)];
  const { unmount } = await mount(state(true, AGENTS, providers));
  expect(tile("claude-code", "on")?.querySelector("[data-onboarding-status]")?.getAttribute("data-onboarding-status")).toBe("ready");
  const line = () => document.querySelector("[data-onboarding-hide-ai]")?.textContent ?? null;
  expect(line()).toBe("Hide AI uses Claude Code");
  await act(async () => { tile("claude-code", "on")!.click(); });
  expect(line()).toBe("Hide AI uses Codex");
  await act(async () => { tile("codex", "on")!.click(); });
  expect(line()).toBeNull();
  await unmount();

  // A switched-on agent that is not signed in is passed over; with nobody signed in there is no line.
  const signedOut = await mount(state(true, AGENTS, [provider("claude", "Claude Code", "claude-code", false), provider("codex", "Codex", "codex", false)]));
  expect(document.querySelector("[data-onboarding-hide-ai]")).toBeNull();
  expect(tile("codex", "on")?.querySelector("[data-onboarding-status]")?.textContent).toContain("Not signed in");
  await signedOut.unmount();
});

it("asks for the sign-in probe while it is open and stops when it goes away", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  expect(events.filter((event) => event.kind === "ai_settings")).toEqual([{ schema_version: 2, kind: "ai_settings", payload: { observing: true } }]);
  await unmount();
  expect(events.filter((event) => event.kind === "ai_settings").at(-1)).toEqual({ schema_version: 2, kind: "ai_settings", payload: { observing: false } });
});

it("says why when its pass could not run here, and Try again has the agents read again before the choice is sent", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  await act(async () => { continueButton().click(); });
  // The kit link failed: the core keeps the step, its row is unavailable and lists no agent.
  await act(async () => {
    const next = state(true, []);
    (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "timed out";
    useShellStore.setState({ ...next, revision: 2 } as never);
  });
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("Hide could not set up agents on this Mac: timed out");
  // An unread kit is not a Mac with no agent installed.
  expect(document.querySelector("[data-first-run-no-agent]")).toBeNull();
  expect(continueButton().textContent).toContain("Try again");
  const before = choices(events).length;
  await act(async () => { continueButton().click(); });
  expect(choices(events).slice(before)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: [], finish: true } }]);
  await act(async () => { useShellStore.setState(state(true, AGENTS, [], {}, 4) as never); });
  expect(continueButton().disabled).toBe(false);
  await unmount();
});

it("says why the kit could not run here before anything is pressed, as after a relaunch", async () => {
  const next = state(true, []);
  (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "timed out";
  const { unmount } = await mount(next);
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("timed out");
  expect(document.querySelector("[data-first-run-no-agent]")).toBeNull();
  expect(continueButton().disabled).toBe(false);
  await unmount();
});

it("opens on a first pass that stopped before changing anything, says what stopped it with the variable and both ways back, and Try again sends no choice", async () => {
  const next = state(true, []);
  next.rest.ui_state.agent_onboarding = null as never;
  Object.assign(next.rest.navigator.devices[0]!.kit, { components: [], refused: { cause: "unsafe", path: "D:\\", variable: "HIDE_STATE_DIR" } });
  const { events, unmount } = await mount(next);
  const line = document.querySelector('[data-first-run-refused="unsafe"]')?.textContent ?? "";
  expect(line).toContain("D:\\");
  expect(line).toContain("HIDE_STATE_DIR");
  expect(line).toContain("~/.hide/state");
  expect(line).not.toMatch(/retire|preflight|kit/i);
  expect(document.querySelector("[data-first-run-no-agent]")).toBeNull();
  expect(continueButton().textContent).toContain("Try again");
  const before = choices(events).length;
  await act(async () => { continueButton().click(); });
  expect(choices(events).slice(before)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: [], finish: true } }]);
  // While the pass runs again the step holds its button; a pass that runs brings the tiles back.
  await act(async () => {
    const running = state(true, [], [], { busy: true }, 2);
    Object.assign(running.rest.navigator.devices[0]!.kit, { components: [], refused: { cause: "unsafe", path: "D:\\", variable: "HIDE_STATE_DIR" } });
    useShellStore.setState(running as never);
  });
  expect(continueButton().disabled).toBe(true);
  expect(document.querySelector("[data-first-run-refused]")).toBeNull();
  await act(async () => { useShellStore.setState(state(true, AGENTS, [], {}, 3) as never); });
  expect(document.querySelector("[data-first-run-refused]")).toBeNull();
  expect(tile("claude-code", "on")).not.toBeNull();
  await unmount();
});

it("lets a refused first step be left with Not now, which closes it with no choice, and names this Mac's command that stops its background service", async () => {
  const next = state(true, []);
  next.rest.ui_state.agent_onboarding = null as never;
  const stop = "HIDE_STATE_DIR='/Volumes/Team/hide' '/Applications/hide.app/Contents/Resources/hide' stop";
  Object.assign(next.rest.navigator.devices[0]!.kit, { components: [], refused: { cause: "unsafe", path: "/Volumes/Team", variable: "HIDE_STATE_DIR", stop } });
  const { events, unmount } = await mount(next);
  const line = document.querySelector('[data-first-run-refused="unsafe"]')?.textContent ?? "";
  expect(line).toContain("quit Hide, run this in a terminal, and open Hide again: " + stop);
  const before = choices(events).length;
  await act(async () => { (document.querySelector("[data-first-run-refused-close]") as HTMLButtonElement).click(); });
  expect(choices(events).slice(before)).toEqual([{ schema_version: 2, kind: "first_run_advance", payload: { from: "agents" } }]);
  await unmount();
});

it("on a Mac that already allows Hide, opens the notification step on, and its Continue turns the switch on with the step (B16)", async () => {
  const notify = vi.fn();
  vi.stubGlobal("hideHost", { kind: "electron", platform: "darwin", notify, notificationPermission: async () => "granted", onCommand: () => () => undefined, onNotificationOpen: () => () => undefined });
  const next = state(true, AGENTS);
  next.rest.ui_state.first_run.step = "notifications";
  const { events, unmount } = await mount(next);
  await act(async () => { await Promise.resolve(); });
  expect(document.querySelector("[data-notification-line]")?.getAttribute("data-notification-line")).toBe("on");
  expect(document.querySelector("[data-notification-turn-on]")).toBeNull();
  const button = document.querySelector('[data-first-run-next="notifications"]') as HTMLButtonElement;
  expect(button.textContent).toBe("Continue");
  await act(async () => { button.click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "first_run_advance", payload: { from: "notifications", notifications: true } }]);
  expect(notify).not.toHaveBeenCalled();
  await unmount();
  vi.unstubAllGlobals();
});

it("on a Mac that blocks Hide with the switch off, Enter presses Turn on, the step's main button (B40)", async () => {
  const notify = vi.fn();
  vi.stubGlobal("hideHost", { kind: "electron", platform: "darwin", notify, notificationPermission: async () => "denied", onCommand: () => () => undefined, onNotificationOpen: () => () => undefined });
  const next = state(true, AGENTS);
  next.rest.ui_state.first_run.step = "notifications";
  const { events, unmount } = await mount(next);
  await act(async () => { await Promise.resolve(); });
  expect(document.querySelector("[data-notification-line]")?.getAttribute("data-notification-line")).toBe("blocked");
  const dialog = document.querySelector("[data-first-run]") as HTMLElement;
  await act(async () => { dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "desktop_notifications_set", payload: { enabled: true } }]);
  expect(notify).toHaveBeenCalledTimes(1);
  await unmount();
  vi.unstubAllGlobals();
});

it("Not now while macOS still asks after Turn on leaves the switch off with the step (B21)", async () => {
  vi.stubGlobal("hideHost", { kind: "electron", platform: "darwin", notify: vi.fn(), notificationPermission: async () => "not_determined", onCommand: () => () => undefined, onNotificationOpen: () => () => undefined });
  const next = state(true, AGENTS);
  next.rest.ui_state.first_run.step = "notifications";
  (next.rest.ui_state as { desktop_notifications?: boolean }).desktop_notifications = true;
  const { events, unmount } = await mount(next);
  await act(async () => { await Promise.resolve(); });
  expect(document.querySelector("[data-notification-line]")?.getAttribute("data-notification-line")).toBe("asking");
  const button = document.querySelector('[data-first-run-next="notifications"]') as HTMLButtonElement;
  expect(button.textContent).toBe("Not now");
  await act(async () => { button.click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "first_run_advance", payload: { from: "notifications", notifications: false } }]);
  await unmount();
  vi.unstubAllGlobals();
});

it("on a desktop with no notification service, says so with Continue and no Turn on (B47)", async () => {
  vi.stubGlobal("hideHost", { kind: "electron", platform: "linux", notify: vi.fn(), notificationPermission: async () => "no_service", onCommand: () => () => undefined, onNotificationOpen: () => () => undefined });
  const next = state(true, AGENTS);
  next.rest.ui_state.first_run.step = "notifications";
  const { events, unmount } = await mount(next);
  await act(async () => { await Promise.resolve(); });
  expect(document.querySelector("[data-notification-line=\"no_service\"]")?.textContent).toContain("This desktop has no notification service, so Hide can't show notifications here.");
  expect(document.querySelector("[data-notification-turn-on]")).toBeNull();
  expect(document.querySelector("[data-notification-test]")).toBeNull();
  const button = document.querySelector('[data-first-run-next="notifications"]') as HTMLButtonElement;
  expect(button.textContent).toBe("Continue");
  await act(async () => { button.click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "first_run_advance", payload: { from: "notifications" } }]);
  await unmount();
  vi.unstubAllGlobals();
});

it("shows an agent installed on a system Hide cannot attach to yet as a dimmed tile with no switch, no install link and nothing under What changes (B49)", async () => {
  const agents = [...AGENTS, agent("opencode", "OpenCode", "unsupported_system")];
  const { unmount } = await mount(state(true, agents));
  const tileEl = tile("opencode", "unavailable");
  expect(tileEl?.getAttribute("role")).toBeNull();
  expect(tileEl?.querySelector("[data-onboarding-status]")?.textContent).toMatch(/^Not on .+ yet$/);
  expect(document.querySelector('[data-onboarding-install="opencode"]')).toBeNull();
  await act(async () => { (document.querySelector("[data-first-run-what-changes]") as HTMLElement).click(); });
  expect(document.querySelector("[data-first-run-changes]")?.textContent).not.toContain("OpenCode");
  await unmount();
});
