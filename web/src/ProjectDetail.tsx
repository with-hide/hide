import { ChevronRightIcon, LoaderCircleIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import type { ProjectStatus } from "./agentScope";
import { agentProductName } from "./agentAdapters";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { readsHere } from "./devices";
import { entranceBytes, layerLines } from "./diskCleanup";
import { LAYER_DOT, openDiskCleanup } from "./DiskCleanupSheet";
import { formatBytes } from "./i18n/format";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { allAgents } from "./paneAgents";
import { costPrefix, dayHeights, projectCost, usd } from "./projectStatus";
import { catalogWorkspaces, localDeviceId, type Checkout, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// What the Project tool's Git and cost tiles open in the 보기 slot (PRD
// lane-two-columns B30, B31): the project's worktrees with its disk, and its
// last seven days of cost. Both draw what the core sent; a removal goes
// through the disk cleanup sheet's confirmation as it does from the Overview.

function useProject(workspaceId: string): Workspace | null {
  return useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.id === workspaceId) ?? null);
}

function Missing() {
  const { t } = useInterfaceTranslation();
  return <p className="px-md py-sm text-caption text-muted-foreground">{t("overview.projectMissing")}</p>;
}

type DotState = ProjectStatus["git"]["dots"][number]["state"];
const DOT_TONES: Record<DotState, string> = { changed: "bg-warning", unpushed: "bg-agent-working", removable: "bg-pr-merged", clean: "bg-border" };

/**
 * The worktree table (B30): the project's disk bar, then a row per worktree
 * marked by the core's state, the ones that can go folded under `정리할 것`
 * with the cleanup sheet one press away. Opening reads the worktrees again,
 * and while it shows (and no Overview tab measures) the project's disk is
 * measured.
 */
