import { ArrowRightIcon, ChevronRightIcon, ListTreeIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { recordWords, lineParts, lineText, lineTone, markTone, rowDimmed, treeLine } from "../agentRow";
import { RowLineText } from "./row-line-text";
import { deviceConnected, localDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { relationState } from "../lineage";
import { cn } from "../lib/utils";
import { remoteTargetOfPane } from "../remote";
import type { AgentRow, RaisedAsk } from "../snapshot";
import { statusText } from "../agentStatus";
import { machineName } from "../screenMachine";
import { useShellStore } from "../store";
import { SIBLINGS_SHOWN } from "../sidebarTree";
import { useUiStore } from "../ui";
import { askWhat, AskLine, DescendantMark, TreeChevron, TreeRails, treePlaces } from "./agent-tree";
import { AgentPrHover } from "./pr-mark";
import { AgentRecordTitle } from "./agent-record-title";
import { rowPulls } from "../prMark";
import { DeviceChip } from "./device-chip";
import { Elapsed } from "./elapsed";
import { StatusMark } from "./status-mark";
import { Command, CommandGroup, CommandItem, CommandList, CommandSeparator } from "./ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Hint } from "./ui/tooltip";

/** A parent's children, most urgent first (B11): asking or raised, working, finished unread, the rest. */
export function childrenOf(parent: AgentRow, byPane: ReadonlyMap<string, AgentRow>): AgentRow[] {
  return (parent.lineage_child_pane_ids ?? [])
    .map((id) => byPane.get(id))
    .filter((row): row is AgentRow => row !== undefined)
    .map((row, index) => ({ row, index }))
    .sort((a, b) => a.row.state.tree_rank - b.row.state.tree_rank || a.index - b.index)
    .map(({ row }) => row);
}

type Line =
  | { kind: "agent"; agent: AgentRow; depth: number; hasChildren: boolean; open: boolean }
  | { kind: "more" | "finished"; parent: string; depth: number; count: number };

/**
 * A child that finished and was read waits folded below its siblings (B18,
 * D-13), unless a descendant below it still asks or works.
 */
const finishedAndRead = (row: AgentRow) => row.status_code === "done" && !row.unread && !row.descendant_mark;

/**
 * The tree popover (PRD D-13, D-27, D-28, D-40; B6, B13, B18, B19): its head
 * is the parent's name with the tree icon and its direct child count, its
 * body the same tree rows two levels deep, each with its own PR mark and,
 * folded, its descendant mark, and its last line opens the Agents graph.
 * Opened from a band's child trigger or "외 N건" (`raised`), the raised descendants come
 * first, their branches open, each with its verb, what and Open. A
 * grandchild with children re-roots the popover on itself. Arrows move,
 * Right and Left open and fold, Enter opens the pane, Escape closes and
 * returns focus to what opened it. No status word and no total are drawn;
 * the status word stays in each row's accessible name.
 */
export function AgentTreePopover({
  parent,
  agents,
  raised = false,
  onOpenChild,
  onTask,
  returnFocus,
  trigger,
  triggerLabel,
}: {
  parent: AgentRow;
  agents: readonly AgentRow[];
  /** Opened for raised descendants: their branches first, with their asks. */
  raised?: boolean;
  onOpenChild: (paneId: string, label: string) => void;
  onTask: (() => void) | null;
  returnFocus: () => void;
  trigger: ReactNode;
  triggerLabel: string;
}) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const [rootId, setRootId] = useState(parent.pane_id);
  const [opening, setOpening] = useState<string | null>(null);
  const byPane = useMemo(() => new Map(agents.map((row) => [row.pane_id, row])), [agents]);
  const root = byPane.get(rootId) ?? parent;
  // The core stores asks on the lineage root, but gives every ancestor its
  // subtree mark. A nested pane uses those same asks within its own subtree.
  const asks = useMemo(() => {
    const lineageRoot = byPane.get(parent.lineage_path_pane_ids?.[0] ?? parent.pane_id) ?? parent;
    const rows = raised ? lineageRoot.raised ?? [] : [];
    return new Map(rows.filter((ask) => lineageRoot.pane_id === parent.pane_id || byPane.get(ask.raised_pane_id)?.lineage_path_pane_ids?.includes(parent.pane_id)).map((ask) => [ask.raised_pane_id, ask]));
  }, [raised, parent, byPane]);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [shownAll, setShownAll] = useState<Set<string>>(() => new Set());
  const [finishedShown, setFinishedShown] = useState<Set<string>>(() => new Set());
  const relation = useUiStore((state) => state.relation);
  const outcome = useShellStore((state) => state.rest?.status?.pane_focus_request);
  const tracked = relation?.sourcePaneId === parent.pane_id && relation.targetPaneId === opening ? relation : null;
  const progress = relationState(tracked, outcome, t);
  const list = useRef<HTMLDivElement>(null);
  const afterClose = useRef<(() => void) | null>(null);
  const directs = childrenOf(root, byPane);
  const empty = directs.length === 0;

  useEffect(() => {
    if (!open) return;
    // Opened with raised requests, their branches start open; otherwise folded.
    const start = new Set<string>();
    if (raised) {
      for (const ask of asks.values()) {
        let at = byPane.get(ask.raised_pane_id);
        while (at?.lineage_parent_pane_id && at.lineage_parent_pane_id !== parent.pane_id) {
          start.add(at.lineage_parent_pane_id);
          at = byPane.get(at.lineage_parent_pane_id);
        }
      }
    }
    setExpanded(start);
    setShownAll(new Set());
    setFinishedShown(new Set());
    setRootId(parent.pane_id);
    // Only on opening: a later snapshot keeps what the operator unfolded.
  }, [open]);
  useEffect(() => {
    if (open && empty) setOpen(false);
  }, [open, empty]);
  useEffect(() => {
    if (opening && tracked && !progress) {
      setOpen(false);
      setOpening(null);
    }
  }, [opening, tracked, progress]);

  const lines: Line[] = [];
  const walk = (of: AgentRow, depth: number) => {
    const siblings = childrenOf(of, byPane);
    const kids = siblings.filter((kid) => !finishedAndRead(kid) || asks.has(kid.pane_id));
    const finished = siblings.filter((kid) => !kids.includes(kid));
    const all = shownAll.has(of.pane_id);
    const shown = [...(all ? kids : kids.slice(0, SIBLINGS_SHOWN)), ...(finishedShown.has(of.pane_id) ? finished : [])];
    for (const kid of shown) {
      const hasChildren = (kid.lineage_child_pane_ids ?? []).some((id) => byPane.has(id));
      const isOpen = depth < 2 && hasChildren && expanded.has(kid.pane_id);
      lines.push({ kind: "agent", agent: kid, depth, hasChildren, open: isOpen });
      if (isOpen) walk(kid, depth + 1);
    }
    const hidden = kids.length - Math.min(kids.length, all ? kids.length : SIBLINGS_SHOWN);
    if (hidden > 0) lines.push({ kind: "more", parent: of.pane_id, depth, count: hidden });
    if (finished.length > 0 && !finishedShown.has(of.pane_id)) lines.push({ kind: "finished", parent: of.pane_id, depth, count: finished.length });
  };
  walk(root, 1);
  const places = treePlaces(lines.map((line) => line.depth - 1));

  const toggle = (pane: string, to?: boolean) =>
    setExpanded((before) => {
      const next = new Set(before);
      if (to ?? !next.has(pane)) next.add(pane);
      else next.delete(pane);
      return next;
    });
  const openChild = (child: AgentRow) => {
    onOpenChild(child.pane_id, child.identity_label);
    const request = useUiStore.getState().relation;
    if (request?.sourcePaneId === parent.pane_id && request.targetPaneId === child.pane_id) setOpening(child.pane_id);
    else setOpen(false);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    const selected = list.current?.querySelector<HTMLElement>("[cmdk-item][data-selected=true]");
    const pane = selected?.dataset.treePane;
    if (!pane) return;
    const line = lines.find((row) => row.kind === "agent" && row.agent.pane_id === pane);
    if (line?.kind !== "agent" || !line.hasChildren) return;
    event.preventDefault();
    if (line.depth >= 2) {
      if (event.key === "ArrowRight") setRootId(pane);
      return;
    }
    toggle(pane, event.key === "ArrowRight");
  };

  return (
    <Popover open={open && !empty} onOpenChange={setOpen}>
      <Hint label={triggerLabel}>
        <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      </Hint>
      <PopoverContent
        align="start"
        className="w-(--size-agent-children-popover) p-none"
        data-agent-tree={root.pane_id}
        data-agent-tree-mode={raised ? "raised" : "tree"}
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          list.current?.focus();
        }}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          const action = afterClose.current;
          afterClose.current = null;
          if (action) action();
          else returnFocus();
        }}
      >
        <Command ref={list} tabIndex={-1} label={t("agents.children.label", { name: root.identity_label })} className="outline-none" onKeyDown={onKeyDown}>
          <div className="flex items-center gap-sm border-b border-border px-md py-sm">
            <AgentMark kind={root.agent_kind} />
            <AgentRecordTitle agent={root} className="text-body font-medium text-foreground" />
            <span className="flex shrink-0 items-center gap-xxs text-muted-foreground" aria-label={t("agentSessions.tree.children", { count: directs.length })}>
              <ListTreeIcon aria-hidden="true" className="size-(--size-icon-sm)" />
              <span aria-hidden="true" className="font-mono text-caption">{directs.length}</span>
            </span>
          </div>
          <CommandList>
            <CommandGroup>
              {lines.map((line, index) =>
                line.kind !== "agent" ? (
                  <CommandItem
                    key={`${line.kind}:${line.parent}`}
                    value={`${line.kind}:${line.parent}`}
                    onSelect={() => (line.kind === "more" ? setShownAll : setFinishedShown)((before) => new Set(before).add(line.parent))}
                    {...(line.kind === "more" ? { "data-tree-more": line.count } : { "data-tree-finished": line.count })}
                    className="gap-none text-subtle-foreground"
                  >
                    <TreeRails place={places[index]!} />
                    <TreeChevron name="" open={null} hangs={line.depth > 1} />
                    <span className="flex items-center gap-xs pl-xxs text-caption font-medium">
                      <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
                      {t(line.kind === "more" ? "agentSessions.tree.more" : "agentSessions.tree.finished", { count: line.count })}
                    </span>
                  </CommandItem>
                ) : (
                  <TreeItem
                    key={line.agent.pane_id}
                    parent={parent}
                    line={line}
                    place={places[index]!}
                    ask={asks.get(line.agent.pane_id) ?? null}
                    pending={progress?.phase === "pending"}
                    onToggle={() => (line.depth >= 2 ? setRootId(line.agent.pane_id) : toggle(line.agent.pane_id))}
                    onOpen={() => openChild(line.agent)}
                    onOpenAsk={(ask) => {
                      const target = byPane.get(ask.open_pane_id);
                      if (target) openChild(target);
                    }}
                  />
                ),
              )}
            </CommandGroup>
            {onTask ? (
              <>
                <CommandSeparator />
                <CommandItem
                  value="__task"
                  onSelect={() => {
                    afterClose.current = onTask;
                    setOpen(false);
                  }}
                  data-agent-tree-task="true"
                  className="text-subtle-foreground"
                >
                  <span className="flex-1">{t("agentSessions.tree.task")}</span>
                  <ArrowRightIcon />
                </CommandItem>
              </>
            ) : null}
          </CommandList>
          {progress ? (
            <div className="flex items-center gap-xs border-t border-border px-md py-sm text-caption" role={progress.phase === "failed" ? "alert" : "status"} data-child-navigation={progress.phase}>
              <span className={cn("min-w-0 flex-1 truncate", progress.phase === "failed" ? "text-destructive" : "text-muted-foreground")}>{progress.phase === "pending" ? t("panes.relation.opening", { name: tracked!.label }) : progress.message}</span>
              {progress.phase === "failed" && progress.retryable ? (
                <button type="button" onClick={() => onOpenChild(opening!, tracked!.label)} className="shrink-0 rounded-xs px-xs outline-none focus-visible:ring-1 focus-visible:ring-ring">
                  {t("common.retry")}
                </button>
              ) : null}
            </div>
          ) : null}
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function TreeItem({
  parent,
  line,
  place,
  ask,
  pending,
  onToggle,
  onOpen,
  onOpenAsk,
}: {
  parent: AgentRow;
  line: Extract<Line, { kind: "agent" }>;
  place: ReturnType<typeof treePlaces>[number];
  ask: RaisedAsk | null;
  pending: boolean;
  onToggle: () => void;
  onOpen: () => void;
  onOpenAsk: (ask: RaisedAsk) => void;
}) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const child = line.agent;
  const parentDevice = parent.device_id ?? remoteTargetOfPane(rest, parent.pane_id) ?? localDeviceId(rest);
  const device = child.device_id ?? remoteTargetOfPane(rest, child.pane_id) ?? localDeviceId(rest);
  const deviceLabel = machineName(rest?.navigator?.devices, device, t);
  const unavailable = !deviceConnected(rest, device);
  // A child on a device that is not connected draws no mark and says so on its line (status-marks B16).
  const reason = unavailable ? t("agents.line.disconnected", { device: deviceLabel ?? device }) : null;
  // A draft's Open goes to the parent holding it, which can sit on another device.
  const askDevice =
    ask === null || ask.open_pane_id === child.pane_id
      ? device
      : ask.open_pane_id === parent.pane_id
        ? parentDevice
        : remoteTargetOfPane(rest, ask.open_pane_id) ?? localDeviceId(rest);
  const askUnavailable = !deviceConnected(rest, askDevice);
  const branch = child.state.branch_badge;
  // The same second line a tree row draws: a block's cause, a waiting ring's 대기 line, a sleep or unknown state (status-marks B29, B31).
  const childLine = ask || unavailable ? null : treeLine(child);
  const minutes = ask?.since_unix_ms == null ? null : Math.max(0, Math.floor((Date.now() - ask.since_unix_ms) / 60_000));
  return (
    <CommandItem
      value={child.pane_id}
      onSelect={onOpen}
      disabled={pending || unavailable}
      data-tree-pane={child.pane_id}
      data-agent-child={child.pane_id}
      data-depth={line.depth}
      aria-label={[child.identity_label, agentProductName(child.agent_kind), recordWords(t, child), child.record_summary, reason ?? (childLine ? lineText(t, child, childLine) : statusText(t, child.status_code)), ask && !unavailable ? `${t(`agentSessions.verb.${ask.verb}`)}: ${askWhat(t, ask.verb, ask.what, ask.subject)}` : null, deviceLabel && device !== parentDevice ? deviceLabel : null].filter(Boolean).join(", ")}
      className={cn("items-stretch gap-none py-none", ask && !unavailable && "ring-1 ring-inset ring-foreground", (unavailable || rowDimmed(child)) && "opacity-(--opacity-dimmed)")}
    >
      <TreeRails place={place} />
      <TreeChevron name={child.identity_label} open={line.hasChildren ? line.open : null} onToggle={onToggle} hangs={line.depth > 1} popup={line.depth >= 2} data-tree-chevron={child.pane_id} />
      <span className="flex min-w-0 flex-1 flex-col gap-xxs py-xs pl-xs">
        <span className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <StatusMark mark={unavailable ? null : child.mark} className={unavailable ? undefined : markTone(child)} />
          <AgentMark kind={child.agent_kind} />
          <AgentRecordTitle agent={child} className="text-foreground" />
          <AgentPrHover agent={child} pulls={rowPulls(child, "own")} />
          {line.hasChildren && !line.open && !unavailable ? <DescendantMark agent={child} /> : null}
          {unavailable ? null : <Elapsed since={child.last_prompt_at_unix_ms} className="shrink-0 font-mono text-micro text-muted-foreground" />}
        </span>
        {ask && !unavailable ? (
          <span className="flex min-w-0 items-center gap-xs">
            <AskLine ask={{ verb: ask.verb, what: ask.what, more: 0, subject: ask.subject }} className="flex-1" />
            <button
              type="button"
              disabled={pending || askUnavailable}
              data-tree-ask-open={ask.open_pane_id}
              className="shrink-0 rounded-xs bg-secondary px-xs py-xxs text-micro text-secondary-foreground outline-none hover:brightness-95 focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
              onClick={(event) => {
                event.stopPropagation();
                onOpenAsk(ask);
              }}
            >
              {t("common.open")}
            </button>
          </span>
        ) : null}
        {childLine ? <RowLineText data-tree-line={childLine.mode} parts={lineParts(t, child, childLine)} className={cn("text-caption", lineTone(childLine, child))} /> : null}
        {ask?.unreceived_by && minutes !== null && !unavailable ? (
          <span className="truncate text-caption text-muted-foreground">{t("agentSessions.unreceived", { name: ask.unreceived_by, minutes })}</span>
        ) : null}
        {branch || (device !== parentDevice && deviceLabel) ? (
          <span className="flex min-w-0 items-center gap-xs text-caption">
            {branch ? <span className="min-w-0 truncate font-mono text-muted-foreground" data-branch-chip={branch}>{branch}</span> : null}
            {device !== parentDevice && deviceLabel ? <DeviceChip label={deviceLabel} className="max-w-2/5" /> : null}
          </span>
        ) : null}
        {reason ? <span className="truncate text-caption text-muted-foreground">{reason}</span> : null}
      </span>
    </CommandItem>
  );
}
