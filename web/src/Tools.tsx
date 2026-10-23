import { BriefcaseIcon, EllipsisIcon, FileDiffIcon, FilesIcon, FolderIcon, FolderOpenIcon, GitBranchIcon, PinIcon, XIcon, type LucideIcon } from "lucide-react";
import { Fragment, useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuShortcut, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { ExplorerTree } from "./ExplorerTree";
import { HistoryList } from "./HistoryList";
import { useMemoryProject } from "./MemoryTab";
import { ProjectTool } from "./ProjectTool";
import { frontProject, memoryAsks } from "./projectStatus";
import { DetailIcon, DetailPanel, useDetailTitle } from "./DetailPanel";
import { DividerGuide, LaneDivider } from "./LaneDivider";
import { commandLabel } from "./shortcutLabels";
import type { CommandId } from "./shortcuts";
import { catalogWorkspaces, frontCheckout, type Checkout } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { ViewAreas } from "./ViewAreas";
import { workspaceKey } from "./viewLayout";
import { TOOLS, TOOL_SCOPES, type LaneFrame, type LaneSizes, type LaneView, type Tool, type WorkspaceView } from "./workspace";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";

// The right lane and its tool rail (PRD lane-two-columns D-01, D-02, D-07;
// PRD right-lane D-05, D-08): the lane is two columns, the viewing column at
// the agents' side showing the checkout's documents (the View areas) or the
// window's 보기 slot (PRD sidebar-task-lens D-50), and the tool column beside
// the rail showing one tool, the Project tool (D-14), Explorer (the file
// tree) or Changes (History). The rail at the body's right edge has the viewing
// column's buttons above a rule and the tools below it; each brings its
// column in front or folds it. Which columns show, the tool, the pin and the
// widths are the front Workspace's, kept by the core; whether an unpinned
// lane is up, whether it is expanded and which column a stacked lane draws
// are this page's (`ui.ts`).

const TOOL_NAMES = { explorer: "panes.lane.explorer", changes: "panes.lane.changes", project: "projectTool.title" } as const satisfies Record<Tool, MessageKey>;
const TOOL_ICONS: Record<Tool, LucideIcon> = { explorer: FolderOpenIcon, changes: FileDiffIcon, project: BriefcaseIcon };
/** The command whose chord the rail icon's hint shows (D-08). */
const LANE_TOOL_COMMAND = { project: "toggle_lane_project", explorer: "toggle_lane_explorer", changes: "toggle_lane_changes" } as const satisfies Record<Tool, CommandId>;

/** Whether a lane drawn this way is on screen. */
export function laneUp(mode: LaneFrame["mode"]): boolean {
  return mode === "overlay" || mode === "docked" || mode === "expanded";
}

const RAIL_BUTTON = "relative flex size-(--size-tool-rail-button) items-center justify-center rounded-md outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled)";

/**
 * What a tool's scope reads as (PRD right-lane D-08, B14, D-37): the front
 * checkout for the checkout's tools (Explorer, Changes), the Project for the
 * Project tool.
 */
function useScope(checkout: Checkout | null): (tool: Tool | "documents") => string {
  const project = useShellStore((s) => (checkout ? catalogWorkspaces(s.rest).find((row) => row.checkouts.some((candidate) => candidate.id === checkout.id)) ?? null : null));
  const name = checkout ? (checkout.branch ?? checkout.label) : "";
  const projectName = project?.label ?? name;
  return (tool) => (tool !== "documents" && TOOL_SCOPES[tool] === "project" ? projectName : name);
}

/**
 * The tool rail (B2, B3, B11, B14): above the rule the viewing column's
 * buttons, the 보기 slot's while it holds something and the documents' with
 * the open views counted, each pressed while its column shows it and
 * folding the column when pressed again; below it one icon per tool, its
 * name, scope and chord in its hint, pressed while the tool column shows
 * it, opening the tool column on it or closing the column when it already
 * shows it. The Project tool counts its project's 할 일 and what its Memory
 * asks (B38), no number at 0. With no document the documents button is off and says so.
 * At the bottom the pin and the `…` stay in place whether the lane is up or
 * not and whichever column it shows (B15).
 */
export function ToolRail({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const views = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined)?.layout?.display_count ?? 0);
  const drawn = useUiStore((s) => s.drawnLane);
  const todo = useShellStore((s) => frontProject(s.rest)?.agent_scope.status?.todo_total ?? 0);
  const asks = useShellStore((s) => {
    const project = frontProject(s.rest);
    return project ? memoryAsks(s.memory?.project, project.id) : 0;
  });
  const scope = useScope(checkout);
  const uiState = useShellStore((s) => s.rest?.ui_state);
  const up = laneUp(drawn.mode);
  const documentsChord = commandLabel("toggle_lane_documents", uiState);
  const documentsShown = up && drawn.view === "documents";
  // The rail is always drawn, so the Project tool's memory reads its Project whichever tool shows.
  useMemoryProject(actions);
  return (
    <nav className="flex w-(--size-tool-rail) shrink-0 flex-col items-center gap-xxs border-l border-border bg-sidebar py-xs" aria-label={t("panes.lane.rail")} data-tool-rail="true">
      <DetailSlotButton actions={actions} pressed={up && drawn.view === "detail"} />
      <Hint label={views > 0 ? `${t("panes.lane.documents")} · ${scope("documents")}` : t("panes.lane.noDocuments")} shortcut={documentsChord || undefined} side="left">
        <button
          type="button"
          aria-pressed={documentsShown}
          aria-label={t("panes.lane.documents")}
          aria-description={views > 0 ? t("panes.workspace.viewsOpen", { count: views }) : t("panes.lane.noDocuments")}
          // `aria-disabled`, not `disabled`: a disabled button takes no pointer events, so its hint could never open (B2).
          aria-disabled={views === 0}
          className={cn(RAIL_BUTTON, "aria-disabled:opacity-(--opacity-disabled) aria-disabled:hover:bg-transparent aria-disabled:hover:text-muted-foreground", documentsShown ? "bg-secondary font-medium text-foreground" : "text-muted-foreground")}
          data-lane-tool="documents"
          onClick={() => views > 0 && actions.toggleDocuments()}
        >
          {documentsShown ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
          <FilesIcon aria-hidden="true" className="size-(--size-icon-lg)" />
          {views > 0 ? (
            <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-primary px-xxs text-micro text-primary-foreground" data-lane-badge={views}>
              {views}
            </span>
          ) : null}
        </button>
      </Hint>
      {/* The viewing column's buttons end and the tools begin (D-07). */}
      <span aria-hidden="true" className="my-xs h-(--size-hairline) w-(--size-icon-lg) shrink-0 bg-border" data-tool-rail-divider="view" />
      {TOOLS.map((tool, index) => {
        const Icon = TOOL_ICONS[tool];
        const pressed = up && drawn.tool === tool;
        const name = t(TOOL_NAMES[tool]);
        const needs = tool === "project" ? todo + asks : 0;
        // What the number adds up, in the hint and for a reader: the 할 일, then what Memory asks.
        const needsWords = needs > 0 ? [todo > 0 ? t("projectTool.todoCount", { count: todo }) : null, asks > 0 ? t("projectTool.memoryAsks", { count: asks }) : null].filter(Boolean).join(" · ") : "";
        // One short rule where the Project's tools end and the checkout's begin (D-37).
        const before = TOOLS[index - 1];
        const boundary = before !== undefined && TOOL_SCOPES[before] !== TOOL_SCOPES[tool];
        return (
          <Fragment key={tool}>
          {boundary ? <span aria-hidden="true" className="my-xs h-(--size-hairline) w-(--size-icon-lg) shrink-0 bg-border" data-tool-rail-divider="true" /> : null}
          <Hint label={needsWords ? `${name} · ${scope(tool)} · ${needsWords}` : `${name} · ${scope(tool)}`} shortcut={commandLabel(LANE_TOOL_COMMAND[tool], uiState) || undefined} side="left">
            <button
              type="button"
              aria-pressed={pressed}
              aria-label={name}
              aria-description={needsWords || undefined}
              className={cn(RAIL_BUTTON, pressed ? "bg-secondary font-medium text-foreground" : "text-muted-foreground")}
              data-lane-tool={tool}
              onClick={() => actions.toggleTool(tool)}
            >
              {pressed ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
              <Icon aria-hidden="true" className="size-(--size-icon-lg)" />
              {needs > 0 ? (
                <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-warning px-xxs text-micro tabular-nums text-background" data-lane-todo-count={needs}>
                  {needs}
                </span>
              ) : null}
            </button>
          </Hint>
          </Fragment>
        );
      })}
      <div className="mt-auto flex flex-col items-center gap-xxs">
        <PinButton actions={actions} />
        <LaneMenu />
      </div>
    </nav>
  );
}

/** The rail's 보기 slot (B2, B11): its item's mark, pressed while the viewing column draws it; absent while the slot is empty. */
function DetailSlotButton({ actions, pressed }: { actions: Actions; pressed: boolean }) {
  const { t } = useInterfaceTranslation();
  const item = useShellStore((s) => s.rest?.ui_state?.detail?.items.at(-1) ?? null);
  const title = useDetailTitle();
  if (!item) return null;
  return (
    <Hint label={title || t("detail.slot")} side="left">
      <button
        type="button"
        aria-pressed={pressed}
        aria-label={title || t("detail.slot")}
        className={cn(RAIL_BUTTON, pressed ? "bg-secondary font-medium text-foreground" : "text-muted-foreground")}
        data-lane-tool="detail"
        data-detail-slot={item.kind}
        onClick={() => actions.toggleDetail()}
      >
        {pressed ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
        <DetailIcon item={item} className="size-(--size-icon-lg)" />
      </button>
    </Hint>
  );
}

/**
 * The rail's pin (B15, B19): filled and pressed while the lane is pinned,
 * its hint saying what a press does, or that a pinned lane is drawn over
 * the agents because the body is too narrow to dock it.
 */
function PinButton({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pinned = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined)?.pinned ?? null);
  const narrow = useUiStore((s) => s.drawnLane.mode === "overlay");
  if (pinned === null) return null;
  const hint = pinned ? (narrow ? t("panes.lane.narrowPinHint") : t("panes.lane.unpinHint")) : t("panes.lane.pinHint");
  return (
    <Hint label={hint} side="left">
      <button
        type="button"
        aria-pressed={pinned}
        aria-label={t("panes.lane.pin")}
        aria-description={hint}
        className={cn(RAIL_BUTTON, pinned ? "bg-secondary font-medium text-foreground" : "text-muted-foreground")}
        data-lane-pin={pinned ? (narrow ? "narrow" : "on") : "off"}
        onClick={() => actions.setLanePinned(!pinned)}
      >
        <PinIcon aria-hidden="true" className={cn("size-(--size-icon-lg)", pinned && "fill-current")} />
      </button>
    </Hint>
  );
}

/** The rail's `…`: Expand, which moves both columns together (B15); with the lane down it has nothing to offer and is off. */
function LaneMenu() {
  const { t } = useInterfaceTranslation();
  const drawn = useUiStore((s) => s.drawnLane);
  const expanded = useUiStore((s) => s.laneExpanded);
  // The pane zoom's chord expands the lane while the keyboard is in it (`keyboard.ts`).
  const expandChord = useShellStore((s) => commandLabel("toggle_zoom", s.rest?.ui_state));
  const up = laneUp(drawn.mode);
  return (
    <DropdownMenu>
      <Hint label={t("panes.lane.more")} side="left">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.more")} disabled={!up} data-lane-more="true">
            <EllipsisIcon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side="left" aria-label={t("panes.lane.more")} data-lane-menu="true">
        <DropdownMenuItem data-lane-menu-item="expand" data-lane-expand={expanded ? "on" : "off"} onSelect={() => useUiStore.getState().setLaneExpanded(!expanded)}>
          {expanded ? t("panes.lane.collapse") : t("panes.lane.expand")}
          {expandChord ? <DropdownMenuShortcut>{expandChord}</DropdownMenuShortcut> : null}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The lane's two columns (B1, B6, B9, B14, B17): the viewing column at the
 * left, the tool column at the right, a divider between them while both are
 * drawn. The documents stay mounted under the 보기 slot and while folded, so
 * their tabs, scroll and previews are as they were when they come back,
 * laid out out of sight at their own width (`documents`).
 */
export function Lane({ view, frame, documents, sizes, checkout, actions }: { view: WorkspaceView; frame: LaneFrame; documents: LaneFrame; sizes: LaneSizes; checkout: Checkout; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [body, setBody] = useState<HTMLElement | null>(null);
  const [guide, setGuide] = useState<number | null>(null);
  const shows: LaneView = frame.view === null ? "none" : frame.view.field === "detail_width" ? "detail" : "documents";
  const both = frame.view !== null && frame.tool !== null;
  return (
    <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1">
      {frame.view !== null ? (
        <div className={cn("relative flex min-h-0 min-w-0 flex-col", both ? "shrink-0" : "flex-1")} style={both ? { width: frame.view.width } : undefined} data-lane-column="view">
          {shows === "detail" ? <DetailPanel actions={actions} /> : null}
          <div className={cn("flex min-h-0 min-w-0 flex-1 flex-col bg-card", shows === "documents" ? undefined : "absolute inset-y-0 right-0")} style={shows === "documents" ? undefined : { width: documents.view?.width ?? frame.view.width }} inert={shows !== "documents"} data-lane-hidden={shows === "documents" ? undefined : "true"} role="region" aria-label={t("panes.lane.documents")} data-view-area={shows === "documents" ? "true" : undefined}>
            <ViewAreas actions={actions} />
          </div>
        </div>
      ) : (
        // No viewing column: the documents are still laid out, out of sight, at their own width.
        <div className="absolute inset-y-0 right-0 flex min-h-0 min-w-0 flex-col" style={{ width: documents.view?.width ?? frame.width }} inert data-lane-hidden="true">
          <ViewAreas actions={actions} />
        </div>
      )}
      {both ? <LaneDivider kind="column" label={t("panes.lane.resizeColumns")} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} /> : null}
      {frame.tool !== null && view.tool !== null ? (
        <ToolColumn tool={view.tool} width={both ? frame.tool.width : null} checkout={checkout} actions={actions} />
      ) : null}
      {guide !== null ? <DividerGuide left={guide} /> : null}
    </div>
  );
}

/**
 * The tool column (B3, B4, B7, B14): its header names the tool and its scope
 * and closes the column with ×; under it the tool itself. The Explorer is
 * the file tree as it is, with no state of its own to turn off (B5).
 */
function ToolColumn({ tool, width, checkout, actions }: { tool: Tool; width: number | null; checkout: Checkout; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const scope = useScope(checkout);
  return (
    <aside className={cn("flex min-h-0 min-w-0 flex-col text-foreground", width === null ? "flex-1" : "shrink-0")} style={width === null ? undefined : { width }} aria-label={t(TOOL_NAMES[tool])} data-lane={tool} data-lane-column="tool">
      {/* The Project tool has its own head: the project, where it is, ⚙ and × (B26). */}
      {tool === "project" ? <ProjectTool actions={actions} /> : <>
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
        <Hint label={t("panes.lane.closeTool")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.closeTool")} data-lane-tool-close={tool} onClick={() => actions.closeTool()}>
            <XIcon />
          </Button>
        </Hint>
      </div>
      <section className="flex min-h-0 min-w-0 flex-1 flex-col" aria-label={t(TOOL_NAMES[tool])} data-tool={tool}>
        {/* History reads the front checkout's Git through its own host. */}
        {tool === "explorer" ? <ExplorerTree actions={actions} hint={t("panes.lane.treeHint")} /> : <HistoryList actions={actions} />}
      </section>
      </>}
    </aside>
  );
}
