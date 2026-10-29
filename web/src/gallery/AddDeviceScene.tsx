// The Add device dialog over an invented Windows PC (PRD wsl-device B4): its
// WSL distributions above an ssh config Host, each state a row can be in.
// It proves the rendered rows; the listing itself is the daemon's.
// `view=devices` is Settings › Devices on the same PC with its connected WSL
// distribution beside an SSH device (B5): the name and where it is.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary, useInterfaceTranslation } from "../i18n/client";
import { AddDevice } from "../settings/AddDevice";
import { DevicesTab } from "../settings/DevicesTab";
import type { Device, DeviceHost, SshHosts, WslDistribution } from "../snapshot";
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

export function AddDeviceScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown add device language ${lang}`);
  const view = params.get("view") ?? "add";
  if (view !== "add" && view !== "devices") throw new Error(`Unknown add device view ${view}`);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    const status = view === "devices" ? { remote: [WSL_ID, "studio"].map((id) => ({ target_id: id, state: "connected", message: null, herdr_version: "0.9.5" })) } : { ssh_hosts: HOSTS };
    const navigator = view === "devices" ? { devices: DEVICES, focused_device_id: WSL_ID, workspaces: [] } : undefined;
    useShellStore.setState({ connection: "live", rest: { navigator, status, ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, view]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="h-screen w-full bg-background text-foreground" data-gallery-scene="add-device" data-scene-ready="true">
          {view === "devices" ? <div className="px-xl py-lg"><DevicesTab actions={actions} /></div> : <Body actions={actions} />}
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
