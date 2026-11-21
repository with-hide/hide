// The pure rules behind pairing this machine with another Hide (PRD
// device-connect-iroh B1-B6, B21, B24-B27, B50, B51): what a paired
// machine's row says and offers, how a pasted link's pairing reads, and how
// long a shared link has left. hided owns every value: the pairings under
// way come in its `pairing` frame, sent to this machine's own windows only,
// and the paired machines in the snapshot's `navigator.peers`. This only
// words them, so each rule is tested without a browser.

import type { MessageKey } from "./i18n/catalogs";
import type { SnapshotRest } from "./snapshot";

/** The daemon's `pairing` frame (hide-peer `pairing::View`). */
export type PairingView = {
  /** The link this machine shares now. */
  offer: { link: string; expires_unix_ms: number } | null;
  /** The machines that wait for this machine's operator. */
  asks: PairAsk[];
  /** The pairing this machine is joining, or how the last one ended. */
  join: PairJoin | null;
  /** What came of asking a machine to be let in the other way. */
  requests: PairRequest[];
};

export type PairAsk = {
  id: number;
  /** The asking machine's connection id. */
  key: string;
  device: string;
  name: string;
  /** `link`: it pasted this machine's link. `request`: a machine already connected asks to open this one too. */
  kind: "link" | "request";
  /** The words to compare; none for a request over a connection that already proved both machines. */
  words: string[] | null;
  expires_unix_ms: number;
};

export type JoinFailure = "no_offer" | "declined" | "host_full" | "full" | "unanswered" | "unreachable" | "update_needed";

export type PairJoin = {
  id: number;
  host: string;
  phase: "connecting" | "comparing" | "settling" | "paired" | "waiting" | "failed";
  failure: JoinFailure | null;
  name: string | null;
  words: string[] | null;
  expires_unix_ms: number | null;
};

/** What a window asks hided's pairing (`hided/src/pairing.rs` `Ask`). */
export type PairingVerb = "share" | "end_share" | "join" | "compare" | "dismiss_join" | "answer" | "pause" | "resume" | "remove" | "request" | "dismiss_request" | "retry" | "look_ssh" | "pair_ssh" | "switch";

/** What a machine reached over SSH said about pairing that way (hided `pairing_ssh::Looked`). */
export type SshLook = { state: "ready" | "absent" | "update_there" | "not_running" | "not_reached"; system?: string };

/** The daemon's answer to one `pairing_*` event, for the window that sent it. */
export type PairingResult = {
  verb: string;
  ok: boolean;
  reason?: string | null;
  request_id?: string | null;
  link?: string;
  alias?: string;
  looked?: SshLook;
  /** `switch`: how the press ended, or what it asks the operator first (hided `Ask::Switch`). */
  switch?: "done" | "held" | "install_needed";
  /** `switch`: the helper device the press was for. */
  device_id?: string;
  /** `switch` `held`: the computer that switched the device first. */
  by?: string;
  /** `switch` `install_needed`: the system of the machine with no Hide. */
  system?: string;
};

export type PairRequest = { key: string; state: "waiting" | "declined" | "unanswered" | "unreachable" };

/** A machine this one is paired with (hide-core `PeerMachine`). */
export type Peer = {
  /** Its connection id. */
  key: string;
  device_id: string;
  name: string;
  /** This machine opens it. */
  opens?: "confirming" | "active" | "paused" | "paused_there" | "removing" | "unpaired" | null;
  /** It opens this machine. */
  opened_by?: "active" | "paused" | null;
  connected: boolean;
  path?: "direct" | "tailscale" | "relay" | "local" | null;
  reason?: "relay_unreachable" | "relay_monthly_limit" | null;
  /** Which side must update before the two connect. */
  update?: "here" | "there" | null;
};

export type Direction = "outgoing" | "incoming";

/** The frame of a computer with no pairing under way (hide-peer `View::default()`). */
export const NOTHING_UNDER_WAY: PairingView = { offer: null, asks: [], join: null, requests: [] };

