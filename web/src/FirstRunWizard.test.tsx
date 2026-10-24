// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { FirstRunWizard, useSetupWalk } from "./FirstRunWizard";
import { TooltipProvider } from "./components/ui/tooltip";
import type { KitAgent } from "./snapshot";
import { useShellStore } from "./store";
import { useStartPanel } from "./startDraft";
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
  skill: { state: "off", why: null, location: null },
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
    navigator: { devices: [{ id: "local", kit: { agents, components: [{ id: "cli", label: "hide", state: "off", why: null, location: "/Users/me/.local/bin/hide" }], busy: kit.busy ?? false } }] },
    status: { background_ai: { provider: null, chosen: false, providers } },
  },
});

/** What the gate sent for the operator's choice: the sign-in probe it asks for while open is not part of it. */
const choices = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => !["ai_settings", "first_run_draft", "herdr_profiles_list"].includes(event.kind));

const tile = (id: string, on: string) => document.querySelector(`[data-onboarding-tile="${id}:${on}"]`) as HTMLElement | null;
const continueButton = () => document.querySelector("[data-onboarding-apply]") as HTMLButtonElement;

afterEach(() => {
  useSetupWalk.getState().close();
  delete window.hideHost;
  useStartPanel.getState().close();
  document.body.innerHTML = "";
});

it("reopens the agent step with the saved disabled choice after reload", async () => {
  const next = state(true, AGENTS);
  Object.assign(next.rest.ui_state, { first_run_choices: { agents: ["claude-code"] } });
  const { events, unmount } = await mount(next);
  expect(tile("claude-code", "on")).not.toBeNull();
  expect(tile("codex", "off")).not.toBeNull();
  await act(async () => continueButton().click());
  expect(choices(events).at(-1)?.payload).toEqual({ agents: ["claude-code"], finish: true });
  await unmount();
});

it("Back from notifications sends navigation alone and preserves an earlier skip despite granted permission", async () => {
  const permission = vi.fn(async () => "granted");
  const notify = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", notificationPermission: permission, notify } as never;
  const next = state(false, AGENTS);
  Object.assign(next.rest.ui_state, { first_run: { version: 1, step: "notifications" }, first_run_choices: { notifications_answered: true }, desktop_notifications: false });
  const { events, unmount } = await mount(next);
  await act(async () => (document.querySelector("[data-first-run-back]") as HTMLButtonElement).click());
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "first_run_back", payload: { from: "notifications" } }]);
  expect(notify).not.toHaveBeenCalled();
  expect(events.some((event) => event.kind === "desktop_notifications")).toBe(false);
  await unmount();
});

it("opens a remote project's first agent from that device's kit, regardless of local sign-in and installation", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const next = state(false, AGENTS, [provider("claude", "Claude Code", "claude-code", true)]);
  Object.assign(next.rest.ui_state, { first_run: { version: 1, step: "project" }, first_run_choices: { project_device: "build" }, workspace_registrations: [] });
  Object.assign(next.rest.navigator, { focused_device_id: "build" });
  next.rest.navigator.devices.push({ id: "build", kind: "remote", label: "Build server", kit: { agents: [agent("opencode", "OpenCode", "available"), agent("claude-code", "Claude Code", "not_installed")].map((row) => ({ ...row, enabled: true })), components: [], busy: false } } as never);
  const { events, unmount } = await mount(next);
  await act(async () => way("project").click());
  await act(async () => useShellStore.setState((current) => ({ revision: current.revision + 1, rest: { ...current.rest, ui_state: { ...current.rest?.ui_state, workspace_registrations: [{ id: "remote-project", label: "project", path: "~/project", device_id: "build" }] } } }) as never));
  expect(useStartPanel.getState().preset).toEqual({ projectId: "remote-project", kind: "opencode" });
  expect(events.find((event) => event.kind === "first_run_advance")?.payload).toEqual({ from: "project" });
  await unmount();
});

