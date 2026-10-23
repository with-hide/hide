import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import type { Device, DeviceHost, Kit, KitComponent } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { DispatchFn } from "../ws";
import { DevicesTab } from "./DevicesTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const part = (id: KitComponent["id"], state: KitComponent["state"], over: Partial<KitComponent> = {}): KitComponent => ({ id, label: id === "codex_hook" ? "Codex hook" : id === "claude_code_hook" ? "Claude Code hook" : "hide command", state, why: null, location: "~/somewhere", ...over });

const kit = (components: KitComponent[], over: Partial<Kit> = {}): Kit => ({ unavailable: null, busy: false, components, agents: [], offers_reinstall: components.some((c) => ["outdated", "not_installed", "removed", "failed"].includes(c.state)), shares_account_with: null, ...over });

const host: DeviceHost = { consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: "SHA256:abc", granted_at_unix_ms: 1, state: "ready", message: null, platform: "macos aarch64", helper_path: "~/.hide/host-helper/current/hided" };

const remote = (id: string, over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id, label: id === "mini" ? "Mac mini" : "Studio", kind: "remote", state: "ready", message: null, ssh_alias: id, agent_count: 0, test: null, host, kit: kit([part("cli", "installed"), part("claude_code_hook", "installed"), part("codex_hook", "installed")]), ...over });

