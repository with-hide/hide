// The phone app's home with one row that needs you, drawn by the phone's own `ListScreen` over invented rows.
// The phone keeps its state in its own store, which this part seeds and restores on the way out.
import { useLayoutEffect } from "react";
import { ListScreen } from "../../mobile/App";
import type { AgentGroup, GroupRow, PhoneAgent } from "../../mobile/protocol";
import { patch, usePhone } from "../../mobile/store";
import "../../mobile/mobile.css";
import { minutesAgo } from "../fixtures/agent";
import { PartFrame } from "../frame";

const MAC = "Work MacBook";

function agent(pane: string, title: string, ago: number, over: Partial<PhoneAgent> = {}): PhoneAgent {
  return {
    device_id: "local", pane_id: pane, root_pane_id: pane, group: "working", mark: "going", wait: null, descendants_working: 0, agent_kind: "claude", title, place: null, device_label: null,
    changed_at_unix_ms: minutesAgo(ago), line: null, status_code: "working", demand: "none", emphasized: false, holds_notification: false, ...over,
  };
}
const row = (value: PhoneAgent): GroupRow => ({ kind: "agent", ...value });

function groups(): AgentGroup[] {
  const asking = row(agent("p1", "api-refactor", 3, {
    group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, place: "orders-app · main",
    line: { text: "Keep the old endpoint for one release?", mode: "request", tone: "warning" },
  }));
  return [
    { group: "needs_you", count: 1, rows: [asking] },
    { group: "working", count: 2, rows: [row(agent("p2", "docs-update", 6, { place: "orders-app · main", line: { text: "editing 3 files", mode: "kept", tone: "subtle" } })), row(agent("p3", "fix-checkout", 12, { place: "orders-app · fix" }))] },
    { group: "seen", count: 1, rows: [row(agent("p4", "release-notes", 90, { group: "seen", mark: "quiet", status_code: "idle", place: "orders-app · main" }))] },
  ];
}

export default function PhonePart() {
  useLayoutEffect(() => {
    const before = usePhone.getState();
    patch({
      screen: "app", connected: true, unreachable: false, macName: MAC, otherPhones: 0, groups: groups(), devices: [], childrenOpen: {}, foldsOpen: {}, detail: null, agentsOmitted: 0, tab: "agents",
    });
    return () => usePhone.setState(before, true);
  }, []);
  return (
    <PartFrame part="phone" height={600}>
      <div className="mx-auto h-full w-full max-w-(--size-lane-panel-ideal) overflow-hidden rounded-xl border border-border bg-background"><ListScreen hosted={false} /></div>
    </PartFrame>
  );
}
