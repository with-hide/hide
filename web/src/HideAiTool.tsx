import { BrainIcon, SettingsIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "./actions";
import { AgentLogo } from "./components/agent-logo";
import { Button } from "./components/ui/button";
import { Switch } from "./components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs";
import { Hint } from "./components/ui/tooltip";
import { HideAiSettings } from "./HideAiSettings";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { MemoryTab } from "./MemoryTab";
import { catalogWorkspaces, frontCheckout, type ProjectSuggestions } from "./snapshot";
import { useShellStore } from "./store";
import { memoryTabCount } from "./suggestionList";
import { SuggestionsTab } from "./SuggestionsTab";
import { useUiStore, type HideAiTab } from "./ui";

/**
 * The lane's Hide AI tool (PRD hide-ai-suggestions D-01, D-08, B1-B3): the
 * front checkout's Project, its head with the switch for all of Hide AI
 * there, ⚙ and ×, and under it the tabs 제안 and Memory, each counting what
 * the operator has to do in it. The core opens both tabs with one
 * `memory_open` (`useMemoryProject`, which the rail runs), decides every
 * count and the switch; the tab shown is this page's (`ui.ts`).
 */
export function HideAiTool({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const projectId = useShellStore((state) => frontCheckout(state.rest)?.workspace_id ?? null);
  const suggestions = useShellStore((state) => state.suggestions?.project ?? null);
  const opened = suggestions && suggestions.workspace_id === projectId ? suggestions : null;
  if (!projectId) return <p className="px-md py-lg text-caption text-muted-foreground">{t("history.openWorkspace")}</p>;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-hide-ai-tool={projectId} data-hide-ai={opened ? (opened.loading ? "loading" : opened.enabled ? "on" : "off") : "loading"}>
      <Head projectId={projectId} project={opened} actions={actions} />
      {!opened || (opened.loading && !opened.unavailable) ? (
        <p role="status" className="px-md py-lg text-caption text-muted-foreground">{t("memory.reading")}</p>
      ) : opened.unavailable ? (
        <div className="flex flex-col items-start gap-sm px-md py-lg" data-hide-ai-unavailable={opened.unavailable}>
          <p className="break-keep text-caption text-subtle-foreground">{t(opened.unavailable === "unregistered" ? "hideAiTool.unregistered" : "hideAiTool.unreadable")}</p>
          {opened.unavailable === "unreadable" ? <Button size="sm" variant="secondary" onClick={() => actions.memoryOpen(opened.workspace_id)}>{t("memory.readAgain")}</Button> : null}
        </div>
      ) : opened.enabled ? (
        <Body project={opened} actions={actions} />
      ) : (
        <Off />
      )}
    </div>
  );
}

/** The head (B3, B17, B19): the brain icon, `Hide AI · <project>`, `조사 중` while a run goes or `조사 못 함` after one could not investigate, the AI chip, the switch, ⚙ and ×. */
function Head({ projectId, project, actions }: { projectId: string; project: ProjectSuggestions | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const label = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === projectId)?.label ?? projectId);
  const ai = useShellStore((state) => state.rest?.status?.background_ai ?? null);
  const answering = ai?.refusal?.using ?? ai?.provider ?? null;
  const provider = answering ? ai?.providers.find((row) => row.id === answering) ?? null : null;
  const ready = project !== null && !project.loading && project.unavailable === null;
  // Hide AI asks nothing while it cannot investigate or every agent is past the usage limit (D-06): the chip stays, dimmed.
  const asksNothing = ready && (project.ai !== "ready" || (ai?.refusal?.reason === "past_limit" && !ai.refusal.using));
  const switchId = `hide-ai-switch-${projectId}`;
  return (
    <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border pl-md pr-xs text-caption" data-lane-header="true">
      <span className="flex min-w-0 flex-1 items-center gap-xs" data-lane-scope={label} data-lane-scope-kind="project">
        <BrainIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-foreground" />
        <span className="shrink-0 font-semibold text-foreground">{t("hideAiTool.title")}</span>
        <span className="min-w-0 truncate text-subtle-foreground"><span aria-hidden="true">· </span>{label}</span>
        {ready && project.enabled && project.status === "investigating" ? (
          <span role="status" className="shrink-0 text-micro text-subtle-foreground" data-hide-ai-investigating="true">{t("hideAiTool.investigating")}</span>
        ) : null}
        {/* The last run could not investigate, so nothing new rose from it; quiet, since the next run settles it (D-35). */}
        {ready && project.enabled && project.status === "not_investigated" ? (
          <span role="status" className="shrink-0 text-micro text-subtle-foreground" data-hide-ai-not-investigated="true">{t("hideAiTool.notInvestigated")}</span>
        ) : null}
      </span>
      {ready && project.enabled && provider ? (
        <span className={cn("flex min-w-0 max-w-1/3 shrink items-center gap-xxs rounded-sm bg-secondary px-xs text-micro text-subtle-foreground", asksNothing && "opacity-(--opacity-dimmed)")} data-hide-ai-chip={provider.agent} data-hide-ai-chip-dimmed={asksNothing ? "true" : undefined}>
          <AgentLogo agent={provider.agent} label={provider.label} />
          <span className="truncate">{provider.label}</span>
        </span>
      ) : null}
      <label htmlFor={switchId} className="sr-only">{t("hideAiTool.switch")}</label>
      <Switch id={switchId} checked={ready && project.enabled} disabled={!ready} onCheckedChange={(on) => actions.hideAiSwitch(projectId, on)} data-hide-ai-switch={ready ? (project.enabled ? "on" : "off") : "loading"} />
      {ready ? <HideAiSettings project={project} actions={actions} /> : (
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("hideAiTool.settings")} disabled>
          <SettingsIcon />
        </Button>
      )}
      <Hint label={t("panes.lane.closeTool")}>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.closeTool")} data-lane-tool-close="memory" onClick={() => actions.closeTool()}>
          <XIcon />
        </Button>
      </Hint>
    </div>
  );
}

