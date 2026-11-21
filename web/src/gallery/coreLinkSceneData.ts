// A core that works with another machine, on invented data (PRD
// core-host-node-move B10, B16, W2, W3): the machines and the link frames each
// state of the `core-link` and `core-link-window` scenes draws. Every name is
// example data.

import type { CoreLink } from "../coreLink";
import type { Device, DeviceHost, Kit, SnapshotRest } from "../snapshot";
import { REFERENCE_FOLDS, sidebarScene, type SceneContent } from "./sceneData";

/** The states a frame of `design/review-targets.json` names, in the order the review lists them. */
export const CORE_LINK_STATES = ["devices", "device-kit-failed", "device-unsupported", "node-menu", "node-split-failed"] as const;
export const CORE_WINDOW_STATES = ["rail-node", "rail-core", "window-updating", "window-unreachable", "window-sign-in", "window-disconnected", "window-asks", "window-host-key", "window-host-key-changed"] as const;
export type CoreLinkState = (typeof CORE_LINK_STATES)[number] | (typeof CORE_WINDOW_STATES)[number];

/**
 * Where the window stands: on the core's machine with only devices it dials
 * (`core`), on the core's machine while another machine dials in (`linked`),
 * or on the machine that dials in (`node`).
 */
export type CoreLinkSide = "core" | "linked" | "node";

/** The machine that dials in, in the node's view: the scene's remote device stands for the MacBook. */
export const NODE_DEVICE = "mini";

const host = (platform: string): DeviceHost => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: "SHA256:example", granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: "~/.hide/host-helper/current/hided" });

const quietKit: Kit = { unavailable: null, busy: false, components: [{ id: "cli", label: "hide command", state: "installed", why: null, location: "~/.local/bin/hide" }], agents: [], offers_reinstall: false, shares_account_with: null };

/**
 * The snapshot for `side`: on the core's side This Mac runs the core and
 * dials Mac mini and build-box; otherwise Mac mini runs it and the MacBook
 * dials in, and on the node's side the window is the MacBook's (its hash
 * names it).
 */
export function coreLinkRest(side: CoreLinkSide, content: SceneContent, nowMs: number): { rest: SnapshotRest; agents: ReturnType<typeof sidebarScene>["agents"] } {
  const scene = sidebarScene(content, { ...REFERENCE_FOLDS, frontDevice: side === "node" ? NODE_DEVICE : "local" }, nowMs, "two");
  const navigator = scene.rest.navigator!;
  const devices = (navigator.devices ?? []).map((device): Device => {
    // The core's own row has no label of its own; the mini keeps the one it was registered under.
    if (device.kind !== "remote") return side === "core" ? { ...device, label: null, machine_name: "MacBook-Pro.local", kit: quietKit } : { ...device, label: "Mac mini", machine_name: "Mac-mini.local", kit: quietKit, host: host("macos-arm64") };
    // The scene's remote device stands for Mac mini on the core's side and for the MacBook otherwise.
    if (device.id === NODE_DEVICE) {
      return side === "core"
        ? { ...device, label: "Mac mini", host: host("macos-arm64"), kit: quietKit }
        : { ...device, label: "MacBook Pro", ssh_alias: null, dials_in: true, host: host("macos-arm64"), kit: quietKit };
    }
    return { ...device, label: "build-box", host: host("linux-x86_64"), kit: quietKit };
  });
  // build-box answered once and is not reachable now, as the bundle draws it.
  const remote = (scene.rest.status?.remote ?? []).map((row) => (row.target_id === NODE_DEVICE ? row : { ...row, state: "unavailable" }));
  return { rest: { ...scene.rest, navigator: { ...navigator, devices }, status: { ...scene.rest.status, remote } } as SnapshotRest, agents: scene.agents };
}

/** The window states: where the node's link stands. */
export const WINDOW_STATES: Partial<Record<CoreLinkState, { connection: "connecting" | "reconnecting"; link: CoreLink }>> = {
  "window-updating": { connection: "connecting", link: { phase: "updating", machine: "Mac mini", alias: "mini" } },
  "window-unreachable": { connection: "reconnecting", link: { phase: "waiting", machine: "Mac mini", alias: "mini", splits: true } },
  "window-sign-in": { connection: "reconnecting", link: { phase: "stopped", machine: null, alias: "mini", problem: "sign_in_needed" } },
  "window-disconnected": { connection: "reconnecting", link: { phase: "disconnected", machine: null, alias: "mini" } },
  // PRD system-ssh-device-link B33: what ssh asks the dial the operator's Retry started, under the strip.
  "window-asks": { connection: "reconnecting", link: { phase: "connecting", machine: null, alias: "mini", sign_in: { question: 3, kind: "secret", prompt: "al@mini's password: ", key_command: null } } },
  "window-host-key": { connection: "reconnecting", link: { phase: "connecting", machine: null, alias: "mini", sign_in: { question: 1, kind: "host_key", host: "mini.example.ts.net (100.64.0.12)", fingerprint: "SHA256:Q2nT0x9a8pKf3vLr7YbW5cHs1dJe6uMg4iNz0oPqRt8", key_command: null } } },
  "window-host-key-changed": { connection: "reconnecting", link: { phase: "stopped", machine: null, alias: "mini", problem: "host_key_changed" } },
};

/** Which side each state is drawn from. */
export function sideOf(state: CoreLinkState): CoreLinkSide {
  if (state === "rail-core") return "linked";
  return state === "node-menu" || state === "node-split-failed" || state === "rail-node" || state === "window-updating" || state === "window-unreachable" || state === "window-sign-in" || state === "window-disconnected" || state === "window-asks" || state === "window-host-key" || state === "window-host-key-changed" ? "node" : "core";
}

/** The device whose ⋯ menu a state opens, and the item it points at. */
export const MENU_STATES: Partial<Record<CoreLinkState, { device: string; item: string }>> = {
  "node-menu": { device: NODE_DEVICE, item: `[data-device-core-disconnect="${NODE_DEVICE}"]` },
};
