import { describe, expect, it } from "vitest";
import { pairedSet } from "./remotePaired";
import type { SnapshotRest } from "./snapshot";

// The machines whose login storage the desktop app keeps: the device rows of
// paired machines the core keeps from its start, never the live connections.
describe("the paired set", () => {
  it("names each paired machine's device row by its device and key, in one order, whatever is connected", () => {
    const rest = {
      navigator: {
        devices: [
          { id: "mac", kind: "local" },
          { id: "pc", kind: "remote", peer_key: "bb".repeat(32) },
          { id: "devbox", kind: "remote", ssh_alias: "devbox" },
          { id: "mini", kind: "remote", peer_key: "aa".repeat(32) },
        ],
      },
      peers: [],
    } as unknown as SnapshotRest;
    expect(JSON.parse(pairedSet(rest)!)).toEqual({ core: "mac", pairs: [{ device: "mini", key: "aa".repeat(32) }, { device: "pc", key: "bb".repeat(32) }] });
    expect(JSON.parse(pairedSet({ navigator: { devices: [{ id: "mac", kind: "local" }] } } as unknown as SnapshotRest)!)).toEqual({ core: "mac", pairs: [] });
  });

  // A snapshot with no device list, or none of its own machine, names
  // nothing: the desktop app then clears no machine's logins.
  it("names nothing when the snapshot does not say which machines the core holds", () => {
    expect(pairedSet({} as SnapshotRest)).toBeNull();
    expect(pairedSet({ navigator: {} } as unknown as SnapshotRest)).toBeNull();
    expect(pairedSet({ navigator: { devices: [] } } as unknown as SnapshotRest)).toBeNull();
  });
});