const state = (devices: Device[]) => ({
  connection: "live" as const,
  rest: {
    navigator: { focused_device_id: "local", devices: [{ agent_scope: emptyScope(), id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, kit: kit([part("cli", "installed")]) }, ...devices] },
    status: { remote: devices.map((device) => ({ target_id: device.id, state: "connected", message: null, herdr_version: "0.9.1" })) },
    ui_state: {},
  },
});

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
    root.render(<TooltipProvider><DevicesTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  return { events, q, row: (id: string) => q(`[data-device-row="${id}"]`), unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

it("says nothing about a healthy kit on a device row, and names the device by alias, platform and Herdr version (B54)", async () => {
  const { row, q, unmount } = await mount(state([remote("mini")]));
  const mini = row("mini")!;
  expect(q('[data-device-subtitle="mini"]')?.textContent).toBe("mini · macos aarch64 · Herdr 0.9.1");
  expect(mini.querySelector("[data-machine-kit], [data-kit-part], [data-kit-problem], [data-kit-reinstall], [data-device-host]")).toBeNull();
  // The old rows named the hide command, the hook files and the helper: none of it is on the row.
  expect(mini.textContent).not.toMatch(/hide command|Claude Code hook|Codex hook|installs to|bound to/);
  // Test and the menu are the only controls a healthy device row carries.
  expect(mini.querySelectorAll("button")).toHaveLength(2);
  expect(q('[data-device-test="mini"]')).not.toBeNull();
  expect(q('[data-device-menu="mini"]')?.getAttribute("aria-label")).toBe("More actions for Mac mini");
  await unmount();
});

it("grows one line under the device for a failed part, with Reinstall, and the line is gone once the kit is whole again (B54)", async () => {
  const broken = remote("mini", { kit: kit([part("cli", "installed"), part("codex_hook", "failed", { why: { cause: "unreadable", value: "~/.codex/hooks.json" } })]) });
  const first = await mount(state([broken]));
  const line = first.q('[data-kit-problem="mini"]')!;
  expect(line.textContent).toBe("✕Hide's kit: Codex hook: Failed: Hide could not read what is there: ~/.codex/hooks.json" + "Reinstall");
  await act(async () => { first.q('[data-kit-reinstall="mini"]')?.click(); });
  expect(sent(first.events, "kit_reinstall")).toHaveLength(1);
  await first.unmount();

  const whole = await mount(state([remote("mini")]));
  expect(whole.q('[data-kit-problem="mini"]')).toBeNull();
  expect(whole.q('[data-kit-reinstall="mini"]')).toBeNull();
  await whole.unmount();
});

it("says a pass that stopped before changing anything once, naming the folder and what to change, and its Retry runs the pass again", async () => {
  const refused = remote("mini", { kit: kit([], { refused: { cause: "unsafe", path: "/srv/shared", variable: null } }) });
  const view = await mount(state([refused]));
  const line = view.q('[data-kit-refused="mini"]')!;
  expect(line.textContent).toBe("✕Hide stopped before changing anything. Another account can change this, or it is a link: /srv/shared. Make it a folder only you can change, then try again." + "Retry");
  expect(view.q('[data-kit-problem="mini"]')).toBeNull();
  await act(async () => { view.q('[data-kit-retry="mini"]')?.click(); });
  expect(sent(view.events, "kit_reinstall")).toEqual([{ schema_version: 2, kind: "kit_reinstall", payload: { device_id: "mini" } }]);
  await view.unmount();

  const running = await mount(state([remote("mini", { kit: kit([], { busy: true, refused: { cause: "busy", path: null, variable: null } }) })]));
  expect((running.q('[data-kit-retry="mini"]') as HTMLButtonElement).disabled).toBe(true);
  await running.unmount();
});

it("says a refusal it cannot name as stopped, with Retry, never an empty line; too many projects is its own line", async () => {
  const future = await mount(state([remote("mini", { kit: kit([], { refused: { cause: "a_cause_from_a_newer_build", path: null, variable: null } as never }) })]));
  expect(future.q('[data-kit-refused="mini"]')!.textContent).toBe("✕Hide stopped before changing anything, for a reason this version of Hide can't name. Update Hide, then try again." + "Retry");
  await future.unmount();

  const projects = await mount(state([remote("mini", { kit: kit([], { refused: { cause: "projects", path: null, variable: null } }) })]));
  expect(projects.q('[data-kit-refused="mini"]')!.textContent).toContain("More projects are registered than it can check.");
  await projects.unmount();
});

it("says in one line when a device's Herdr stays older than Hide's, with the step its cause allows, and nothing once it moved or while it is out of reach (PRD device-herdr-update B3, B8)", async () => {
  const behind = (cause: "cannot_hand_off" | "handoff_failed") => remote("mini", { herdr_behind: { server_version: "0.9.1", version: "0.9.3", cause } });
  const failed = await mount(state([behind("handoff_failed")]));
  const line = failed.q('[data-device-herdr-behind="mini:handoff_failed"]')!;
  expect(line.textContent).toContain("Herdr 0.9.1 on this device is older than Hide's 0.9.3");
  expect(line.textContent).toContain("restart Herdr on this device");
  expect(line.textContent).not.toMatch(/reopen Hide/i);
  await failed.unmount();

  const old = await mount(state([behind("cannot_hand_off")]));
  expect(old.q('[data-device-herdr-behind="mini:cannot_hand_off"]')?.textContent).toContain("Update Herdr on this device.");
  await old.unmount();

  const away = await mount(state([remote("mini", { state: "unavailable", herdr_behind: { server_version: "0.9.1", version: "0.9.3", cause: "handoff_failed" } })]));
  expect(away.q("[data-device-herdr-behind]")).toBeNull();
  await away.unmount();

  const moved = await mount(state([remote("mini", { herdr_behind: null })]));
  expect(moved.q("[data-device-herdr-behind]")).toBeNull();
  await moved.unmount();
});

it("does not call a part that is not on the machine, or one the operator switched off, a problem, and never repeats its reason (B56)", async () => {
  const quiet = remote("mini", { kit: kit([part("cli", "installed"), part("codex_hook", "absent", { why: { cause: "agent_absent" } }), part("claude_code_hook", "off")]) });
  const { row, unmount } = await mount(state([quiet]));
  expect(row("mini")!.textContent).not.toContain("Not on this machine");
  expect(row("mini")!.textContent).not.toContain("Coordination");
  await unmount();
});

it("shows a reinstall under way, and the first install, as one pending line", async () => {
  const reinstalling = await mount(state([remote("mini", { kit: kit([part("claude_code_hook", "removed")], { busy: true }) })]));
  expect(reinstalling.q('[data-kit-reinstall="mini"]')?.hasAttribute("disabled")).toBe(true);
  expect(reinstalling.q('[data-kit-reinstall="mini"]')?.textContent).toBe("Reinstalling…");
  await reinstalling.unmount();

  const first = await mount(state([remote("mini", { kit: kit([], { busy: true }) })]));
  expect(first.q('[data-machine-kit="mini:busy"]')?.textContent).toContain("Installing");
  await first.unmount();
});

it("shows the helper only when it needs the operator while the connection is fine, with the actions that fix it (B54)", async () => {
  const stopped = remote("mini", { host: { ...host, state: "unavailable", message: "The helper did not answer." } });
  const unavailable = await mount(state([stopped]));
  expect(unavailable.q('[data-device-host="mini:unavailable"]')?.textContent).toContain("helper unavailable");
  await act(async () => { unavailable.q('[data-device-host-retry="mini"]')?.click(); });
  expect(sent(unavailable.events, "device_host_retry")).toHaveLength(1);
  await unavailable.unmount();

  const notAllowed = await mount(state([remote("studio", { host: { ...host, consent: "none", state: "not_allowed", helper_root: null } })]));
  expect(notAllowed.q('[data-device-host="studio:not_allowed"]')?.textContent).toContain("helper not allowed");
  expect(notAllowed.q('[data-device-host-allow-inline="studio"]')).not.toBeNull();
  await notAllowed.unmount();

  // A device that is not connected says so on its row and nothing about its helper.
  const down = await mount(state([remote("mini", { state: "unavailable", host: { ...host, state: "unavailable" } })]));
  expect(down.q('[data-device-host="mini:unavailable"]')).toBeNull();
  await down.unmount();
});

it("opens Add device from one button above the list, and shows the Add dialog only then (B50)", async () => {
  const { q, unmount } = await mount(state([remote("mini")]));
  expect(q("[data-add-device]")).toBeNull();
  await act(async () => { q("[data-device-add-open]")?.click(); });
  expect(document.body.querySelector("[data-add-device-dialog]")).not.toBeNull();
  await unmount();
});

/** Opens a row's ⋯ the way a keyboard does: Radix opens the menu on Enter at its trigger. */
async function openMenu(q: (selector: string) => HTMLElement | null, id: string) {
  await act(async () => {
    q(`[data-device-menu="${id}"]`)?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
}

const frame = (over: Record<string, unknown>) => ({ state: "idle", direction: "forward", device: "mini", intent: "i", sent: 0, total: 0, failed: [], step: null, cause: null, node: null, ...over });

it("crowns the core's machine and offers the core to a connected device it dials, dimmed with why otherwise (PRD core-host-node-move B2)", async () => {
  const next = state([remote("mini"), remote("studio")]);
  next.rest.status.remote[1]!.state = "stale";
  const { q, events, unmount } = await mount(next);
  expect(q('[data-device-core="local"]')?.textContent).toBe("core");
  expect(q('[data-device-core="mini"]')).toBeNull();
  await openMenu(q, "studio");
  const dimmed = document.querySelector('[data-device-move="studio"]') as HTMLElement;
  expect(dimmed.hasAttribute("data-disabled")).toBe(true);
  expect(dimmed.textContent).toBe("Move the core to this device…Not connected");
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  await openMenu(q, "mini");
  await act(async () => { (document.querySelector('[data-device-move="mini"]') as HTMLElement).click(); });
  expect(sent(events, "core_move").map((event) => event.payload)).toEqual([{ action: "check", device: "mini" }]);
  expect(document.querySelector("[data-core-move-dialog]")?.getAttribute("data-core-move-dialog")).toBe("checking");
  await unmount();
});

it("tells a window whose core is still taking a move that another move is under way (core_pending)", async () => {
  const { q, unmount } = await mount(state([remote("mini")]));
  await openMenu(q, "mini");
  await act(async () => { (document.querySelector('[data-device-move="mini"]') as HTMLElement).click(); });
  await act(async () => { useShellStore.setState({ coreMoveRefusal: "core_pending" }); });
  expect(document.querySelector('[data-core-move-refused="core_pending"]')?.textContent).toBe("Another move is under way");
  await unmount();
});

it("walks the move dialog through the supervisor's frames: failing checks with their fixes, confirm, steps, the result (B3 to B5)", async () => {
  const { q, events, unmount } = await mount(state([remote("mini")]));
  await openMenu(q, "mini");
  await act(async () => { (document.querySelector('[data-device-move="mini"]') as HTMLElement).click(); });
  const dialog = () => document.querySelector("[data-core-move-dialog]") as HTMLElement;
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "checking" }) as never }); });
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "checks_failed", checked: 6, failed: [{ check: "gh", detail: "not logged in" }, { check: "gui_session", detail: "no session" }] }) as never }); });
  expect(dialog().getAttribute("data-core-move-dialog")).toBe("checks_failed");
  expect(dialog().querySelector("[data-core-move-count]")?.textContent).toBe("2 to fix · 4 passed");
  expect(dialog().querySelector('[data-core-move-command="gh"]')?.textContent).toBe("gh auth login");
  expect(dialog().querySelector('[data-core-move-check="gui_session"]')?.textContent).toBe("× Desktop loginLog in on Mac mini");
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "ready" }) as never }); });
  expect(dialog().getAttribute("data-core-move-dialog")).toBe("confirm");
  await act(async () => { (dialog().querySelector("[data-core-move-start]") as HTMLElement).click(); });
  expect(sent(events, "core_move").at(-1)?.payload).toEqual({ action: "start", device: "mini" });
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "copying", step: "copy" }) as never }); });
  expect(dialog().getAttribute("data-core-move-dialog")).toBe("moving");
  expect([...dialog().querySelectorAll("[data-core-move-mark]")].map((row) => row.getAttribute("data-core-move-mark"))).toEqual(["done", "done", "run", "todo", "todo"]);
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "rolled_back", step: "start_target" }) as never }); });
  expect(dialog().querySelector("[data-core-move-failed]")?.textContent).toBe("× The core did not start on Mac mini");
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "done", step: "reattach", node: "local" }) as never }); });
  expect(dialog().getAttribute("data-core-move-dialog")).toBe("done");
  await act(async () => { (dialog().querySelector("[data-core-move-undo]") as HTMLElement).click(); });
  expect(sent(events, "core_move").at(-1)?.payload).toEqual({ action: "check_back" });
  expect(dialog().getAttribute("data-core-move-direction")).toBe("back");
  await unmount();
});

