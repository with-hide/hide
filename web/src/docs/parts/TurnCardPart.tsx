// The turn card the product opens from a pane's header, drawn by the real `TurnCard` over two invented
// sessions: one that asks a question and one that finished. `state=question` or `state=answer` shows one.
import { TurnCard } from "../../PaneTurnCard";
import type { AgentRow, PaneHeader } from "../../snapshot";
import { docsAgent, minutesAgo } from "../fixtures/agent";
import { PartFrame } from "../frame";
import type { PartProps } from "../renderers";

const operator = { kind: "operator" } as const;

const QUESTION: { label: string; header: PaneHeader; agent: AgentRow } = {
  label: "Waiting for your answer",
  header: {
    working: false,
    notice: null,
    band: { kind: "answer", reason: "Keep the old endpoint for one release?", since_unix_ms: minutesAgo(2), read: false, action: null, more: 0 },
    request: { text: "Move the orders API to v2", images: 0 },
  },
  agent: docsAgent("docs-question", {
    request: {
      verb: "answer", verb_since_unix_ms: minutesAgo(2), pull_requests: [], later_by: null,
      request: { text: "Move the orders API to v2 and update the callers", cut: false, images: 0, at_unix_ms: minutesAgo(14), sender: operator },
      request_summary: "Move the orders API to v2",
      recent_turns: [{ request: "Add paging to the orders list", end: "done", line: "Paging added, tests pass", at_unix_ms: minutesAgo(55) }],
    },
  }),
};

const ANSWER: { label: string; header: PaneHeader; agent: AgentRow } = {
  label: "Finished",
  header: { working: false, finished: true, notice: null, band: null, progress: "Moved to v2, 14 callers updated", request: { text: "Rename the export button", images: 0 } },
  agent: docsAgent("docs-answer", {
    request: {
      verb: "result", verb_since_unix_ms: minutesAgo(6), end: "done", line: "Renamed the button and its test", pull_requests: [], later_by: null,
      request: { text: "Rename the export button to Download CSV", cut: false, images: 0, at_unix_ms: minutesAgo(22), sender: operator },
      request_summary: "Rename the export button",
      answer: { offset: 1, end: "done", line: "The button now says Download CSV", head: "Done. The button says Download CSV in the toolbar and in the menu, and its test checks the new label.", whole: false, at_unix_ms: minutesAgo(6) },
    },
  }),
};

export default function TurnCardPart({ state }: PartProps) {
  const cards = state === "question" ? [QUESTION] : state === "answer" ? [ANSWER] : [QUESTION, ANSWER];
  return (
    <PartFrame part="turn-card">
      <div className="mx-auto flex max-w-(--size-turn-card) flex-col gap-lg">
        {cards.map(({ label, header, agent }) => (
          <div key={agent.id} className="flex flex-col gap-xs" data-docs-card={agent.id}>
            <div className="text-micro text-muted-foreground">{label}</div>
            <div className="rounded-md border border-border bg-popover text-caption text-popover-foreground shadow-lg">
              <TurnCard agent={agent} header={header} onRecord={null} onReadWhole={() => undefined} onAsk={() => undefined} />
            </div>
          </div>
        ))}
      </div>
    </PartFrame>
  );
}
