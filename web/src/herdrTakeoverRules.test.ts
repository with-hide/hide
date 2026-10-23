import { expect, it } from "vitest";
import { coreLogPath, herdrGate } from "./herdrTakeoverRules";
import type { HerdrTakeover, SnapshotRest } from "./snapshot";

const takeover = (overrides: Partial<HerdrTakeover>): HerdrTakeover => ({
  kind: "move", stage: "offered", server_version: "0.8.2", version: "0.9.5", same_protocol: false, failure: null, path_herdr_behind: null, ...overrides,
});
const rest = (state: string, received: number | null, offer: HerdrTakeover | null = null) =>
  ({ status: { herdr: { state, received_protocol: received, expected_protocol: 22 }, herdr_takeover: offer } }) as unknown as SnapshotRest;

it("holds Hide for every server on another protocol and never for one on Hide's", () => {
  expect(herdrGate(rest("protocol_mismatch", 20, takeover({})))).toBe("move");
  expect(herdrGate(rest("protocol_mismatch", 10, takeover({ kind: "restart" })))).toBe("restart");
  expect(herdrGate(rest("protocol_mismatch", 23, takeover({ kind: "update_hide" })))).toBe("update_hide");
  // Before the core's look answers, or where it offers nothing, the protocol alone decides.
  expect(herdrGate(rest("protocol_mismatch", 20))).toBe("older");
  expect(herdrGate(rest("protocol_mismatch", 23))).toBe("newer");
  expect(herdrGate(rest("protocol_mismatch", 20, takeover({ kind: "look", stage: "failed" })))).toBe("look");
  // A look that failed while the server is merely not answering is the wizard's step, not a hold.
  expect(herdrGate(rest("unreachable", null, takeover({ kind: "look", stage: "failed" })))).toBeNull();
  expect(herdrGate(rest("connected", 22, takeover({ same_protocol: true, server_version: "0.9.4" })))).toBeNull();
  expect(herdrGate(rest("connected", 22, takeover({ stage: "moved" })))).toBeNull();
  expect(herdrGate(rest("connected", 22))).toBeNull();
});

it("finds the core's log beside its state file on either system's spelling", () => {
  expect(coreLogPath("/Users/op/.hide/state/core-state.json")).toBe("/Users/op/.hide/state/Logs/core.jsonl");
  expect(coreLogPath("C:\\Users\\op\\.hide\\state\\core-state.json")).toBe("C:\\Users\\op\\.hide\\state\\Logs\\core.jsonl");
});