it("names the moving core's own machine when its Hide AI is not set up, and the machine taking it for that machine's sign-ins", async () => {
  const { q, unmount } = await mount(state([remote("mini")]));
  await openMenu(q, "mini");
  await act(async () => { (document.querySelector('[data-device-move="mini"]') as HTMLElement).click(); });
  const dialog = () => document.querySelector("[data-core-move-dialog]") as HTMLElement;
  const failed = [{ check: "ai_setup", detail: "Hide AI is not set up: no agent is chosen" }, { check: "ai", detail: "codex: logged_out" }];
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "checks_failed", checked: 12, failed }) as never }); });
  expect(dialog().querySelector('[data-core-move-check="ai_setup"]')?.textContent).toBe("× Hide AI on This computerTurn it on and choose an agent in Settings › Hide AI");
  expect(dialog().querySelector('[data-core-move-check="ai"]')?.textContent).toBe("× Hide AI sign-inSign in to Hide AI's agents on Mac mini");
  await unmount();
});

it("gives up a move's wait only from its confirm, which the strip's action opens, and shows what is left (amendment 4)", async () => {
  const { events, unmount } = await mount(state([remote("mini")]));
  const dialog = () => document.querySelector("[data-core-move-dialog]") as HTMLElement | null;
  const waiting = frame({ state: "waiting", step: "reattach", cause: { kind: "unreachable" }, abandon: true });
  await act(async () => { useShellStore.setState({ coreMove: waiting as never }); });
  await act(async () => { useUiStore.getState().openMoveGiveUp(); });
  expect(dialog()?.getAttribute("data-core-move-dialog")).toBe("give_up");
  expect(dialog()?.querySelector("h2")?.textContent).toBe("Stop waiting for Mac mini?");
  expect(dialog()?.querySelector("[data-core-move-give-up-risk]")?.textContent).toBe("If Mac mini is only cut off, not gone, its core keeps running until this Mac reaches it.");
  expect(sent(events, "core_move")).toEqual([]);
  // Cancel goes back to the steps, whose footer offers it again.
  await act(async () => { ([...dialog()!.querySelectorAll("button")].find((button) => button.textContent === "Cancel") as HTMLElement).click(); });
  expect(dialog()?.getAttribute("data-core-move-dialog")).toBe("moving");
  await act(async () => { (dialog()!.querySelector("[data-core-move-give-up]") as HTMLElement).click(); });
  await act(async () => { (dialog()!.querySelector("[data-core-move-give-up-go]") as HTMLElement).click(); });
  expect(sent(events, "core_move").map((event) => event.payload)).toEqual([{ action: "abandon" }]);
  await act(async () => { useShellStore.setState({ coreMoveRefusal: "move_not_waiting" }); });
  expect(dialog()?.querySelector('[data-core-move-refused="move_not_waiting"]')?.textContent).toBe("The move is no longer waiting");
  // A wait that ended leaves no confirm, and the result names what is left.
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "rolling_back", step: "reattach" }) as never }); });
  expect(dialog()?.querySelector("[data-core-move-give-up]")).toBeNull();
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "abandoned" }) as never }); });
  expect(dialog()?.getAttribute("data-core-move-dialog")).toBe("abandoned");
  // A move back's wait takes the core here, with no risk line.
  await act(async () => { useShellStore.setState({ coreMove: frame({ state: "waiting", direction: "back", device: null, step: "start_target", abandon: true }) as never }); });
  await act(async () => { useUiStore.getState().openMoveGiveUp(); });
  expect(dialog()?.getAttribute("data-core-move-direction")).toBe("back");
  expect(dialog()?.querySelector("h2")?.textContent).toBe("Take the core here?");
  expect(dialog()?.querySelector("[data-core-move-give-up-risk]")).toBeNull();
  expect(dialog()?.querySelector("[data-core-move-give-up-go]")?.textContent).toBe("Take it here");
  await unmount();
});

