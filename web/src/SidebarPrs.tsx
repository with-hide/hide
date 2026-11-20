import { TriangleAlertIcon } from "lucide-react";
import { useMemo } from "react";
import type { Actions } from "./actions";
import { Hint } from "./components/ui/tooltip";
import { frontDeviceId } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { boardProjects } from "./navigation";
import { PrList } from "./PrList";
import { prListModel, readingWarning } from "./prWork";
import { useRequestViewDemand } from "./requestViewDemand";
import { SidebarProjectFilter, ToolProjectFilter, useSidebarProject } from "./SidebarProjectFilter";
import { useShellStore } from "./store";
import type { ToolCell } from "./agentScope";
import { toolProjectChoice, toolSidebarProjects } from "./toolCells";

// The sidebar's PR list (PRD lane-two-columns D-20, B41; PRD
// sidebar-task-lens D-17, D-45): every pull request of the rail's device in
// the core's groups, rows, order and folds, narrowed to one project by the
// project chip the 태스크 list shares. A row opens its card as a card tab on a desk.
// In a tool's cell it lists the pull requests of the checkouts the tool's
// sessions work in, on every device it reached, narrowed by the same chip
// read for the tool (`ToolProjectFilter`; operator decision 2026-11-15).

export function SidebarPrs({ actions, tool = null }: { actions: Actions; tool?: ToolCell | null }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const live = useShellStore((state) => state.connection === "live");
  const deviceId = frontDeviceId(rest);
  const stored = useSidebarProject(deviceId);
  const filter = tool ? toolProjectChoice(rest, tool) : stored;
  // A tool's projects, whole, decide its empty cell; the chip only narrows what is listed.
  const held = useMemo(() => (tool ? toolSidebarProjects(boardProjects(rest, agents), tool) : null), [rest, agents, tool]);
  const projects = useMemo(() => (held ?? boardProjects(rest, agents, deviceId)).filter((item) => !filter || (item.workspace.id === filter.workspace_id && item.workspace.device_id === filter.device_id)), [held, rest, agents, deviceId, filter]);
  const model = useMemo(() => prListModel(projects, null, Date.now()), [projects]);
  // The board's GitHub reads follow what is on screen.
  useRequestViewDemand(actions, live);
  // ⚠ while the last read failed or left a question beside the list unanswered, its hint saying which (work-tab-pr-lens B43).
  const staleText = readingWarning(model.reading, t);
  const reading = model.reading.state === "reading" && model.groups.length === 0;
  const open = model.groups.filter((group) => group.group !== "merged").reduce((sum, group) => sum + group.entries.length + group.team.length + group.old.length, 0);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-sidebar-prs="true" data-tool-cell={tool?.connection}>
      <div className="flex shrink-0 items-center gap-xs px-md py-xs text-micro text-muted-foreground" data-prs-header="true">
        {tool ? <ToolProjectFilter tool={tool} chosen={filter} actions={actions} /> : <SidebarProjectFilter deviceId={deviceId} actions={actions} />}
        <span className="shrink-0" data-prs-total={reading ? undefined : open}>{reading ? t("projectTool.sidebar.prsLoading") : t("projectTool.sidebar.prsCount", { count: open })}</span>
        {staleText ? (
          <Hint label={staleText}>
            <span role="img" aria-label={staleText} className="inline-flex shrink-0 text-warning" data-prs-stale="true"><TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span>
          </Hint>
        ) : null}
      </div>
      {/* A tool's cell with no work says so where the list's own line stands, in the tool's words (B42). */}
      {held?.length === 0
        ? <p className="px-md py-sm text-caption text-muted-foreground" data-prs-view="empty" data-tool-cell-empty="true">{t("taskLens.tool.empty")}</p>
        : model.projects === 0 && model.reading.state !== "blocked"
        ? <p className="px-md py-sm text-caption text-muted-foreground" data-prs-view="no-remote">{t("prs.noRemote")}</p>
        : <PrList model={model} actions={actions} onlyCheckout={false} showProject={filter === null} onAllCheckouts={() => actions.setSidebarProject(null)} />}
    </div>
  );
}
