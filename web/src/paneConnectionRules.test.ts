import { expect, it } from "vitest";
import { connectionCopy, notConnected, offersReopen, offersSharedServerOff, reopenFailureKey, reopenPending, sharedServerOutcome } from "./paneConnectionRules";
import type { PaneConnection } from "./snapshot";

const connection = (over: Partial<PaneConnection>): PaneConnection => ({ connected: false, can_reopen: true, reason: "started_before_hide", reopen: null, ...over });

it("draws a chip only for a pane the core says is not connected, with a reason", () => {
  expect(notConnected(undefined)).toBeNull();
  expect(notConnected(null)).toBeNull();
  expect(notConnected(connection({ connected: true, reason: null, can_reopen: false }))).toBeNull();
  expect(notConnected(connection({ reason: null }))).toBeNull();
  expect(notConnected(connection({}))?.reason).toBe("started_before_hide");
});

it("offers Reopen only where the core says it can and the reason is one Reopen fixes", () => {
  const offered = (over: Partial<PaneConnection>) => offersReopen(notConnected(connection(over))!);
  expect(offered({ reason: "started_before_hide" })).toBe(true);
  expect(offered({ reason: "codex_shared_server" })).toBe(true);
  expect(offered({ reason: "started_before_hide", can_reopen: false })).toBe(false);
  expect(offered({ reason: "setup_needed", can_reopen: true })).toBe(false);
  expect(offered({ reason: "update_agent", can_reopen: true })).toBe(false);
  expect(connectionCopy("setup_needed").reopen).toBeNull();
});

it("offers the shared-server link for the shared server alone", () => {
  const offered = (reason: PaneConnection["reason"]) => offersSharedServerOff(notConnected(connection({ reason }))!);
  expect(offered("codex_shared_server")).toBe(true);
  expect(offered("started_before_hide")).toBe(false);
  expect(offered("setup_needed")).toBe(false);
});

it("names every refusal code a Reopen can leave and reads an unknown code as the generic one", () => {
  const key = (reason: string) => reopenFailureKey(connection({ reopen: { state: "failed", reason: reason as never } }));
  expect(new Set(["agent_busy", "session_gone", "codex_unread", "start_refused", "end_refused"].map(key)).size).toBe(5);
  expect(key("from_a_newer_daemon")).toBe("panes.connection.reopenFailed.generic");
  expect(reopenFailureKey(connection({ reopen: { state: "pending" } }))).toBeNull();
  expect(reopenFailureKey(connection({}))).toBeNull();
  expect(reopenPending(connection({ reopen: { state: "pending" } }))).toBe(true);
  expect(reopenPending(connection({ reopen: { state: "failed", reason: "agent_busy" } }))).toBe(false);
});

it("shows a finished shared-server answer only to the operator who asked, and a pending one always", () => {
  expect(sharedServerOutcome(null, true)).toBeNull();
  expect(sharedServerOutcome({ state: "pending" }, false)).toEqual({ phase: "pending" });
  expect(sharedServerOutcome({ state: "done" }, false)).toBeNull();
  expect(sharedServerOutcome({ state: "done" }, true)).toEqual({ phase: "done" });
  expect(sharedServerOutcome({ state: "failed", reason: "timed_out" }, false)).toBeNull();
  expect(sharedServerOutcome({ state: "failed", reason: "timed_out" }, true)).toEqual({ phase: "failed", key: "panes.connection.sharedServerFailed.timedOut" });
  // Autostart went off but the running server did not stop: its own line, not the generic one (PRD codex-daemon-apply B7).
  expect(sharedServerOutcome({ state: "failed", reason: "stop_failed" }, true)).toEqual({ phase: "failed", key: "panes.connection.sharedServerFailed.stopFailed" });
});
