// Pairing two Hides over an invented pair of machines (PRD
// device-connect-iroh B1-B6, B21, B24-B27, B50): `view=devices` is Settings ›
// Devices with a shared link and a paired machine in each state a row can be
// in, and the MacBook's two terminals here so its removal offers to end them;
// `view=allow` and `view=request` are the allow window, with and without
// words; `view=join` is the pasted link's words, and `view=failed` how a
// pairing ended; `view=other` is the same Devices list on a screen that is
// not this computer's own window, which only lists (B52); `view=ways`
// is the rows a device reached over SSH or WSL opens on (operator decision
// 2026-11-24, Doop canvas 1N77v1reLZ), one `frame` per state of that
// canvas: the list, each condition a machine can be in beside 서버로 연결,
// a switch under way, done, failed and refused, a switched machine opened,
// one that restarted with nobody logged in, and how to turn automatic
// login on;
// `view=update` is two Hides too far apart to connect (B22): the
// computer this one asked to update, the one that asked this computer, and
// this computer's own notice in the corner. It proves the rendered parts;
// the pairing is the daemon's.
import { useEffect, useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { ToastStack } from "../components/ui/toast";
import { TooltipProvider } from "../components/ui/tooltip";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { PairingDialogs } from "../PairingDialogs";
import { PeerUpdateNotice } from "../PeerUpdateNotice";
import type { PairingResult, PairingView, Peer, SwitchCondition } from "../pairing";
import { DevicesTab } from "../settings/DevicesTab";
import type { UpdateStatus } from "../host";
import type { Device, DeviceSwitchStep } from "../snapshot";
import type { SwitchOutcome } from "../settings/switchWays";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { standInHost } from "../practice/mount";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

const VIEWS = ["devices", "other", "allow", "request", "join", "failed", "ways", "update"] as const;
type View = (typeof VIEWS)[number];

const WORDS: Record<string, string[]> = { ko: ["바다", "연필", "구름", "사과"], en: ["engine", "bicycle", "muscle", "into"] };
const key = (letter: string) => letter.repeat(64);
const peer = (letter: string, name: string, over: Partial<Peer>): Peer => ({ key: key(letter), device_id: name.toLowerCase().replaceAll(" ", "-"), name, opens: null, opened_by: null, connected: true, path: "direct", reason: null, update: null, ...over });

/** One machine per state a row says (B21, B25, B27, B50, B51). */
const PEERS: Peer[] = [
  peer("a", "MacBook", { opened_by: "active", path: "relay" }),
  peer("b", "Mac mini", { opens: "active", opened_by: "active" }),
  peer("i", "Home server", { opens: "active", path: "tailscale" }),
  peer("c", "Windows PC", { opens: "active", connected: false, path: null, reason: "relay_unreachable" }),
  peer("d", "연구실 빌드 서버 자동화 장비", { opens: "paused", connected: false, path: null }),
  peer("e", "Studio", { opens: "paused_there" }),
  peer("f", "Old laptop", { opens: "unpaired", connected: false, path: null }),
  peer("g", "Office PC", { opens: "removing", connected: false, path: null }),
  peer("h", "Linux box", { opens: "active", connected: false, path: null, update: "there" }),
  // The Hide this computer started there over SSH or wsl.exe, which it
  // keeps at its own version (B23).
  peer("0", "render-box", { opens: "active", update: "updating" }),
  peer("1", "WSL Debian", { opens: "active", path: "local", update: "update_rolled_back" }),
];

/** Hides too far apart to connect (B22): one this computer opens and may ask, one it asked, and one that opens this computer and asked it. */
const UPDATE_PEERS: Peer[] = [
  peer("k", "Linux box", { opens: "active", connected: false, path: null, update: "there" }),
  peer("l", "Windows PC", { opens: "active", connected: false, path: null, update: "there_asked" }),
  peer("m", "MacBook", { opened_by: "active", connected: false, path: null, update: "here_asked" }),
];

/** The MacBook runs two terminals here, so removing it offers to end them (B26). */
const GUEST_PANES = [{ device_id: "macbook", key: key("a"), panes: [{ id: "guest:macbook:pane:w1:p1", pane: "w1:p1", since_unix_ms: 0 }, { id: "guest:macbook:pane:w1:p2", pane: "w1:p2", since_unix_ms: 0 }] }];

const LOCAL: Device = { agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };

const helper = (id: string, label: string, over: Partial<Device> = {}): Device => ({ ...LOCAL, id, label, kind: "remote", state: "ready", ssh_alias: id, ...over });

const MAC = { platform: "macos aarch64", consent: "granted", state: "ready" } as Device["host"];
const LINUX = { platform: "linux x86_64", consent: "granted", state: "ready" } as Device["host"];
const WINDOWS = { platform: "windows x86_64", consent: "granted", state: "ready" } as Device["host"];
/** A device that was switched to its machine's own Hide: the paired machine's device, under the id it had. */
const server = (id: string, label: string, letter: string, host: Device["host"], over: Partial<Device> = {}): Device => helper(id, label, { ssh_alias: null, peer_key: key(letter), switched: true, host, ...over });

/**
 * One frame of the canvas: the devices listed, the paired machines among
 * them, the row that is open, what its machine answers a read with, and
 * what this window keeps of a switch.
 */
type WaysFrame = {
  devices: Device[];
  peers?: Peer[];
  open?: string;
  /** What the open device's machine says when it is read; a string is the reason the read was refused. */
  look?: SwitchCondition | string;
  step?: DeviceSwitchStep;
  outcome?: SwitchOutcome;
  /** The device switched while this window watched, which says once what starts fresh there. */
  landed?: string;
  /** Devices that are not connected. */
  down?: string[];
};

const MINI = helper("mac-mini", "Mac mini", { host: MAC });
const BUILD = helper("build-server", "build-server", { host: LINUX });
const WSL = helper("wsl-ubuntu", "WSL Ubuntu", { ssh_alias: null, wsl_distro: "Ubuntu", host: LINUX });
const PC = helper("windows-pc", "Windows PC", { host: WINDOWS });
const mini = (look: WaysFrame["look"], over: Partial<WaysFrame> = {}): WaysFrame => ({ devices: [MINI], open: "mac-mini", look, ...over });
const build = (look: WaysFrame["look"], over: Partial<WaysFrame> = {}): WaysFrame => ({ devices: [BUILD], open: "build-server", look, ...over });
const MINI_SERVER = { devices: [server("mac-mini", "Mac mini", "b", MAC)], peers: [peer("b", "Mac mini", { opens: "active" })] };

const WAYS: Record<string, WaysFrame> = {
  "2": { devices: [MINI, server("build-server", "build-server", "i", LINUX), PC], peers: [peer("i", "build-server", { opens: "active" })] },
  "7": { devices: [MINI, BUILD, WSL] },
  "3-a": mini({ kind: "auto_login_on" }),
  "3-b": mini({ kind: "auto_login_off", how: true }),
  "3-b-filevault": mini({ kind: "auto_login_off", how: false }),
  "3-c": build({ kind: "linger_on" }),
  "3-d": build({ kind: "linger_off" }),
  "3-e": build({ kind: "linger_admin", command: "sudo loginctl enable-linger me" }),
  "3-f": { devices: [WSL], open: "wsl-ubuntu", look: { kind: "no_systemd" } },
  "3-g": { devices: [PC], open: "windows-pc" },
  "3-h": build("ssh_not_reached"),
  "3-i": mini({ kind: "signed_out" }),
  "4-a": mini({ kind: "auto_login_on" }, { step: "starting" }),
  "4-b": { ...MINI_SERVER, open: "mac-mini", look: { kind: "auto_login_on" }, landed: "mac-mini" },
  "4-c": mini({ kind: "auto_login_on" }, { outcome: { kind: "failed", reason: "ssh_core_not_started" } }),
  "4-d": mini({ kind: "auto_login_on" }, { outcome: { kind: "held", by: "Office MacBook", terminals: 3 } }),
  "5": { devices: [server("build-server", "build-server", "i", LINUX)], peers: [peer("i", "build-server", { opens: "active" })], open: "build-server", look: { kind: "linger_on" } },
  "5-b": { devices: MINI_SERVER.devices, peers: [peer("b", "Mac mini", { opens: "active", connected: false, path: null })], look: { kind: "signed_out" }, down: ["mac-mini"] },
  "6": mini({ kind: "auto_login_off", how: true }),
};

/** What the gallery's daemon answers a read of `device` with, in the frame shown. */
function lookAnswer(frame: WaysFrame, request_id: unknown, device: unknown): PairingResult {
  const base = { verb: "look_switch", request_id: typeof request_id === "string" ? request_id : null };
  if (typeof frame.look !== "object") return { ...base, ok: false, reason: frame.look ?? "device_unknown" };
  return { ...base, ok: true, device_id: String(device), condition: frame.look, user: "me" };
}

function pairingOf(view: View, lang: string): PairingView | null {
  // hided sends such a screen no `pairing` frame.
  if (view === "other") return null;
  const words = WORDS[lang] ?? WORDS.en!;
  const soon = Date.now() + 101_000;
  const idle: PairingView = { offer: null, asks: [], join: null, requests: [] };
  if (view === "ways" || view === "update") return idle;
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
  const frameId = params.get("frame") ?? "2";
  const frame = view === "ways" ? WAYS[frameId] : undefined;
  if (view === "ways" && !frame) throw new Error(`Unknown device ways frame ${frameId}`);
  // The gallery's daemon: a read of a device's machine is answered with the frame's condition, and nothing is switched.
  const actions = useMemo(
    () =>
      createActions((event) => {
        if (frame && event.kind === "pairing_look_switch") {
          const { request_id, device } = event.payload as { request_id?: unknown; device?: unknown };
          queueMicrotask(() => useShellStore.setState({ pairingResult: lookAnswer(frame, request_id, device) }));
        }
        return true;
      }),
    [frame],
  );
  // `firewall=missing`: a Windows PC whose firewall rules are not in place
  // (B32), drawn through a stand-in of the desktop host's two firewall calls.
  const firewall = params.get("firewall") === "missing";
  // `view=update`: the desktop app, whose updates the notice offers; the
  // stand-in reports no update found yet and does nothing on a press.
  const updating = view === "update";
  // `side=there`: the computer that waits for the other to update, whose notice asks it to.
  const updatePeers = params.get("side") === "there" ? UPDATE_PEERS.filter((row) => row.update !== "here_asked") : UPDATE_PEERS;
  useLayoutEffect(() => {
    if (!firewall && !updating) return;
    const before = window.hideHost;
    window.hideHost = Object.assign(
      standInHost(firewall ? "win32" : "darwin"),
      firewall ? { firewallState: async () => "missing" as const, allowFirewall: async () => "declined" as const } : {},
      updating
        ? {
            updates: {
              onStatus: (listener: (status: UpdateStatus) => void) => {
                listener({ phase: { kind: "idle", checkedAt: null }, prerelease: false, installOnQuit: true, dismissed: null, required: false });
                return () => undefined;
              },
              check: () => undefined,
              restart: () => undefined,
              dismiss: () => undefined,
              setPrerelease: () => undefined,
            },
          }
        : {},
    );
    return () => {
      window.hideHost = before;
    };
  }, [firewall, updating]);
  const ways = useMemo(() => frame?.devices.map((device) => (device.id === frame.open && frame.step ? { ...device, switch_step: frame.step } : device)) ?? [], [frame]);
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      pairing: pairingOf(view, lang),
      pairingResult: null,
      rest: {
        navigator: { devices: [ways.some((device) => device.wsl_distro) ? { ...LOCAL, host: WINDOWS } : LOCAL, ...ways.map((device) => (frame?.down?.includes(device.id) ? { ...device, state: "unavailable" } : device))], peers: view === "devices" || view === "other" ? PEERS : frame ? (frame.peers ?? []) : updating ? updatePeers : [], guest_panes: view === "devices" ? GUEST_PANES : [], focused_device_id: "local", workspaces: [] },
        status: { remote: ways.filter((device) => !frame?.down?.includes(device.id)).map((device) => ({ target_id: device.id, state: "connected", message: null })) },
        ui_state: {
          interface_language: lang as "en" | "ko" | "ja" | "zh-CN",
          // The two projects the switched machine's own Hide now lists too.
          workspace_registrations: frame?.landed ? ["app", "site"].map((name) => ({ id: `workspace:${name}`, label: name, path: `/home/me/${name}`, device_id: frame.landed, pinned: false, primary_checkout_id: null })) : [],
        },
      },
    } as never);
    useUiStore.setState({ switchedDevices: frame?.landed ? [frame.landed] : [], devicesOpen: frame?.open ? [frame.open] : [], switchOutcomes: frame?.outcome && frame.open ? { [frame.open]: frame.outcome } : {} });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, view, frame, ways, updating, updatePeers]);
  // Frame 6 is the steps 방법 opens: pressed once the read has drawn it.
  useEffect(() => {
    if (view !== "ways" || frameId !== "6") return;
    const timer = window.setInterval(() => {
      const how = document.querySelector<HTMLElement>("[data-device-auto-login-how]");
      if (!how) return;
      window.clearInterval(timer);
      how.click();
    }, 50);
    return () => window.clearInterval(timer);
  }, [view, frameId]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="pairing" data-scene-ready="true">
          {view === "devices" || view === "other" || view === "ways" || updating ? <div className="max-w-[calc(var(--size-settings-sheet-w)-var(--size-settings-nav-w))] px-xl py-lg"><DevicesTab actions={actions} /></div> : null}
          <PairingDialogs actions={actions} />
          {updating ? <ToastStack><PeerUpdateNotice actions={actions} /></ToastStack> : null}
        </div>
      </TooltipProvider>
    </>
  );
}
