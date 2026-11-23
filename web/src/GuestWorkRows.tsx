import { useMemo } from "react";
import type { Actions } from "./actions";
import { StatusMark } from "./components/status-mark";
import { RowButton } from "./components/ui/row-button";
import { AgentRowMark } from "./AgentRowMark";
import { Elapsed } from "./components/elapsed";
import { SessionRowBody } from "./components/session-row";
import { GuestMark, useGuestPanes } from "./GuestPaneScreen";
import { guestGroups, guestPlaces, peerMachines, peerReasonKey, peerRows, type GuestPane, type PeerRow } from "./guestWork";
import { ownWindow } from "./pairing";
import { tileMonogram } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import type { RowMark } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

const MARKS: readonly string[] = ["turn", "look", "going", "waiting", "done", "unread", "quiet", "unknown", "asleep"] satisfies RowMark[];

/** The mark another machine drew its row with, when this build knows it. */
function markOf(mark: string): RowMark | null {
  return MARKS.includes(mark) ? (mark as RowMark) : null;
}

/**
 * One pane of another machine's work on this computer (PRD
 * device-connect-iroh B10): its folder's name and the machine's letters. It
 * opens the pane in the center; the work's state is the other machine's to
 * show, so the row carries none.
 */
function GuestPaneRow({ pane, actions, inset }: { pane: GuestPane; actions: Actions; /** Where the name starts, to line up under a checkout's own rows. */ inset?: string }) {
  const { t } = useInterfaceTranslation();
  const selected = useUiStore((s) => s.screen?.kind === "guest" && s.screen.paneId === pane.id);
  return (
    <li>
      <RowButton
        aria-label={t("pairing.work.rowAria", { label: pane.label, name: pane.machine })}
        aria-current={selected ? "true" : undefined}
        data-guest-row={pane.id}
        data-task-focus="row"
        className={cn("min-h-(--size-control-lg) gap-sm rounded-sm px-sm py-0 text-caption", selected ? "bg-secondary text-foreground hover:bg-secondary" : "text-subtle-foreground hover:text-foreground")}
        style={inset ? { paddingLeft: inset } : undefined}
        onClick={() => actions.openGuestPane(pane.id)}
      >
        <span className="min-w-0 flex-1 truncate">{pane.label}</span>
        <GuestMark pane={pane} />
      </RowButton>
    </li>
  );
}

const NO_ROWS: ReadonlySet<string> = new Set();

/**
 * The panes of other machines' work started in one of `projectId`'s folders
 * that stand at the project's end (B10): every one when the list draws no
 * checkout rows, else those whose checkout `rows` leaves out
 * (`guestPlaces`).
 */
export function GuestProjectRows({ projectId, rows = NO_ROWS, actions }: { projectId: string; rows?: ReadonlySet<string>; actions: Actions }) {
  const panes = useGuestPanes();
  const atEnd = useMemo(() => guestPlaces(panes, projectId, rows).atEnd, [panes, projectId, rows]);
  if (atEnd.length === 0) return null;
  return (
    <>
      {atEnd.map((pane) => (
        <GuestPaneRow key={pane.id} pane={pane} actions={actions} />
      ))}
    </>
  );
}

/** The panes of other machines' work started in `checkoutId`'s folder, after that checkout's own rows (B10). */
export function GuestCheckoutRows({ projectId, checkoutId, inset, actions }: { projectId: string; checkoutId: string; inset: string; actions: Actions }) {
  const panes = useGuestPanes();
  const mine = useMemo(() => guestPlaces(panes, projectId, new Set([checkoutId])).byCheckout.get(checkoutId) ?? [], [panes, projectId, checkoutId]);
  if (mine.length === 0) return null;
  return (
    <ul data-checkout-guests={checkoutId}>
      {mine.map((pane) => (
        <GuestPaneRow key={pane.id} pane={pane} actions={actions} inset={inset} />
      ))}
    </ul>
  );
}

/**
 * The panes of other machines' work that stand under no project of this
 * computer, grouped by the machine that asked for them (B10). `projects`
 * are the projects whose rows draw their own; with none given every pane is
 * listed here, for a list that draws no projects.
 */
export function GuestMachineGroups({ actions, projects = null }: { actions: Actions; projects?: ReadonlySet<string> | null }) {
  const { t } = useInterfaceTranslation();
  const panes = useGuestPanes();
  const groups = useMemo(() => guestGroups(panes.map((pane) => (pane.projectId !== null && projects?.has(pane.projectId) ? pane : { ...pane, projectId: null }))), [panes, projects]);
  if (groups.length === 0) return null;
  return (
    <>
      {groups.map((group) => (
        <li key={group.deviceId} className="mt-xs" data-guest-group={group.deviceId}>
          <div className="flex h-(--size-sidebar-section-header) items-center px-sm text-micro font-medium text-subtle-foreground" id={`guest-group-${group.deviceId}`}>
            {t("pairing.work.group", { name: group.machine })}
          </div>
          <ul aria-label={t("pairing.work.groupAria", { name: group.machine })}>
            {group.panes.map((pane) => (
              <GuestPaneRow key={pane.id} pane={pane} actions={actions} />
            ))}
          </ul>
        </li>
      ))}
    </>
  );
}

