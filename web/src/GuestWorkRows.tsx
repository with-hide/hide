import { useMemo } from "react";
import type { Actions } from "./actions";
import { DeviceChip } from "./components/device-chip";
import { StatusMark } from "./components/status-mark";
import { RowButton } from "./components/ui/row-button";
import { AgentMark } from "./AgentMark";
import { GuestMark, useGuestPanes } from "./GuestPaneScreen";
import { guestGroups, peerRows, type GuestPane, type PeerRow } from "./guestWork";
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
function GuestPaneRow({ pane, actions }: { pane: GuestPane; actions: Actions }) {
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
        onClick={() => actions.openGuestPane(pane.id)}
      >
        <span className="min-w-0 flex-1 truncate">{pane.label}</span>
        <GuestMark pane={pane} />
      </RowButton>
    </li>
  );
}

/** The panes of other machines' work started in one of `projectId`'s folders, under that project (B10). */
export function GuestProjectRows({ projectId, actions }: { projectId: string; actions: Actions }) {
  const panes = useGuestPanes();
  const mine = useMemo(() => panes.filter((pane) => pane.projectId === projectId), [panes, projectId]);
  if (mine.length === 0) return null;
  return (
    <>
      {mine.map((pane) => (
        <GuestPaneRow key={pane.id} pane={pane} actions={actions} />
      ))}
    </>
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

/** A connected machine's own rows in `group`, as that machine told them. */
export function usePeerRows(group: "needs_you" | "done"): PeerRow[] {
  const work = useShellStore((s) => s.rest?.navigator?.peer_work);
  const peers = useShellStore((s) => s.rest?.navigator?.peers);
  return useMemo(() => peerRows(useShellStore.getState().rest, group), [work, peers, group]);
}

/**
 * Another machine's own Needs You or Done rows (D-50, B49), after this
 * computer's: each wears that machine's filled chip, since the work is that
 * machine's and opens in its tile. This computer rings nothing for them; the
 * machine that owns the work picks where its notice goes.
 */
export function PeerWorkRows({ group, actions }: { group: "needs_you" | "done"; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rows = usePeerRows(group);
  if (rows.length === 0) return null;
  return (
    <>
      {rows.map(({ row, deviceId, machine }) => (
        <li key={`${deviceId}:${row.id}`}>
          <RowButton
            aria-label={t("pairing.work.peerRowAria", { title: row.title, name: machine })}
            data-peer-row={`${deviceId}:${row.id}`}
            data-peer-group={group}
            data-task-focus="row"
            className="min-h-(--size-control-lg) items-start gap-sm rounded-sm px-sm py-xs text-caption text-subtle-foreground hover:text-foreground"
            // That computer's work opens on its own screen with the work chosen (D-78, D-81); without a desktop to draw it, its tile.
            onClick={() => {
              if (!actions.openComputerWork(deviceId, { op: "show", pane: row.id })) actions.focusDevice(deviceId);
            }}
          >
            <span className="flex h-(--size-sidebar-line) shrink-0 items-center gap-xs">
              <StatusMark mark={markOf(row.mark)} read={row.read === true} />
              <AgentMark kind={row.agent_kind} />
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="flex min-w-0 items-center gap-xs">
                <span className="min-w-0 truncate text-foreground">{row.title}</span>
                <DeviceChip label={machine} variant="remote" className="max-w-2/5" />
              </span>
              {row.detail || row.place ? <span className="min-w-0 truncate text-muted-foreground">{row.detail ?? row.place}</span> : null}
            </span>
          </RowButton>
        </li>
      ))}
    </>
  );
}