it("puts a node's own machine first as This Mac, with the core back and the end of its link in its menu and no SSH test (B2, B16)", async () => {
  window.location.hash = "#token=t&node=mbp";
  const node = remote("mbp", { label: "MacBook Pro", dials_in: true, ssh_alias: null });
  const next = state([remote("mini"), node]);
  (next.rest.navigator.devices[0] as Record<string, unknown>).machine_name = "Mac Studio";
  const { q, events, unmount } = await mount(next);
  const names = [...document.querySelectorAll("[data-device-name]")].map((row) => row.textContent);
  expect(names).toEqual(["This Mac", "Mac Studio", "Mac mini"]);
  expect(q('[data-device-core="local"]')).not.toBeNull();
  expect(q('[data-device-test="mbp"]')).toBeNull();
  await openMenu(q, "mbp");
  expect(document.querySelector('[data-device-remove="mbp"]')).toBeNull();
  expect(document.querySelector('[data-device-core-disconnect="mbp"]')?.textContent).toBe("Disconnect from the Mac Studio core…");
  await act(async () => { (document.querySelector('[data-device-move-back="mbp"]') as HTMLElement).click(); });
  expect(sent(events, "core_move").map((event) => event.payload)).toEqual([{ action: "check_back" }]);
  await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
  await openMenu(q, "mini");
  expect(document.querySelector('[data-device-move="mini"]')).toBeNull();
  window.location.hash = "";
  await unmount();
});
