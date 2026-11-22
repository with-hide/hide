import { describe, expect, it } from "vitest";
import { NOTHING_UNDER_WAY, directionKey, isPairLink, joinFailureKey, listedDevices, mayRequest, openedMachines, ownWindow, pairedMachines, peerState, refusalKey, requestLine, timeLeft, type Peer } from "./pairing";
import type { SnapshotRest } from "./snapshot";

const peer = (over: Partial<Peer> = {}): Peer => ({ key: "ab".repeat(32), device_id: "windows-pc", name: "Windows PC", opens: "active", opened_by: null, connected: true, path: "direct", reason: null, update: null, ...over });

describe("a paired machine's row (PRD device-connect-iroh B21, B24-B27, B50, B51)", () => {
  it("says who opens whom from the reading machine's side", () => {
    expect(directionKey(peer())).toBe("pairing.direction.outgoing");
    expect(directionKey(peer({ opens: null, opened_by: "active" }))).toBe("pairing.direction.incoming");
    expect(directionKey(peer({ opened_by: "active" }))).toBe("pairing.direction.both");
    // A host that dropped this machine's pairing is not opened: only the direction that stands is said.
    expect(directionKey(peer({ opens: "unpaired", opened_by: "active" }))).toBe("pairing.direction.incoming");
    expect(directionKey(peer({ opens: "unpaired" }))).toBe("pairing.direction.outgoing");
  });

  it("says only how a connected machine is reached", () => {
    expect(peerState(peer())).toEqual({ key: "pairing.state.direct", tone: "ok", action: null, reason: null });
    expect(peerState(peer({ path: "relay" })).key).toBe("pairing.state.relay");
    // D-86: a direct path between two Tailscale addresses is named apart.
    expect(peerState(peer({ path: "tailscale" }))).toEqual({ key: "pairing.state.tailscale", tone: "ok", action: null, reason: null });
  });

  it("offers Retry with the relay's reason for a machine it opens and cannot reach, and nothing for one that only opens it", () => {
    expect(peerState(peer({ connected: false, path: null, reason: "relay_monthly_limit" }))).toEqual({ key: "pairing.state.notConnected", tone: "warn", action: "retry", reason: "pairing.reason.relayMonthlyLimit" });
    expect(peerState(peer({ connected: false, reason: "relay_unreachable" })).reason).toBe("pairing.reason.relayUnreachable");
    expect(peerState(peer({ opens: null, opened_by: "active", connected: false })).action).toBeNull();
  });

  it("puts the state the operator acts on before the connection: update, pair again, a removal still owed, a pause", () => {
    expect(peerState(peer({ update: "there", connected: false })).key).toBe("pairing.state.updateThere");
    expect(peerState(peer({ update: "here" })).key).toBe("pairing.state.updateHere");
    // B22: only a computer that opens the older one, with that direction
    // on, may ask it to update; this computer's own update is its notice's.
    expect(peerState(peer({ update: "there", connected: false })).action).toBe("update_there");
    expect(peerState(peer({ update: "there", opens: "paused" })).action).toBeNull();
    expect(peerState(peer({ update: "there", opens: null, opened_by: "active" })).action).toBeNull();
    expect(peerState(peer({ update: "there_asked" }))).toMatchObject({ key: "pairing.state.updateThereAsked", tone: "muted", action: null });
    expect(peerState(peer({ update: "here_asked" }))).toMatchObject({ key: "pairing.state.updateHereAsked", action: null });
    // B23: the Hide this computer started there is being updated, or was
    // not, with why and Retry, whether or not the two are connected.
    expect(peerState(peer({ update: "updating" }))).toMatchObject({ key: "pairing.state.updating", action: null });
    for (const [update, reason] of [
      ["update_not_reached", "pairing.update.notReached"],
      ["update_rolled_back", "pairing.update.rolledBack"],
      ["update_busy", "pairing.update.busy"],
      ["update_failed", "pairing.update.failed"],
    ] as const) {
      expect(peerState(peer({ update }))).toEqual({ key: "pairing.state.updateFailed", tone: "warn", action: "retry", reason });
    }
    expect(peerState(peer({ opens: "unpaired", connected: false }))).toMatchObject({ key: "pairing.state.unpaired", action: "remove_outgoing" });
    expect(peerState(peer({ opens: "removing", connected: false })).key).toBe("pairing.state.removing");
    expect(peerState(peer({ opens: "paused", connected: false }))).toMatchObject({ key: "pairing.state.paused", action: "resume_outgoing" });
    expect(peerState(peer({ opens: null, opened_by: "paused" }))).toMatchObject({ key: "pairing.state.paused", action: "resume_incoming" });
    // With two directions the line names the paused one.
    expect(peerState(peer({ opens: "active", opened_by: "paused" }))).toMatchObject({ key: "pairing.state.pausedIncoming", action: "resume_incoming" });
    expect(peerState(peer({ opens: "paused", opened_by: "active", connected: false }))).toMatchObject({ key: "pairing.state.pausedOutgoing", action: "resume_outgoing" });
    // The other side's pause is told, and is not this machine's to undo.
    expect(peerState(peer({ opens: "paused_there" }))).toMatchObject({ key: "pairing.state.pausedThere", action: null });
    expect(peerState(peer({ opens: "confirming", connected: false })).key).toBe("pairing.state.confirming");
  });

  it("asks to be let in the other way only over a live connection from a machine it does not open yet", () => {
    expect(mayRequest(peer({ opens: null, opened_by: "active" }))).toBe(true);
    expect(mayRequest(peer({ opened_by: "active" }))).toBe(false);
    expect(mayRequest(peer({ opens: null, opened_by: "active", connected: false }))).toBe(false);
    expect(mayRequest(peer({ opens: null, opened_by: "paused" }))).toBe(false);
    expect(requestLine({ key: "k", state: "declined" })).toBe("pairing.request.declined");
    expect(requestLine({ key: "k", state: "unanswered" })).toBe("pairing.request.unanswered");
    expect(requestLine({ key: "k", state: "unreachable" })).toBe("pairing.request.unreachable");
    expect(requestLine(undefined)).toBeNull();
  });
});

