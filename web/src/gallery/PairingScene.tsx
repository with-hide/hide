// Pairing two Hides over an invented pair of machines (PRD
// device-connect-iroh B1-B6, B21, B24-B27, B50): `view=devices` is Settings ›
// Devices with a shared link and a paired machine in each state a row can be
// in, and the MacBook's two terminals here so its removal offers to end them;
// `view=allow` and `view=request` are the allow window, with and without
// words; `view=join` is the pasted link's words, and `view=failed` how a
// pairing ended; `view=other` is the same Devices list on a screen that is
// not this computer's own window, which only lists (B52); `view=switch` is
// three helper devices on the old connection (B35, B36, B41): one that
// offers the switch, one whose switch is connecting and one whose switch
// stopped, with the strip of the one in front (`front=`) over the list, and
// `view=held` the question a later computer is asked first (B69). It proves
// the rendered parts; the pairing is the daemon's.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { PairingDialogs } from "../PairingDialogs";
import type { PairingView, Peer } from "../pairing";
import { DeviceSwitchDialogs, DeviceSwitchStrip } from "../settings/DeviceSwitch";
import { DevicesTab } from "../settings/DevicesTab";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { standInHost } from "../practice/mount";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

const VIEWS = ["devices", "other", "allow", "request", "join", "failed", "switch", "held"] as const;
type View = (typeof VIEWS)[number];

const WORDS: Record<string, string[]> = { ko: ["바다", "연필", "구름", "사과"], en: ["engine", "bicycle", "muscle", "into"] };
const key = (letter: string) => letter.repeat(64);
const peer = (letter: string, name: string, over: Partial<Peer>): Peer => ({ key: key(letter), device_id: name.toLowerCase().replaceAll(" ", "-"), name, opens: null, opened_by: null, connected: true, path: "direct", reason: null, update: null, ...over });

/** One machine per state a row says (B21, B25, B27, B50, B51). */
const PEERS: Peer[] = [
  peer("a", "MacBook", { opened_by: "active", path: "relay" }),
  peer("b", "Mac mini", { opens: "active", opened_by: "active" }),
  peer("c", "Windows PC", { opens: "active", connected: false, path: null, reason: "relay_unreachable" }),
  peer("d", "연구실 빌드 서버 자동화 장비", { opens: "paused", connected: false, path: null }),
  peer("e", "Studio", { opens: "paused_there" }),
  peer("f", "Old laptop", { opens: "unpaired", connected: false, path: null }),
  peer("g", "Office PC", { opens: "removing", connected: false, path: null }),
  peer("h", "Linux box", { opens: "active", connected: false, path: null, update: "there" }),
];

/** The MacBook runs two terminals here, so removing it offers to end them (B26). */
const GUEST_PANES = [{ device_id: "macbook", key: key("a"), panes: [{ id: "guest:macbook:pane:w1:p1", pane: "w1:p1", since_unix_ms: 0 }, { id: "guest:macbook:pane:w1:p2", pane: "w1:p2", since_unix_ms: 0 }] }];

const LOCAL: Device = { agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };

const helper = (id: string, label: string, over: Partial<Device> = {}): Device => ({ ...LOCAL, id, label, kind: "remote", state: "ready", ssh_alias: id, ...over });

/** One helper device per state its switch can be in (B35, B36, B41). */
const HELPERS: Device[] = [
  helper("build-box", "build-box"),
  helper("mac-mini", "Mac mini", { switch: { state: "connecting" } }),
  helper("windows-pc", "Windows PC", { switch: { state: "failed", reason: "ssh_not_reached" } }),
];

/** This computer runs three terminals on the Windows PC, which a switch with a folder of its own closes (B69). */
const HELPER_PROJECTS = [{ id: "bot", device_id: "windows-pc", removal: { pane_count: 3, running_agent_count: 0 }, checkouts: [] }];

function pairingOf(view: View, lang: string): PairingView | null {
  // hided sends such a screen no `pairing` frame.
  if (view === "other") return null;
  const words = WORDS[lang] ?? WORDS.en!;
  const soon = Date.now() + 101_000;
  const idle: PairingView = { offer: null, asks: [], join: null, requests: [] };
  if (view === "switch" || view === "held") return idle;
  if (view === "devices") return { ...idle, offer: { link: "hide://pair#v=1&id=7f3a9c1e…&dev=windows-pc&relay=https%3A%2F%2Fdevice-relay.withhide.dev&s=4be2…", expires_unix_ms: Date.now() + 521_000 }, requests: [{ key: key("a"), state: "declined" }] };
  if (view === "allow") return { ...idle, asks: [{ id: 1, key: key("a"), device: "macbook", name: "MacBook", kind: "link", words, expires_unix_ms: soon }] };
  if (view === "request") return { ...idle, asks: [{ id: 2, key: key("c"), device: "windows-pc", name: "Windows PC", kind: "request", words: null, expires_unix_ms: soon }] };
  if (view === "join") return { ...idle, join: { id: 3, host: key("c"), phase: "comparing", failure: null, name: "Windows PC", words, expires_unix_ms: soon } };
  return { ...idle, join: { id: 4, host: key("c"), phase: "failed", failure: "declined", name: "Windows PC", words: null, expires_unix_ms: null } };
}

export function PairingScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown pairing language ${lang}`);
  const view = (params.get("view") ?? "devices") as View;
  if (!VIEWS.includes(view)) throw new Error(`Unknown pairing view ${view}`);
  const actions = useMemo(() => createActions(() => true), []);
  // `firewall=missing`: a Windows PC whose firewall rules are not in place
  // (B32), drawn through a stand-in of the desktop host's two firewall calls.
  const firewall = params.get("firewall") === "missing";
  useLayoutEffect(() => {
    if (!firewall) return;
    const before = window.hideHost;
    window.hideHost = Object.assign(standInHost("win32"), {
      firewallState: async () => "missing" as const,
      allowFirewall: async () => "declined" as const,
    });
    return () => {
      window.hideHost = before;
    };
  }, [firewall]);
  const switching = view === "switch" || view === "held";
  const front = params.get("front") ?? "build-box";
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      pairing: pairingOf(view, lang),
      pairingResult: view === "held" ? { verb: "switch", ok: true, switch: "held", by: lang === "ko" ? "회사 Mac" : "Office Mac", device_id: "windows-pc", request_id: "gallery" } : null,
      rest: {
        navigator: { devices: switching ? [LOCAL, ...HELPERS] : [LOCAL], peers: view === "devices" || view === "other" ? PEERS : [], guest_panes: view === "devices" ? GUEST_PANES : [], focused_device_id: switching ? front : "local", workspaces: switching ? HELPER_PROJECTS : [] },
        status: { remote: switching ? HELPERS.map((device) => ({ target_id: device.id, state: "connected", message: null })) : [] },
        ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" },
      },
    } as never);
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, view, switching, front]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="pairing" data-scene-ready="true">
          {switching ? <DeviceSwitchStrip actions={actions} /> : null}
          {view === "devices" || view === "other" || switching ? <div className="max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-xl py-lg"><DevicesTab actions={actions} /></div> : null}
          <PairingDialogs actions={actions} />
          <DeviceSwitchDialogs actions={actions} />
        </div>
      </TooltipProvider>
    </>
  );
}
