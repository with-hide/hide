// The machines this window's operator has paired with, as the desktop app
// keeps their screens' login storage (docs/BROWSER_DISPLAYS.md, A page in
// another computer's screen): each device row of a paired machine
// (`peer_key`), by its device and the key it was paired under. The core
// keeps those rows from its start until a told list of pairings ends one, so
// a daemon that has not heard from its machines yet names them all.

import type { SnapshotRest } from "./snapshot";

/** The paired set, as JSON sorted by device, so an equal set compares equal. */
export function pairedSet(rest: SnapshotRest): string {
  const pairs = (rest.navigator?.devices ?? [])
    .flatMap((device) => (device.peer_key ? [{ device: device.id, key: device.peer_key }] : []))
    .sort((a, b) => (a.device < b.device ? -1 : a.device > b.device ? 1 : a.key < b.key ? -1 : 1));
  return JSON.stringify(pairs);
}
