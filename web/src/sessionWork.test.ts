import { legacyAgentRow } from "../test/legacyAgentRow";
import type { BoardProject } from "./projectBoard";
import { expect, it } from "vitest";
import type { MissedWork, SessionWorkSummary } from "./snapshot";
import { holdsRequest, openMissedWork, outputPrState, recordTitle, requestPreview, sessionPlace, settledOutput } from "./sessionWork";

const missed = (id: string, at: number, status: MissedWork["status"] = "open"): MissedWork => ({ id, kind: "problem", text: id, status, at_unix_ms: at, turn_offset: 1, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const work = (id: string, cwd: string, items: MissedWork[]): SessionWorkSummary => ({
  session: { device_id: "local", agent: "claude", session_id: id }, title: "Current goal", fallback_title: "Claude", native_title: null, manual_title: null,
  cwd, ended_at_unix_ms: null, open_missed: items.filter((item) => item.status === "open").length, subtree_open_missed: 0, missed: items,
});

it("collects every open missed item, of live and ended sessions alike, newest first", () => {
  const rows = [work("live", "/repo", [missed("a", 1), missed("handled", 9, "resolved")]), work("ended", "/repo/sub", [missed("b", 3)]), work("elsewhere", "/other", [missed("c", 5)])];
  expect(openMissedWork(rows).map(({ item }) => item.id)).toEqual(["c", "b", "a"]);
  expect(openMissedWork([work("none", "/repo", [missed("done", 1, "dismissed")])])).toEqual([]);
});

it("titles a record by its manual name, else the model's goal or the native name", () => {
  const row = work("s", "/repo", []);
  expect(recordTitle(row, true)).toBe("Current goal");
  expect(recordTitle(row, false)).toBe("Claude");
  expect(recordTitle({ ...row, manual_title: "Mine" }, true)).toBe("Mine");
});

it("shows a request's summary, else its first 120 characters on one line with `…` (D-06, B5)", () => {
  expect(requestPreview({ summary: "Write the PRD", text: "ㅇㅇ PRD 써줘" })).toEqual({ text: "Write the PRD", summarized: true, cut: false });
  expect(requestPreview({ summary: null, text: "two\nlines" })).toEqual({ text: "two lines", summarized: false, cut: false });
  const cut = requestPreview({ summary: null, text: "가".repeat(130) });
  expect([...cut.text]).toHaveLength(121);
  expect(cut.cut).toBe(true);
});

it("reads merged and closed PRs as settled and every other state as still moving (B19)", () => {
  expect(["merged", "closed", "open", "review", "ci_failed"].map((state) => settledOutput({ state }))).toEqual([true, true, false, false, false]);
});

it("maps a recorded PR state to the PR mark's state", () => {
  expect(["ci_failed", "review", "merged", "closed"].map(outputPrState)).toEqual(["failed", "pending", "merged", "closed"]);
});

it("resolves the live pane by device, provider and native session together", () => {
  const agents = [
    legacyAgentRow({ pane_id: "wrong", agent_kind: "codex", session_id: "shared", device_id: "local" }),
    legacyAgentRow({ pane_id: "right", agent_kind: "claude", session_id: "shared", device_id: "local" }),
  ];
  const workspace = { device_id: "local", path: "/repo", checkouts: [], agent_scope: { members: agents.map((row) => ({ pane_id: row.pane_id, checkout_id: "c" })) } };
  const projects = [{ workspace, agents, device: null }] as unknown as BoardProject[];
  const session = { device_id: "local", agent: "claude", session_id: "shared" };
  expect(sessionPlace(projects, session, null, null)?.agent?.pane_id).toBe("right");
  expect(sessionPlace(projects, { ...session, agent: "opencode" }, null, null)).toBeNull();
});

it("finds a History hit in the line a repeat was merged into (B23)", () => {
  expect(holdsRequest({ offset: 10, repeats: [14, 18] }, 18)).toBe(true);
  expect(holdsRequest({ offset: 10 }, 10)).toBe(true);
  expect(holdsRequest({ offset: 10, repeats: [14] }, 12)).toBe(false);
});
