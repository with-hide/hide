// The core move's screens on invented data (PRD core-host-node-move B2 to B5,
// B10, B16, W1 to W3): the machines and the move frames each state of the
// `core-move` and `core-move-window` scenes draws. Every name is example data.

import type { CoreLink, MoveView } from "../coreMove";
import type { Device, DeviceHost, Kit, SnapshotRest } from "../snapshot";
import { REFERENCE_FOLDS, sidebarScene, type SceneContent } from "./sceneData";

/** The states a frame of `design/review-targets.json` names, in the order the review lists them. */
export const CORE_MOVE_STATES = ["devices", "devices-menu", "devices-offline", "node-menu", "checks-failed", "confirm", "moving", "failed", "done", "waiting", "give-up", "take-here", "abandoned"] as const;
export const CORE_WINDOW_STATES = ["rail-node", "rail-core", "window-moving", "window-updating", "window-unreachable", "window-disconnected", "window-waiting"] as const;
export type CoreMoveState = (typeof CORE_MOVE_STATES)[number] | (typeof CORE_WINDOW_STATES)[number];

/**
 * Where the window stands: on the core's machine with only devices it dials
 * (`core`), on the core's machine while another machine dials in (`linked`),
 * or on the machine that dials in (`node`).
 */
export type CoreMoveSide = "core" | "linked" | "node";

/** The machine that dials in, in the node's view: the scene's remote device stands for the MacBook. */
export const NODE_DEVICE = "mini";

const host = (platform: string): DeviceHost => ({ consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: "SHA256:example", granted_at_unix_ms: 1, state: "ready", message: null, platform, helper_path: "~/.hide/host-helper/current/hided" });

const quietKit: Kit = { unavailable: null, busy: false, components: [{ id: "cli", label: "hide command", state: "installed", reason: null, location: "~/.local/bin/hide" }], agents: [], offers_reinstall: false, shares_account_with: null };

/**
 * The snapshot for `side`: on the core's side This Mac runs the core and
 * dials Mac mini and build-box; otherwise Mac mini runs it and the MacBook
 * dials in, and on the node's side the window is the MacBook's (its hash
 * names it).
 */
export function coreMoveRest(side: CoreMoveSide, content: SceneContent, nowMs: number): { rest: SnapshotRest; agents: ReturnType<typeof sidebarScene>["agents"] } {
  const scene = sidebarScene(content, { ...REFERENCE_FOLDS, frontDevice: side === "node" ? NODE_DEVICE : "local" }, nowMs, "two");
  const navigator = scene.rest.navigator!;
  const devices = (navigator.devices ?? []).map((device): Device => {
    // The core's own row has no label of its own; the mini keeps the one it was registered under before the core moved there.
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
  const remote = (scene.rest.status?.remote ?? []).map((row) => (row.target_id === NODE_DEVICE ? { ...row, herdr_version: "0.9.3" } : { ...row, state: "unavailable", herdr_version: "0.9.1" }));
  return { rest: { ...scene.rest, navigator: { ...navigator, devices }, status: { ...scene.rest.status, remote } } as SnapshotRest, agents: scene.agents };
}

const view = (over: Partial<MoveView>): MoveView => ({ state: "idle", direction: "forward", device: "mini", intent: "move-example", sent: 0, total: 0, failed: [], step: null, cause: null, node: null, ...over });

/** The supervisor's frame each dialog state answers with. */
export const DIALOG_VIEWS: Partial<Record<CoreMoveState, MoveView>> = {
  "checks-failed": view({ state: "checks_failed", intent: null, checked: 11, failed: [{ check: "gh", detail: "gh auth status: not logged in" }, { check: "sleep", detail: "sleep 1 on AC power" }] }),
  confirm: view({ state: "ready", checked: 11 }),
  moving: view({ state: "copying", step: "copy", sent: 3, total: 5 }),
  failed: view({ state: "rolled_back", step: "start_target", cause: { kind: "start_target" } }),
  done: view({ state: "done", step: "reattach" }),
  // Amendment 4: the peer took the move's link and then stopped answering.
  waiting: view({ state: "waiting", step: "reattach", cause: { kind: "unreachable" }, abandon: true }),
  "give-up": view({ state: "waiting", step: "reattach", cause: { kind: "unreachable" }, abandon: true }),
  "take-here": view({ state: "waiting", direction: "back", device: null, step: "start_target", cause: { kind: "unreachable" }, abandon: true }),
  abandoned: view({ state: "abandoned" }),
};

/** The dialog states that open at the give-up confirm, as the strip's action opens it. */
export const GIVE_UP_STATES: ReadonlySet<CoreMoveState> = new Set(["give-up", "take-here"]);

/** The window states: the move holding the window, or where the node's link stands. */
export const WINDOW_STATES: Partial<Record<CoreMoveState, { connection: "moving" | "connecting" | "reconnecting"; move?: MoveView; link?: CoreLink }>> = {
  "window-moving": { connection: "moving", move: view({ state: "copying", step: "copy" }) },
  "window-updating": { connection: "connecting", link: { phase: "updating", machine: "Mac mini", alias: "mini" } },
  "window-unreachable": { connection: "reconnecting", link: { phase: "waiting", machine: "Mac mini", alias: "mini" } },
  "window-disconnected": { connection: "reconnecting", link: { phase: "disconnected", machine: null, alias: "mini" } },
  "window-waiting": { connection: "moving", move: view({ state: "waiting", step: "reattach", cause: { kind: "unreachable" }, abandon: true }) },
};

/** Which side each state is drawn from. */
export function sideOf(state: CoreMoveState): CoreMoveSide {
  if (state === "rail-core") return "linked";
  return state === "node-menu" || state === "take-here" || state === "rail-node" || state === "window-updating" || state === "window-unreachable" || state === "window-disconnected" ? "node" : "core";
}

/** The device whose ⋯ menu a state opens, and the item it points at. */
export const MENU_STATES: Partial<Record<CoreMoveState, { device: string; item: string }>> = {
  "devices-menu": { device: "mini", item: '[data-device-move="mini"]' },
  "devices-offline": { device: "build-box", item: '[data-device-move="build-box"]' },
  "node-menu": { device: NODE_DEVICE, item: `[data-device-move-back="${NODE_DEVICE}"]` },
};
