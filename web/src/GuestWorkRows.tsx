import { useMemo } from "react";
import type { Actions } from "./actions";
import { DeviceChip } from "./components/device-chip";
import { StatusMark } from "./components/status-mark";
import { RowButton } from "./components/ui/row-button";
import { AgentRowMark } from "./AgentRowMark";
import { Elapsed } from "./components/elapsed";
import { SessionRowBody } from "./components/session-row";
import { GuestMark, useGuestPanes } from "./GuestPaneScreen";
import { byMachine, guestPlaces, markOf, peerReasonKey, peerRows, type GuestPane, type PeerRow } from "./guestWork";
import { ownWindow } from "./pairing";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/**
 * One pane of another machine's work on this computer (PRD
 * device-connect-iroh B10), drawn as this computer's own rows are: the mark
 * its machine draws it with, its agent's logo while rows draw logos, and the
 * name its machine calls it by. Its machine is named once by the head it
 * stands under (`GuestMachineRows`). It opens the pane in the center. A
 * machine that told no mark leaves the mark's slot empty, so names line up.
 */
function GuestPaneRow({ pane, actions, inset }: { pane: GuestPane; actions: Actions; inset: string }) {
  const { t } = useInterfaceTranslation();
  const selected = useUiStore((s) => s.screen?.kind === "guest" && s.screen.paneId === pane.id);
  return (
    <li>
      <RowButton
        aria-label={t("pairing.work.rowAria", { label: pane.label, name: pane.machine })}
        aria-current={selected ? "true" : undefined}
        data-guest-row={pane.id}
        data-task-focus="row"
        className={cn("min-h-(--size-control-lg) gap-sm rounded-sm py-0 pr-sm text-caption", selected ? "bg-secondary text-foreground hover:bg-secondary" : "text-subtle-foreground hover:text-foreground")}
        style={{ paddingLeft: inset }}
        onClick={() => actions.openGuestPane(pane.id)}
      >
        <StatusMark mark={pane.status} data-guest-row-status={pane.status ?? undefined} />
        {pane.agent ? <AgentRowMark kind={pane.agent} /> : null}
        <span className="min-w-0 flex-1 truncate">{pane.label}</span>
      </RowButton>
    </li>
  );
}

/**
 * Other machines' work in one place, one head per machine (operator review
 * 2026-11-25): the machine's letters once, `<machine>에서 시킨 일` (or its
 * words for work outside every project) and how many, then its rows one
 * tree step in. `inset` is where the head starts.
 */
