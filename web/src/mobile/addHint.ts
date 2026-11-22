// The one-time hint that a machine the open computer opens can be added to
// this phone too (PRD device-connect-iroh B56, D-57): a machine that runs
// work of its own after [바꾸기] keeps that work, which the phone sees only
// once it adds that machine. The open computer names those machines in its
// meta frame (`machines`); the phone hints at one it keeps under no saved
// computer, until the hint is closed or [추가] is pressed.

import type { PairedHost } from "./hosts";
import type { OtherMachine } from "./protocol";

/** The machines whose hint this phone closed or acted on, by machine id. */
export const ADD_HINTS_KEY = "hide.phone.add-hints.v1";
/** As many as a computer names; past it the oldest is forgotten. */
const MAX_HINTED = 16;

/**
 * The machine to hint at: the first one the open computer names that no saved computer is, by the machine id its hello gave, or by
 * name while that computer's hello has not given one on this phone yet, and whose hint was not closed or acted on.
 */
export function hintedMachine(others: readonly OtherMachine[], hosts: readonly PairedHost[], hinted: readonly string[]): OtherMachine | null {
  return others.find((other) => !hinted.includes(other.machine) &&
    !hosts.some((host) => host.machine === undefined ? host.name === other.name : host.machine === other.machine)) ?? null;
}

/** The machines already hinted at; a list this bundle cannot read is an error, never an empty one. */
export function readHinted(storage: Pick<Storage, "getItem">): string[] {
  const raw = storage.getItem(ADD_HINTS_KEY);
  if (raw === null) return [];
  const hinted: unknown = JSON.parse(raw);
  if (!Array.isArray(hinted) || hinted.length > MAX_HINTED || !hinted.every((machine) => typeof machine === "string")) {
    throw new Error("add_hints_invalid");
  }
  return hinted;
}

/** Records that `machine`'s hint was closed or acted on, so it never shows again. */
export function markHinted(storage: Storage, machine: string): void {
  const hinted = readHinted(storage).filter((known) => known !== machine);
  storage.setItem(ADD_HINTS_KEY, JSON.stringify([...hinted, machine].slice(-MAX_HINTED)));
}
