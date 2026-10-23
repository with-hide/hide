import { expect, it } from "vitest";
import { coreLogPath, herdrGate, movedLine, newerWait } from "./herdrTakeoverRules";
import type { UpdateStatus } from "./host";
import type { HerdrTakeover, SnapshotRest } from "./snapshot";

const takeover = (overrides: Partial<HerdrTakeover>): HerdrTakeover => ({
  kind: "move", stage: "offered", server_version: "0.8.2", version: "0.9.5", same_protocol: false, failure: null, path_herdr: null, ...overrides,
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
  // The moved step comes at once, before the connection reads Hide's Herdr (measured: about a second on 0.8.2).
  expect(herdrGate(rest("protocol_mismatch", 20, takeover({ stage: "moved" })))).toBeNull();
  expect(herdrGate(rest("connected", 22))).toBeNull();
});

it("finds the core's log beside its state file on either system's spelling", () => {
  expect(coreLogPath("/srv/op/.hide/state/core-state.json")).toBe("/srv/op/.hide/state/Logs/core.jsonl");
  expect(coreLogPath("D:\\op\\.hide\\state\\core-state.json")).toBe("D:\\op\\.hide\\state\\Logs\\core.jsonl");
});

it("leaves a moved line only for a terminal herdr Hide could not bring up", () => {
  expect(movedLine(takeover({ stage: "moved", path_herdr: { stage: "behind", version: "0.8.2" } }))).toBe("0.8.2");
  expect(movedLine(takeover({ stage: "moved", path_herdr: { stage: "updating", version: "0.8.2" } }))).toBeNull();
  expect(movedLine(takeover({ stage: "moved", path_herdr: { stage: "updated", version: "0.9.5" } }))).toBeNull();
  expect(movedLine(takeover({ stage: "moved" }))).toBeNull();
  expect(movedLine(null)).toBeNull();
});

it("reads the wait for a Hide update from this app's update status", () => {
  const status = (phase: UpdateStatus["phase"]): UpdateStatus => ({ phase, prerelease: false, installOnQuit: true });
  expect(newerWait(null)).toBe("unknown");
  expect(newerWait(status({ kind: "idle", checkedAt: null }))).toBe("checking");
  expect(newerWait(status({ kind: "checking" }))).toBe("checking");
  expect(newerWait(status({ kind: "idle", checkedAt: 5 }))).toBe("none");
  expect(newerWait(status({ kind: "downloading", version: "0.4.3", percent: 0 }))).toBe("downloading");
  expect(newerWait(status({ kind: "ready", version: "0.4.3" }))).toBe("downloading");
  expect(newerWait(status({ kind: "failed", stage: "check" }))).toBe("failed");
  expect(newerWait(status({ kind: "off", reason: "development" }))).toBe("failed");
});
