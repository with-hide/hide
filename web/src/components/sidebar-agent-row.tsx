import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { memo, useRef, type KeyboardEvent, type ReactNode } from "react";
import { AgentRowMark } from "../AgentRowMark";
import { agentProductName } from "../agentAdapters";
import { agentClosing } from "../close";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { branchChip, forkLabel, lineParts, lineTone, markTone, markWords, rowAccessibleName, rowDimmed, rowSentence, sidebarLine, treeLine } from "../agentRow";
import { recordWords } from "../agentRow";
import { AgentRecordTitle } from "./agent-record-title";
import { RowLineText } from "./row-line-text";
import { homeTurnLine, originLine } from "../hideAiOrigin";
import { AgentForkMark } from "./fork-mark";
import { RowLetterMark } from "./letter-mark";
import { cn } from "../lib/utils";
import type { AgentRow } from "../snapshot";
import type { AgentMenuItem } from "../workspaceManage";
import { AskLine, DescendantMark, TreeChevron, TreeRails, type TreePlace } from "./agent-tree";
import { AgentPrHover, PrHoverList, useAgentStaleness, type MarkPulls } from "./pr-mark";
import { ownPulls, type PrStaleness, type SessionPull } from "../prMark";
import { AgentTreePopover } from "./agent-tree-popover";
import { Elapsed } from "./elapsed";
import { EntryContextMenu, type MenuEntry } from "./entry-menu";
import { DeviceChip } from "./device-chip";
import { CheckoutCardHint } from "./pr-card";
import { pullRequestCard } from "../projects";
import { catalogWorkspaces } from "../snapshot";
import { StatusMark } from "./status-mark";
import { Badge } from "./ui/badge";
import { Keycap } from "./ui/keycap";
import { Hint } from "./ui/tooltip";

/**
 * A control on the right of a sidebar row that waits for the pointer: its
 * slot is always there, so the title and the time never move, and it shows
 * under the pointer, while focus is anywhere inside the row, while the row's
 * menu is open, and always on an input with no hover (PRD sidebar-readability
 * D-3, B2, B3), as the Projects rows' `ROW_REVEALED` in sidebar.tsx does.
 */
export const REVEALED_CONTROL =
  "opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100 group-data-[state=open]/row:opacity-100 focus-visible:opacity-100 hoverless:opacity-100";

/**
 * An agent row's right-click menu (PRD sidebar-context-menus D-04): its items
 * are read when it opens, and the choice names the row it was opened on. One
 * object serves every row of a list, so the rows stay memoized.
 */
export type AgentRowMenu = {
  items: (agent: AgentRow) => MenuEntry<AgentMenuItem["id"]>[];
  onSelect: (agent: AgentRow, id: AgentMenuItem["id"]) => void;
};

/**
 * The fold slot of a sidebar row with nothing to fold. Every row, agent,
 * checkout or project, ends in its time and then this slot, so the times and
 * the chevrons of the whole list stand in one column each (D-3).
 */
export function FoldLane() {
  return <span aria-hidden="true" data-fold-slot="true" className="w-(--size-lineage-chevron) shrink-0" />;
}

/** Where a sidebar row sits in a drawn tree, and how its chevron opens its children. */
export type SidebarTreeSlot = {
  place: TreePlace;
  /** Null for a row with no children; a grandchild's chevron opens the popover instead. */
  open: boolean | null;
  onToggle: () => void;
  /** Set on a grandchild with children: its chevron opens the tree popover rooted on it. */
  popover: { agents: AgentRow[]; onOpenChild: (paneId: string, label: string) => void; onTask: () => void } | null;
};

/**
 * One agent in the sidebar, in Agents and under a checkout in Projects (PRD
 * sidebar-readability D-2..D-6; agent-hierarchy-screens D-12, D-37 to D-40,
 * B10 to B17). Line one is the status mark, the provider mark, the stable
 * task name, a branch chip when a delegated row's checkout differs from its
 * parent's, the device chip, the row's own PR icon, the descendant mark
 * while folded, and the elapsed time. In a drawn tree (`tree`) the rails and
 * the chevron lane stand left of the marks, and a tree row is one line but
 * for a child on another device, whose second line names that device. A
 * Needs You row (`ask`) has no PR or mark: its second line is the verb and
 * what to do. Elsewhere a request or news takes the second line; a quiet
 * sentence is only in the tooltip. `place` is the Agents list's context
 * line; Projects passes none because the rows above say it.
 *
 * Nothing here grows or moves on hover, focus, selection or an open menu:
 * those change a fill, a ring and the chevron's opacity only. The title is
 * 12/400 in every state; attention and selection brighten it rather than
 * thicken it (PRD sidebar-typography D-02). `number` is the digit an ⌥ hold
 * shows at the row's top right (PRD electron-digit-shortcuts-hints B5).
 */