/** With the switch off nothing is read and nothing is counted (B3): one line says so and what turning it on reads. */
function Off() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-xs px-md py-lg" data-hide-ai-off="true">
      <span className="text-body text-foreground">{t("hideAiTool.off.title")}</span>
      <p className="break-keep text-caption text-muted-foreground">{t("hideAiTool.off.body")}</p>
    </div>
  );
}

function Body({ project, actions }: { project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const tab = useUiStore((state) => state.hideAiTab);
  const memory = memoryTabCount(project);
  const intro = useIntroLine(project, actions);
  return (
    <Tabs value={tab} onValueChange={(next) => useUiStore.getState().showHideAiTab(next as HideAiTab)} className="min-h-0 flex-1 gap-0">
      <div className="flex shrink-0 flex-col gap-sm px-md pt-sm">
        {intro ? (
          <div className="flex flex-col gap-xxs" data-hide-ai-intro="true">
            <p className="break-keep text-caption text-subtle-foreground">{t("hideAiTool.intro.usage")}</p>
            <p className="break-keep text-caption text-muted-foreground">{t("hideAiTool.sent")}</p>
          </div>
        ) : null}
        <TabsList aria-label={t("hideAiTool.tabs")}>
          <TabCount value="suggestions" name={t("hideAiTool.tab.suggestions")} count={project.counts.suggestions} />
          <TabCount value="memory" name={t("hideAiTool.tab.memory")} count={memory} />
        </TabsList>
      </div>
      <TabsContent value="suggestions" className="flex min-h-0 flex-col" data-tool="suggestions">
        <SuggestionsTab project={project} actions={actions} />
      </TabsContent>
      <TabsContent value="memory" className="flex min-h-0 flex-col" data-tool="memory">
        <MemoryTab actions={actions} />
      </TabsContent>
    </Tabs>
  );
}

function TabCount({ value, name, count }: { value: HideAiTab; name: string; count: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <TabsTrigger value={value} aria-description={count > 0 ? t("hideAiTool.tabCount", { count }) : undefined} data-hide-ai-tool-tab={value}>
      {name}
      {count > 0 ? (
        <span aria-hidden="true" className={cn("flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-warning px-xxs text-micro tabular-nums text-background")} data-hide-ai-tool-tab-count={count}>
          {count}
        </span>
      ) : null}
    </TabsTrigger>
  );
}

/**
 * The one-time line (B3): on the first open with Hide AI on, the subscription
 * and what is sent. The core marks it seen once the tab shows it, and the open
 * tab keeps it until it closes, so it is read once and never flickers away.
 */
function useIntroLine(project: ProjectSuggestions, actions: Actions): boolean {
  const [shown, setShown] = useState<string | null>(null);
  const unseen = !project.intro_seen;
  useEffect(() => {
    if (!unseen) return;
    setShown(project.workspace_id);
    actions.suggestionsIntroSeen();
  }, [actions, unseen, project.workspace_id]);
  return unseen || shown === project.workspace_id;
}
