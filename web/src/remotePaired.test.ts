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
    expect(JSON.parse(pairedSet(rest))).toEqual([{ device: "mini", key: "aa".repeat(32) }, { device: "pc", key: "bb".repeat(32) }]);
    expect(pairedSet({ navigator: { devices: [] } } as unknown as SnapshotRest)).toBe("[]");
  });
});