function GuestMachineRows({ panes, inset, outside = false, actions }: { panes: readonly GuestPane[]; inset: string; /** The work stands in no project of this computer. */ outside?: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const machines = useMemo(() => byMachine(panes), [panes]);
  const rowInset = `calc(${inset} + var(--size-lineage-indent))`;
  return (
    <ul data-guest-machines={outside ? "outside" : "project"}>
      {machines.map((group) => {
        const label = t(outside ? "pairing.work.outside" : "pairing.work.group", { name: group.machine });
        return (
          <li key={group.deviceId} data-guest-group={group.deviceId}>
            <div className="flex h-(--size-control-compact) min-w-0 items-center gap-xs pr-sm text-caption text-muted-foreground" style={{ paddingLeft: inset }} data-guest-head={group.deviceId}>
              <GuestMark pane={group} />
              <span className="min-w-0 truncate">{label}</span>
              <span className="shrink-0" data-guest-count={group.panes.length}>{group.panes.length}</span>
            </div>
            <ul aria-label={t("pairing.work.groupAria", { name: group.machine })}>
              {group.panes.map((pane) => (
                <GuestPaneRow key={pane.id} pane={pane} actions={actions} inset={rowInset} />
              ))}
            </ul>
          </li>
        );
      })}
    </ul>
  );
}

const NO_CHECKOUTS: ReadonlySet<string> = new Set();

/**
 * The panes of other machines' work started in one of `projectId`'s folders
 * that stand at the project's end, inside it (B10): those whose checkout the
 * tree draws no row for and no fold keeps (`guestPlaces`). The caller draws
 * it only while the project is open.
 */
export function GuestProjectRows({ projectId, rows = NO_CHECKOUTS, folded = NO_CHECKOUTS, inset, actions }: { projectId: string; rows?: ReadonlySet<string>; folded?: ReadonlySet<string>; inset: string; actions: Actions }) {
  const panes = useGuestPanes();
  const atEnd = useMemo(() => guestPlaces(panes, projectId, { open: true, rows, folded }).atEnd, [panes, projectId, rows, folded]);
  if (atEnd.length === 0) return null;
  return (
    <li data-project-guests={projectId}>
      <GuestMachineRows panes={atEnd} inset={inset} actions={actions} />
    </li>
  );
}

/** The panes of other machines' work started in `checkoutId`'s folder, after that checkout's own rows (B10). */
export function GuestCheckoutRows({ projectId, checkoutId, inset, actions }: { projectId: string; checkoutId: string; inset: string; actions: Actions }) {
  const panes = useGuestPanes();
  const mine = useMemo(() => guestPlaces(panes, projectId, { open: true, rows: new Set([checkoutId]), folded: NO_CHECKOUTS }).byCheckout.get(checkoutId) ?? NO_PANES, [panes, projectId, checkoutId]);
  if (mine.length === 0) return null;
  return <GuestMachineRows panes={mine} inset={inset} actions={actions} />;
}

const NO_PANES: GuestPane[] = [];

/**
 * The pane of another machine's work the center shows, when it is one of
 * `projectId`'s: a fold that hides its row is marked as holding it, as for
 * this computer's own checkout in front.
 */
export function useGuestInFront(projectId: string): GuestPane | null {
  const paneId = useUiStore((s) => (s.screen?.kind === "guest" ? s.screen.paneId : null));
  const panes = useGuestPanes();
  return useMemo(() => (paneId === null ? null : (panes.find((pane) => pane.id === paneId && pane.projectId === projectId) ?? null)), [paneId, panes, projectId]);
}

/**
 * The panes of other machines' work that stand under no project of this
 * computer, one head per machine that asked for them, named so the
 * difference reads in words (B10). `projects` are the projects whose rows
 * draw their own.
 */
export function GuestMachineGroups({ actions, projects, inset }: { actions: Actions; projects: ReadonlySet<string>; inset: string }) {
  const panes = useGuestPanes();
  const outside = useMemo(() => panes.filter((pane) => pane.projectId === null || !projects.has(pane.projectId)), [panes, projects]);
  if (outside.length === 0) return null;
  return (
    <li className="mt-xs" data-guest-outside="true">
      <GuestMachineRows panes={outside} inset={inset} outside actions={actions} />
    </li>
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
 * computer's, one machine's rows together in the order the machines told
 * them. Each row is drawn as this computer's own are: the status mark and the
 * agent's logo as the logo view mode says, the title and the time since it
 * changed, then where it runs and why it waits, and its second line ends with
 * the machine's chip, the one another device's row in 내 차례 wears (operator
 * review 2026-11-23). This computer rings nothing for them; the machine that
 * owns the work picks where its notice goes.
 */
export function PeerWorkRows({ group, actions, treeIndent = false }: { group: "needs_you" | "done"; actions: Actions; /** Leave the tree chevron's room, so the rows line up under a list whose rows have one. */ treeIndent?: boolean }) {
  const { t } = useInterfaceTranslation();
  const rows = usePeerRows(group);
  return (
    <>
      {rows.map(({ row, deviceId, machine }) => {
        const reasonKey = peerReasonKey(row);
        const reason = row.detail ?? (reasonKey ? t(reasonKey) : null);
        return (
          <li key={`${deviceId}:${row.id}`} data-peer-machine={deviceId}>
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
                {/* Under the task list the line is drawn as its own rows' second line: a step down, in micro text.
                    Room goes in order, not by share: why it waits keeps its width (up to three fifths), the place keeps what it can,
                    and the chip takes only what is left, from its mark and a letter up to two fifths. */}
                <span aria-hidden="true" className={cn("flex min-w-0 items-center gap-xs leading-(--size-sidebar-line-detail)", treeIndent ? "mt-xs text-micro" : "text-caption")}>
                  {row.place ? <span className="min-w-0 shrink truncate text-muted-foreground">{row.place}</span> : null}
                  {reason ? <span className={cn("max-w-3/5 shrink-0 truncate", group === "needs_you" ? "text-warning" : "text-muted-foreground")} data-peer-reason="true">{reason}</span> : null}
                  <span className="flex min-w-0 max-w-2/5 shrink-0 grow basis-xxxl">
                    <DeviceChip label={machine} className="max-w-full" />
                  </span>
                </span>
              </SessionRowBody>
            </RowButton>
          </li>
        );
      })}
    </>
  );
}