export const SidebarAgentRow = memo(function SidebarAgentRow({
  agent,
  device,
  place,
  depth,
  tree = null,
  ask = false,
  remote = null,
  selected,
  onOpen,
  onOpenPullRequest,
  inset,
  branchShown = true,
  number = null,
  menu,
  family,
}: {
  agent: AgentRow;
  device: string | null;
  place: string | null;
  /** How deep under the root drawn above it; a root is 0. */
  depth: number;
  /** Set where the row is drawn in a tree with its rails and chevron. */
  tree?: SidebarTreeSlot | null;
  /** A Needs You row: its second line is the ask, with no PR or descendant mark (B2). */
  ask?: boolean;
  /** A child on another device than its parent: that device's name and whether it can be reached (B14). */
  remote?: { label: string; reachable: boolean } | null;
  selected: boolean;
  onOpen: (paneId: string) => void;
  /** Opens a PR from the PR icon's card; `external` asks for the default browser. */
  onOpenPullRequest: (url: string, external: boolean) => void;
  /** Where a root's first column starts. */
  inset: string;
  /** False where the row above already names the row's checkout. */
  branchShown?: boolean;
  /** The digit a modifier hold shows on this row, or null while none shows. */
  number?: number | null;
  /** The row's right-click menu; the menu key or ⇧F10 on the focused row opens it too. */
  menu: AgentRowMenu;
  /** On a root with children: its own and every descendant's PRs, which its mark gathers as the 태스크 row's does. */
  family?: SessionPull[];
}) {
  const { t } = useInterfaceTranslation();
  const main = useRef<HTMLButtonElement>(null);
  // Each row reads only the facts it draws, so a snapshot that moves nothing here renders nothing.
  const closing = useShellStore((s) => agentClosing(s.rest?.status?.async_operations, agent.pane_id));
  const agentPrStaleness = useAgentStaleness(agent, family);
  const line = tree ? treeLine(agent) : sidebarLine(agent);
  const branch = branchShown ? branchChip(agent) : null;
  const attention = agent.state.attention;
  const unreachable = remote !== null && !remote.reachable;
  // A row on a device that is not connected shows what Hide last knew of it,
  // which may be long over: it draws no mark, and its line says why (status-marks B16).
  const offline = unreachable ? t("agents.line.disconnected", { device: remote.label }) : null;
  const titleTone = agent.state.title_emphasized || (selected && agent.state.selection_emphasizes_title) ? "text-foreground" : "text-subtle-foreground";
  // The mark's state and word ride the row's hint, which the mark sits inside (B3).
  const hint = [device ? `${agent.identity_label} · ${device}` : agent.identity_label, agentProductName(agent.agent_kind), recordWords(t, agent), agent.record_summary, offline ?? markWords(t, agent), offline ? null : rowSentence(t, agent), place].filter(Boolean).join("\n");
  const staleness = ask ? undefined : agentPrStaleness;
  const folded = tree ? tree.open !== true : true;
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!tree || tree.open === null || event.metaKey || event.ctrlKey || event.altKey) return;
    if (tree.popover) {
      // A grandchild's children open in its popover: Right opens it as the chevron does (B13, B15).
      if (event.key !== "ArrowRight") return;
      event.preventDefault();
      event.currentTarget.closest("li")?.querySelector<HTMLButtonElement>("[data-tree-chevron]")?.click();
      return;
    }
    if ((event.key === "ArrowRight" && !tree.open) || (event.key === "ArrowLeft" && tree.open)) {
      event.preventDefault();
      tree.onToggle();
    }
  };
  return (
    <EntryContextMenu
      asChild
      label={t("common.entryActions", { name: agent.identity_label })}
      title={agent.identity_label}
      items={() => menu.items(agent)}
      onSelect={(id) => menu.onSelect(agent, id)}
      data-agent-menu={agent.pane_id}
    >
    <li
      data-pane={agent.pane_id}
      data-attention={attention ? "true" : "false"}
      data-delegated={agent.delegated ? "true" : "false"}
      data-waiting={agent.wait ?? "false"}
      data-agent-device={device ?? undefined}
      data-depth={depth}
      data-unreachable={unreachable ? "true" : undefined}
      className={cn("group/row relative flex rounded-sm text-body", tree ? "items-stretch pr-xs" : "items-start gap-xs py-xs pr-xs", selected ? tree ? "bg-secondary" : "bg-selected" : "hover:bg-accent", (unreachable || rowDimmed(agent)) && "opacity-(--opacity-dimmed)")}
      style={{ paddingLeft: inset }}
    >
      <Hint label={hint} reveals>
        <button
          ref={main}
          type="button"
          disabled={unreachable}
          aria-label={[offline ? [agent.identity_label, remote?.label, agentProductName(agent.agent_kind), offline].filter(Boolean).join(", ") : rowAccessibleName(t, agent, device ?? remote?.label ?? null), forkLabel(t, agent), closing ? t("agents.closingName") : null, place].filter(Boolean).join(", ")}
          aria-current={selected ? "true" : undefined}
          aria-expanded={tree && tree.open !== null && !tree.popover ? tree.open : undefined}
          data-agent-open={agent.pane_id}
          className="absolute inset-0 rounded-sm outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
          onClick={() => onOpen(agent.pane_id)}
          onKeyDown={onKeyDown}
        />
      </Hint>
      {tree ? <TreeRails place={tree.place} /> : null}
      {tree && tree.open !== null && depth > 0 ? <span className="absolute inset-y-0 flex w-(--size-agent-mark)" style={{ left: `calc(${inset} + var(--size-lineage-indent) * ${depth - 1})` }}><RowChevron agent={agent} tree={tree} returnFocus={() => main.current?.focus()} /></span> : null}
      <span className={cn("flex min-w-0 flex-1 items-start gap-xs", tree && "py-xs")}>
        <span className="pointer-events-none flex min-h-(--size-sidebar-line) shrink-0 items-center gap-xs">
          <StatusMark mark={unreachable ? null : agent.mark} className={markTone(agent)} data-agent-status-mark={unreachable ? "none" : agent.mark} data-agent-status={agent.status_code} />
          {/* A Hide AI row is never in a tree; the Hide owl stands beside the status mark (PRD agent-mark-quiet D-04). The provider's logo follows the view mode or an ⌥ hold, with the owl as its badge (PRD agent-logo-view-mode D-04, D-09). */}
          <AgentRowMark kind={agent.agent_kind} hideAi={agent.hide_ai && !tree ? "sidebar" : null} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
            {/* The row button's name already reads all of this out. */}
            <AgentRecordTitle agent={agent} className={cn("pointer-events-none", titleTone)} />
            <AgentForkMark agent={agent} />
            {closing ? (
              <span aria-hidden="true" className="pointer-events-none shrink-0 text-caption text-muted-foreground" data-agent-closing="true">
                {t("agents.closing")}
              </span>
            ) : null}
            {branch ? (
              <Badge aria-hidden="true" variant="secondary" className="pointer-events-none min-w-0 max-w-2/5 shrink font-mono" data-branch-chip={branch}>
                <span className="truncate">{branch}</span>
              </Badge>
            ) : null}
            {device ? (
              <DeviceChip label={device} className="pointer-events-none max-w-2/5" />
            ) : null}
            {ask ? null : <AgentPrHover agent={agent} family={family} staleness={staleness} card={(icon) => <PrHoverCard agent={agent} pulls={family ?? ownPulls(agent)} icon={icon} staleness={staleness} onOpenPullRequest={onOpenPullRequest} />} />}
            {ask || !folded ? null : <span className="pointer-events-none flex shrink-0"><DescendantMark agent={agent} /></span>}
            {agent.letters_waiting ? <RowLetterMark /> : null}
            {/* A time the core never measured draws nothing, and nothing stands in for it. */}
            <Elapsed since={agent.last_prompt_at_unix_ms} aria-hidden="true" className="pointer-events-none shrink-0 font-mono text-caption text-muted-foreground" data-agent-elapsed="true" />
            {/* Every row ends in the fold slot, so every time ends on one column (sidebar-readability D-3). */}
            <FoldLane />
          </span>
          {offline ? (
            <span aria-hidden="true" data-agent-line="disconnected" className="pointer-events-none truncate text-caption leading-(--size-sidebar-line-detail) text-subtle-foreground">
              {offline}
            </span>
          ) : ask && agent.state.ask ? <span aria-hidden="true" className="pointer-events-none flex min-w-0 leading-(--size-sidebar-line-detail)"><AskLine ask={agent.state.ask} className="flex-1" /></span> : ask && agent.hide_ai?.turn && !(line && agent.status_code === "error") ? (
            // What the row's home waits for, after the agent's own ask or block cause (PRD hide-ai-work-home D-14, D-16).
            <span aria-hidden="true" data-agent-home-turn={agent.hide_ai.turn.kind} className="pointer-events-none truncate text-caption leading-(--size-sidebar-line-detail) text-warning">
              {homeTurnLine(t, agent.hide_ai.turn)}
            </span>
          ) : line ? (
            <RowLineText aria-hidden="true" data-agent-line={line.mode} parts={lineParts(t, agent, line)} className={cn("pointer-events-none text-caption leading-(--size-sidebar-line-detail)", lineTone(line, agent))} />
          ) : null}
          {tree && agent.hide_ai ? (
            <span aria-hidden="true" data-agent-task={originLine(t, agent.hide_ai)} className="pointer-events-none truncate text-caption leading-(--size-sidebar-line-detail) text-muted-foreground">
              {originLine(t, agent.hide_ai)}
            </span>
          ) : null}
          {remote && !offline ? (
            <span aria-hidden="true" data-agent-remote={remote.label} className="pointer-events-none flex min-w-0 pt-xxs">
              <DeviceChip label={remote.label} />
            </span>
          ) : null}
          {place ? (
            <span aria-hidden="true" data-agent-place={place} className="pointer-events-none truncate text-caption leading-(--size-sidebar-line-detail) text-muted-foreground">
              {place}
            </span>
          ) : null}
        </span>
      </span>
      {tree && depth === 0 && tree.open !== null ? <button type="button" tabIndex={-1} aria-expanded={tree.open} aria-label={t(tree.open ? "agentSessions.tree.collapse" : "agentSessions.tree.expand", { name: agent.identity_label })} data-tree-chevron={agent.pane_id} className="absolute top-xs right-xs flex h-(--size-sidebar-line) w-(--size-lineage-chevron) items-center justify-center rounded-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={tree.onToggle}>
        {tree.open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
      </button> : null}
      {number !== null ? <Keycap number={number} /> : null}
    </li>
    </EntryContextMenu>
  );
});

