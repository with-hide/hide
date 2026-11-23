// Settings of a Windows PC's Hide, opened in two windows (docs/UI_BEHAVIOR.md,
// Settings: each machine's install kit): `window=remote` is the PC's screen
// that a Mac shows, whose desktop app calls the PC `window`, and
// `window=own` is the PC's own window. The PC's own row carries no system in
// the remote case, as a core that has not told one, so the title comes from
// what the showing window says alone. The bridge here is a stand-in for the
// desktop's remote preload: it says only who shows the screen.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { NOTHING_UNDER_WAY } from "../pairing";
import { SettingsGate } from "../SettingsSheet";
import type { Device, DeviceHost } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

/** What the showing Mac calls the PC in the remote case. */
const SHOWN_AS = "window";

/** The PC's own window knows its system, as its desktop app tells it. */
const PC_HOST: DeviceHost = { consent: "this_machine", helper_root: null, cli_dir: null, contract: 1, bound_identity: null, granted_at_unix_ms: null, state: "ready", message: null, platform: "windows x86_64", helper_path: null };

/** Puts the stand-in bridge on the page before anything hears the viewer. */
function standInBridge(lang: string): void {
  (window as unknown as { hideRemote?: unknown }).hideRemote = {
    bridge: 1,
    socket: { events: () => undefined, open: () => undefined, send: () => undefined, close: () => undefined },
    clipboardWrite: async () => false,
    onMenuCommand: () => undefined,
    onVisible: () => undefined,
    onViewer: (listener: (viewer: { language: string; name: string | null }) => void) => listener({ language: lang, name: SHOWN_AS }),
  };
}

export function RemoteSettingsScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown language ${lang}`);
  const shown = params.get("window") ?? "remote";
  if (shown !== "remote" && shown !== "own") throw new Error(`Unknown settings window ${shown}`);
  if (shown === "remote") standInBridge(lang);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    const pc: Device = { agent_scope: EMPTY_GALLERY_SCOPE, id: "windows-pc", label: null, machine_name: "DESKTOP-7Q2", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, ...(shown === "own" ? { host: PC_HOST } : {}) };
    useShellStore.setState({
      connection: "live",
      pairing: NOTHING_UNDER_WAY,
      rest: { navigator: { devices: [pc], peers: [], focused_device_id: "windows-pc", workspaces: [] }, status: { remote: [] }, ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" } },
    } as never);
    useUiStore.setState({ overlay: "settings", settingsTab: "general" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, shown]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="h-screen w-full bg-background" data-gallery-scene="remote-settings" data-scene-window={shown} data-scene-ready="true" />
      <SettingsGate actions={actions} />
    </TooltipProvider>
  );
}
