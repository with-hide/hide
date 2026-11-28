// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import type { Device, SshHost, SshHosts, WslDistribution } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { DispatchFn } from "../ws";
import { AddDevice } from "./AddDevice";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const host = (alias: string, over: Partial<SshHost> = {}): SshHost => ({ alias, address: `fixture@${alias}.example.invalid:22`, added_as: null, problem: null, ...over });
const hosts = (list: SshHost[], over: Partial<SshHosts> = {}): SshHosts => ({ state: "ready", truncated: false, hosts: list, ...over });
const distro = (name: string, over: Partial<WslDistribution> = {}): WslDistribution => ({ name, device_name: `WSL ${name}`, wsl_version: 2, default: false, added_as: null, problem: null, ...over });

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(sshHosts: SshHosts | undefined, devices: Device[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const paired = vi.fn();
  const shown = vi.fn();
  await act(async () => {
    useShellStore.setState({ connection: "live", pairingResult: null, rest: { status: { ssh_hosts: sshHosts }, ui_state: {} } } as never);
    root.render(<AddDevice actions={actions} devices={devices} helperRoot="~/.hide/host-helper" cliDir="~/.local/bin" onClose={paired} onShowDevice={shown} />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  const rerender = async (next: Device[]) => {
    await act(async () => {
      root.render(<AddDevice actions={actions} devices={next} helperRoot="~/.hide/host-helper" cliDir="~/.local/bin" onClose={paired} onShowDevice={shown} />);
    });
  };
  // hided's answer to the last `pairing_<verb>` event this form sent.
  const answer = async (verb: string, result: Record<string, unknown>) => {
    const request = sent(events, `pairing_${verb}`).at(-1)?.payload.request_id;
    await act(async () => {
      useShellStore.setState({ pairingResult: { verb, request_id: request, ...result } } as never);
    });
  };
  return { events, q, container, rerender, answer, paired, shown, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

it("asks hided for the Hosts once when it opens (B50)", async () => {
  const { events, unmount } = await mount(hosts([host("lab")]));
  expect(sent(events, "ssh_hosts_list")).toHaveLength(1);
  await unmount();
});

it("lists each Host with its address, and fills the name and adds only the alias and the name once one is chosen (B50)", async () => {
  const { events, q, answer, unmount } = await mount(hosts([host("studio"), host("lab")]));
  expect(q('[data-ssh-host="studio"]')?.textContent).toContain("fixture@studio.example.invalid:22");
  // No username, port or key input, and no alias input.
  expect(q("[data-device-alias]")).toBeNull();
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  expect((q("[data-device-label]") as HTMLInputElement).value).toBe("lab");
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "absent" }, system: "windows", leaves: "windows" });
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  const register = sent(events, "register_device");
  expect(register).toHaveLength(1);
  // The core gives the device its id: the shell sends none.
  expect(register[0]?.payload).toEqual({ label: "lab", ssh_alias: "lab", host_consent: true });
  await unmount();
});
it("connects a Host whose own Hide runs as a server, and installs nothing there (PRD device-connect-iroh B7; operator decision 2026-11-28)", async () => {
  const { events, q, answer, paired, unmount } = await mount(hosts([host("lab")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  // Choosing the Host is the one look: an ssh that asks nobody.
  expect(sent(events, "pairing_look_ssh").map((event) => event.payload.alias)).toEqual(["lab"]);
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "ready" }, system: "macos", leaves: null, condition: { kind: "auto_login_on" } });
  expect(q('[data-add-way="connect"]')?.textContent).toContain("Recommended");
  expect(q('[data-add-way="connect"] [data-device-way-headline]')?.textContent).toBe("Even when this computer sleeps,it keeps running on lab while that device is on");
  expect(q('[data-add-way-read="connect"]')?.textContent).toBe("macOS · Hide is there → connects without installing anything");
  // A Mac that logs itself in after a restart has nothing more to say.
  expect(q("[data-add-way-restart]")).toBeNull();
  // Nothing is installed, so there is no name to give and no install to agree to.
  expect(q("[data-device-label]")).toBeNull();
  expect(q("[data-add-device-advanced]")).toBeNull();
  expect(q('[data-add-device="connect"]')?.textContent).toBe("Connect as a server");
  await act(async () => { q('[data-add-device="connect"]')?.click(); });
  expect(sent(events, "pairing_pair_ssh").map((event) => event.payload.alias)).toEqual(["lab"]);
  expect(sent(events, "register_device")).toHaveLength(0);
  expect(q('[data-add-device="connect"]')?.hasAttribute("disabled")).toBe(true);
  await answer("pair_ssh", { ok: true, device_id: "lab-machine", name: "Lab" });
  expect(paired).toHaveBeenCalledTimes(1);
  await unmount();
});
it("says before the press that a Mac running Hide with automatic login off needs one login after a restart (operator decision 2026-11-28)", async () => {
  const { q, answer, unmount } = await mount(hosts([host("lab"), host("studio")]));
  const restart = async (alias: string, how: boolean) => {
    await act(async () => { q(`[data-ssh-host="${alias}"] [role="radio"]`)?.click(); });
    await answer("look_ssh", { ok: true, alias, looked: { state: "ready" }, system: "macos", leaves: null, condition: { kind: "auto_login_off", how } });
    return q('[data-add-way="connect"] [data-add-way-restart]')?.textContent;
  };
  expect(await restart("lab", true)).toBe("Automatic login is off · after lab restarts, log in to that Mac once and it connects again");
  // FileVault keeps automatic login from being turned on: the same line.
  expect(await restart("studio", false)).toBe("Automatic login is off · after studio restarts, log in to that Mac once and it connects again");
  await unmount();
});
it("adds a Host that could be a server over SSH from the small row under the card, and leaves the Hide there as it is (operator decisions 2026-11-24, 2026-11-28)", async () => {
  const { events, q, answer, unmount } = await mount(hosts([host("lab"), host("studio")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  // Nothing to choose until the Host said what it is.
  expect(q("[data-add-device-over-ssh]")).toBeNull();
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "ready" }, system: "macos", leaves: null });
  expect(q("[data-add-device-direct]")?.textContent).toContain("Connect directly over SSH · Only a small tool goes there");
  expect(q("[data-add-device-over-ssh]")?.textContent).toBe("Connect this way");
  await act(async () => { q("[data-add-device-over-ssh]")?.click(); });
  expect(sent(events, "register_device").map((event) => event.payload)).toEqual([{ label: "lab", ssh_alias: "lab", host_consent: true }]);
  // Nothing pairs with the Hide there, and nothing stops it.
  expect(sent(events, "pairing_pair_ssh")).toHaveLength(0);
  await unmount();
  // A Host with no Hide offers the same row beside its install.
  const other = await mount(hosts([host("studio")]));
  await act(async () => { other.q('[data-ssh-host="studio"] [role="radio"]')?.click(); });
  await other.answer("look_ssh", { ok: true, alias: "studio", looked: { state: "absent" }, system: "linux", leaves: null });
  expect(other.q("[data-add-device-over-ssh]")?.textContent).toBe("Connect this way");
  await other.unmount();
});

it("installs Hide on a Host with none that can take one and connects the two, its steps in the card until it ends (operator decision 2026-11-28)", async () => {
  const { events, q, answer, rerender, paired, unmount } = await mount(hosts([host("gpu-box")]));
  await act(async () => { q('[data-ssh-host="gpu-box"] [role="radio"]')?.click(); });
  await answer("look_ssh", { ok: true, alias: "gpu-box", looked: { state: "absent" }, system: "linux", leaves: null });
  expect(q('[data-add-way="install"]')?.textContent).toContain("Recommended");
  expect(q('[data-add-way-read="install"]')?.textContent).toBe("Linux · No Hide yet → Hide is installed on gpu-box");
  // The device is named, and what its helper installs is agreed to here.
  expect((q("[data-device-label]") as HTMLInputElement).value).toBe("gpu-box");
  expect(q('[data-add-device-advanced="true"]')).not.toBeNull();
  expect(q('[data-add-device="server"]')?.textContent).toBe("Install and connect as a server");
  await act(async () => { q('[data-add-device="server"]')?.click(); });
  // One ask: hided adds it, waits for it and starts Hide there.
  expect(sent(events, "pairing_add_server").map((event) => ({ alias: event.payload.alias, name: event.payload.name }))).toEqual([{ alias: "gpu-box", name: "gpu-box" }]);
  expect(sent(events, "register_device")).toHaveLength(0);
  expect(q('[data-add-device="server"]')?.textContent).toBe("Connecting…");
  expect(q("[data-add-device-over-ssh]")).toBeNull();
  // The core's step of the device's switch is drawn in the card once the device is listed.
  expect(q("[data-device-switch-step]")?.getAttribute("data-device-switch-step")).toBe("checking");
  await rerender([{ id: "gpu-machine", ssh_alias: "gpu-box", label: "gpu-box", kind: "remote", switch_step: "pairing" } as Device]);
  expect(q("[data-device-switch-step]")?.getAttribute("data-device-switch-step")).toBe("pairing");
  expect(paired).not.toHaveBeenCalled();
  await answer("add_server", { ok: true, switch: "done", device_id: "gpu-machine" });
  expect(paired).toHaveBeenCalledTimes(1);
  await unmount();
});

it("opens the added device's row with why when its Hide did not start, and says here why a machine was not added", async () => {
  const { q, answer, rerender, shown, unmount } = await mount(hosts([host("gpu-box")]));
  await act(async () => { q('[data-ssh-host="gpu-box"] [role="radio"]')?.click(); });
  await answer("look_ssh", { ok: true, alias: "gpu-box", looked: { state: "absent" }, system: "linux", leaves: null });
  await act(async () => { q('[data-add-device="server"]')?.click(); });
  await answer("add_server", { ok: false, reason: "device_not_added" });
  expect(q('[data-ssh-pair-refused="device_not_added"]')).not.toBeNull();
  expect(q('[data-add-device="server"]')?.hasAttribute("disabled")).toBe(false);
  expect(shown).not.toHaveBeenCalled();
  // Pressed again, the device was added and its Hide did not start: the row says why, with its retry.
  await act(async () => { q('[data-add-device="server"]')?.click(); });
  await rerender([{ id: "gpu-machine", ssh_alias: "gpu-box", label: "gpu-box", kind: "remote" } as Device]);
  await answer("add_server", { ok: false, reason: "ssh_core_not_started" });
  expect(shown).toHaveBeenCalledWith("gpu-machine");
  expect(useUiStore.getState().switchOutcomes["gpu-machine"]).toEqual({ kind: "failed", reason: "ssh_core_not_started" });
  useUiStore.getState().setSwitchOutcome("gpu-machine", null);
  await unmount();
});
it("waits for the Host's own answer before it adds, so a machine that runs Hide is never added as a helper device meanwhile (B7)", async () => {
  const { events, q, answer, unmount } = await mount(hosts([host("lab"), host("studio")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  // Asked and not answered: the press does nothing yet, and the form says what it waits for.
  expect(q('[data-ssh-look="looking"]')?.textContent).toBe("Checking for Hide there…");
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  expect(sent(events, "register_device")).toHaveLength(0);
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "ready" }, system: "macos", leaves: null });
  expect(q('[data-ssh-look="looking"]')).toBeNull();
  expect(q('[data-add-device="connect"]')?.hasAttribute("disabled")).toBe(false);
  // A look hided refused looked at nothing: the Host is added over SSH as before, with no reason to give.
  await act(async () => { q('[data-ssh-host="studio"] [role="radio"]')?.click(); });
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await answer("look_ssh", { ok: false, reason: "ssh_busy" });
  expect(q("[data-ssh-look]")).toBeNull();
  expect(q("[data-add-way-reason]")).toBeNull();
  expect(q('[data-add-way="ssh"]')).not.toBeNull();
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  expect(sent(events, "register_device").map((event) => event.payload.ssh_alias)).toEqual(["studio"]);
  await unmount();
});
it("says why a pairing over SSH was refused and lets it be pressed again", async () => {
  const { q, answer, paired, unmount } = await mount(hosts([host("lab")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "ready" }, system: "macos", leaves: null });
  await act(async () => { q('[data-add-device="connect"]')?.click(); });
  await answer("pair_ssh", { ok: false, reason: "device_already_connected" });
  expect(q('[data-ssh-pair-refused="device_already_connected"]')?.textContent).toBe("Already connected");
  expect(q('[data-add-device="connect"]')?.hasAttribute("disabled")).toBe(false);
  expect(paired).not.toHaveBeenCalled();
  await unmount();
});
it("says in one line why a Host is not connected as a server, and makes SSH the press (operator decision 2026-11-28)", async () => {
  const { events, q, answer, unmount } = await mount(hosts([host("lab"), host("studio"), host("win"), host("edge"), host("box")]));
  const reason = async (alias: string, said: Record<string, unknown>) => {
    await act(async () => { q(`[data-ssh-host="${alias}"] [role="radio"]`)?.click(); });
    await answer("look_ssh", { ok: true, alias, ...said });
    expect(q('[data-add-way="ssh"]')?.textContent).toContain("Connect directly over SSH");
    expect(q('[data-add-device="true"]')?.textContent).toBe("Connect");
    // SSH is the way now, so it is not offered again under the card.
    expect(q("[data-add-device-direct]")).toBeNull();
    // A fact to read, not a warning: the line is in the muted tone.
    expect(q("[data-add-way-reason]")?.classList.contains("text-muted-foreground")).toBe(true);
    return q("[data-add-way-reason]")?.textContent;
  };
  expect(await reason("lab", { looked: { state: "update_there" }, system: "windows", leaves: null })).toBe("Update Hide on that PC");
  expect(await reason("studio", { looked: { state: "not_running", system: "macos" }, system: "macos", leaves: null })).toBe("Log in on that Mac");
  expect(await reason("win", { looked: { state: "absent" }, system: "windows", leaves: "windows" })).toBe("Windows can't be connected as a server with a button yet · to use it as a server, install Hide on that PC and connect with a link");
  expect(await reason("edge", { looked: { state: "not_reached" }, system: null, leaves: null })).toBe("SSH could not sign in without asking, so it cannot be connected as a server");
  expect(await reason("box", { looked: { state: "absent" }, system: "linux", leaves: "no_systemd" })).toBe("This device has no systemd, so it cannot be connected as a server");
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  expect(sent(events, "register_device").map((event) => event.payload.ssh_alias)).toEqual(["box"]);
  expect(sent(events, "pairing_pair_ssh")).toHaveLength(0);
  expect(sent(events, "pairing_add_server")).toHaveLength(0);
  await unmount();
});
it("waits for the device its alias added, under whatever id the core gives it, and then closes", async () => {
  const { q, answer, rerender, paired, unmount } = await mount(hosts([host("lab")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "absent" }, system: "linux", leaves: "no_systemd" });
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  expect(q('[data-add-device="true"]')?.textContent).toBe("Adding…");
  // Its first connection names it by its machine's id, not by the alias.
  await rerender([{ id: "4c1d2e3f-0a1b-4c2d-8e3f-5a6b7c8d9e0f", ssh_alias: "lab" } as Device]);
  expect(paired).toHaveBeenCalledTimes(1);
  await unmount();
});

it("dims a Host a device already uses, or whose address is unknown, and cannot choose it (B51, B52)", async () => {
  const { q, unmount } = await mount(hosts([host("mini", { added_as: "Mac mini" }), host("twin", { added_as: "Mac mini" }), host("broken", { address: null, problem: "ssh_failed" }), host("lab")]));
  expect(q('[data-ssh-host-note="mini"]')?.textContent).toBe("Added over SSH as Mac mini");
  expect(q('[data-ssh-host-note="twin"]')?.textContent).toBe("Added over SSH as Mac mini");
  expect(q('[data-ssh-host-note="broken"]')?.textContent).toBe("ssh couldn't read this host");
  for (const alias of ["mini", "twin", "broken"]) expect(q(`[data-ssh-host="${alias}"] [role="radio"]`)?.hasAttribute("disabled")).toBe(true);
  expect(q('[data-ssh-host="lab"] [role="radio"]')?.hasAttribute("disabled")).toBe(false);
  await unmount();
});

// with-hide/hide issue 3007: a machine this computer already reaches answers any look with this computer's own helper there, so
// adding it again could only say "update" or "refused"; what puts Hide there is that device's Connect as a server.
it("points a Host or distribution this computer already reaches to its device's Connect as a server, and adds nothing", async () => {
  const devices = [
    { id: "4c1d2e3f-mini", ssh_alias: "mini", label: "Mac mini", kind: "remote" },
    { id: "wsl-ubuntu-1a2b", ssh_alias: null, wsl_distro: "Ubuntu", label: "WSL Ubuntu", kind: "remote" },
  ] as Device[];
  const { events, q, shown, unmount } = await mount(hosts([host("mini", { added_as: "Mac mini" }), host("twin", { added_as: "Mac mini" }), host("lab")], { wsl: [distro("Ubuntu", { added_as: "WSL Ubuntu" })] }), devices);
  expect(q('[data-ssh-host-server="lab"]')).toBeNull();
  for (const [selector, id] of [
    ['[data-ssh-host-server="mini"]', "4c1d2e3f-mini"],
    // Another alias of the same machine names the device the core named for it.
    ['[data-ssh-host-server="twin"]', "4c1d2e3f-mini"],
    ['[data-wsl-distro-server="Ubuntu"]', "wsl-ubuntu-1a2b"],
  ] as const) {
    expect(q(selector)?.textContent).toBe("Connect as a server ›");
    await act(async () => { q(selector)?.click(); });
    expect(shown).toHaveBeenLastCalledWith(id);
  }
  // Nothing was looked at over SSH or added again.
  expect(sent(events, "pairing_look_ssh")).toHaveLength(0);
  expect(sent(events, "pairing_look_wsl")).toHaveLength(0);
  expect(sent(events, "register_device")).toHaveLength(0);
  expect(q('[data-ssh-host="mini"] [role="radio"]')?.hasAttribute("disabled")).toBe(true);
  await unmount();
});

it("keeps a name the person wrote when another Host is chosen", async () => {
  const { q, unmount } = await mount(hosts([host("studio"), host("lab")]));
  await act(async () => { q('[data-ssh-host="studio"] [role="radio"]')?.click(); });
  const name = q("[data-device-label]") as HTMLInputElement;
  expect(name.value).toBe("studio");
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(name, "Studio Mac");
    name.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  expect(name.value).toBe("Studio Mac");
  await unmount();
});

it("shows only what to add to ~/.ssh/config when there is no Host, with no form (B52)", async () => {
  const { q, container, unmount } = await mount(hosts([]));
  expect(q('[data-ssh-hosts-state="empty"]')).not.toBeNull();
  expect(container.textContent).toContain("Add a Host entry to ~/.ssh/config");
  expect(container.querySelector("input, button")).toBeNull();
  await unmount();
});

it("says it is reading until hided has answered, and keeps the earlier Hosts while it reads again", async () => {
  const first = await mount(hosts([], { state: "loading" }));
  expect(first.q('[data-ssh-hosts-state="loading"]')).not.toBeNull();
  expect(first.q("[data-device-label]")).toBeNull();
  await first.unmount();
  const again = await mount(hosts([host("lab")], { state: "loading" }));
  expect(again.q('[data-ssh-host="lab"]')).not.toBeNull();
  await again.unmount();
});

it("keeps what Hide installs under Advanced, three lines long, and asks for no socket (B53)", async () => {
  const { q, unmount } = await mount(hosts([host("lab")]));
  const advanced = q('[data-add-device-advanced="true"]') as HTMLDetailsElement;
  expect(advanced.open).toBe(false);
  expect(q('[data-add-device-installs="true"]')?.querySelectorAll("li")).toHaveLength(3);
  expect(advanced.querySelector("[data-device-socket]")).toBeNull();
  await unmount();
});

// The core clears `last_error` at its next event of any kind, so a refusal that only lived there would
// let the form read "no error yet" as "still pending" and lock itself again (code-web 3, `useRefusalSince`).
it("keeps a refused Add enabled with its reason after the core clears last_error", async () => {
  const { q, answer, unmount } = await mount(hosts([host("lab")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  await answer("look_ssh", { ok: true, alias: "lab", looked: { state: "absent" }, system: "windows", leaves: "windows" });
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  // Sent and not answered: the form waits.
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  expect(q("[data-device-label]")?.hasAttribute("disabled")).toBe(true);

  const refused = { kind: "device.duplicate", message: "Device lab is already registered", retryable: false, occurred_at: Date.now() + 1000 };
  await act(async () => { useShellStore.setState({ rest: { ...useShellStore.getState().rest, status: { ...useShellStore.getState().rest?.status, last_error: refused } } } as never); });
  // Worded by its kind; the core's sentence is the log's.
  expect(q('[data-add-device-error="true"]')?.textContent).toBe("This device has already been added.");
  expect(q('[data-add-device-error="true"]')?.getAttribute("data-core-error")).toBe("device.duplicate");
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(false);
  expect(q("[data-device-label]")?.hasAttribute("disabled")).toBe(false);

  // An unrelated event arrives and the core takes last_error: the refusal stays, and the form stays usable.
  await act(async () => { useShellStore.setState({ rest: { ...useShellStore.getState().rest, status: { ...useShellStore.getState().rest?.status, last_error: null } } } as never); });
  // Worded by its kind; the core's sentence is the log's.
  expect(q('[data-add-device-error="true"]')?.textContent).toBe("This device has already been added.");
  expect(q('[data-add-device-error="true"]')?.getAttribute("data-core-error")).toBe("device.duplicate");
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(false);
  expect(q("[data-device-label]")?.hasAttribute("disabled")).toBe(false);

  // Adding again is a new request: the refusal belonged to the last one, so the form waits again.
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  expect(q('[data-add-device-error="true"]')).toBeNull();
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await unmount();
});

// PRD wsl-device B4: this PC's WSL distributions sit beside the Hosts under the same consent, the
// default one first; one is added by its name alone and takes the name the core gives it (D-20).
it("lists this PC's WSL distributions first and adds one under the core's name with the same consent (B4)", async () => {
  const { events, q, answer, unmount } = await mount(hosts([host("lab")], { wsl: [distro("Debian"), distro("Ubuntu", { default: true })] }));
  const rows = [...document.querySelectorAll("[data-wsl-distro], [data-ssh-host]")].map((row) => row.getAttribute("data-wsl-distro") ?? row.getAttribute("data-ssh-host"));
  expect(rows).toEqual(["Ubuntu", "Debian", "lab"]);
  await act(async () => { q('[data-wsl-distro="Ubuntu"] [role="radio"]')?.click(); });
  // Choosing it is the one look, through wsl.exe; Add waits for its answer.
  expect(sent(events, "pairing_look_wsl").map((event) => event.payload.distro)).toEqual(["Ubuntu"]);
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await answer("look_wsl", { ok: true, distro: "Ubuntu", looked: { state: "absent" } });
  const name = q("[data-device-label]") as HTMLInputElement;
  expect(name.value).toBe("WSL Ubuntu");
  expect(name.hasAttribute("disabled")).toBe(true);
  // The bridged engine is the core's to find: no socket to type for a distribution.
  expect(q("[data-device-socket]")).toBeNull();
  expect(q('[data-add-device-installs="true"]')?.querySelectorAll("li")).toHaveLength(3);
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  const register = sent(events, "register_device");
  expect(register).toHaveLength(1);
  expect(register[0]?.payload).toEqual({ wsl_distro: "Ubuntu", host_consent: true });
  await unmount();
});

// PRD device-connect-iroh D-27, B43: a distribution that runs its own Hide is connected to as a
// Hide through wsl.exe, and nothing is installed in it.
it("connects to a distribution whose own Hide runs, and installs nothing there (D-27)", async () => {
  const { events, q, answer, paired, unmount } = await mount(hosts([], { wsl: [distro("Ubuntu", { default: true })] }));
  await act(async () => { q('[data-wsl-distro="Ubuntu"] [role="radio"]')?.click(); });
  await answer("look_wsl", { ok: true, distro: "Ubuntu", looked: { state: "ready" } });
  expect(q('[data-ssh-look="ready"]')?.textContent).toContain("Hide is already there · this only connects");
  expect(q("[data-device-label]")).toBeNull();
  await act(async () => { q('[data-add-device="connect"]')?.click(); });
  expect(sent(events, "pairing_pair_wsl").map((event) => event.payload.distro)).toEqual(["Ubuntu"]);
  expect(sent(events, "register_device")).toHaveLength(0);
  // WSL could not run in it: one line says so, and nothing was added.
  await answer("pair_wsl", { ok: false, reason: "wsl_not_reached" });
  expect(q('[data-ssh-pair-refused="wsl_not_reached"]')?.textContent).toBe("WSL could not open this distribution");
  await act(async () => { q('[data-add-device="connect"]')?.click(); });
  await answer("pair_wsl", { ok: true, device_id: "distro-machine", name: "Ubuntu" });
  expect(paired).toHaveBeenCalledTimes(1);
  await unmount();
});

// B28, and WSL 1 left to its own command: a distribution that cannot connect is dimmed with why, and
// one already added says under which name.
it("dims a distribution that is added, unfinished or on WSL 1, with why (B28)", async () => {
  const { q, unmount } = await mount(hosts([], { wsl: [distro("Ubuntu", { added_as: "WSL Ubuntu" }), distro("Fresh", { problem: "setup_unfinished" }), distro("Legacy", { wsl_version: 1, problem: "wsl1" }), distro("Arch")] }));
  expect(q('[data-wsl-distro-note="Ubuntu"]')?.textContent).toBe("Added as WSL Ubuntu");
  expect(q('[data-wsl-distro-note="Fresh"]')?.textContent).toBe("Finish setting up the distribution first");
  expect(q('[data-wsl-command="Legacy"]')?.textContent).toBe("wsl --set-version Legacy 2");
  for (const name of ["Ubuntu", "Fresh", "Legacy"]) expect(q(`[data-wsl-distro="${name}"] [role="radio"]`)?.hasAttribute("disabled")).toBe(true);
  expect(q('[data-wsl-distro="Arch"] [role="radio"]')?.hasAttribute("disabled")).toBe(false);
  // Distributions to choose from are a list, not the empty ~/.ssh/config hint.
  expect(q('[data-ssh-hosts-state="empty"]')).toBeNull();
  await unmount();
});

// B25: a listing that could not read the distributions says so; it never reads as "no WSL".
it("says the distributions could not be read rather than showing none (B25)", async () => {
  const { q, unmount } = await mount(hosts([host("lab")], { wsl: [], wsl_problem: "unreadable" }));
  expect(q('[data-wsl-problem="unreadable"]')?.textContent).toBe("This PC's WSL distributions couldn't be read. Reopen this to try again.");
  await unmount();
});
