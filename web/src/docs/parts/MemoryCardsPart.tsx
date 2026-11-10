// A short list of Project Memory cards, drawn by the product's own Memory tab over four invented cards:
// what the project's agents were told, which kind of fact each is, and how often it was used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../../actions";
import { MemoryTab } from "../../MemoryTab";
import type { MemoryRow, ProjectMemory, SnapshotRest } from "../../snapshot";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { minutesAgo } from "../fixtures/agent";
import { docsProject } from "../fixtures/project";
import { PartFrame } from "../frame";

const DAY = 1440;
const card = (number: number, title: string, kind: MemoryRow["kind"], given: number, pulled: number, human: number, age: number): MemoryRow => ({
  number, title, kind, provided_session_count: given, pull_count: pulled, human_source_count: human, updated_at_unix_ms: minutesAgo(age), verifying: false, promotion: null,
});
const ROWS: MemoryRow[] = [
  card(3, "Run the tests with scripts/verify.sh, not by hand", "rule", 9, 12, 3, DAY),
  card(7, "The orders API keeps its old endpoint for one release", "decision", 6, 4, 2, 3 * DAY),
  card(12, "The staging database resets every night at 02:00", "fact", 4, 2, 1, 5 * DAY),
  card(17, "A slow checkout test usually means the fixture is stale", "lesson", 2, 1, 1, 60),
];

function fixture() {
  const { rest, project, checkout } = docsProject();
  rest.navigator!.focused_workspace_id = project.id;
  rest.navigator!.focused_checkout_id = checkout.id;
  const memory: ProjectMemory = {
    workspace_id: project.id, unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
    rows: ROWS, questions: [], analysis: "idle", consent_required: false, usable_cards: ROWS.length, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null, expanded: null, search: null,
  };
  const ready = { ...rest, status: { ...rest.status, background_ai: { enabled: true, provider: "claude", chosen: true, unavailable_reason: null, providers: [{ id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", selectable: true }] } } } as unknown as SnapshotRest;
  return { rest: ready, memory };
}

export default function MemoryCardsPart() {
  const { rest, memory } = useMemo(fixture, []);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, agents: [], connection: "gone", memory: { project: memory } });
    useUiStore.setState({ screen: { kind: "workspace" } });
  }, [rest, memory]);
  return (
    <PartFrame part="memory-cards">
      <div className="flex min-w-0 flex-col rounded-md border border-border bg-card"><MemoryTab actions={actions} /></div>
    </PartFrame>
  );
}