/** A tree row's chevron: it folds the row's children in place, or on a grandchild opens the popover rooted on it (B13). */
function RowChevron({ agent, tree, returnFocus }: { agent: AgentRow; tree: SidebarTreeSlot; returnFocus: () => void }) {
  const { t } = useInterfaceTranslation();
  if (!tree.popover || tree.open === null) {
    return <TreeChevron name={agent.identity_label} open={tree.open} onToggle={tree.onToggle} hangs={tree.place.depth > 0} data-tree-chevron={agent.pane_id} />;
  }
  const { agents, onOpenChild, onTask } = tree.popover;
  const label = t("agentSessions.tree.expand", { name: agent.identity_label });
  return (
    <span className="relative w-(--size-lineage-chevron) shrink-0 self-stretch">
      <span aria-hidden="true" className="absolute top-(--size-lineage-elbow-y) right-xxs left-none h-(--size-hairline) bg-lineage-rail" />
      <AgentTreePopover
        parent={agent}
        agents={agents}
        onOpenChild={onOpenChild}
        onTask={onTask}
        returnFocus={returnFocus}
        triggerLabel={label}
        trigger={
          <button type="button" tabIndex={-1} aria-label={label} aria-haspopup="dialog" data-tree-chevron={agent.pane_id} className="relative z-10 mt-xs flex h-(--size-sidebar-line) w-full items-center justify-center rounded-xs bg-sidebar text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
            <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </button>
        }
      />
    </span>
  );
}

/**
 * Under the pointer, the sidebar PR mark shows the existing PR card for one
 * PR, and each PR with its state and title for several (B23), a root's
 * family naming who holds each.
 */
function PrHoverCard({ agent, pulls, icon, staleness, onOpenPullRequest }: { agent: AgentRow; pulls: MarkPulls; icon: ReactNode; staleness?: PrStaleness; onOpenPullRequest: (url: string, external: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  const trigger = <span className="relative z-10 flex shrink-0" data-pr-hover={agent.pane_id}>{icon}</span>;
  const single = pulls.length === 1 ? pulls[0]!.pull : null;
  const pr = useShellStore((s) => (single ? catalogWorkspaces(s.rest).flatMap((project) => project.pull_requests ?? []).find((row) => row.url === single.url) : undefined));
  if (pr && !staleness?.stale) {
    return (
      <CheckoutCardHint card={pullRequestCard(pr, t)} description={`#${pr.number} ${pr.title}`} onOpenPullRequest={onOpenPullRequest}>
        {trigger}
      </CheckoutCardHint>
    );
  }
  return <PrHoverList agent={agent} pulls={pulls} staleness={staleness} onOpen={(pull) => onOpenPullRequest(pull.url, false)}>{trigger}</PrHoverList>;
}
