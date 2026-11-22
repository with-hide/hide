import { expect, it } from "vitest";
import { computerTiles, numberedComputers, screenState } from "./computers";
import type { Peer } from "./pairing";
import type { SnapshotRest } from "./snapshot";

const peer = (over: Partial<Peer>): Peer => ({ key: "k", device_id: "host-a", name: "studio", opens: "active", opened_by: null, connected: true, ...over });

// PRD device-connect-iroh D-37, B47: this computer, its devices and its
// paired machines are one group, numbered together; a paired machine's tile
// shows that computer's screen, a device's its part of this one.
it("numbers this computer, each device and each paired machine together, a paired machine's tile showing its screen", () => {
  const switched = peer({ key: "k-mini", device_id: "mini-host", name: "mini" });
  const opened = peer({ key: "k-win", device_id: "win-host", name: "win" });
  const rest = {
    navigator: {
      devices: [
        { id: "local", kind: "local", label: "This Mac" },
        { id: "ssh-box", kind: "remote", label: "box" },
        { id: "mini", kind: "remote", label: "mini", peer_key: "k-mini" },
      ],
      peers: [switched, opened, peer({ key: "k-in", device_id: "in-only", opens: null, opened_by: "active" })],
    },
  } as unknown as SnapshotRest;
  const tiles = computerTiles(rest);
  expect(tiles.map((tile) => [tile.id, tile.screen?.device_id ?? null])).toEqual([
    ["local", null],
    ["ssh-box", null],
    ["mini", "mini-host"],
    ["win-host", "win-host"],
  ]);
  expect([...numberedComputers(tiles)]).toEqual([[1, "local"], [2, "ssh-box"], [3, "mini"], [4, "win-host"]]);
});

// B21: each reason a screen cannot show has its own words.
it("says why a paired machine's screen cannot show", () => {
  expect(screenState(peer({}))).toBe("ready");
  expect(screenState(peer({ connected: false }))).toBe("not_connected");
  expect(screenState(peer({ opens: "paused" }))).toBe("paused");
  expect(screenState(peer({ opens: "paused_there" }))).toBe("paused_there");
  expect(screenState(peer({ update: "there" }))).toBe("update_there");
  // Asked to update, the screen still says which side must update, never that it is not connected.
  expect(screenState(peer({ update: "here_asked", connected: false }))).toBe("update_here");
  expect(screenState(peer({ update: "there_asked", connected: false }))).toBe("update_there");
  expect(screenState(peer({ opens: "unpaired" }))).toBe("no_grant");
  expect(screenState(peer({ opens: "confirming", connected: false }))).toBe("confirming");
});