/**
 * Whether this window is this computer's own (B52): the only screen that
 * shares it, adds a machine to it or acts on one it is paired with. hided
 * sends the `pairing` frame to this computer's own windows and to no other
 * screen (another computer's view of this one, a phone) and refuses that
 * screen's asks, so a screen without the frame lists the paired machines
 * and offers nothing it would be refused. The one place this is decided.
 */
export function ownWindow(pairing: PairingView | null | undefined): boolean {
  return pairing != null;
}

/**
 * Whether this build offers connections between two Hides, as hided's
 * hello (`daemon` frame) says: off in every build until that feature
 * ships, and then Settings > Devices shows no pairing and adds a machine
 * the way it did before pairing. The one place the window reads it.
 */
export function deviceConnectOffered(daemon: { device_connect?: boolean } | null | undefined): boolean {
  return daemon?.device_connect === true;
}

/** Every machine this one is paired with, in either direction. */
export function pairedMachines(rest: SnapshotRest | null | undefined): readonly Peer[] {
  return rest?.navigator?.peers ?? NONE;
}

/** One empty list, so a store selector answers the same value while nothing is paired. */
const NONE: readonly Peer[] = [];

/**
 * The paired machines this one opens that have a tile of their own on the
 * rail (B3): the ones the core has not made a device of. A machine this one
 * opens with a confirmed pairing is a device under its own id (hide-core
 * `runtime/peer_devices.rs`), and that device's tile is the machine's one
 * tile, which leads to its screen; until then (the other side has not
 * confirmed yet) its tile here leads to Settings. The one place a tile's list
 * is read, so one machine is never drawn twice.
 */
export function openedMachines(rest: SnapshotRest | null | undefined): Peer[] {
  const devices = rest?.navigator?.devices ?? [];
  return pairedMachines(rest).filter((peer) => peer.opens != null && !devices.some((device) => device.peer_key === peer.key));
}

/**
 * The devices Settings lists as rows of their own: every device but a paired
 * machine's (`Device.peer_key`), whose one row there is its pairing's
 * (`PeerRow`: directions, pause, remove). That device still serves the rail
 * and the panes.
 */
export function listedDevices<T extends { peer_key?: string | null }>(devices: readonly T[]): T[] {
  return devices.filter((device) => device.peer_key == null);
}

/**
 * How many terminals the machine paired under `key` runs on this computer
 * (hide-core `guest_panes`): what a removal may end (B26).
 */
export function guestPaneCount(rest: SnapshotRest | null | undefined, key: string): number {
  return rest?.navigator?.guest_panes?.find((guest) => guest.key === key)?.panes.length ?? 0;
}

/** Who opens whom, in the words of the machine reading it. */
export function directionKey(peer: Peer): MessageKey {
  // A host that dropped this machine's pairing is opened no more: beside a
  // direction that still stands, only that one is said.
  const opens = peer.opens != null && !(peer.opens === "unpaired" && peer.opened_by != null);
  if (opens && peer.opened_by != null) return "pairing.direction.both";
  return opens ? "pairing.direction.outgoing" : "pairing.direction.incoming";
}

/** What the row can do about its state: nothing, or the one action its line offers. */
export type PeerAction = "retry" | "resume_outgoing" | "resume_incoming" | "remove_outgoing" | null;

/**
 * A paired machine's one state line (B21, B25, B27): the state the operator
 * would act on first, said once. A machine that connects says only how.
 */
