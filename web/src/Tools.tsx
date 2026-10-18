import { BrainIcon, EllipsisIcon, FileDiffIcon, FilesIcon, FolderIcon, GitBranchIcon, GitPullRequestIcon, Maximize2Icon, Minimize2Icon, PinIcon, XIcon, type LucideIcon } from "lucide-react";
import { Fragment, useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuShortcut, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { ExplorerTree } from "./ExplorerTree";
import { HistoryList } from "./HistoryList";
import { MemoryTab, useMemoryProject } from "./MemoryTab";
import { usePrsNeedsYou, PrsPanel } from "./PrsPanel";
import { DetailIcon, DetailPanel, useDetailTitle } from "./DetailPanel";
import { DividerGuide, LaneDivider } from "./LaneDivider";
import { commandLabel } from "./shortcutLabels";
import type { CommandId } from "./shortcuts";
import { catalogWorkspaces, frontCheckout, type Checkout } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { ViewAreas } from "./ViewAreas";
import { workspaceKey } from "./viewLayout";
import { TOOLS, TOOL_SCOPES, type LaneFrame, type LaneSizes, type LaneTool, type Tool, type WorkspaceView } from "./workspace";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";

// The right lane and its tool rail (PRD right-lane D-01, D-05, D-08, D-09):
// one lane shows one tool at a time, Memory, PRs, Files (the View areas,
// with the file tree as a column of its own) and Changes (History), or the
// window's 보기 slot (PRD sidebar-task-lens D-50), and the rail at the body's
// right edge opens, switches and closes it. The tool,
// the pin, the tree and the widths are the front Workspace's, kept by the
// core; whether an unpinned lane is up and whether it is expanded are this
// page's (`ui.ts`).

const TOOL_NAMES = { files: "panes.lane.files", changes: "panes.lane.changes", prs: "prs.title", memory: "panes.lane.memory" } as const satisfies Record<Tool, MessageKey>;
const TOOL_ICONS: Record<Tool, LucideIcon> = { files: FilesIcon, changes: FileDiffIcon, prs: GitPullRequestIcon, memory: BrainIcon };
/** The command whose chord the rail icon's hint shows (D-12): Files names ⇧⌘E, which opens it with its tree. */
const LANE_TOOL_COMMAND = { prs: "toggle_lane_prs", files: "toggle_file_tree", changes: "toggle_lane_changes", memory: "toggle_lane_memory" } as const satisfies Record<Tool, CommandId>;

/** Whether a lane drawn this way is on screen. */
export function laneUp(mode: LaneFrame["mode"]): boolean {
  return mode === "overlay" || mode === "docked" || mode === "expanded";
}

/**
 * What a tool's scope reads as (D-08, B14, D-37): the front checkout for the
 * checkout's tools (Files, Changes), the Project for the Project's (Memory,
 * PRs), with PRs naming all of it.
 */
function useScope(checkout: Checkout | null): (tool: Tool) => string {
  const { t } = useInterfaceTranslation();
  const project = useShellStore((s) => (checkout ? catalogWorkspaces(s.rest).find((row) => row.checkouts.some((candidate) => candidate.id === checkout.id)) ?? null : null));
  const name = checkout ? (checkout.branch ?? checkout.label) : "";
  const projectName = project?.label ?? name;
  return (tool) => {
    const base = TOOL_SCOPES[tool] === "project" ? projectName : name;
    return tool === "prs" ? t("panes.lane.scopeProject", { project: base }) : base;
  };
}

/**
 * The tool rail (B1, B2, B23): one icon per tool, its name in its hint, pressed
 * while the lane shows that tool; pressing it opens the lane on that tool,
 * or closes the lane when it already shows it. The Files icon counts the
 * views open while the lane does not show them (B17), Memory's dot says
 * its Project has something to ask, and PRs counts its Needs You rows
 * (sidebar-task-lens B29). While the 보기 slot holds something, a fifth
 * button above Memory brings it back (B46).
 */
export function ToolRail({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const views = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined)?.layout?.display_count ?? 0);
  const drawn = useUiStore((s) => s.drawnLane);
  const asking = useShellStore((state) => {
    const project = state.memory?.project;
    return Boolean(project?.enabled && project.questions.length > 0 && project.workspace_id === frontCheckout(state.rest)?.workspace_id);
  });
  const scope = useScope(checkout);
  const uiState = useShellStore((s) => s.rest?.ui_state);
  const prsNeed = usePrsNeedsYou();
  const up = laneUp(drawn.mode);
  // The rail is always drawn, so Memory's dot reads its Project whichever tool shows.
  useMemoryProject(actions);
  return (
    <nav className="flex w-(--size-tool-rail) shrink-0 flex-col items-center gap-xxs border-l border-border bg-sidebar py-xs" aria-label={t("panes.lane.rail")} data-tool-rail="true">
      <DetailSlotButton actions={actions} pressed={up && drawn.tool === "detail"} />
      {TOOLS.map((tool, index) => {
        const Icon = TOOL_ICONS[tool];
        const pressed = up && drawn.tool === tool;
        const name = t(TOOL_NAMES[tool]);
        const count = tool === "files" && !pressed ? views : 0;
        const needs = tool === "prs" ? prsNeed : 0;
        // One short rule where the Project's tools end and the checkout's begin (D-37).
        const before = TOOLS[index - 1];
        const boundary = before !== undefined && TOOL_SCOPES[before] !== TOOL_SCOPES[tool];
        return (
          <Fragment key={tool}>
          {boundary ? <span aria-hidden="true" className="my-xs h-(--size-hairline) w-(--size-icon-lg) shrink-0 bg-border" data-tool-rail-divider="true" /> : null}
          <Hint label={`${name} · ${scope(tool)}`} shortcut={commandLabel(LANE_TOOL_COMMAND[tool], uiState) || undefined} side="left">
            <button
              type="button"
              aria-pressed={pressed}
              aria-label={name}
              aria-description={count > 0 ? t("panes.workspace.viewsOpen", { count }) : needs > 0 ? t("work.count", { count: needs }) : undefined}
              className={`relative flex size-(--size-tool-rail-button) items-center justify-center rounded-md outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring ${pressed ? "bg-secondary font-medium text-foreground" : "text-muted-foreground"}`}
              data-lane-tool={tool}
              onClick={() => actions.toggleTool(tool)}
            >
              {pressed ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
              <Icon aria-hidden="true" className="size-(--size-icon-lg)" />
              {count > 0 ? (
                <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-primary px-xxs text-micro text-primary-foreground" data-lane-badge={count}>
                  {count}
                </span>
              ) : null}
              {needs > 0 ? (
                <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-warning px-xxs text-micro tabular-nums text-background" data-lane-prs-count={needs}>
                  {needs}
                </span>
              ) : null}
              {tool === "memory" && asking ? <span role="img" aria-label={t("memory.questionsWaiting")} className="absolute right-xs top-xs size-(--size-status-mark) rounded-full bg-warning" data-lane-tool-dot="memory" /> : null}
            </button>
          </Hint>
          </Fragment>
        );
      })}
      <LaneMenu actions={actions} />
    </nav>
  );
}

/** The rail's 보기 slot (B46): its item's mark, pressed while the lane draws it; absent while the slot is empty. */
function DetailSlotButton({ actions, pressed }: { actions: Actions; pressed: boolean }) {
  const { t } = useInterfaceTranslation();
  const item = useShellStore((s) => s.rest?.ui_state?.detail?.items.at(-1) ?? null);
  const title = useDetailTitle();
  if (!item) return null;
  return (
    <>
    <Hint label={title || t("detail.slot")} side="left">
      <button
        type="button"
        aria-pressed={pressed}
        aria-label={title || t("detail.slot")}
        className={`relative flex size-(--size-tool-rail-button) items-center justify-center rounded-md outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring ${pressed ? "bg-secondary font-medium text-foreground" : "text-muted-foreground"}`}
        data-lane-tool="detail"
        data-detail-slot={item.kind}
        onClick={() => actions.toggleDetail()}
      >
        {pressed ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
        <DetailIcon item={item} className="size-(--size-icon-lg)" />
      </button>
    </Hint>
    {/* The slot is no tool: one short rule sets it apart from the tools below. */}
    <span aria-hidden="true" className="my-xs h-(--size-hairline) w-(--size-icon-lg) shrink-0 bg-border" data-tool-rail-divider="detail" />
    </>
  );
}

/** The rail's `…`: the lane's own commands with their chords, so ⌘⇧E has a place to be found. */
function LaneMenu({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const view = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined) ?? null);
  const drawn = useUiStore((s) => s.drawnLane);
  const expanded = useUiStore((s) => s.laneExpanded);
  const treeChord = useShellStore((s) => commandLabel("toggle_file_tree", s.rest?.ui_state));
  const up = laneUp(drawn.mode);
  // The tree fills a files lane with no view, so there is nothing to turn off.
  const treeAlone = up && drawn.tool === "files" && (view?.layout?.display_count ?? 0) === 0;
  return (
    <DropdownMenu>
      <Hint label={t("panes.lane.more")} side="left">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" className="mt-auto text-muted-foreground" aria-label={t("panes.lane.more")} data-lane-more="true">
            <EllipsisIcon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side="left" aria-label={t("panes.lane.more")} data-lane-menu="true">
        <DropdownMenuItem data-lane-menu-item="tree" disabled={treeAlone} onSelect={() => actions.toggleFileTree()}>
          {t("panes.lane.tree")}
          {treeChord ? <DropdownMenuShortcut>{treeChord}</DropdownMenuShortcut> : null}
        </DropdownMenuItem>
        {view?.tool ? (
          <DropdownMenuItem data-lane-menu-item="pin" onSelect={() => actions.setLanePinned(!view.pinned)}>
            {view.pinned ? t("panes.lane.unpinHint") : t("panes.lane.pinHint")}
          </DropdownMenuItem>
        ) : null}
        {up ? (
          <DropdownMenuItem data-lane-menu-item="expand" onSelect={() => useUiStore.getState().setLaneExpanded(!expanded)}>
            {t("panes.lane.expand")}
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The lane (B2, B4, B5, B8, B9-B14): its header names the tool and its
 * scope, with the pin, expand and close at its right; its body is the tool.
 * The files tool stays mounted under another tool, so its documents, tabs,
 * scroll and previews are as they were when it comes back (B2), laid out out
 * of sight at its own width (`files`) rather than the other tool's; its tree
 * column is drawn only while the files tool shows.
 */
export function Lane({ view, tool: drawn, frame, files, sizes, checkout, actions }: { view: WorkspaceView; tool: LaneTool; frame: LaneFrame; files: LaneFrame; sizes: LaneSizes; checkout: Checkout; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const expanded = frame.mode === "expanded";
  // The pane zoom's chord expands the lane while the keyboard is in it (`keyboard.ts`).
  const expandChord = useShellStore((s) => commandLabel("toggle_zoom", s.rest?.ui_state));
  const scope = useScope(checkout);
  // Pinned in a window too narrow to dock, the tooltip reads as the approved
  // board draws it (PRD right-lane B19, D-17).
  const pinHint = !view.pinned ? t("panes.lane.pinHint") : frame.narrowPin ? t("panes.lane.pinNarrowHint") : t("panes.lane.unpinHint");
  const filesShown = drawn === "files";
  // The 보기 slot draws its own head; the files tool stays mounted beneath it.
  const tool: Tool = drawn === "detail" ? (view.tool ?? "files") : drawn;
  if (drawn === "detail") {
    return (
      <div className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col">
        <DetailPanel actions={actions} />
        <div className="absolute inset-y-0 right-0 flex min-h-0 min-w-0" style={{ width: files.width }} inert data-lane-hidden="true">
          <FilesTool view={view} frame={files} sizes={sizes} shown={false} actions={actions} />
        </div>
      </div>
    );
  }
  return (
    <aside className="flex h-full min-h-0 min-w-0 flex-1 flex-col text-foreground" aria-label={t("panes.lane.aria")} data-lane={tool}>
      <div className="flex h-[var(--size-tab-strip)] shrink-0 items-center gap-xxs border-b border-border pl-md pr-xs text-caption" data-lane-header="true">
        <span className="min-w-0 flex-1 truncate" data-lane-scope={scope(tool)} data-lane-scope-kind={TOOL_SCOPES[tool]}>
          <span className="font-semibold text-foreground">{t(TOOL_NAMES[tool])}</span>
          {/* The icon replaces the dot to the eye; the dot stays for a reader, so the text still says `name · scope` (B54). */}
          <span className="sr-only"> · </span>
          <span className="ml-xs text-subtle-foreground">
            {TOOL_SCOPES[tool] === "project" ? <FolderIcon aria-hidden="true" className="mr-xs inline size-(--size-icon-sm) align-middle" /> : <GitBranchIcon aria-hidden="true" className="mr-xs inline size-(--size-icon-sm) align-middle" />}
            {scope(tool)}
          </span>
        </span>
        <Hint label={pinHint}>
          <Button variant={view.pinned ? "secondary" : "ghost"} size="icon-sm" className={view.pinned ? "text-primary" : "text-muted-foreground"} aria-pressed={view.pinned} aria-label={t("panes.lane.pin")} data-lane-pin={view.pinned ? (frame.narrowPin ? "narrow" : "on") : "off"} onClick={() => actions.setLanePinned(!view.pinned)}>
            <PinIcon className={view.pinned ? "fill-current" : undefined} />
          </Button>
        </Hint>
        <Hint label={t("panes.lane.expand")} shortcut={expandChord || undefined}>
          <Button variant={expanded ? "secondary" : "ghost"} size="icon-sm" className="text-muted-foreground" aria-pressed={expanded} aria-label={t("panes.lane.expand")} data-lane-expand={expanded ? "on" : "off"} onClick={() => useUiStore.getState().setLaneExpanded(!expanded)}>
            {expanded ? <Minimize2Icon /> : <Maximize2Icon />}
          </Button>
        </Hint>
        <Hint label={t("panes.lane.close")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.close")} data-lane-close="true" onClick={() => actions.closeLane()}>
            <XIcon />
          </Button>
        </Hint>
      </div>
      <div className="relative flex min-h-0 min-w-0 flex-1">
        <div className={cn("flex min-h-0 min-w-0", filesShown ? "flex-1" : "absolute inset-y-0 right-0")} style={filesShown ? undefined : { width: files.width }} inert={!filesShown} data-lane-hidden={filesShown ? undefined : "true"}>
          <FilesTool view={view} frame={files} sizes={sizes} shown={filesShown} actions={actions} />
        </div>
        {filesShown ? null : (
          <section className="flex min-h-0 min-w-0 flex-1 flex-col" aria-label={t(TOOL_NAMES[tool])} data-tool={tool}>
            {/* History reads the front checkout's Git through its own host. */}
            {tool === "prs" ? <PrsPanel actions={actions} /> : tool === "memory" ? <MemoryTab actions={actions} /> : <HistoryList actions={actions} />}
          </section>
        )}
      </div>
    </aside>
  );
}

/**
 * The files tool (B9-B13): the View areas, and the file tree as a column at
 * their left while it is on; with no view open, the tree alone (B12). The
 * tree reads the selected device's checkout through its helper.
 */
function FilesTool({ view, frame, sizes, shown, actions }: { view: WorkspaceView; frame: LaneFrame; sizes: LaneSizes; shown: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [body, setBody] = useState<HTMLElement | null>(null);
  const [guide, setGuide] = useState<number | null>(null);
  const treeChord = useShellStore((s) => commandLabel("toggle_file_tree", s.rest?.ui_state));
  const treeFooter = treeChord ? t("panes.lane.treeFooter", { chord: treeChord }) : t("panes.lane.treeFooterBare");
  // One place for the tree whether it fills the lane or stands beside a
  // document, so a pick from the tree alone keeps its filter and scroll.
  const tree = frame.treeOnly || frame.tree !== null;
  return (
    <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1">
      {tree ? (
        <div className={cn("flex min-h-0 flex-col", frame.treeOnly ? "min-w-0 flex-1" : "shrink-0")} style={frame.treeOnly ? undefined : { width: frame.tree ?? undefined }} data-lane-tree={frame.treeOnly ? "only" : "beside"}>
          {shown ? <ExplorerTree actions={actions} hint={t(frame.treeOnly ? "panes.lane.treeOnlyHint" : "panes.lane.treeHint")} footer={frame.treeOnly ? t("panes.lane.treeOnlyFooter") : treeFooter} /> : null}
        </div>
      ) : null}
      {frame.tree !== null && !frame.treeOnly ? <LaneDivider kind="tree" label={t("panes.lane.resizeTree")} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} /> : null}
      {frame.treeOnly ? null : (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-card" role="region" aria-label={t("panes.lane.files")} data-view-area="true">
          <ViewAreas actions={actions} />
        </div>
      )}
      {guide !== null ? <DividerGuide left={guide} /> : null}
    </div>
  );
}