/** The last step on screen: a first run that reached it, with nothing added yet. */
const startStep = () => {
  const next = state(false, AGENTS);
  Object.assign(next.rest.navigator.devices[0]!, { kind: "local" });
  Object.assign(next.rest.ui_state, { first_run: { version: 1, step: "project" }, workspace_registrations: [] });
  return next;
};

const way = (id: "practice" | "project") => document.querySelector<HTMLButtonElement>(`[data-first-run-way="${id}"]`)!;
const navigation = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "first_run_advance").map((event) => event.payload);

it("asks how to start with two ways and Skip for now, and no choice to confirm", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", openPractice: vi.fn() } as never;
  const { unmount } = await mount(startStep());
  expect(way("practice")).not.toBeNull();
  expect(way("project")).not.toBeNull();
  expect(document.querySelector("[data-first-run-skip]")).not.toBeNull();
  // Each way is the button: no selected tile waits for a Continue.
  expect(document.querySelector("[data-first-run-next], [data-onboarding-apply], [role=radio]")).toBeNull();
  expect(document.querySelector("[data-add-project-browse]")).toBeNull();
  await unmount();
});

it("practice first ends the wizard and opens the practice window on chapter 0", async () => {
  const openPractice = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", openPractice } as never;
  const { events, unmount } = await mount(startStep());
  await act(async () => way("practice").click());
  expect(navigation(events)).toEqual([{ from: "project" }]);
  expect(openPractice).toHaveBeenCalledWith(expect.any(String), "overview");
  await unmount();
});

it("Add a project shows Add project's views, keeps them through a cancelled pick, and Back returns to the two ways", async () => {
  const pickFolder = vi.fn(async () => null);
  const openPractice = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder, openPractice } as never;
  const { events, unmount } = await mount(startStep());
  await act(async () => way("project").click());
  expect(events.filter((event) => event.kind === "first_run_draft").at(-1)?.payload).toMatchObject({ project_form: { view: "add" } });
  await act(async () => document.querySelector<HTMLButtonElement>("[data-add-project-browse]")!.click());
  expect(pickFolder).toHaveBeenCalledOnce();
  expect(document.querySelector("[data-add-project-browse]")).not.toBeNull();
  expect(navigation(events)).toEqual([]);
  await act(async () => document.querySelector<HTMLButtonElement>("[data-first-run-back]")!.click());
  expect(way("project")).not.toBeNull();
  expect(events.filter((event) => event.kind === "first_run_draft").at(-1)?.payload).toMatchObject({ project_form: { view: "" } });
  // Back inside the step is not the wizard's Back to notifications.
  expect(events.some((event) => event.kind === "first_run_back")).toBe(false);
  expect(openPractice).not.toHaveBeenCalled();
  await unmount();
});

it("Skip for now ends the wizard on the empty Home without opening the practice window", async () => {
  const openPractice = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", openPractice } as never;
  const { events, unmount } = await mount(startStep());
  await act(async () => document.querySelector<HTMLButtonElement>("[data-first-run-skip]")!.click());
  expect(navigation(events)).toEqual([{ from: "project" }]);
  expect(openPractice).not.toHaveBeenCalled();
  await unmount();
});

