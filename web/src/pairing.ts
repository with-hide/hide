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
export type PairingVerb = "share" | "end_share" | "join" | "compare" | "dismiss_join" | "answer" | "pause" | "resume" | "remove" | "request" | "dismiss_request" | "retry" | "update" | "look_ssh" | "pair_ssh" | "look_wsl" | "pair_wsl" | "rename" | "look_switch" | "switch" | "add_server";

/** What a machine reached over SSH said about pairing that way (hided `pairing_ssh::Looked`). */
export type SshLook = { state: "ready" | "absent" | "update_there" | "refused" | "not_running" | "not_reached"; system?: string };

/** The daemon's answer to one `pairing_*` event, for the window that sent it. */
export type PairingResult = {
  verb: string;
  ok: boolean;
  reason?: string | null;
  request_id?: string | null;
  link?: string;
  alias?: string;
  looked?: SshLook;
  /** With `look_ssh`: the system the Host's account runs, and for one with no Hide why none can be started there (hided `pairing_ssh::HostLook`). */
  system?: string | null;
  leaves?: string | null;
  /** With `look_switch` and `switch`: the device asked about, what its machine said, and the login name a Mac said. */
  device_id?: string;
  condition?: SwitchCondition;
  user?: string | null;
  /** With `switch` and `add_server`: how the press ended; `held` names the computer that switched the device first and how many terminals this one runs there. */
  switch?: "done" | "left" | "held" | "install_needed";
  by?: string;
  terminals?: number | null;
};

/**
 * What a device's machine said of the rule every switch passes (hided
 * `pairing_ssh::Condition::payload`): a Mac's automatic login and whether
 * someone is logged in now, a Linux account's linger, and the systems that
 * are not switched. `how` is false while FileVault keeps a Mac from
 * offering automatic login; `command` is what an administrator runs.
 */
export type SwitchCondition =
  | { kind: "auto_login_on" | "signed_out" | "linger_on" | "linger_off" | "no_systemd" | "windows" | "unknown" }
  | { kind: "auto_login_off"; how: boolean }
  | { kind: "linger_admin"; command: string };

export type PairRequest = { key: string; state: "waiting" | "declined" | "unanswered" | "unreachable" };

/** A machine this one is paired with (hide-core `PeerMachine`). */
export type Peer = {
  /** Its connection id. */
  key: string;
  device_id: string;
  /** The name it is shown under: the one the operator chose, else its own. */
  name: string;
  /** `name` is the one the operator chose (`PeerMachine::named`); absent from an older daemon. */
  named?: boolean;
  /** This machine opens it. */
  opens?: "confirming" | "active" | "paused" | "paused_there" | "removing" | "unpaired" | null;
  /** It opens this machine. */
  opened_by?: "active" | "paused" | null;
  connected: boolean;
  path?: "direct" | "tailscale" | "relay" | "local" | "ssh" | null;
  reason?: "relay_unreachable" | "relay_monthly_limit" | null;
  /**
   * Which side must update before the two connect, or where this machine's
   * update of the Hide it started there stands (B23).
   */
  update?: "here" | "there" | "here_asked" | "there_asked" | "updating" | "update_not_reached" | "update_rolled_back" | "update_busy" | "update_failed" | null;
  /**
   * The device of this computer that is this machine and still reaches it
   * over SSH or WSL (hide-core `PeerMachine::helper_device`): that device's
   * row and tile are the machine's only ones.
   */
  helper_device?: string | null;
  /**
   * This computer's SSH route to it, while the route waits for the operator
   * (hide-core `PeerMachine::ssh`): why it did not connect over SSH. Never
   * the alias, which is this computer's own.
   */
  ssh?: { blocked: PeerSshBlocked } | null;
};

/** Why an SSH route waits for the operator (`contracts/snapshot-wire-enums.json`: `peer_ssh_blocked`). */
export type PeerSshBlocked = "sign_in" | "check_host_key" | "reopen_hide" | "update_there" | "retry" | "not_saved";

/** Why this machine's update of the Hide it started on `peer` did not happen; none when it did not fail. */
function updateFailure(update: Peer["update"]): MessageKey | null {
  switch (update) {
    case "update_not_reached":
      return "pairing.update.notReached";
    case "update_rolled_back":
      return "pairing.update.rolledBack";
    case "update_busy":
      return "pairing.update.busy";
    case "update_failed":
      return "pairing.update.failed";
    default:
      return null;
  }
}

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

/** Every machine this one is paired with, in either direction. */
export function pairedMachines(rest: SnapshotRest | null | undefined): readonly Peer[] {
  return rest?.navigator?.peers ?? NONE;
}

/**
 * The paired machines that have a row of their own in Settings and may have
 * a tile of their own on the rail: every one but a machine that is still a
 * helper device of this computer (`Peer.helper_device`, which the core
 * writes), whose device's row and tile are its only ones. A move that paired
 * the two and did not take the device leaves it so until the next one.
 */