describe("this computer's own window (B52)", () => {
  it("is the screen hided sent the pairing frame, and no other", () => {
    expect(ownWindow(NOTHING_UNDER_WAY)).toBe(true);
    // Another computer's view of this one, or a phone: no frame ever comes.
    expect(ownWindow(null)).toBe(false);
    expect(ownWindow(undefined)).toBe(false);
  });
});

describe("the paired machines a screen reads", () => {
  const rest = (peers: Peer[] | undefined, devices: { id: string; peer_key?: string | null }[] = []) => ({ navigator: { peers, devices } }) as unknown as SnapshotRest;

  it("is one stable empty list while nothing is paired, so a selector does not redraw", () => {
    expect(pairedMachines(rest(undefined))).toBe(pairedMachines(null));
  });

  it("gives a rail tile only to a machine this one opens", () => {
    const opened = peer();
    const guest = peer({ key: "cd".repeat(32), device_id: "macbook", opens: null, opened_by: "active" });
    expect(openedMachines(rest([opened, guest]))).toEqual([opened]);
  });

  it("leaves a machine the core made a device of to that device's tile, so one machine is one tile", () => {
    const confirmed = peer();
    const waiting = peer({ key: "ef".repeat(32), device_id: "mini", opens: "confirming", connected: false });
    const device = { id: "windows-pc", peer_key: confirmed.key };
    expect(openedMachines(rest([confirmed, waiting], [{ id: "local" }, device]))).toEqual([waiting]);
    // A pause keeps the device, and so its one tile.
    expect(openedMachines(rest([peer({ opens: "paused" })], [device]))).toEqual([]);
    // An SSH device that only says the same id is another machine: both have a tile.
    expect(openedMachines(rest([confirmed], [{ id: "windows-pc", peer_key: null }]))).toEqual([confirmed]);
  });

  it("lists in Settings every device but a paired machine's, whose row is its pairing's", () => {
    const devices: { id: string; peer_key?: string | null }[] = [{ id: "local" }, { id: "mini", peer_key: null }, { id: "windows-pc", peer_key: "ab".repeat(32) }, { id: "studio" }];
    expect(listedDevices(devices).map((device) => device.id)).toEqual(["local", "mini", "studio"]);
  });
});

describe("a pairing's outcome and a link's time", () => {
  it("words an ended pairing by what the operator does next (B4-B6)", () => {
    for (const failure of ["no_offer", "declined", "unanswered"] as const) expect(joinFailureKey(failure)).toBe("pairing.failed.ended");
    expect(joinFailureKey("unreachable")).toBe("pairing.failed.unreachable");
    expect(joinFailureKey("host_full")).toBe("pairing.failed.hostFull");
    expect(joinFailureKey("full")).toBe("pairing.failed.full");
    expect(joinFailureKey("update_needed")).toBe("pairing.failed.updateNeeded");
  });

  it("words only the refusals a person can act on", () => {
    expect(refusalKey("link_invalid")).toBe("pairing.refused.linkInvalid");
    expect(refusalKey("device_already_connected")).toBe("pairing.refused.already");
    expect(refusalKey("device_from_other_screen")).toBeNull();
    expect(refusalKey(undefined)).toBeNull();
  });

  it("counts a link down as minutes and seconds and stops at zero", () => {
    expect(timeLeft(9 * 60_000, 19_000)).toBe("8:41");
    expect(timeLeft(60_000, 59_001)).toBe("0:01");
    expect(timeLeft(60_000, 90_000)).toBe("0:00");
  });

  it("knows a shared link from an SSH alias", () => {
    expect(isPairLink("  hide://pair#v=1&id=ab ")).toBe(true);
    expect(isPairLink("mini")).toBe(false);
  });
});
