// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import type { Device, SshHost, SshHosts, WslDistribution } from "../snapshot";
import { useShellStore } from "../store";
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
  await act(async () => {
    useShellStore.setState({ connection: "live", rest: { status: { ssh_hosts: sshHosts }, ui_state: {} } } as never);
    root.render(<AddDevice actions={actions} devices={devices} helperRoot="~/.hide/host-helper" cliDir="~/.local/bin" />);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  return { events, q, container, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const sent = (events: Parameters<DispatchFn>[0][], kind: string) => events.filter((event) => event.kind === kind);

it("asks hided for the Hosts once when it opens (B50)", async () => {
  const { events, unmount } = await mount(hosts([host("lab")]));
  expect(sent(events, "ssh_hosts_list")).toHaveLength(1);
  await unmount();
});

it("lists each Host with its address, and fills the name and adds only the alias and the name once one is chosen (B50)", async () => {
  const { events, q, unmount } = await mount(hosts([host("studio"), host("lab")]));
  expect(q('[data-ssh-host="studio"]')?.textContent).toContain("fixture@studio.example.invalid:22");
  // No username, port or key input, and no alias input.
  expect(q("[data-device-alias]")).toBeNull();
  expect(q('[data-add-device="true"]')?.hasAttribute("disabled")).toBe(true);
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
  expect((q("[data-device-label]") as HTMLInputElement).value).toBe("lab");
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  const register = sent(events, "register_device");
  expect(register).toHaveLength(1);
  expect(register[0]?.payload).toMatchObject({ id: "lab", label: "lab", ssh_alias: "lab", host_consent: true, herdr_socket_path: null });
  await unmount();
});

it("dims a Host a device already uses, or whose address is unknown, and cannot choose it (B51, B52)", async () => {
  const { q, unmount } = await mount(hosts([host("mini", { added_as: "Mac mini" }), host("twin", { added_as: "Mac mini" }), host("broken", { address: null, problem: "ssh_failed" }), host("lab")]));
  expect(q('[data-ssh-host-note="mini"]')?.textContent).toBe("Added as Mac mini");
  expect(q('[data-ssh-host-note="twin"]')?.textContent).toBe("Added as Mac mini");
  expect(q('[data-ssh-host-note="broken"]')?.textContent).toBe("ssh couldn't read this host");
  for (const alias of ["mini", "twin", "broken"]) expect(q(`[data-ssh-host="${alias}"] [role="radio"]`)?.hasAttribute("disabled")).toBe(true);
  expect(q('[data-ssh-host="lab"] [role="radio"]')?.hasAttribute("disabled")).toBe(false);
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

it("keeps what Hide installs and the Herdr socket under Advanced, three lines long (B53)", async () => {
  const { q, unmount } = await mount(hosts([host("lab")]));
  const advanced = q('[data-add-device-advanced="true"]') as HTMLDetailsElement;
  expect(advanced.open).toBe(false);
  expect(q('[data-add-device-installs="true"]')?.querySelectorAll("li")).toHaveLength(3);
  expect(advanced.querySelector("[data-device-socket]")).not.toBeNull();
  await unmount();
});

// The core clears `last_error` at its next event of any kind, so a refusal that only lived there would
// let the form read "no error yet" as "still pending" and lock itself again (code-web 3, `useRefusalSince`).
it("keeps a refused Add enabled with its reason after the core clears last_error", async () => {
  const { q, unmount } = await mount(hosts([host("lab")]));
  await act(async () => { q('[data-ssh-host="lab"] [role="radio"]')?.click(); });
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
  const { events, q, unmount } = await mount(hosts([host("lab")], { wsl: [distro("Debian"), distro("Ubuntu", { default: true })] }));
  const rows = [...document.querySelectorAll("[data-wsl-distro], [data-ssh-host]")].map((row) => row.getAttribute("data-wsl-distro") ?? row.getAttribute("data-ssh-host"));
  expect(rows).toEqual(["Ubuntu", "Debian", "lab"]);
  await act(async () => { q('[data-wsl-distro="Ubuntu"] [role="radio"]')?.click(); });
  const name = q("[data-device-label]") as HTMLInputElement;
  expect(name.value).toBe("WSL Ubuntu");
  expect(name.hasAttribute("disabled")).toBe(true);
  // The bridged Herdr is the core's to find: no socket to type for a distribution.
  expect(q("[data-device-socket]")).toBeNull();
  expect(q('[data-add-device-installs="true"]')?.querySelectorAll("li")).toHaveLength(3);
  await act(async () => { q('[data-add-device="true"]')?.click(); });
  const register = sent(events, "register_device");
  expect(register).toHaveLength(1);
  expect(register[0]?.payload).toEqual({ wsl_distro: "Ubuntu", host_consent: true });
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