it("keeps a credentialed clone URL in the open form and sends it only for explicit Clone", async () => {
  window.hideHost = { kind: "electron", platform: "darwin", pickFolder: async () => null } as never;
  const next = state(false, AGENTS);
  Object.assign(next.rest.navigator.devices[0]!, { kind: "local" });
  Object.assign(next.rest.ui_state, {
    first_run: { version: 1, step: "project" },
    first_run_choices: { project_form: { view: "clone", url: "https://example.test/repo.git", parent: "~/work" } },
  });
  Object.assign(next, { cloneTarget: { parent: "~/work", name: "repo", path: "~/work/repo", reason: null } });
  const { events, unmount } = await mount(next);
  const input = document.querySelector<HTMLInputElement>("[data-clone-url]")!;
  const raw = "https://alice:private-token@example.test/repo.git";
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, raw);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(input.value).toBe(raw);
  const drafts = events.filter((event) => event.kind === "first_run_draft");
  expect(drafts.at(-1)?.payload).toMatchObject({ project_form: { url: "" } });
  expect(JSON.stringify(drafts)).not.toContain("private-token");
  expect(events.some((event) => event.kind === "clone_repository")).toBe(false);
  await act(async () => document.querySelector<HTMLButtonElement>("[data-clone-submit]")!.click());
  expect(events.find((event) => event.kind === "clone_repository")?.payload).toEqual({ url: raw, parent: "~/work", name: "repo" });
  await unmount();
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

const AGENTS = [agent("claude-code", "Claude Code", "available"), agent("antigravity", "Antigravity", "not_installed"), agent("codex", "Codex", "available")];

it("shows every agent as a tile, the set-up ones on and the others dimmed with an install link, and continues with what is left on", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  expect(tile("claude-code", "on")).not.toBeNull();
  expect(tile("codex", "on")).not.toBeNull();
  expect(tile("antigravity", "unavailable")).not.toBeNull();
  expect(tile("antigravity", "unavailable")?.getAttribute("role")).toBeNull();
  // B9: the official install page of an agent that is not installed.
  expect(document.querySelector('[data-onboarding-install="antigravity"]')?.getAttribute("href")).toMatch(/^https:\/\//);
  // A logo or a monogram, never nothing: Antigravity has a bundled mark, an agent without one a monogram.
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
  const failed = AGENTS.map((entry) => (entry.id === "codex" ? { ...entry, enabled: true, skill: { state: "failed" as const, why: { cause: "not_changed" }, location: "/x" } } : { ...entry, enabled: entry.availability === "available" }));
  await act(async () => { useShellStore.setState(state(true, failed, [], { busy: false }, 3) as never); });
  expect(continueButton().disabled).toBe(false);
  expect(document.querySelector('[data-first-run-failed="codex"]')?.textContent).toContain("Codex was not set up: Hide could not install or change it.");
  expect(continueButton().textContent).toBe("Try again");
  expect(choices(events).filter((event) => event.kind === "agent_onboarding_apply")).toHaveLength(1);
  await unmount();
});

it("lists under What changes where Hide writes for each agent left on, and the hide command's link (B11)", async () => {
  const located = AGENTS.map((entry) => ({ ...entry, skill: { state: "off" as const, why: null, location: `/Users/me/.${entry.id}/skills/hide` } }));
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
  (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "unreadable";
  const { events, unmount } = await mount(next);
  await act(async () => { useSetupWalk.getState().open(); });
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("Hide could not read its kit on this device.");
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

it("draws what Continue sends: an explicit choice survives availability refreshes without adding another agent", async () => {
  const { events, unmount } = await mount(state(true, AGENTS));
  await act(async () => { tile("codex", "on")!.click(); });
  // A later poll finds another agent installed after the explicit choice.
  await act(async () => {
    useShellStore.setState(state(true, AGENTS.map((entry) => (entry.id === "antigravity" ? { ...entry, availability: "available" as const } : entry))) as never);
  });
  expect(tile("antigravity", "off")).not.toBeNull();
  expect(tile("codex", "off")).not.toBeNull();
  await act(async () => { continueButton().click(); });
  expect(choices(events)).toEqual([{ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents: ["claude-code"], finish: true } }]);
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
    (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "unreadable";
    useShellStore.setState({ ...next, revision: 2 } as never);
  });
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("Could not set up agents on this device: Hide could not read its kit on this device.");
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
  (next.rest.navigator.devices[0]!.kit as { unavailable?: string }).unavailable = "standalone";
  const { unmount } = await mount(next);
  expect(document.querySelector("[data-first-run-unavailable]")?.textContent).toContain("This Hide is not running from the installed app");
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
