// The machines this window's operator has paired with, as the desktop app
// keeps their screens' login storage (docs/BROWSER_DISPLAYS.md, A page in
// another computer's screen): each device row of a paired machine
// (`peer_key`), by its device and the key it was paired under. The core
// keeps those rows from its start until a told list of pairings ends one, so
// a daemon that has not heard from its machines yet names them all.

import { localDeviceId, type SnapshotRest } from "./snapshot";

/**
 * The paired set with the core that holds it, as JSON sorted by device, so
 * an equal set compares equal; null when the snapshot carries no device list
 * or no row of the core's own machine, which names nothing rather than an
 * empty set.
 */
export function pairedSet(rest: SnapshotRest): string | null {
  const devices = rest.navigator?.devices;
  const core = localDeviceId(rest);
  if (!devices || !core) return null;
  const pairs = devices
    .flatMap((device) => (device.peer_key ? [{ device: device.id, key: device.peer_key }] : []))
    .sort((a, b) => (a.device < b.device ? -1 : a.device > b.device ? 1 : a.key < b.key ? -1 : 1));
  return JSON.stringify({ core, pairs });
}
