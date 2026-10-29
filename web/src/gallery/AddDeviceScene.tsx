// The Add device dialog over an invented Windows PC (PRD wsl-device B4): its
// WSL distributions above an ssh config Host, each state a row can be in.
// It proves the rendered rows; the listing itself is the daemon's.
// `view=devices` is Settings › Devices on the same PC with its connected WSL
// distribution beside an SSH device (B5): the name and where it is.
// `view=sign-in` and `view=failures` are its SSH devices waiting on the
// operator (PRD system-ssh-device-link B4-B14) and stopped in each of the
// three failure states, sign in, server key and cannot connect, on a Mac, or
// with `system=windows` on the PC.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary, useInterfaceTranslation } from "../i18n/client";
import { AddDevice } from "../settings/AddDevice";
import { DevicesTab } from "../settings/DevicesTab";
import type { Device, DeviceHost, DeviceProblem, DeviceSignIn, SshHosts, WslDistribution } from "../snapshot";
import { useShellStore } from "../store";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

const distro = (name: string, over: Partial<WslDistribution> = {}): WslDistribution => ({ name, device_name: `WSL ${name}`, wsl_version: 2, default: false, added_as: null, problem: null, ...over });

const HOSTS: SshHosts = {
  state: "ready",
  truncated: false,
  hosts: [{ alias: "studio", address: "you@studio.example:22", added_as: null, problem: null }],
  wsl: [
    distro("Ubuntu-24.04", { default: true }),
    distro("Debian", { added_as: "WSL Debian" }),
    distro("Fedora", { problem: "setup_unfinished" }),
    distro("Ubuntu-20.04", { wsl_version: 1, problem: "wsl1" }),
  ],
};

const WSL_ID = "wsl-ubuntu-24-04-5d1c2a7e";
const host = (platform: string): DeviceHost => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 1, bound_identity: null, granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: null });
const device = (id: string, over: Partial<Device>): Device => ({ agent_scope: EMPTY_GALLERY_SCOPE, id, label: null, kind: "remote", state: "ready", message: null, ssh_alias: null, agent_count: 0, test: null, ...over });

/** This PC, its WSL Ubuntu and an SSH studio, both connected. */
const DEVICES: Device[] = [
  device("local", { kind: "local", state: "local", host: { ...host("windows x86_64"), consent: "this_machine" } }),
  device(WSL_ID, { label: "WSL Ubuntu-24.04", wsl_distro: "Ubuntu-24.04", host: { ...host("linux x86_64"), windows_drives: "/mnt" } }),
  device("studio", { label: "studio", ssh_alias: "studio", host: host("macos aarch64") }),
];

const MAC = device("local", { kind: "local", state: "local", host: { ...host("macos aarch64"), consent: "this_machine" } });
const waiting = (id: string, sign_in: DeviceSignIn | null, problem: DeviceProblem | null = null): Device =>
  device(id, { label: id, ssh_alias: id, state: "unavailable", sign_in, problem, host: { ...host("macos aarch64"), state: sign_in ? "connecting" : "unavailable" } });
const failed = (id: string, problem: DeviceProblem, message: string | null = null): Device => ({ ...waiting(id, null, problem), message });

/** What each SSH device waits on from the operator (B4, B6, B8, B9, B11, B12, B14). */
const SIGN_IN: Device[] = [
  waiting("studio", { question: 4, key_command: "ssh-add --apple-use-keychain ~/.ssh/id_ed25519", kind: "secret", prompt: "Enter passphrase for key '/Users/example/.ssh/id_ed25519':", key_file: "/Users/example/.ssh/id_ed25519" }),
  waiting("build-box", { question: 2, key_command: null, kind: "host_key", host: "[build-box.example]:2222", fingerprint: "SHA256:Jq8F2n1xR0a+e3uY7kz6m4W0cB9vT5pLhGd2sQ1oE8I" }),
  waiting("lab", { question: 3, key_command: null, kind: "browser", url: "https://login.tailscale.com/a/7f3e9c21d4" }),
  waiting("yubi-mini", { question: 5, key_command: null, kind: "touch", prompt: "Confirm user presence for key ED25519-SK SHA256:Qw3r…" }),
  waiting("pi", null, "sign_in_needed"),
];

/**
 * The failure states a device row draws: sign in (needed, refused), the
 * changed server key, cannot connect over ssh's last line or Hide's sentence
 * where ssh has none, and a Hide an update replaced, which only opening Hide
 * again mends.
 */
const FAILURES: Device[] = [
  failed("studio", "sign_in_needed"),
  failed("build-box", "sign_in_refused"),
  failed("lab", "host_key_changed"),
  failed("edge", "cannot_connect", "tailscale: tailnet policy does not permit you to SSH to this node"),
  failed("pi", "cannot_connect", "ssh: Could not resolve hostname pi: nodename nor servname provided, or not known"),
  failed("ci", "cannot_connect", "kex_exchange_identification: read: Connection reset by peer"),
  failed("old-box", "openssh_old"),
  failed("tower", "ssh_missing"),
  failed("mac-mini", "hide_updated"),
];

export function AddDeviceScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown add device language ${lang}`);
  const view = params.get("view") ?? "add";
  if (view !== "add" && view !== "devices" && view !== "sign-in" && view !== "failures") throw new Error(`Unknown add device view ${view}`);
  const windows = params.get("system") === "windows";
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    const ssh = view === "sign-in" ? SIGN_IN : view === "failures" ? FAILURES : null;
    const local = windows ? DEVICES[0]! : MAC;
    const status = ssh
      ? { remote: ssh.map((row) => ({ target_id: row.id, state: row.sign_in ? "connecting" : "unavailable", message: null, herdr_version: null })) }
      : view === "devices" ? { remote: [WSL_ID, "studio"].map((id) => ({ target_id: id, state: "connected", message: null, herdr_version: "0.9.5" })) } : { ssh_hosts: HOSTS };
    const navigator = ssh ? { devices: [local, ...ssh], focused_device_id: local.id, workspaces: [] } : view === "devices" ? { devices: DEVICES, focused_device_id: WSL_ID, workspaces: [] } : undefined;
    useShellStore.setState({ connection: "live", rest: { navigator, status, ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, view, windows]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="add-device" data-scene-ready="true">
          {view === "add" ? <Body actions={actions} /> : <div className="px-xl py-lg"><DevicesTab actions={actions} /></div>}
        </div>
      </TooltipProvider>
    </>
  );
}

function Body({ actions }: { actions: ReturnType<typeof createActions> }) {
  const { t } = useInterfaceTranslation();
  return (
    <Dialog open>
      <DialogContent data-add-device-dialog="true">
        <DialogHeader>
          <DialogTitle>{t("devices.addTitle")}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <AddDevice actions={actions} devices={[]} helperRoot="~/.hide/host-helper" cliDir="~/.local/bin" />
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