export function WorktreesView({ workspaceId, actions }: { workspaceId: string; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const project = useProject(workspaceId);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const overviewOpen = useUiStore((s) => s.overviewOpen);
  const [foldOpen, setFoldOpen] = useState(false);
  useEffect(() => actions.refreshProjectOverview(workspaceId), [actions, workspaceId]);
  const measures = project !== null && readsHere(project, devices);
  // One tab's disk is watched at a time; the Overview's Projects tab takes it while it shows.
  useEffect(() => {
    if (!measures || overviewOpen) return;
    actions.watchDisk([workspaceId]);
    return () => actions.watchDisk([]);
  }, [actions, workspaceId, measures, overviewOpen]);
  const states = useMemo(() => new Map((project?.agent_scope.status?.git.dots ?? []).map((dot) => [dot.checkout_id, dot.state])), [project]);
  if (!project) return <Missing />;
  const rows = project.checkouts.filter((checkout) => states.has(checkout.id));
  const kept = rows.filter((checkout) => states.get(checkout.id) !== "removable");
  const removable = rows.filter((checkout) => states.get(checkout.id) === "removable");
  const lines = measures ? layerLines(project, t) : null;
  const size = entranceBytes(project);
  return (
    <div className="min-h-0 flex-1 space-y-md overflow-y-auto px-md py-md text-caption" data-worktrees-view={project.id}>
      {measures ? (
        <section className="space-y-xs" data-worktrees-storage="true">
          <h3 className="flex items-center gap-xs font-medium text-subtle-foreground">
            {t("projectTool.worktrees.storage")}
            {size ? <span className="font-mono text-muted-foreground">{size.partial ? "≥ " : ""}{formatBytes(language, size.bytes)}</span> : null}
            {project.disk?.measuring ? <LoaderCircleIcon role="status" aria-label={t("overviewProjects.measuring")} className="size-(--size-icon-sm) animate-spin text-muted-foreground" /> : null}
          </h3>
          {lines ? (
            <>
              <span aria-hidden="true" className="flex h-(--lens-bar-height) w-full overflow-hidden rounded-xs bg-muted">
                {lines.map((line) => <span key={line.key} className={LAYER_DOT[line.key]} style={{ flexGrow: line.bytes }} />)}
              </span>
              <ul className="flex flex-wrap gap-x-md gap-y-xxs text-micro text-muted-foreground">
                {lines.map((line) => (
                  <li key={line.key} className="inline-flex items-center gap-xs">
                    <span aria-hidden="true" className={cn("size-(--size-status-mark) rounded-full", LAYER_DOT[line.key])} />{line.label} <span className="font-mono">{formatBytes(language, line.bytes)}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : <span aria-hidden="true" className="block h-(--lens-bar-height) w-full rounded-xs bg-muted" />}
        </section>
      ) : null}
      {rows.length === 0 ? <p className="text-muted-foreground">{t("projectTool.worktrees.empty")}</p> : (
        <ul className="flex min-w-0 flex-col" data-worktrees-rows={kept.length}>
          {kept.map((checkout) => <WorktreeRow key={checkout.id} checkout={checkout} state={states.get(checkout.id)!} />)}
        </ul>
      )}
      {removable.length > 0 ? (
        <section className="flex min-w-0 flex-col gap-xs" data-worktrees-cleanup={removable.length}>
          <div className="flex items-center gap-sm">
            <button type="button" aria-expanded={foldOpen} className="flex items-center gap-xxs rounded-xs px-xs text-caption text-pr-merged outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => setFoldOpen(!foldOpen)} data-worktrees-fold="true">
              {t("projectTool.worktrees.cleanup", { count: removable.length })}
              <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", foldOpen && "rotate-90")} />
            </button>
            {measures ? <Button size="sm" variant="ghost" onClick={() => openDiskCleanup(project.id, "done")} data-worktrees-cleanup-open="true">{t("projectTool.worktrees.cleanupAction")}</Button> : null}
          </div>
          {foldOpen ? <ul className="flex min-w-0 flex-col">{removable.map((checkout) => <WorktreeRow key={checkout.id} checkout={checkout} state="removable" />)}</ul> : null}
        </section>
      ) : null}
    </div>
  );
}

function WorktreeRow({ checkout, state }: { checkout: Checkout; state: DotState }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const bytes = checkout.worktree?.disk?.total_bytes ?? null;
  return (
    <li className="flex min-w-0 items-center gap-xs border-b border-border py-xs" data-worktree-row={checkout.id} data-worktree-state={state}>
      <span aria-hidden="true" className={cn("size-(--size-status-mark) shrink-0 rounded-full", DOT_TONES[state])} />
      <span className="min-w-0 flex-1 truncate text-foreground" title={checkout.path}>{checkout.branch ?? checkout.label}</span>
      {checkout.pull_request ? <PrMark state={checkout.pull_request.state} number={checkout.pull_request.number} compact /> : null}
      <span className="shrink-0 text-micro text-muted-foreground">{t(`projectTool.worktrees.state.${state}`)}</span>
      {bytes !== null ? <span className="w-(--size-control-lg) shrink-0 text-right font-mono text-micro text-muted-foreground">{formatBytes(language, bytes)}</span> : null}
    </li>
  );
}

/**
 * The project's cost (B31): the seven days' total at API prices, a bar per
 * day, the models and the three sessions that spent the most, each opening
 * its record. Agents Hide has no price for are named apart and in no amount.
 */
export function ProjectCostView({ workspaceId, actions }: { workspaceId: string; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const project = useProject(workspaceId);
  const local = useShellStore((s) => project?.device_id === localDeviceId(s.rest));
  const usage = useShellStore((s) => s.usage);
  const agents = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  if (!project) return <Missing />;
  if (!local) return <p className="px-md py-sm text-caption text-muted-foreground">{t("projectTool.cost.otherDevice")}</p>;
  if (usage === null) return <div role="status" className="m-md h-(--size-control-lg) rounded-sm bg-muted" data-cost-view="pending" />;
  const cost = projectCost(usage.nodes[0]?.projects, project.id);
  if (!cost) return <p className="px-md py-sm text-caption text-muted-foreground" data-cost-view="empty">{t("projectTool.cost.empty")}</p>;
  const heights = dayHeights(cost.days);
  const sessionName = (agent: string, sessionId: string) => agents.find((row) => row.session_id === sessionId)?.identity_label ?? t("projectTool.cost.session", { agent: agentProductName(agent) });
  return (
    <div className="min-h-0 flex-1 space-y-lg overflow-y-auto px-md py-md text-caption" data-cost-view={project.id}>
      <section className="space-y-xxs">
        <p className="text-headline font-semibold tabular-nums text-foreground" data-cost-total={cost.micro_usd}>{costPrefix(cost)} {usd(cost.micro_usd, language)}</p>
        <p className="text-micro text-muted-foreground">
          {[t("projectTool.cost.apiEquivalent"), cost.partial ? t("projectTool.cost.partial") : null, cost.limited ? t("projectTool.cost.limited") : null, cost.stale ? t("usage.history.stalePrice") : null, cost.waiting ? t("usage.history.waitingPrice") : null].filter(Boolean).join(" · ")}
        </p>
        {cost.unpriced_agents.length > 0 ? <p className="text-micro text-muted-foreground" data-cost-unpriced={cost.unpriced_agents.length}>{t("projectTool.cost.unpriced", { agents: cost.unpriced_agents.map((agent) => agentProductName(agent)).join(", ") })}</p> : null}
      </section>
      <section className="space-y-xs">
        <h3 className="font-medium text-subtle-foreground">{t("projectTool.cost.days")}</h3>
        <div className="flex h-(--size-control-lg) items-end gap-xs" data-cost-days={cost.days.length}>
          {cost.days.map((day, index) => (
            <Hint key={day.date} label={`${day.date} · ${usd(day.micro_usd, language)}`}>
              <span tabIndex={0} className="flex h-full flex-1 items-end outline-none focus-visible:ring-1 focus-visible:ring-ring">
                <span className="w-full rounded-t-xs bg-primary" style={{ height: `${Math.max(heights[index]! * 100, heights[index]! > 0 ? 6 : 2)}%`, opacity: heights[index]! > 0 ? 1 : 0.3 }} data-cost-day={day.date} />
              </span>
            </Hint>
          ))}
        </div>
        <div className="flex gap-xs text-micro text-muted-foreground">{cost.days.map((day) => <span key={day.date} className="flex-1 text-center font-mono">{day.date.slice(5)}</span>)}</div>
      </section>
      {cost.models.length > 0 ? (
        <section className="space-y-xs">
          <h3 className="font-medium text-subtle-foreground">{t("projectTool.cost.models")}</h3>
          <ul className="flex flex-col" data-cost-models={cost.models.length}>
            {cost.models.map((row) => (
              <li key={`${row.agent}:${row.model ?? ""}`} className="flex min-w-0 items-center gap-sm border-b border-border py-xs">
                <span className="min-w-0 flex-1 truncate"><span className="text-foreground">{row.model ?? t("projectTool.cost.unnamedModel")}</span> <span className="text-muted-foreground">· {agentProductName(row.agent)}</span></span>
                <span className="shrink-0 font-mono tabular-nums">{usd(row.micro_usd, language)}</span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {cost.sessions.length > 0 ? (
        <section className="space-y-xs">
          <h3 className="font-medium text-subtle-foreground">{t("projectTool.cost.sessions")}</h3>
          <ul className="flex flex-col" data-cost-sessions={cost.sessions.length}>
            {cost.sessions.map((row) => (
              <li key={`${row.agent}:${row.session_id}`} className="min-w-0">
                <button type="button" className="flex w-full min-w-0 items-center gap-sm rounded-xs px-xs py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openSessionRecord({ device_id: project.device_id, agent: row.agent, session_id: row.session_id })} data-cost-session={row.session_id}>
                  <span className="min-w-0 flex-1 truncate text-foreground">{sessionName(row.agent, row.session_id)}</span>
                  <span className="shrink-0 font-mono tabular-nums">{usd(row.micro_usd, language)}</span>
                  <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
