import { BrainIcon, BriefcaseIcon, EllipsisIcon, FileDiffIcon, FilesIcon, FolderIcon, FolderOpenIcon, GitBranchIcon, XIcon, type LucideIcon } from "lucide-react";
import { Fragment, useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuShortcut, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { ExplorerTree } from "./ExplorerTree";
import { HistoryList } from "./HistoryList";
import { HideAiTool } from "./HideAiTool";
import { useMemoryProject } from "./MemoryTab";
import { ProjectTool } from "./ProjectTool";
import { frontProject } from "./projectStatus";
import { memoryTabCount } from "./suggestionList";
import { DividerGuide, LaneDivider } from "./LaneDivider";
import { commandLabel } from "./shortcutLabels";
import type { CommandId } from "./shortcuts";
import { catalogWorkspaces, frontCheckout, type Checkout } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { ViewAreas } from "./ViewAreas";
import { usePaneViewRows } from "./ViewComments";
import { workspaceKey } from "./viewLayout";
import { TOOLS, TOOL_SCOPES, agentEntries, workspaceViewOf, type LaneFrame, type LaneSizes, type Tool, type WorkspaceView } from "./workspace";
import { drawnPair } from "./pairView";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";

// The right lane and its tool rail (PRD lane-two-columns D-01, D-02, D-07;
// PRD right-lane D-05, D-08): the lane is two columns, the viewing column at
// the agents' side showing the front Agent tab's desk (its View areas, cards
// among their tabs; PRD lane-desk-pair-view D-06), and the tool column beside
// the rail showing one tool, the Project tool (D-14), the Project's Memory,
// Explorer (the file tree) or Changes (History). The rail at the body's right
// edge has the desk's button above a rule and the tools below it; each brings
// its column in front or folds it. Which columns show, the tool and the
// widths are the front Workspace's, and the desk the front tab's, kept by
// the core; whether the lane is expanded and which column a stacked lane
// draws are this page's (`ui.ts`).

const TOOL_NAMES = { explorer: "panes.lane.explorer", changes: "panes.lane.changes", project: "projectTool.title", memory: "panes.lane.memory" } as const satisfies Record<Tool, MessageKey>;
const TOOL_ICONS: Record<Tool, LucideIcon> = { explorer: FolderOpenIcon, changes: FileDiffIcon, project: BriefcaseIcon, memory: BrainIcon };
/** The command whose chord the rail icon's hint shows (D-08). */
const LANE_TOOL_COMMAND = { project: "toggle_lane_project", memory: "toggle_lane_memory", explorer: "toggle_lane_explorer", changes: "toggle_lane_changes" } as const satisfies Record<Tool, CommandId>;

/** Whether a lane drawn this way is on screen. */
export function laneUp(mode: LaneFrame["mode"]): boolean {
  return mode !== "closed";
}

const RAIL_BUTTON = "relative flex size-(--size-tool-rail-button) items-center justify-center rounded-md outline-none hover:bg-secondary hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled)";

/**
 * What a tool's scope reads as (PRD right-lane D-08, B14, D-37): the front
 * checkout for the checkout's tools (Explorer, Changes), the Project for the
 * Project and Memory tools.
 */
function useScope(checkout: Checkout | null): (tool: Tool) => string {
  const project = useShellStore((s) => (checkout ? catalogWorkspaces(s.rest).find((row) => row.checkouts.some((candidate) => candidate.id === checkout.id)) ?? null : null));
  const name = checkout ? (checkout.branch ?? checkout.label) : "";
  const projectName = project?.label ?? name;
  return (tool) => (TOOL_SCOPES[tool] === "project" ? projectName : name);
}

/**
 * The tool rail (B2, B3, B11, B14): above the rule the desk's button, pressed
 * while the viewing column shows it and folding the column when pressed
 * again, with no number (the hint says how many Views its front Agent tab's
 * desk holds); below it one icon per tool, its name, scope and chord in its
 * hint, pressed while the tool column shows it, opening the tool column on it
 * or closing the column when it already shows it. The Project tool counts its
 * project's 흘린 것 and the Hide AI tool what its two tabs ask of the
 * operator, each a gray number, none at 0 or while Settings > General turns
 * the rail's counts off: the warning colour belongs to the operator's 내 차례
 * alone. With an empty desk the button is off and says so. At the bottom the
 * `…` stays in place whether the lane is up or not and whichever column it
 * shows (B15).
 */
export function ToolRail({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const views = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined)?.tray.count ?? 0);
  // The desk's scope: the name the front Agent tab's strip tab shows.
  const deskTab = useShellStore((s) => {
    const tab = (s.rest?.workspace_view as WorkspaceView | undefined)?.tray.tab_id;
    return tab && checkout ? agentEntries(checkout).find((entry) => entry.source_id === tab)?.label ?? "" : "";
  });
  const drawn = useUiStore((s) => s.drawnLane);
  const strays = useShellStore((s) => frontProject(s.rest)?.agent_scope.status?.strays_total ?? 0);
  // The Hide AI tool's number is the core's: its 제안 and its Memory tab's own, none while its switch is off (PRD hide-ai-suggestions D-01, B3).
  const hideAi = useShellStore((s) => {
    const project = frontProject(s.rest);
    const opened = s.suggestions?.project;
    return project && opened && opened.workspace_id === project.id ? opened.counts : null;
  });
  const scope = useScope(checkout);
  const uiState = useShellStore((s) => s.rest?.ui_state);
  // Settings > General: the gray counts on the Project and Hide AI icons; the hint and the reader's description say them either way.
  const countsShown = uiState?.rail_counts_shown ?? true;
  const up = laneUp(drawn.mode);
  const documentsChord = commandLabel("toggle_lane_documents", uiState);
  // In focus mode the button is the desk beside the pane, which it folds and stands as ⌘E does: pressed while the desk
  // stands, and while folded its hint says the left pane's Views, the pane head's `뷰 N개` (D-63). It does nothing while the
  // desk holds no tab the strip shows, the core's `shown` count (D-60).
  const pair = useShellStore((s) => drawnPair(workspaceViewOf(s.rest)));
  const pairViews = usePaneViewRows(pair?.pane_id ?? "").length;
  const documentsShown = pair ? pair.alone !== true : up && drawn.view === "documents";
  const listed = pair ? pairViews : views;
  const available = pair ? (pair.shown ?? 0) > 0 : views > 0;
  // The rail is always drawn, so the Hide AI tool's count reads its Project whichever tool shows.
  useMemoryProject(actions);
  return (
    <nav className="flex w-(--size-tool-rail) shrink-0 flex-col items-center gap-xxs border-l border-border bg-sidebar py-xs" aria-label={t("panes.lane.rail")} data-tool-rail="true">
      <Hint label={available ? [t("panes.lane.documents"), deskTab, listed > 0 ? t("panes.workspace.viewsOpen", { count: listed }) : ""].filter(Boolean).join(" · ") : t("panes.lane.noDocuments")} shortcut={documentsChord || undefined} side="left">
        <button
          type="button"
          aria-pressed={documentsShown}
          aria-label={t("panes.lane.documents")}
          aria-description={!available ? t("panes.lane.noDocuments") : listed > 0 ? t("panes.workspace.viewsOpen", { count: listed }) : undefined}
          // `aria-disabled`, not `disabled`: a disabled button takes no pointer events, so its hint could never open (B2).
          aria-disabled={!available}
          className={cn(RAIL_BUTTON, "aria-disabled:opacity-(--opacity-disabled) aria-disabled:hover:bg-transparent aria-disabled:hover:text-muted-foreground", documentsShown ? "bg-secondary font-medium text-foreground" : "text-muted-foreground")}
          data-lane-tool="documents"
          onClick={() => available && actions.toggleDocuments()}
        >
          {documentsShown ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" data-lane-tool-bar="true" /> : null}
          <FilesIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </button>
      </Hint>
      {/* The viewing column's buttons end and the tools begin (D-07). */}
      <span aria-hidden="true" className="my-xs h-(--size-hairline) w-(--size-icon-lg) shrink-0 bg-border" data-tool-rail-divider="view" />
      {TOOLS.map((tool, index) => {
        const Icon = TOOL_ICONS[tool];
        const pressed = up && drawn.tool === tool;
        const name = t(TOOL_NAMES[tool]);
        const needs = tool === "project" ? strays : tool === "memory" ? (hideAi?.rail ?? 0) : 0;
        // What the number counts, in the hint and for a reader.
        const needsWords = needs === 0 ? "" : tool === "project" ? t("projectTool.railCount", { count: needs }) : t("hideAiTool.railCount", { suggestions: hideAi?.suggestions ?? 0, memory: hideAi ? memoryTabCount({ counts: hideAi }) : 0 });
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
              {needs > 0 && countsShown ? (
                <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-raised px-xxs text-micro tabular-nums text-foreground shadow-raised" data-lane-todo-count={needs}>
                  {needs}
                </span>
              ) : null}
            </button>
          </Hint>
          </Fragment>
        );
      })}
      <div className="mt-auto flex flex-col items-center gap-xxs">
        <LaneMenu />
      </div>
    </nav>
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
 * drawn. The desk stays mounted while folded, so its tabs, scroll and
 * previews are as they were when it comes back, laid out out of sight at its
 * own width (`documents`).
 */
export function Lane({ view, frame, documents, sizes, checkout, actions }: { view: WorkspaceView; frame: LaneFrame; documents: LaneFrame; sizes: LaneSizes; checkout: Checkout; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [body, setBody] = useState<HTMLElement | null>(null);
  const [guide, setGuide] = useState<number | null>(null);
  const both = frame.view !== null && frame.tool !== null;
  return (
    <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1">
      {frame.view !== null ? (
        <div className={cn("relative flex min-h-0 min-w-0 flex-col", both ? "shrink-0" : "flex-1")} style={both ? { width: frame.view.width } : undefined} data-lane-column="view">
          <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-card" role="region" aria-label={t("panes.lane.documents")} data-view-area="true">
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
      {/* The Project tool has its own head: the project, where it is, ⚙ and × (B26); so has the Hide AI tool: its switch, ⚙ and × (PRD hide-ai-suggestions B1, B3). */}
      {tool === "project" ? <ProjectTool actions={actions} /> : tool === "memory" ? <HideAiTool actions={actions} /> : <>
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
