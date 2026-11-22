// The rail's computer tiles (PRD device-connect-iroh D-37, B17, B21, B47):
// this computer, each paired machine and each SSH or WSL device not yet
// switched to a Hide of its own, one group before `+`, numbered together by
// ⌥⌘n. A paired machine's tile shows that computer's own Hide screen,
// another device's tile shows that device's part of this one, as before.
// Which tile shows a screen and in what state it is are read here, from the
// snapshot, so the rail, the number keys and the stage agree.

import type { MessageKey } from "./i18n/catalogs";
import { railDeviceIds } from "./devices";
import { numberedPanes } from "./numbering";
import { openedMachines, pairedMachines, type Peer } from "./pairing";
import type { Digit } from "./shortcuts";
import type { SnapshotRest } from "./snapshot";

/**
 * One computer tile: `id` is what it is selected by, a device id or, for a
 * paired machine the core has not made a device of, that machine's id;
 * `screen` is the paired machine whose Hide screen it shows, null for a
 * device whose part of this Hide it shows.
 */
export type ComputerTile = { id: string; screen: Peer | null };

/**
 * The tiles top to bottom: the rail's devices in the core's order, the
 * window's own machine first, a device that is a paired machine (`peer_key`)
 * showing that machine's screen, then each paired machine this one opens
 * that is no device yet (`openedMachines`).
 */
export function computerTiles(rest: SnapshotRest | null | undefined): ComputerTile[] {
  const devices = rest?.navigator?.devices;
  const peers = pairedMachines(rest);
  const tiles = railDeviceIds(devices).map((id): ComputerTile => {
    const key = devices?.find((device) => device.id === id)?.peer_key;
    const peer = key ? peers.find((row) => row.key === key && row.opens != null) ?? null : null;
    return { id, screen: peer };
  });
  return [...tiles, ...openedMachines(rest).map((peer): ComputerTile => ({ id: peer.device_id, screen: peer }))];
}

/** The tile id at each number ⌥⌘n selects; a tile drawn before the core named its machine has none. */
export function numberedComputers(tiles: readonly ComputerTile[]): Map<Digit, string> {
  return numberedPanes(tiles.map((tile) => tile.id).filter((id) => id !== ""));
}

/**
 * Whether a paired machine's screen can be shown now, or why not (B21):
 * each reason has its own words, and none is a failure of this window.
 * `no_grant`: it does not let this computer in (it never did, took it back,
 * or is being removed). `update`: one side must update first (B22).
 */
export type ScreenState = "ready" | "not_connected" | "confirming" | "paused" | "paused_there" | "update_here" | "update_there" | "no_grant";

export function screenState(peer: Peer): ScreenState {
  if (peer.opens == null || peer.opens === "unpaired" || peer.opens === "removing") return "no_grant";
  // An ask to update changes who is asked, not which side must update (B22).
  if (peer.update === "here" || peer.update === "here_asked") return "update_here";
  if (peer.update === "there" || peer.update === "there_asked") return "update_there";
  if (peer.opens === "paused") return "paused";
  if (peer.opens === "paused_there") return "paused_there";
  if (peer.opens === "confirming") return "confirming";
  return peer.connected ? "ready" : "not_connected";
}

/** The words a tile's hint and the stage say for a screen that cannot show. */
export function screenStateKey(state: Exclude<ScreenState, "ready">): MessageKey {
  switch (state) {
    case "not_connected":
      return "pairing.state.notConnected";
    case "confirming":
      return "pairing.state.confirming";
    case "paused":
      return "pairing.state.paused";
    case "paused_there":
      return "pairing.state.pausedThere";
    case "update_here":
      return "pairing.state.updateHere";
    case "update_there":
      return "pairing.state.updateThere";
    case "no_grant":
      return "remoteScreen.noGrant";
  }
}

/**
 * The paired computer `deviceId` names, by its device id or its pairing's:
 * one that runs its own Hide, whose work started there is its own (PRD
 * device-connect-iroh D-59). `screen` is the pairing this window could show
 * its screen by, or null when there is none; null when `deviceId` is no
 * paired computer.
 */
export function pairedComputer(rest: SnapshotRest | null | undefined, deviceId: string): { screen: Peer | null } | null {
  const tile = computerTiles(rest).find((row) => row.screen !== null && (row.id === deviceId || row.screen.device_id === deviceId));
  if (tile) return { screen: tile.screen };
  return rest?.navigator?.devices?.some((device) => device.id === deviceId && device.peer_key) ? { screen: null } : null;
}
