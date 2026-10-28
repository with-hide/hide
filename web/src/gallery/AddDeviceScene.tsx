// The Add device dialog over an invented Windows PC (PRD wsl-device B4): its
// WSL distributions above an ssh config Host, each state a row can be in.
// It proves the rendered rows; the listing itself is the daemon's.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { InterfaceLanguageBoundary, useInterfaceTranslation } from "../i18n/client";
import { AddDevice } from "../settings/AddDevice";
import type { SshHosts, WslDistribution } from "../snapshot";
import { useShellStore } from "../store";

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

export function AddDeviceScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown add device language ${lang}`);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ connection: "live", rest: { status: { ssh_hosts: HOSTS }, ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" } } } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang]);
  return <><InterfaceLanguageBoundary /><div className="h-screen w-full" data-gallery-scene="add-device" data-scene-ready="true"><Body actions={actions} /></div></>;
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
