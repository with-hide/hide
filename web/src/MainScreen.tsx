import { UsageOverview } from "./UsageOverview";
import { BrandOwl } from "./components/brand-owl";
import { firstRunStep } from "./firstRun";
import { useSetupWalk } from "./FirstRunWizard";
import { useInterfaceTranslation } from "./i18n/client";
import { ChevronDownIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, type ReactNode } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Input } from "./components/ui/input";
import { Kbd } from "./components/ui/kbd";
import { Tabs, TabsList, TabsTrigger } from "./components/ui/tabs";
import { useEscapeLayer } from "./components/ui/layer";
import { cn } from "./lib/utils";
import { boardProjects, mainSections, type DeviceAvailability } from "./navigation";
import { readsHere } from "./devices";
import { machineName } from "./screenMachine";
import type { Device } from "./snapshot";
import { ageWords } from "./overviewLens";
import { requireInterfaceLanguage } from "./i18n/locale";
import { useShellStore } from "./store";
import { MAIN_VIEWS, useUiStore, type MainView } from "./ui";
import { hostBridge } from "./host";
import { commandLabel } from "./shortcutLabels";
import { everyAgent } from "./overviewTasks";
import { backlogTally } from "./overviewBacklog";
import { TaskRecordPanel } from "./OverviewRecord";
import { BacklogItemPanel, BacklogLabelFilter, OverviewBacklog } from "./BacklogTab";
import { OverviewProjects, ProjectPanel } from "./ProjectsTab";

// The Overview of one device (PRD overview-device-tasks D-01..D-03, B1-B5;
// PRD sidebar-task-lens D-20, B38): 백로그 and Projects with Usage at the
// end, each tab's controls at the row's right, and the panel a record, a row
// or a project opens beside the tab. The device's tasks are the sidebar's
// 태스크 list. Everything drawn is a value the snapshot carries; a device that
// cannot answer shows why in the body and dims the tabs (B4).

const TAB_LABEL: Record<MainView, "overviewTasks.backlogTab" | "overview.projects" | "usage.title"> = {
  backlog: "overviewTasks.backlogTab",
  projects: "overview.projects",
  usage: "usage.title",
};