/**
 * A connected machine's own rows in `group`, as that machine told them, on
 * this computer's own window only: another computer showing this one's
 * screen shows this computer's work and nobody else's.
 */
export function usePeerRows(group: "needs_you" | "done"): PeerRow[] {
  const own = useShellStore((s) => ownWindow(s.pairing));
  const work = useShellStore((s) => s.rest?.navigator?.peer_work);
  const peers = useShellStore((s) => s.rest?.navigator?.peers);
  return useMemo(() => (own ? peerRows(useShellStore.getState().rest, group) : NO_PEER_ROWS), [own, work, peers, group]);
}

const NO_PEER_ROWS: PeerRow[] = [];

/**
 * Another machine's own Needs You or Done rows (D-50, B49), after this
 * computer's, under one small head per machine with its letters, its name
 * and its count; this computer's own rows keep no head. Each row is drawn as
 * this computer's own are: the status mark and the agent's logo as the logo
 * view mode says, the title and the time since it changed, then where it
 * runs and why it waits. This computer rings nothing for them; the machine
 * that owns the work picks where its notice goes.
 */
export function PeerWorkRows({ group, actions, treeIndent = false }: { group: "needs_you" | "done"; actions: Actions; /** Leave the tree chevron's room, so the rows line up under a list whose rows have one. */ treeIndent?: boolean }) {
  const rows = usePeerRows(group);
  const machines = useMemo(() => peerMachines(rows), [rows]);
  if (rows.length === 0) return null;
  return (
    <>
      {machines.map((machine) => (
        <li key={machine.deviceId} data-peer-machine={machine.deviceId} className={treeIndent ? "mt-xs" : undefined}>
          {/* Under a list whose rows keep a tree chevron's room, the head stands on the rows' inner line, inside the state's head (operator review 2026-11-22). */}
          <div className={cn("flex h-(--size-sidebar-section-header) items-center gap-xs text-micro font-medium text-muted-foreground", treeIndent ? "pr-xs" : "px-xs")} data-peer-machine-head={machine.deviceId}>
            {treeIndent ? <span aria-hidden="true" className="w-(--size-lineage-chevron) shrink-0" /> : null}
            <span aria-hidden="true" className="rounded-xs bg-secondary px-xxs font-semibold text-subtle-foreground">{tileMonogram(machine.name)}</span>
            <span className="min-w-0 truncate">{machine.name}</span>
            <span className="tabular-nums">{machine.rows.length}</span>
          </div>
          <ul aria-label={machine.name} className="flex min-w-0 flex-col">
            <PeerMachineRows group={group} rows={machine.rows} actions={actions} treeIndent={treeIndent} />
          </ul>
        </li>
      ))}
    </>
  );
}

function PeerMachineRows({ group, rows, actions, treeIndent }: { group: "needs_you" | "done"; rows: PeerRow[]; actions: Actions; treeIndent: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      {rows.map(({ row, deviceId, machine }) => {
        const reasonKey = peerReasonKey(row);
        const reason = row.detail ?? (reasonKey ? t(reasonKey) : null);
        return (
          <li key={`${deviceId}:${row.id}`}>
            <RowButton
              aria-label={[t("pairing.work.peerRowAria", { title: row.title, name: machine }), row.place, reason].filter(Boolean).join(", ")}
              data-peer-row={`${deviceId}:${row.id}`}
              data-peer-group={group}
              data-task-focus="row"
              className={cn("items-start py-xs text-caption text-subtle-foreground hover:text-foreground", treeIndent ? "rounded-xs pr-xs pl-none" : "min-h-(--size-control-lg) rounded-sm px-sm")}
              // That computer's work opens on its own screen with the work chosen (D-78, D-81); without a desktop to draw it, its tile.
              onClick={() => {
                if (actions.openComputerWork(deviceId, { op: "show", pane: row.id }) !== "handed") actions.focusDevice(deviceId);
              }}
            >
              {treeIndent ? <span aria-hidden="true" className="w-(--size-lineage-chevron) shrink-0" /> : null}
              <SessionRowBody
                mark={<>
                  <StatusMark mark={markOf(row.mark)} read={row.read === true} />
                  <AgentRowMark kind={row.agent_kind} />
                </>}
                head={<>
                  <span className={cn("min-w-0 flex-1 truncate text-foreground", treeIndent && "font-medium")}>{row.title}</span>
                  <Elapsed since={row.changed_at_unix_ms} aria-hidden="true" className="shrink-0 font-mono text-caption text-muted-foreground" data-peer-elapsed="true" />
                </>}
              >
                {row.place || reason ? (
                  // Under the task list the line is drawn as its own rows' second line: a step down, in micro text.
                  <span aria-hidden="true" className={cn("flex min-w-0 gap-xs leading-(--size-sidebar-line-detail)", treeIndent ? "mt-xs items-baseline text-micro" : "text-caption")}>
                    {row.place ? <span className="min-w-0 shrink truncate text-muted-foreground">{row.place}</span> : null}
                    {reason ? <span className={cn("min-w-0 truncate", group === "needs_you" ? "text-warning" : "text-muted-foreground")} data-peer-reason="true">{reason}</span> : null}
                  </span>
                ) : null}
              </SessionRowBody>
            </RowButton>
          </li>
        );
      })}
    </>
  );
}
