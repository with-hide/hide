import { describe, expect, it } from "vitest";
import { gatherGroups, hostLine, listScopeOf } from "./gather";
import type { AgentGroup, GroupRow, PhoneAgent } from "./protocol";

function row(pane: string, changed: number | null, over: Partial<PhoneAgent> = {}): GroupRow {
  return { kind: "agent", device_id: "d", pane_id: pane, root_pane_id: pane, group: "needs_you", mark: "turn", wait: null, descendants_working: 0, agent_kind: "claude", title: pane, place: null, device_label: null, changed_at_unix_ms: changed, line: null, status_code: "waiting", demand: "none", emphasized: true, holds_notification: true, ...over };
}

const mac = { room: "a".repeat(43), name: "MacBook" };
const mini = { room: "b".repeat(43), name: "맥미니" };
const panes = (rows: readonly { row: GroupRow }[]) => rows.map(({ row: r }) => (r.kind === "agent" ? r.pane_id : r.sleep_id));

// PRD device-connect-iroh B55: every paired computer's 내 차례 and 읽을 것 in one place, each row with its computer.
describe("gatherGroups", () => {
  it("joins each computer's rows newest first, adds their counts and keeps each computer's own fold", () => {
    const macGroups: AgentGroup[] = [
      { group: "needs_you", count: 3, rows: [row("mac-new", 300), row("mac-old", 100)], more: [row("mac-folded", 50)] },
      { group: "working", count: 1, rows: [row("mac-working", 400, { group: "working" })] },
    ];
    const miniGroups: AgentGroup[] = [
      { group: "needs_you", count: 1, rows: [row("mini", 200)] },
      { group: "done", count: 1, rows: [row("mini-done", 10, { group: "done" })] },
    ];
    const gathered = gatherGroups([{ host: mac, here: true, groups: macGroups }, { host: mini, here: false, groups: miniGroups }]);
    expect(gathered.map((group) => group.group)).toEqual(["needs_you", "done"]);
    const needs = gathered[0]!;
    expect(needs.count).toBe(4);
    expect(panes(needs.rows)).toEqual(["mac-new", "mini", "mac-old"]);
    expect(needs.rows.map((item) => item.host.name)).toEqual(["MacBook", "맥미니", "MacBook"]);
    expect(needs.rows.map((item) => item.here)).toEqual([true, false, true]);
    expect(panes(needs.more)).toEqual(["mac-folded"]);
    expect(gathered[1]!.rows[0]!.host).toBe(mini);
  });

  it("carries no group but 내 차례 and 읽을 것, and leaves out a group no computer has", () => {
    const only = gatherGroups([{ host: mac, here: true, groups: [{ group: "working", count: 2, rows: [row("w", 1)] }, { group: "seen", count: 1, rows: [row("s", 1)] }] }]);
    expect(only).toEqual([]);
  });

  it("puts a row the core has no time for after every timed one", () => {
    const gathered = gatherGroups([
      { host: mac, here: true, groups: [{ group: "needs_you", count: 1, rows: [row("untimed", null)] }] },
      { host: mini, here: false, groups: [{ group: "needs_you", count: 1, rows: [row("timed", 1)] }] },
    ]);
    expect(panes(gathered[0]!.rows)).toEqual(["timed", "untimed"]);
  });
});

// PRD device-connect-iroh D-69, B62: each other computer says its own state, and a list that arrived needs no line.
describe("hostLine", () => {
  it("names why another computer's rows are not there", () => {
    expect(hostLine(undefined)).toBe("connecting");
    expect(hostLine({ online: true, lastSeen: 1 })).toBe("connecting");
    expect(hostLine({ online: false, lastSeen: 1 })).toBe("offline");
    expect(hostLine({ online: false, lastSeen: null, protocolMismatch: 6 })).toBe("update");
    expect(hostLine({ online: true, lastSeen: 1, gathered: [] })).toBeNull();
  });
});

describe("listScopeOf", () => {
  it("shows every computer unless the address asks for this one alone", () => {
    expect(listScopeOf(new URLSearchParams("host=x"))).toBe("all");
    expect(listScopeOf(new URLSearchParams("host=x&one=1"))).toBe("one");
  });
});