export function MainScreen({ actions, deviceId }: { actions: Actions; deviceId: string }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const agents = useShellStore((s) => s.agents);
  const view = useUiStore((s) => s.mainView);
  const setView = useUiStore((s) => s.setMainView);
  const panel = useUiStore((s) => s.overview.panel);
  const section = useMemo(() => mainSections(rest, agents, deviceId)[0] ?? null, [rest, agents, deviceId]);
  const availability: DeviceAvailability = section?.availability ?? { state: "ready" };
  const projects = useMemo(() => boardProjects(rest, agents, deviceId), [rest, agents, deviceId]);
  const rows = useMemo(() => everyAgent(rest, agents), [rest, agents]);
  const backlog = useMemo(() => backlogTally(projects), [projects]);
  // Every Git project whose issues the core reads is read while the backlog is on screen.
  const devices = rest?.navigator?.devices;
  const localGit = useMemo(() => projects.filter(({ workspace }) => workspace.is_git && readsHere(workspace, devices)).map(({ workspace }) => workspace.id).join("\n"), [projects, devices]);
  const reads = view === "backlog";
  useEffect(() => {
    if (!reads || !localGit) return;
    for (const id of localGit.split("\n")) actions.readProjectTasks(id);
  }, [actions, reads, localGit]);
  // Esc closes the panel before the Overview (B16).
  useEscapeLayer(panel !== null, () => useUiStore.getState().setOverview({ panel: null }));
  const unavailable = availability.state === "unavailable" && section ? section.device : null;
  // A Home with no project says what to do first in the backlog's and the Projects' place (sidebar-task-lens B42).
  const noProject = availability.state === "ready" && projects.length === 0;
  const counts: Record<MainView, ReactNode> = {
    backlog: <Count value={backlog.open} />,
    projects: <Count value={projects.length} />,
    usage: null,
  };
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-background" aria-label={t("overview.title")} data-main-screen="true" data-main-view={view}>
      <OpeningStatus actions={actions} />
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-md border-b border-border px-lg py-xs">
        <Tabs value={view} onValueChange={(value) => { setView(value as MainView); useUiStore.getState().setOverview({ panel: null }); }}>
          <TabsList aria-label={t("overview.viewLabel")} className={cn(unavailable && "opacity-(--opacity-dimmed)")}>
            {MAIN_VIEWS.map((choice) => (
              <TabsTrigger key={choice} value={choice} data-main-tab={choice} className={choice === "usage" ? "ml-md" : undefined}>
                {t(TAB_LABEL[choice])}
                {counts[choice]}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        {unavailable || noProject ? null : view === "backlog" ? <BacklogControls projects={projects} /> : null}
      </div>
      {unavailable ? (
        <DeviceUnanswered device={unavailable} availability={availability} actions={actions} />
      ) : (
        <div className="relative flex min-h-0 flex-1">
          <div className="min-h-0 min-w-0 flex-1 overflow-y-auto" data-overview-body={view}>
            {availability.state === "loading" ? <p role="status" className="px-lg pt-sm text-caption text-muted-foreground" data-device-loading="true">{availability.text}</p> : null}
            {view === "usage" ? (
              <UsageOverview actions={actions} />
            ) : noProject ? (
              <NoProjectView actions={actions} />
            ) : view === "backlog" ? (
              <OverviewBacklog projects={projects} deviceId={deviceId} actions={actions} />
            ) : (
              <OverviewProjects projects={projects} deviceId={deviceId} actions={actions} />
            )}
          </div>
          {panel && view !== "usage" ? (
            <aside
              className="flex w-(--size-lane-min) shrink-0 flex-col border-l border-border bg-background overview-narrow:absolute overview-narrow:inset-y-0 overview-narrow:right-0 overview-narrow:z-10 overview-narrow:shadow-lg"
              data-overview-panel={panel.kind}
            >
              {panel.kind === "record" ? (
                <TaskRecordPanel key={`${panel.session.device_id}:${panel.session.session_id}`} panel={panel} rows={rows} actions={actions} />
              ) : panel.kind === "item" ? (
                <BacklogItemPanel key={`${panel.workspaceId}:${panel.taskKey}`} panel={panel} projects={projects} actions={actions} />
              ) : (
                <ProjectPanel key={panel.workspaceId} panel={panel} projects={projects} deviceId={deviceId} actions={actions} />
              )}
            </aside>
          ) : null}
        </div>
      )}
    </section>
  );
}

function Count({ value }: { value: number }) {
  return <span className="font-mono text-caption tabular-nums text-muted-foreground" data-tab-count={value}>{value}</span>;
}

/** The backlog's project chip (B5): All projects, or one project of the device. */
function ProjectChip({ projects }: { projects: readonly { id: string; label: string }[] }) {
  const { t } = useInterfaceTranslation();
  const projectId = useUiStore((s) => s.overview.projectId);
  const chosen = projects.find((project) => project.id === projectId) ?? null;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant={chosen ? "secondary" : "outline"} size="sm" className="max-w-(--size-panel-min)" aria-label={t("overviewTasks.projectFilter")} data-overview-project-chip={chosen?.id ?? "all"}>
          <span className="min-w-0 truncate">{chosen?.label ?? t("overviewTasks.allProjects")}</span>
          <ChevronDownIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-(--radix-dropdown-menu-content-available-height) overflow-y-auto">
        <DropdownMenuRadioGroup value={chosen?.id ?? ""} onValueChange={(value) => useUiStore.getState().setOverview({ projectId: value || null })}>
          <DropdownMenuRadioItem value="">{t("overviewTasks.allProjects")}</DropdownMenuRadioItem>
          <DropdownMenuSeparator />
          {projects.map((project) => <DropdownMenuRadioItem key={project.id} value={project.id} data-overview-project-choice={project.id}>{project.label}</DropdownMenuRadioItem>)}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SearchBox({ value, placeholder, onChange, name }: { value: string; placeholder: string; onChange: (value: string) => void; name: string }) {
  return (
    <span className="relative flex w-(--size-panel-min) items-center">
      <SearchIcon aria-hidden="true" className="pointer-events-none absolute left-sm size-(--size-icon) text-muted-foreground" />
      <Input type="search" value={value} placeholder={placeholder} aria-label={placeholder} className="pl-xl" onChange={(event) => onChange(event.target.value)} data-overview-search={name} />
    </span>
  );
}

/** The backlog's label chips and search (B27), and the project chip (B5). */
function BacklogControls({ projects }: { projects: readonly import("./projectBoard").BoardProject[] }) {
  const { t } = useInterfaceTranslation();
  const query = useUiStore((s) => s.overview.backlogQuery);
  const set = useUiStore((s) => s.setOverview);
  return (
    <span className="flex flex-wrap items-center gap-sm" data-backlog-controls="true">
      <BacklogLabelFilter projects={projects} />
      <ProjectChip projects={projects.map(({ workspace }) => workspace)} />
      <SearchBox value={query} placeholder={t("backlog.search")} onChange={(backlogQuery) => set({ backlogQuery })} name="backlog" />
    </span>
  );
}

/** A device that does not answer (B4): why, and Reconnect, in place of the tabs' content. */
function DeviceUnanswered({ device, availability, actions }: { device: Device; availability: DeviceAvailability; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const name = useShellStore((s) => machineName(s.rest?.navigator?.devices, device.id, t));
  // B4: since when the device stopped answering, as the core recorded it on that transition; its reason is the tooltip.
  const since = useShellStore((s) => s.rest?.status?.remote?.find((row) => row.target_id === device.id)?.unreachable_since_unix_ms ?? null);
  if (availability.state !== "unavailable") return null;
  const retry = availability.retry;
  const words = since === null ? t("overviewTasks.unanswered", { device: name }) : t("overviewTasks.unansweredSince", { device: name, since: ageWords(requireInterfaceLanguage(i18n.language), Date.now() - since, t) });
  return (
    <div role="status" className="flex flex-1 flex-col items-center justify-center gap-md p-xl text-center" data-device-unavailable={device.id}>
      <p className="text-body text-subtle-foreground" title={availability.text}>{words}</p>
      {retry ? (
        <Button variant="secondary" onClick={() => (retry === "helper" ? actions.retryDeviceHost(device.id) : actions.retryDevice(device.id))} data-device-retry={device.id}>
          {t("overviewTasks.reconnect")}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * The Home with no project (PRD first-run-onboarding D-16, B27; PRD
 * sidebar-task-lens B42): the next step in one sentence with Add project and
 * its chord. A browser tab cannot pick a folder, so it shows the sentence alone.
 */
function NoProjectView({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const firstRun = useShellStore((s) => firstRunStep(s.rest, hostBridge() !== null));
  const setup = useSetupWalk((s) => s.step);
  const chord = commandLabel("new_workspace");
  return (
    <div className="flex min-h-full flex-1 flex-col items-center justify-center gap-md p-xl text-center" data-main-empty-home="no_project">
      {firstRun === null && setup === null ? <BrandOwl pose="ai" className="size-(--size-owl-empty)" /> : null}
      <p className="text-subhead break-keep text-subtle-foreground">{t("firstRun.home.noProject")}</p>
      {hostBridge() ? (
        <span className="flex items-center gap-sm">
          <Button onClick={() => actions.openAddProject()} data-main-empty-home-action="new_workspace">
            {t("firstRun.home.addProject")}
          </Button>
          {chord ? <Kbd aria-hidden="true">{chord}</Kbd> : null}
        </span>
      ) : null}
    </div>
  );
}

/**
 * A Workspace or agent asked for from here that is not in front yet: a quiet
 * line while it is on its way, and the refusal with Dismiss when it was not
 * brought forward, so the operator stays where they were (B2, B21).
 */
export function OpeningStatus({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const opening = useUiStore((s) => s.opening);
  if (!opening) return null;
  if (!opening.failure) {
    return (
      <p role="status" className="shrink-0 px-md pt-sm text-caption text-muted-foreground" data-opening="pending">
        {t("overview.opening")}
      </p>
    );
  }
  return (
    <div role="alert" className="mx-md mt-sm flex shrink-0 items-center gap-sm rounded-sm bg-card px-sm py-xs text-caption text-warning" data-opening="failed">
      <span className="min-w-0 flex-1 break-words">{t("overview.notOpened", { reason: opening.failure })}</span>
      <Button variant="secondary" onClick={() => actions.dismissOpening()} data-opening-dismiss="true">
        {t("workspace.dismiss")}
      </Button>
    </div>
  );
}
