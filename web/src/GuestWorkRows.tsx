import { useMemo } from "react";
import type { Actions } from "./actions";
import { DeviceChip } from "./components/device-chip";
import { FoldRow } from "./components/fold-row";
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
 * stands under (`GuestMachineRows`); `inset` is where its mark starts. It
 * opens the pane in the center. A
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

const NO_FOLDS: { device_id: string; folder: string }[] = [];

/**
 * Where a place's heads and rows stand: where the head starts, where the
 * rows' marks start (the column of the place's own rows), and the column
 * whose center the rail left of the rows runs down (the glyph column a
 * tab's guide hangs from).
 */
export type GuestColumns = { head: string; rows: string; rail: string };

/**
 * Other machines' work in one place, one head per machine (operator review
 * 2026-11-25): a fold row with the machine's letters once,
 * `<machine>에서 시킨 일` and how many, then its rows, whose marks stand on
 * the column of the place's own rows, with a tree rail down their left that
 * ties them to the head.
 * A head folds as the list's other folds do, and the core keeps it by the
 * machine and `folder`, the place's folder (`""` outside every project);
 * folded, it shows its count and nothing more.
 */
function GuestMachineRows({ panes, folder, columns, actions }: { panes: readonly GuestPane[]; folder: string; columns: GuestColumns; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const machines = useMemo(() => byMachine(panes), [panes]);
  const folds = useShellStore((s) => s.rest?.ui_state?.folded_guest_work ?? NO_FOLDS);
  return (
    <ul data-guest-machines={folder === "" ? "outside" : "project"}>
      {machines.map((group) => {
        const open = !folds.some((fold) => fold.device_id === group.deviceId && fold.folder === folder);
        return (
          <li key={group.deviceId} data-guest-group={group.deviceId}>
            <FoldRow
              label={t("pairing.work.group", { name: group.machine })}
              count={group.panes.length}
              expanded={open}
              lead={<GuestMark pane={group} />}
              inset={columns.head}
              onToggle={() => actions.toggleGuestWorkFold(group.deviceId, folder)}
              data-guest-head={group.deviceId}
            />
            {open ? (
              <ul className="relative" aria-label={t("pairing.work.groupAria", { name: group.machine })}>
                <span aria-hidden="true" className="pointer-events-none absolute inset-y-xxs w-(--size-hairline) bg-lineage-rail" style={{ left: `calc(${columns.rail} - var(--size-hairline) / 2)` }} data-guest-rail={group.deviceId} />
                {group.panes.map((pane) => (
                  <GuestPaneRow key={pane.id} pane={pane} actions={actions} inset={columns.rows} />
                ))}
              </ul>
            ) : null}
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
 * it only while the project is open; `folder` is the project's.
 */
export function GuestProjectRows({ projectId, folder, rows = NO_CHECKOUTS, folded = NO_CHECKOUTS, columns, actions }: { projectId: string; folder: string; rows?: ReadonlySet<string>; folded?: ReadonlySet<string>; columns: GuestColumns; actions: Actions }) {
  const panes = useGuestPanes();
  const atEnd = useMemo(() => guestPlaces(panes, projectId, { open: true, rows, folded }).atEnd, [panes, projectId, rows, folded]);
  if (atEnd.length === 0) return null;
  return (
    <li data-project-guests={projectId}>
      <GuestMachineRows panes={atEnd} folder={folder} columns={columns} actions={actions} />
    </li>
  );
}

/** The panes of other machines' work started in `checkoutId`'s folder, after that checkout's own rows (B10); `folder` is the checkout's. */
export function GuestCheckoutRows({ projectId, checkoutId, folder, columns, actions }: { projectId: string; checkoutId: string; folder: string; columns: GuestColumns; actions: Actions }) {
  const panes = useGuestPanes();
  const mine = useMemo(() => guestPlaces(panes, projectId, { open: true, rows: new Set([checkoutId]), folded: NO_CHECKOUTS }).byCheckout.get(checkoutId) ?? NO_PANES, [panes, projectId, checkoutId]);
  if (mine.length === 0) return null;
  return <GuestMachineRows panes={mine} folder={folder} columns={columns} actions={actions} />;
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
 * computer, one head per machine that asked for them, worded as every
 * other head is (B10). `projects` are the projects whose rows draw their
 * own.
 */
export function GuestMachineGroups({ actions, projects, columns }: { actions: Actions; projects: ReadonlySet<string>; columns: GuestColumns }) {
  const panes = useGuestPanes();
  const outside = useMemo(() => panes.filter((pane) => pane.projectId === null || !projects.has(pane.projectId)), [panes, projects]);
  if (outside.length === 0) return null;
  return (
    <li className="mt-xs" data-guest-outside="true">
      <GuestMachineRows panes={outside} folder="" columns={columns} actions={actions} />
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