export function machinesWithOwnRow(peers: readonly Peer[]): readonly Peer[] {
  return peers.some((peer) => peer.helper_device != null) ? peers.filter((peer) => peer.helper_device == null) : peers;
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
  return machinesWithOwnRow(pairedMachines(rest)).filter((peer) => peer.opens != null && !devices.some((device) => device.peer_key === peer.key));
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
export type PeerAction = "retry" | "resume_outgoing" | "resume_incoming" | "remove_outgoing" | "update_there" | null;

/**
 * A paired machine's one state line (B21, B25, B27): the state the operator
 * would act on first, said once. A machine that connects says only how.
 */
export function peerState(peer: Peer): { key: MessageKey; tone: "ok" | "muted" | "warn"; action: PeerAction; reason: MessageKey | null } {
  const line = (key: MessageKey, tone: "ok" | "muted" | "warn", action: PeerAction = null, reason: MessageKey | null = null) => ({ key, tone, action, reason });
  // Two Hides too far apart to connect (B22): the older one updates. This
  // computer's own app does it here, from its notice (`PeerUpdateNotice`)
  // or because the other computer, which opens this one, asked; the other
  // computer is asked from here only by a computer that opens it.
  if (peer.update === "here") return line("pairing.state.updateHere", "warn");
  if (peer.update === "here_asked") return line("pairing.state.updateHereAsked", "warn");
  if (peer.update === "there") return line("pairing.state.updateThere", "warn", peer.opens === "active" ? "update_there" : null);
  if (peer.update === "there_asked") return line("pairing.state.updateThereAsked", "muted");
  // The Hide this machine started there is this machine's to update: the
  // row says it is doing so, or why it did not and offers it again (B23).
  if (peer.update === "updating") return line("pairing.state.updating", "muted");
  const failed = updateFailure(peer.update);
  if (failed) return line("pairing.state.updateFailed", "warn", "retry", failed);
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
  // Connected says only that: the path it takes is the row's Connection
  // details (`peerRouteKey`), and a relay path adds its tip (`relayTip`).
  return line("pairing.state.connected", "ok");
}

/** The path a connected machine's connection takes now, in one word (D-86); null while it is not connected. */
export function peerRouteKey(peer: Peer): MessageKey | null {
  if (!peer.connected) return null;
  switch (peer.path) {
    case "relay":
      return "pairing.state.relay";
    case "tailscale":
      return "pairing.state.tailscale";
    case "ssh":
      return "pairing.state.ssh";
    default:
      return "pairing.state.direct";
  }
}

/** The line a relay row says under its name, its key and whether it offers 다시 시도. */
export type RelayTip = { key: MessageKey; retry: boolean };

const PARKED_TIP: Record<PeerSshBlocked, MessageKey> = {
  sign_in: "devices.relayTip.ssh.signIn",
  check_host_key: "devices.relayTip.ssh.checkHostKey",
  reopen_hide: "devices.relayTip.ssh.reopenHide",
  update_there: "devices.relayTip.ssh.updateThere",
  retry: "devices.relayTip.ssh.retry",
  not_saved: "devices.relayTip.ssh.notSaved",
};

/**
 * What a row whose connection goes through the relay says under its name:
 * that Tailscale on both computers would connect them directly, or, while
 * this computer's SSH route to it waits for the operator, why it did not
 * connect over SSH, with 다시 시도 where a retry can change it (reopening Hide
 * is the only way past a Hide updated since it started). None on any other
 * path.
 */
export function relayTip(peer: Peer): RelayTip | null {
  if (!peer.connected || peer.path !== "relay" || peerState(peer).key !== "pairing.state.connected") return null;
  const blocked = peer.opens != null ? peer.ssh?.blocked : undefined;
  if (!blocked) return { key: "devices.relayTip", retry: false };
  return { key: PARKED_TIP[blocked], retry: blocked !== "reopen_hide" };
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
 * connecting installs nothing there, what has to happen on that machine
 * before the two can connect as Hides, or, for a machine with no Hide of its
 * own, the way that works: add it over SSH here, then its row's
 * 서버로 연결 puts Hide there (with-hide/hide issue 3007); the look itself installs
 * nothing (B7, D-45). None for one ssh did not reach without asking: adding
 * it is unchanged.
 */
export function sshLookLine(look: SshLook | null | undefined): { key: MessageKey; tone: "ok" | "warn" | "muted" } | null {
  switch (look?.state) {
    case "ready":
      return { key: "pairing.ssh.ready", tone: "ok" };
    case "absent":
      return { key: "pairing.ssh.absentAdd", tone: "muted" };
    case "update_there":
      return { key: "pairing.ssh.updateThere", tone: "warn" };
    case "refused":
      return { key: "pairing.ssh.refusedThere", tone: "warn" };
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
    case "ssh_refused_there":
      return "pairing.ssh.refusedThere";
    case "ssh_hide_not_running":
      return "pairing.ssh.notRunning";
    case "ssh_not_reached":
      return "pairing.ssh.notReached";
    case "wsl_not_reached":
      return "devices.switch.failed.wslNotReached";
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
