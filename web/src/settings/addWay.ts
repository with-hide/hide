// How Add device connects the SSH Host it chose (operator decision
// 2026-11-28, mockup agents/runs/ssh-server-recommend/mockup-v2.html): as a
// server where it can be, its own Hide connected to this one or one started
// there, and over SSH where it cannot, with the one line of why. hided reads
// the Host once over a login that asks nobody (`pairing_ssh::HostLook`);
// this only says which way that answer leads to.

import type { MessageKey } from "../i18n/catalogs";
import type { SshLook, SwitchCondition } from "../pairing";

/**
 * What hided answered about the chosen Host: nothing yet, its look (none
 * when it refused to look), the system it read, for a Host with no Hide why
 * none can be started there, and for one that runs Hide where it stands on
 * the switch rule (`pairing_ssh::HostLook`).
 */
export type HostAnswer = { answered: false } | { answered: true; looked: SshLook | null; system: string | null; leaves: string | null; condition: SwitchCondition | null };

/**
 * The way a chosen Host is added: `connect` pairs with the Hide it runs
 * (`pair_ssh`), saying first when that Mac needs one login after a restart
 * (`signInAfterRestart`: its automatic login is off, or FileVault rules it
 * out); `install` adds it and starts Hide there (`add_server`); and `ssh`
 * adds it over SSH alone, with `reason` saying why it is not a server (none
 * when nothing was read).
 */
export type AddWay = { kind: "looking" } | { kind: "connect"; system: string | null; signInAfterRestart: boolean } | { kind: "install"; system: string | null } | { kind: "ssh"; reason: MessageKey | null };

export function addWay(answer: HostAnswer): AddWay {
  if (!answer.answered) return { kind: "looking" };
  const { looked, system, leaves, condition } = answer;
  switch (looked?.state) {
    case undefined:
      // A look hided refused looked at nothing: the Host is added as before.
      return { kind: "ssh", reason: null };
    case "ready":
      return { kind: "connect", system, signInAfterRestart: condition?.kind === "auto_login_off" };
    case "absent":
      return leaves === null ? { kind: "install", system } : { kind: "ssh", reason: leavesKey(leaves) };
    // A login that asks for an answer, or a machine ssh did not reach: the
    // two read alike from here.
    case "not_reached":
      return { kind: "ssh", reason: "pairing.add.leaves.notReached" };
    case "update_there":
      return { kind: "ssh", reason: "pairing.ssh.updateThere" };
    case "refused":
      return { kind: "ssh", reason: "pairing.ssh.refusedThere" };
    case "not_running":
      return { kind: "ssh", reason: looked.system === "macos" ? "pairing.ssh.signInMac" : "pairing.ssh.notRunning" };
  }
}

/** Why a Host with no Hide of its own is not connected as a server, by hided's code. */
function leavesKey(code: string): MessageKey {
  switch (code) {
    case "windows":
      return "pairing.add.leaves.windows";
    case "no_systemd":
      return "devices.ways.noSystemd";
    case "ssh_mac_from_mac_only":
      return "pairing.add.leaves.macOnly";
    case "ssh_not_reached":
      return "pairing.add.leaves.notReached";
    default:
      return "devices.ways.unknown";
  }
}

/** A system as `std::env::consts::OS` spells it, as its maker names it; none for one Hide does not name. */
export function systemName(system: string | null): string | null {
  switch (system) {
    case "macos":
      return "macOS";
    case "linux":
      return "Linux";
    case "windows":
      return "Windows";
    default:
      return null;
  }
}
