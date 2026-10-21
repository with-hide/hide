import type { TFunction } from "i18next";
import type { Subtree, SubtreeRow, SubtreeState } from "../close";
import { useInterfaceTranslation } from "../i18n/client";
import { statusText } from "../agentStatus";
import type { MessageKey } from "../i18n/catalogs";
import { markTone, markWords, recordWords } from "../agentRow";
import { machineName } from "../screenMachine";
import type { RowMark as RowMarkValue } from "../snapshot";
import { useShellStore } from "../store";
import { AgentRecordTitle } from "./agent-record-title";
import { DeviceChip } from "./device-chip";
import { StatusMark } from "./status-mark";
import { Hint } from "./ui/tooltip";

// The descendants a close or a removal would take with it (PRD
// close-agent-subtree D-17, D-32, D-33, D-42): a summary line of what needs
// the operator, then the tree, each row as the sidebar draws it. Colour is the
// status marks' alone. A row that is working, waiting for an answer, holding
// an unread result or unreadable is bright and says its state in a neutral
// word; a quiet one is dimmed, and its mark's tooltip and the row's
// accessible name carry its state.

// The four marks mean here what they mean on every row (B21): a request
// waiting for an answer is `!`, work in progress the ring, an unread result
// the dot; an unknown state has no mark and says so in its word.
const SUMMARY: { state: Exclude<SubtreeState, "quiet">; mark: RowMarkValue; word: MessageKey }[] = [
  { state: "waiting", mark: "turn", word: "agents.subtree.waiting" },
  { state: "working", mark: "going", word: "agents.subtree.working" },
  { state: "unread", mark: "unread", word: "agents.subtree.unread" },
  { state: "unknown", mark: "unknown", word: "agents.subtree.unknown" },
];

/** What the list is for: the agents that close with the one closed, or the agents outside a project that a removal stops. */
export type SubtreeScope = "close" | "removal";

const LIST_LABEL: Record<SubtreeScope, MessageKey> = { close: "agents.subtree.closesWith", removal: "agents.subtree.outside" };

/** The summary in words, zero kinds left out, for a screen reader and a test. */
export function subtreeSummaryWords(counts: Subtree["counts"], t: TFunction<"translation">): string {
  return SUMMARY.filter(({ state }) => counts[state] > 0)
    .map(({ state, word }) => `${t(word)} ${counts[state]}`)
    .join(" · ");
}

export function SubtreeList({ subtree, targetDevice, scope = "close" }: { subtree: Subtree; targetDevice: string | undefined; scope?: SubtreeScope }) {
  const { t } = useInterfaceTranslation();
  const parts = SUMMARY.filter(({ state }) => subtree.counts[state] > 0);
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-subtree-list="true">
      {parts.length > 0 ? (
        <p className="flex flex-wrap items-center gap-x-sm gap-y-xxs text-caption text-subtle-foreground" data-subtree-summary={subtreeSummaryWords(subtree.counts, t)}>
          {parts.map(({ state, mark, word }) => (
            <span key={state} className="inline-flex items-center gap-xxs" data-subtree-count={state}>
              <StatusMark mark={mark} />
              <span>
                {t(word)} {subtree.counts[state]}
              </span>
            </span>
          ))}
        </p>
      ) : null}
      <ul className="flex max-h-(--size-relationship-list-max) min-w-0 flex-col gap-xxs overflow-y-auto rounded-md border border-border p-xs" aria-label={t(LIST_LABEL[scope])}>
        {subtree.rows.map((row) => (
          <SubtreeItem key={row.agent.pane_id} row={row} targetDevice={targetDevice} />
        ))}
      </ul>
    </div>
  );
}

function SubtreeItem({ row, targetDevice }: { row: SubtreeRow; targetDevice: string | undefined }) {
  const { t } = useInterfaceTranslation();
  const { agent } = row;
  const quiet = row.state === "quiet";
  // A sleeping agent's moon is never dimmed (status-marks D-21).
  const bright = row.target || !quiet || agent.mark === "asleep";
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const device = agent.device_id && agent.device_id !== targetDevice ? machineName(devices, agent.device_id, t) : null;
  return (
    <li
      tabIndex={0}
      aria-label={[agent.identity_label, recordWords(t, agent), agent.record_summary, device, statusText(t, agent.status_code)].filter(Boolean).join(", ")}
      data-subtree-row={agent.pane_id}
      data-subtree-state={row.state}
      data-subtree-target={row.target ? "true" : undefined}
      className={`flex min-w-0 items-start gap-xs rounded-xs px-xs py-xxs outline-none focus-visible:ring-2 focus-visible:ring-ring ${bright ? "" : "opacity-(--opacity-read-status)"}`}
      style={{ paddingInlineStart: `calc(var(--spacing-xs) + var(--spacing-lg) * ${row.depth})` }}
    >
      <RowMark mark={agent.mark} tone={markTone(agent)} status={quiet ? markWords(t, agent) : null} />
      <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-xs gap-y-xxs" aria-hidden="true">
        <AgentRecordTitle agent={agent} className={row.target ? "font-medium text-foreground" : "text-foreground"} />
        {quiet ? null : <span className="shrink-0 text-caption text-subtle-foreground" data-subtree-status="true">{statusText(t, agent.status_code)}</span>}
        {device ? <DeviceChip label={device} className="max-w-full" /> : null}
      </span>
    </li>
  );
}

/**
 * A close list row's status mark. A quiet row shows no status word, so its
 * mark says the state on hover (D-42); the row's accessible name says it too.
 */
export function RowMark({ mark: rowMark, tone, status }: { mark: RowMarkValue; tone: string; status: string | null }) {
  const mark = <StatusMark mark={rowMark} className={`mt-xxs ${tone}`} />;
  if (!status) return mark;
  return (
    <Hint label={status} reveals>
      <span className="inline-flex shrink-0" data-row-mark-hint={status}>
        {mark}
      </span>
    </Hint>
  );
}
