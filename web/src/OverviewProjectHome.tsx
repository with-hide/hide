import { ArrowLeftIcon, ArrowRightIcon, GitBranchIcon } from "lucide-react";
import { useMemo } from "react";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { statusText } from "./agentStatus";
import { AgentMark } from "./AgentMark";
import { Button } from "./components/ui/button";
import { StatusMark } from "./components/status-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { scopeAgents } from "./overviewLens";
import type { BoardProject } from "./projectBoard";
import { ProjectPanel } from "./ProjectsTab";
import { sessionOfAgentOn } from "./sessionWork";
import { useUiStore, type MainView } from "./ui";

/** A project's facts and retained controls beside its actual checkouts and sessions. */
export function OverviewProjectHome({ project, deviceId, actions }: { project: BoardProject; deviceId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { workspace } = project;
  const sessions = useMemo(() => scopeAgents([project]), [project]);
  const back = () => useUiStore.getState().setOverview({ projectId: null, panel: null });
  const choose = (view: MainView) => { useUiStore.getState().setMainView(view); useUiStore.getState().setOverview({ panel: null }); };
  return <div className="flex min-h-0 flex-col gap-md p-lg" data-overview-project-home={workspace.id}>
    <div className="flex flex-wrap items-center justify-between gap-sm">
      <Button variant="ghost" size="sm" onClick={back}><ArrowLeftIcon />{t("overviewRedesign.allProjects")}</Button>
      <nav className="flex flex-wrap gap-sm" aria-label={t("overviewRedesign.projectHome")}>
        <Button variant="outline" size="sm" onClick={() => choose("work")}>{t("overviewRedesign.work")}<ArrowRightIcon /></Button>
        <Button variant="outline" size="sm" onClick={() => choose("backlog")}>{t("overviewTasks.backlogTab")}<ArrowRightIcon /></Button>
        <Button variant="outline" size="sm" onClick={() => choose("history")}>{t("overviewRedesign.history")}<ArrowRightIcon /></Button>
      </nav>
    </div>
    <div className="grid min-h-0 grid-cols-2 gap-xl overview-narrow:grid-cols-1">
      <ProjectPanel panel={{ kind: "project", workspaceId: workspace.id }} projects={[project]} deviceId={deviceId} actions={actions} onClose={back} />
      <div className="flex min-w-0 flex-col gap-xl">
        <section className="min-w-0">
          <h2 className="mb-sm text-subhead font-semibold">{t("overviewRedesign.sessions")} <span className="font-mono text-caption text-muted-foreground">{sessions.length}</span></h2>
          {sessions.length === 0 ? <p className="text-caption text-muted-foreground">{t("overviewRedesign.noSessions")}</p> : <ul>
            {sessions.map(({ agent, checkout }) => {
              const session = sessionOfAgentOn(agent, workspace.device_id);
              return <li key={agent.pane_id} className="border-b border-border py-sm">
                <button type="button" disabled={!session} aria-label={`${agent.identity_label} · ${statusText(t, agent.status_code)}`} className="flex w-full min-w-0 items-start gap-sm rounded-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-default" onClick={() => { if (session) useUiStore.getState().setOverview({ panel: { kind: "record", session } }); }} data-project-home-session={agent.pane_id}>
                  <StatusMark mark={agent.mark} label={statusText(t, agent.status_code)} className="mt-xxs" /><AgentMark kind={agent.agent_kind} />
                  <span className="min-w-0 flex-1"><span className="block break-words text-body">{agent.identity_label}</span><span className="text-micro text-muted-foreground">{agentProductName(agent.agent_kind)} · {checkout.label} · {statusText(t, agent.status_code)}</span></span>
                </button>
              </li>;
            })}
          </ul>}
        </section>
        <section className="min-w-0">
          <h2 className="mb-sm text-subhead font-semibold">{t("overviewRedesign.checkouts")} <span className="font-mono text-caption text-muted-foreground">{workspace.checkouts.length}</span></h2>
          {workspace.checkouts.length === 0 ? <p className="text-caption text-muted-foreground">{t("overviewRedesign.noCheckouts")}</p> : <ul>
            {workspace.checkouts.map((checkout) => <li key={checkout.id} className="flex min-w-0 items-center gap-sm border-b border-border py-sm">
              <GitBranchIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1"><span className="block truncate text-caption">{checkout.label}</span><span className="block truncate font-mono text-micro text-muted-foreground">{checkout.branch ?? checkout.path}</span></span>
              <Button variant="ghost" size="sm" disabled={!checkout.exists} aria-label={t("overviewRedesign.openCheckout", { checkout: checkout.label })} onClick={() => { actions.closeOverview(); actions.openWorkspace(workspace.device_id, workspace.id, checkout.id); }} data-project-home-checkout={checkout.id}>{t("overviewTasks.open")}<ArrowRightIcon /></Button>
            </li>)}
          </ul>}
        </section>
      </div>
    </div>
  </div>;
}