export function peerState(peer: Peer): { key: MessageKey; tone: "ok" | "muted" | "warn"; action: PeerAction; reason: MessageKey | null } {
  const line = (key: MessageKey, tone: "ok" | "muted" | "warn", action: PeerAction = null, reason: MessageKey | null = null) => ({ key, tone, action, reason });
  if (peer.update === "here") return line("pairing.state.updateHere", "warn");
  if (peer.update === "there") return line("pairing.state.updateThere", "warn");
  if (peer.opens === "unpaired") return line("pairing.state.unpaired", "warn", "remove_outgoing");
  if (peer.opens === "removing") return line("pairing.state.removing", "warn");
  // A pair with two directions says which one is paused: "paused" beside
  // "each opens the other" would leave the operator to guess.
  const both = peer.opens != null && peer.opened_by != null;
  if (peer.opens === "paused") return line(both ? "pairing.state.pausedOutgoing" : "pairing.state.paused", "muted", "resume_outgoing");
  if (peer.opened_by === "paused") return line(both ? "pairing.state.pausedIncoming" : "pairing.state.paused", "muted", "resume_incoming");
  if (peer.opens === "paused_there") return line("pairing.state.pausedThere", "muted");
  if (peer.opens === "confirming") return line("pairing.state.confirming", "muted");
  if (!peer.connected) {
    const reason: MessageKey | null = peer.reason === "relay_monthly_limit" ? "pairing.reason.relayMonthlyLimit" : peer.reason === "relay_unreachable" ? "pairing.reason.relayUnreachable" : null;
    return line("pairing.state.notConnected", "warn", peer.opens != null ? "retry" : null, reason);
  }
  // The path the connection takes now, in one word (D-86).
  return line(peer.path === "relay" ? "pairing.state.relay" : peer.path === "tailscale" ? "pairing.state.tailscale" : "pairing.state.direct", "ok");
}

/** Whether this machine may ask `peer` to be let in the other way (B50): it is opened by a machine it does not open. */
export function mayRequest(peer: Peer): boolean {
  return peer.opened_by === "active" && peer.opens == null && peer.connected;
}

/** What came of the last request to `peer`, as its row says it (B50). */
export function requestLine(request: PairRequest | undefined): MessageKey | null {
  switch (request?.state) {
    case "waiting":
      return "pairing.request.waiting";
    case "declined":
      return "pairing.request.declined";
    case "unanswered":
      return "pairing.request.unanswered";
    case "unreachable":
      return "pairing.request.unreachable";
    default:
      return null;
  }
}

/** How a pairing that ended without connecting reads (B4-B6). */
export function joinFailureKey(failure: JoinFailure | null): MessageKey {
  switch (failure) {
    case "host_full":
      return "pairing.failed.hostFull";
    case "full":
      return "pairing.failed.full";
    case "update_needed":
      return "pairing.failed.updateNeeded";
    case "unreachable":
      return "pairing.failed.unreachable";
    default:
      return "pairing.failed.ended";
  }
}

/**
 * The one line a chosen SSH host shows once its own Hide answered (B7):
 * connecting installs nothing there, or what has to happen on that machine
 * before the two can connect as Hides. None for a machine with no Hide of
 * its own or one ssh did not reach without asking: adding it is unchanged.
 */
export function sshLookLine(look: SshLook | null | undefined): { key: MessageKey; tone: "ok" | "warn" } | null {
  switch (look?.state) {
    case "ready":
      return { key: "pairing.ssh.ready", tone: "ok" };
    case "update_there":
      return { key: "pairing.ssh.updateThere", tone: "warn" };
    case "not_running":
      return { key: look.system === "macos" ? "pairing.ssh.signInMac" : "pairing.ssh.notRunning", tone: "warn" };
    default:
      return null;
  }
}

/** Why the daemon refused what a window asked, for the refusals a person can act on; the rest stay in the log. */
export function refusalKey(reason: string | null | undefined): MessageKey | null {
  switch (reason) {
    case "link_invalid":
      return "pairing.refused.linkInvalid";
    case "link_newer":
      return "pairing.refused.linkNewer";
    case "link_own":
      return "pairing.refused.linkOwn";
    case "device_already_connected":
      return "pairing.refused.already";
    case "peer_unavailable":
      return "pairing.refused.unavailable";
    case "device_limit":
      return "pairing.failed.full";
    case "device_own":
      return "pairing.refused.own";
    case "ssh_hide_absent":
      return "pairing.ssh.absent";
    case "ssh_update_there":
      return "pairing.ssh.updateThere";
    case "ssh_hide_not_running":
      return "pairing.ssh.notRunning";
    case "ssh_not_reached":
      return "pairing.ssh.notReached";
    default:
      return null;
  }
}

/** The time a link or an ask has left, as `m:ss`; `0:00` once it is over. */
export function timeLeft(expiresUnixMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.ceil((expiresUnixMs - nowMs) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** Whether a line of text is a link another Hide shared. */
export function isPairLink(text: string): boolean {
  return text.trim().startsWith("hide://pair#");
}
