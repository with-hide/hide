import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { NOTHING_UNDER_WAY } from "../pairing";
import type { Device, DeviceHost, Kit, KitComponent } from "../snapshot";
import { useShellStore } from "../store";
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
    status: { remote: devices.map((device) => ({ target_id: device.id, state: "connected", message: null })) },
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
    // This computer's own window: hided sent it the `pairing` frame.
    useShellStore.setState({ pairing: NOTHING_UNDER_WAY, ...next } as never);
    root.render(<TooltipProvider><DevicesTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  return { events, q, row: (id: string) => q(`[data-device-row="${id}"]`), unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

it("says nothing about a healthy kit on a device row, and names the device by its name and system (B54; operator review 2026-11-23)", async () => {
  const { row, q, unmount } = await mount(state([remote("mini")]));
  const mini = row("mini")!;
  expect(q('[data-device-subtitle="mini"]')?.textContent).toBe("macOS");
  expect(q('[data-device-state="mini:ready"]')?.textContent).toBe("●connected");
  expect(mini.querySelector("[data-machine-kit], [data-kit-part], [data-kit-problem], [data-kit-reinstall], [data-device-host]")).toBeNull();
  // The old rows named the hide command, the hook files and the helper: none of it is on the row.
  expect(mini.textContent).not.toMatch(/hide command|Claude Code hook|Codex hook|installs to|bound to/);
  // The menu is the only control a healthy device row carries; Test is in ⋯. The row says nothing of how it is reached.
  expect(mini.querySelectorAll("button")).toHaveLength(1);
  expect(mini.querySelector("[data-device-needs]")).toBeNull();
  expect(q('[data-device-menu="mini"]')?.getAttribute("aria-label")).toBe("More actions for Mac mini");
  await openMenu(q, "mini");
  const items = [...document.querySelectorAll('[data-device-menu-content="mini"] [role="menuitem"]')].map((item) => item.textContent);
  expect(items).toEqual(["Rename", "Test", "Connection details…", "Revoke helper…", "Remove…"]);
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

it("shows helper failures on a connected device and consent before Herdr can connect (B54, bootstrap B1-B2)", async () => {
  const stopped = remote("mini", { host: { ...host, state: "unavailable", message: "The helper did not answer." } });
  const unavailable = await mount(state([stopped]));
  expect(unavailable.q('[data-device-reason="mini"]')?.textContent).toContain("helper unavailable");
  // The helper's own words are Connection details'.
  expect(unavailable.row("mini")?.textContent).not.toContain("The helper did not answer.");
  await act(async () => { unavailable.q('[data-device-host-retry="mini"]')?.click(); });
  expect(sent(unavailable.events, "device_host_retry")).toHaveLength(1);
  await unavailable.unmount();

  const notAllowed = await mount(state([remote("studio", { host: { ...host, consent: "none", state: "not_allowed", helper_root: null } })]));
  expect(notAllowed.q('[data-device-reason="studio"]')?.textContent).toContain("helper not allowed");
  expect(notAllowed.q('[data-device-host-allow-inline="studio"]')).not.toBeNull();
  await notAllowed.unmount();

  // A failed helper can prevent the device session from connecting.
  const down = await mount(state([remote("mini", { state: "unavailable", host: { ...host, state: "unavailable" } })]));
  expect(down.q('[data-device-reason="mini"]')?.textContent).toContain("helper unavailable");
  expect(down.q('[data-device-host-retry="mini"]')).not.toBeNull();
  await down.unmount();

  for (const message of ["Herdr 0.8.2 uses an older protocol", "Herdr was not found"]) {
    const blocked = await mount(state([remote("studio", {
      state: "unavailable",
      message,
      host: { ...host, consent: "none", state: "not_allowed", helper_root: null },
    })]));
    try {
      expect(blocked.q('[data-device-reason="studio"]')?.textContent).toContain("helper not allowed");
      await act(async () => { blocked.q('[data-device-host-allow-inline="studio"]')?.click(); });
      expect(document.body.querySelector('[data-device-host-allow-confirm="studio"]')).not.toBeNull();
      await act(async () => { (document.body.querySelector('[data-device-host-allow-go]') as HTMLButtonElement).click(); });
      expect(sent(blocked.events, "device_host_consent")).toEqual([{ schema_version: 2, kind: "device_host_consent", payload: { device_id: "studio", allow: true } }]);
    } finally {
      await blocked.unmount();
    }
  }
});

it("says an Intel Mac reached over ssh could not connect, once and in plain words, without a helper retry", async () => {
  const message = "Intel Mac devices are not supported; Hide ships for Apple Silicon Macs only";
  const view = await mount(state([remote("mini", {
    state: "unavailable", problem: "unsupported", message,
    host: { ...host, platform: "macos-x86_64", consent: "granted", state: "unsupported", message },
  })]));
  try {
    // One state, one plain reason.
    expect(view.q('[data-device-state="mini:unavailable"]')?.textContent).toBe("!not connected");
    expect(view.q('[data-device-reason="mini"]')?.textContent).toBe("Hide does not run on this device's system.");
    // The helper is the link: its unsupported line would say the same reason again.
    expect(view.q('[data-device-host]')).toBeNull();
    expect(view.row("mini")?.textContent).not.toContain(message);
    expect(view.q('[data-device-host-retry="mini"]')).toBeNull();
  } finally {
    await view.unmount();
  }
});

it.each([
  "The matching release's device asset could not be downloaded",
  "Device archive size or SHA-256 does not match this app",
  "This local/main Hide build has no matching device release",
])("shows the kit failure and retries before a device session connects: %s", async (reason) => {
  const message = `Device kit download failed: ${reason}. Retry the device connection`;
  const view = await mount(state([remote("mini", {
    state: "unavailable", message: "Herdr uses an older protocol",
    host: { ...host, consent: "granted", state: "unavailable", message },
  })]));
  try {
    expect(view.q('[data-device-reason="mini"]')?.textContent).toBe("helper unavailable");
    expect(view.row("mini")?.textContent).not.toContain(message);
    // What the helper said stays one press away, in Connection details.
    await openMenu(view.q, "mini");
    await act(async () => { (document.querySelector('[data-device-details="mini"]') as HTMLElement).click(); });
    expect(document.querySelector('[data-device-details-dialog="mini"] [data-device-detail="message"]')?.textContent).toContain(message);
    await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    const retry = view.q('[data-device-host-retry="mini"]');
    expect(retry).not.toBeNull();
    await act(async () => { retry!.click(); });
    expect(sent(view.events, "device_host_retry")).toEqual([{ schema_version: 2, kind: "device_host_retry", payload: { device_id: "mini" } }]);
  } finally {
    await view.unmount();
  }
});

it("shows a paired machine once, as its pairing's row, and an SSH device beside it as its own (PRD device-connect-iroh B24)", async () => {
  const key = "ab".repeat(32);
  // The core made a device of the machine this one opens: the rail and the panes go by it.
  const next = state([remote("windows-pc", { label: "Windows PC", ssh_alias: null, peer_key: key }), remote("mini")]);
  const peers = [{ key, device_id: "windows-pc", name: "Windows PC", opens: "active", opened_by: null, connected: true, path: "direct", reason: null, update: null }];
  const { row, q, unmount } = await mount({ ...next, rest: { ...next.rest, navigator: { ...next.rest.navigator, peers } } } as never);
  expect(document.querySelectorAll('[data-peer-row="windows-pc"]')).toHaveLength(1);
  expect(row("windows-pc")).toBeNull();
  expect(row("mini")).not.toBeNull();
  // With the paired machine as the only other one, the list is not called empty.
  expect(q('[data-peer-row="windows-pc"]')?.textContent).toContain("Windows PC");
  await unmount();
});

// Operator review 2026-11-23: this computer with its share, then every other machine in one group, then the phones; one Add window adds a Hide by its link or a machine over SSH.
it("lists this computer, then every other machine in one group, and adds either kind from one window", async () => {
  const { q, events, unmount } = await mount(state([remote("mini")]));
  const sections = [...document.querySelectorAll("[data-device-section]")].map((section) => section.getAttribute("data-device-section"));
  expect(sections).toEqual(["this", "other", "phones"]);
  const own = q('[data-device-section="this"] [data-device-row="local"]')!;
  // This computer's row is its name and system, with Share; nothing about the daemon or what is selected.
  // jsdom is no Mac: the row names this machine as its system does.
  expect(own.querySelector("[data-device-name]")?.textContent).toBe("This computer");
  expect(own.querySelector("[data-device-state]")).toBeNull();
  expect(own.textContent).not.toMatch(/local, no SSH alias|this daemon's machine|selected/);
  expect(own.querySelector("[data-pair-share-make]")?.textContent).toBe("Share");
  expect(q('[data-device-section="other"] [data-device-row="mini"]')).not.toBeNull();
  expect(q("[data-devices-guide]")).toBeNull();
  // The line on top says what connecting gives, and Try it opens the practice's connection chapter.
  expect(q("[data-devices-intro]")?.textContent).toContain("use it just like This computer");
  await act(async () => { q("[data-device-add-open]")?.click(); });
  const dialog = () => document.body.querySelector("[data-add-device-dialog]");
  // It asks computer or phone first; a computer opens on a Hide's link, and the other choice is the SSH list.
  await act(async () => { dialog()?.querySelector<HTMLElement>('[data-add-choice="computer"]')?.click(); });
  expect(dialog()?.querySelector("[data-add-by-link]")).not.toBeNull();
  expect(dialog()?.querySelector('[data-ssh-hosts-state]')).toBeNull();
  await act(async () => { dialog()?.querySelector<HTMLElement>('[data-add-kind-choice="ssh"]')?.click(); });
  expect(dialog()?.querySelector("[data-add-by-link]")).toBeNull();
  expect(dialog()?.querySelector("[data-ssh-hosts-state]")).not.toBeNull();
  expect(events.length).toBeGreaterThan(0);
  await unmount();
});

it("guides a first device in three steps while no other machine is connected", async () => {
  const { q, unmount } = await mount(state([]));
  const guide = q("[data-devices-guide]")!;
  expect([...guide.querySelectorAll("li")].map((step) => step.textContent)).toEqual([
    "Press + Add device",
    "Open the link on that computer, or pick it over SSH and Hide installs itself there",
    "Then it works just like This computer: open a terminal on that computer and run agents",
  ]);
  await unmount();
  const other = await mount({ ...state([]), pairing: null } as never);
  expect(other.q("[data-devices-guide]")).toBeNull();
  expect(other.q("[data-devices-empty]")?.textContent).toBe("No other device yet.");
  await other.unmount();
});

it("renames an SSH device from its ⋯, which the core keeps (c2)", async () => {
  const { q, events, unmount } = await mount(state([remote("mini")]));
  await openMenu(q, "mini");
  await act(async () => { (document.querySelector('[data-device-rename="mini"]') as HTMLElement).click(); });
  const input = document.querySelector('[data-device-rename-input="mini"]') as HTMLInputElement;
  expect(input.value).toBe("Mac mini");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Studio Mac");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => { (document.querySelector('[data-device-rename-go="mini"]') as HTMLElement).click(); });
  expect(sent(events, "rename_device")).toEqual([{ schema_version: 2, kind: "rename_device", payload: { device_id: "mini", name: "Studio Mac" } }]);
  await unmount();
});

it("opens Add device from one button above the list, and shows the Add dialog only then (B50)", async () => {
  const { q, unmount } = await mount(state([remote("mini")]));
  expect(q("[data-add-device]")).toBeNull();
  await act(async () => { q("[data-device-add-open]")?.click(); });
  expect(document.body.querySelector("[data-add-device-dialog]")).not.toBeNull();
  await unmount();
});

it("offers no Add device on a screen that is not this computer's own window (PRD device-connect-iroh B52)", async () => {
  const { q, unmount } = await mount({ ...state([remote("mini")]), pairing: null } as never);
  expect(q('[data-device-row="mini"]')).not.toBeNull();
  expect(q("[data-device-add-open]")).toBeNull();
  await unmount();
});

/** Opens a row's ⋯ the way a keyboard does: Radix opens the menu on Enter at its trigger. */
async function openMenu(q: (selector: string) => HTMLElement | null, id: string) {
  await act(async () => {
    q(`[data-device-menu="${id}"]`)?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
}

it("offers no device the core (PRD device-connect-iroh D-35)", async () => {
  const { q, unmount } = await mount(state([remote("mini")]));
  await openMenu(q, "mini");
  expect(document.querySelector('[data-device-menu-content="mini"]')).not.toBeNull();
  expect(document.querySelector('[data-device-move="mini"]')).toBeNull();
  await unmount();
});

it("puts a node's own machine first as This Mac, with the end of its link in its menu and no SSH test (B16)", async () => {
  window.location.hash = "#token=t&node=mbp";
  const node = remote("mbp", { label: "MacBook Pro", dials_in: true, ssh_alias: null });
  const next = state([remote("mini"), node]);
  (next.rest.navigator.devices[0] as Record<string, unknown>).machine_name = "Mac Studio";
  const { q, unmount } = await mount(next);
  const names = [...document.querySelectorAll("[data-device-name]")].map((row) => row.textContent);
  expect(names).toEqual(["This Mac", "Mac Studio", "Mac mini"]);
  expect(q('[data-device-test="mbp"]')).toBeNull();
  await openMenu(q, "mbp");
  expect(document.querySelector('[data-device-remove="mbp"]')).toBeNull();
  expect(document.querySelector('[data-device-core-disconnect="mbp"]')?.textContent).toBe("Disconnect from the Mac Studio core…");
  expect(document.querySelector('[data-device-move-back="mbp"]')).toBeNull();
  window.location.hash = "";
  await unmount();
});

it("says on a node's own row why its machine was not made a Hide of its own, with a Retry that asks the node again (PRD device-connect-iroh B45)", async () => {
  window.location.hash = "#token=t&node=mbp";
  const next = state([remote("mini"), remote("mbp", { label: "MacBook Pro", dials_in: true, ssh_alias: null })]);
  (next.rest.navigator.devices[0] as Record<string, unknown>).machine_name = "Mac Studio";
  const { events, q, unmount } = await mount(next);
  expect(q("[data-device-split]")).toBeNull();
  await act(async () => useShellStore.setState({ coreSplit: "split_refused" }));
  expect(q('[data-device-row="mbp"] [data-device-split="split_refused"]')?.textContent).toContain("Hide on Mac Studio refused");
  // An SSH login that worked while the new connection did not is its own cause.
  await act(async () => useShellStore.setState({ coreSplit: "split_not_connected" }));
  expect(q('[data-device-row="mbp"] [data-device-split="split_not_connected"]')?.textContent).toContain("the new connection to Mac Studio could not be made");
  // Only the window's own machine says it: the other rows are the core's to draw.
  expect(document.querySelectorAll("[data-device-split]")).toHaveLength(1);
  await act(async () => q('[data-device-split-retry="mbp"]')!.click());
  expect(sent(events, "core_link")).toEqual([{ schema_version: 2, kind: "core_link", payload: { action: "retry" } }]);
  window.location.hash = "";
  await unmount();
});

// PRD system-ssh-device-link: what ssh asks and how it failed, on the device's row.
const waiting = (id: string, sign_in: Device["sign_in"]) => {
  const next = state([remote(id, { state: "unavailable", sign_in, host: { ...host, state: "connecting" } })]);
  next.rest.status.remote = [{ target_id: id, state: "connecting", message: null }];
  return next;
};

it("asks ssh's question in a masked field, sends the answer to that question and clears it, and cancels (B6, B8, B9)", async () => {
  const view = await mount(waiting("mini", { question: 7, key_command: "ssh-add --apple-use-keychain /Users/al/.ssh/id_ed25519", kind: "secret", prompt: "Enter passphrase for key '/Users/al/.ssh/id_ed25519':", key_file: "/Users/al/.ssh/id_ed25519" }));
  try {
    expect(view.q('[data-device-sign-in="mini:secret"]')?.textContent).toContain("Enter passphrase for key '/Users/al/.ssh/id_ed25519':");
    const field = view.q('[data-device-sign-in-field="mini"]') as HTMLInputElement;
    expect(field.type).toBe("password");
    expect(view.q('[data-device-sign-in-key-command="mini"]')?.textContent).toContain("ssh-add --apple-use-keychain /Users/al/.ssh/id_ed25519");
    // A device mid-attempt offers no Retry beside its question.
    expect(view.q('[data-device-retry="mini"]')).toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "hunter2");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { view.q('[data-device-sign-in-submit="mini"]')!.click(); });
    expect(sent(view.events, "device_sign_in")).toEqual([{ schema_version: 2, kind: "device_sign_in", payload: { device_id: "mini", question: 7, reply: "text", text: "hunter2" } }]);
    expect(field.value).toBe("");
    await act(async () => { view.q('[data-device-sign-in-cancel="mini"]')!.click(); });
    expect(sent(view.events, "device_sign_in")[1]).toEqual({ schema_version: 2, kind: "device_sign_in", payload: { device_id: "mini", question: 7, reply: "cancel" } });
  } finally {
    await view.unmount();
  }
});

it("shows a server it has not met with its fingerprint, and neither trust nor cancel is chosen for the operator (B12)", async () => {
  const view = await mount(waiting("mini", { question: 2, key_command: null, kind: "host_key", host: "[mini.example]:2222", fingerprint: "SHA256:Jq8F2n1xR0a+e3uY7kz6m4W0cB9vT5pLhGd2sQ1oE8I" }));
  try {
    expect(view.q('[data-device-sign-in-fingerprint="mini"]')?.textContent).toBe("SHA256:Jq8F2n1xR0a+e3uY7kz6m4W0cB9vT5pLhGd2sQ1oE8I");
    const trust = view.q('[data-device-sign-in-trust="mini"]')!;
    expect(trust.textContent).toBe("Trust this server and connect");
    expect([trust, view.q('[data-device-sign-in-cancel="mini"]')]).not.toContain(document.activeElement);
    await act(async () => { trust.click(); });
    expect(sent(view.events, "device_sign_in")).toEqual([{ schema_version: 2, kind: "device_sign_in", payload: { device_id: "mini", question: 2, reply: "accept" } }]);
  } finally {
    await view.unmount();
  }
});

it("opens a sign-in page in the default browser and says it waits for it (B4)", async () => {
  const opened = vi.spyOn(window, "open").mockReturnValue(null);
  const view = await mount(waiting("mini", { question: 1, key_command: null, kind: "browser", url: "https://login.tailscale.com/a/1a2b3c" }));
  try {
    expect(view.q('[data-device-sign-in="mini:browser"]')?.textContent).toContain("Waiting for confirmation in the browser");
    await act(async () => { view.q('[data-device-sign-in-open="mini"]')!.click(); });
    expect(opened).toHaveBeenCalledWith("https://login.tailscale.com/a/1a2b3c", "_blank", "noopener,noreferrer");
  } finally {
    opened.mockRestore();
    await view.unmount();
  }
});

it("draws a changed server key with its terminal check and a connection that could not be made over ssh's last line, with Retry (B13; operator decision 2026-10-30)", async () => {
  const unavailable = { ...host, state: "unavailable" as const, message: "Host key verification failed." };
  const changed = await mount(state([remote("mini", { state: "unavailable", problem: "host_key_changed", message: "Host key verification failed.", host: unavailable })]));
  try {
    expect(changed.q('[data-device-reason="mini"]')?.textContent).toBe("Server key changed");
    expect(changed.q('[data-device-problem-command="mini"]')?.textContent).toBe("ssh mini");
    // The terminal check is its one action: a retry before it would be refused again.
    expect(changed.q('[data-device-retry="mini"]')).toBeNull();
    expect(changed.row("mini")?.textContent).not.toContain("Host key verification failed.");
    // The helper is the link: its unavailable line would say the same failure again.
    expect(changed.q('[data-device-host]')).toBeNull();
  } finally {
    await changed.unmount();
  }
  const policy = "tailscale: tailnet policy does not permit you to SSH to this node";
  const lost = await mount(state([remote("mini", { state: "unavailable", problem: "cannot_connect", message: policy, host: { ...unavailable, message: policy } })]));
  try {
    expect(lost.q('[data-device-reason="mini"]')?.textContent).toBe("Couldn't connect");
    // ssh's last line is Connection details'.
    expect(lost.row("mini")?.textContent).not.toContain(policy);
    expect(lost.q('[data-device-retry="mini"]')?.textContent).toBe("Retry");
  } finally {
    await lost.unmount();
  }
  const missing = await mount(state([remote("mini", { state: "unavailable", problem: "ssh_missing", message: "/usr/bin/ssh was not found on this computer", host: unavailable })]));
  try {
    expect(missing.q('[data-device-reason="mini"]')?.textContent).toBe("This computer has no ssh. Install the OpenSSH client.");
    expect(missing.row("mini")?.textContent).not.toContain("was not found");
  } finally {
    await missing.unmount();
  }
});

it("draws a Hide an update replaced as opening Hide again, with no Retry from it", async () => {
  const unavailable = { ...host, state: "unavailable" as const, message: "Hide was updated since it started." };
  const view = await mount(state([remote("mini", { state: "unavailable", problem: "hide_updated", message: "Hide was updated since it started.", host: unavailable })]));
  try {
    expect(view.q('[data-device-reason="mini"]')?.textContent).toBe("Hide was updated after it started. Quit Hide and open it again to connect.");
    // Only Hide opened again can connect: a retry from this one starts the same old program.
    expect(view.q('[data-device-retry="mini"]')).toBeNull();
    expect(view.q('[data-device-host]')).toBeNull();
  } finally {
    await view.unmount();
  }
});

it("reads a refused sign-in with the same Sign in as a needed one", async () => {
  const view = await mount(state([remote("mini", { state: "unavailable", problem: "sign_in_refused", message: "Permission denied (publickey,password).", host: { ...host, state: "unavailable", message: "Permission denied (publickey,password)." } })]));
  try {
    expect(view.q('[data-device-reason="mini"]')?.textContent).toBe("Sign-in refused");
    expect(view.q('[data-device-retry="mini"]')?.textContent).toBe("Sign in");
    expect(view.row("mini")?.textContent).not.toContain("Permission denied");
  } finally {
    await view.unmount();
  }
});

it("stops a reconnect nobody could be asked for at Sign in, which starts an attempt that may ask (B8, B14)", async () => {
  const next = state([remote("mini", { state: "unavailable", problem: "sign_in_needed", host: { ...host, state: "unavailable", message: "ssh needs an answer" } })]);
  next.rest.status.remote = [{ target_id: "mini", state: "unavailable", message: null }];
  const view = await mount(next);
  try {
    expect(view.q('[data-device-reason="mini"]')?.textContent).toBe("Sign-in needed");
    const signIn = view.q('[data-device-retry="mini"]')!;
    expect(signIn.textContent).toBe("Sign in");
    await act(async () => { signIn.click(); });
    expect(sent(view.events, "retry_connect")).toEqual([{ schema_version: 2, kind: "retry_connect", payload: { target_id: "mini" } }]);
  } finally {
    await view.unmount();
  }
});

it("points a Windows device whose Hide did not start at turning Smart App Control off, and no other failure (issue 2855)", async () => {
  const windows = { ...host, platform: "windows x86_64" };
  const silent = await mount(state([remote("win", { label: "Windows PC", state: "unavailable", problem: "windows_not_started", host: windows })]));
  const line = silent.q('[data-device-reason="win"]');
  expect(line?.textContent).toBe("Hide did not start on Windows · if Smart App Control is on, how to turn it off");
  expect(silent.q('[data-device-sac-docs="win"]')?.getAttribute("href")).toBe("https://withhide.dev/docs/reference/troubleshooting/#smart-app-control-blocks-hide-on-windows-11");
  await silent.unmount();

  const other = await mount(state([remote("win", { label: "Windows PC", state: "unavailable", problem: "cannot_connect", host: windows })]));
  expect(other.q('[data-device-sac-docs="win"]')).toBeNull();
  await other.unmount();
});
