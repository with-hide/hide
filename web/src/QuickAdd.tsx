import { LoaderCircleIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { Actions } from "./actions";
import { BacklogDestinationPicker } from "./BacklogDestinationPicker";
import { Button } from "./components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./components/ui/dialog";
import { frontDeviceId } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { boardProjects } from "./navigation";
import { backlogTitle, destinationOf, githubSource, type BacklogDestination } from "./overviewBacklog";
import { cardOfPane, everyAgent, isMemoKey, itemToken, overviewDeviceOf } from "./overviewTasks";
import { cycleIndex, quickAddChoice } from "./quickAddChoice";
import { catalogWorkspaces, frontCheckout } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** How long the made line stays before the input closes (B40). */
const CONFIRM_MS = 2400;

/** ⌘⇧M (PRD overview-device-tasks B37-B42): one line into a project's backlog, from any screen. */
export function QuickAdd({ actions }: { actions: Actions }) {
  const open = useUiStore((s) => s.quickAdd !== null);
  if (!open) return null;
  return <QuickAddDialog actions={actions} />;
}

function QuickAddDialog({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const agents = useShellStore((s) => s.agents);
  // The choice is read once, when the input opens: the place it opened over decides it.
  const [initial] = useState(() => {
    const state = useUiStore.getState();
    const shown = state.overviewOpen || state.screen?.kind === "main";
    const named = state.screen?.kind === "main" ? state.screen.deviceId : undefined;
    const device = shown ? overviewDeviceOf(rest, state.overview.deviceId, named) : frontDeviceId(rest);
    const panel = shown ? state.overview.panel : null;
    const root = panel?.kind === "record" ? ([...everyAgent(rest, agents).values()].find((row) => row.session_id === panel.session.session_id)?.pane_id ?? null) : null;
    const taskProject = root ? (cardOfPane(boardProjects(rest, agents, device), root)?.project.id ?? null) : null;
    const front = frontCheckout(rest);
    const frontWorkspace = front ? (catalogWorkspaces(rest).find((workspace) => workspace.checkouts.some((checkout) => checkout.id === front.id))?.id ?? null) : null;
    return quickAddChoice(rest, catalogWorkspaces(rest), device, shown ? state.overview : null, taskProject, frontWorkspace);
  });
  const [index, setIndex] = useState(initial.index);
  const [title, setTitle] = useState("");
  const [intent, setIntent] = useState<string | null>(null);
  const [chosen, setChosen] = useState<BacklogDestination>({ kind: "local" });
  const viewed = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  const returnFocus = useRef(useUiStore.getState().quickAdd?.returnFocus ?? null);
  // The project as the snapshot has it now, so a source read meanwhile changes the destination line.
  const project = useMemo(() => {
    const id = initial.projects[index]?.id;
    return id ? (catalogWorkspaces(rest).find((workspace) => workspace.id === id) ?? initial.projects[index]!) : null;
  }, [rest, initial, index]);
  const item = useShellStore((s) => (intent ? (s.rest?.issue_work?.backlog?.find((row) => row.intent === intent) ?? null) : null));
  const made = item?.phase === "created" && item.task_key !== null ? item.task_key : null;
  useEffect(() => {
    if (!made) return;
    const timer = window.setTimeout(() => actions.closeQuickAdd(), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [made, actions]);
  const close = () => actions.closeQuickAdd();
  const source = githubSource(project);
  const target: BacklogDestination = chosen.kind === "github" && chosen.repository === source?.name ? chosen : { kind: "local" };
  const destination = target.kind === "local" ? "local" : destinationOf(source);
  const sending = intent !== null && item?.phase !== "failed";
  // A failure no retry can change (a refusal, a repository that moved) hands the input back for a new add.
  const final = item?.phase === "failed" && !item.retry;

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Tab" && !event.shiftKey) {
      event.preventDefault();
      if (intent === null || final) {
        setIndex((current) => cycleIndex(current, initial.projects.length, false));
        setChosen({ kind: "local" });
        setTitle("");
      }
      return;
    }
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
    event.preventDefault();
    if (!project || sending) return;
    // A failed add is tried again under its own intent, so an issue GitHub made before the failure is joined, not made twice (B41).
    if (intent !== null && !final) return actions.retryBacklog(intent);
    const text = title.trim();
    if (!text) return;
    if (intent !== null) actions.dismissBacklog(intent);
    setIntent(actions.addBacklog(project.id, text, undefined, target));
  };

  const view = () => {
    if (!project || !made) return;
    viewed.current = true;
    close();
    actions.openBacklogItem(project.device_id, project.id, made);
    useUiStore.setState((state) => (state.overviewOpen ? { overviewReturnFocus: returnFocus.current } : {}));
  };

  return (
    <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
      <DialogContent
        className="top-(--size-settings-sheet-window-inset) w-(--size-search-sheet-w) translate-y-0 gap-xs p-md"
        returnFocusTo={() => (viewed.current ? null : returnFocus.current)}
        onOpenAutoFocus={(event) => { event.preventDefault(); input.current?.focus(); }}
        data-quick-add={made ? "made" : sending ? "creating" : item?.phase === "failed" ? "failed" : "open"}
      >
        <DialogTitle className="sr-only">{t("quickAdd.title")}</DialogTitle>
        <DialogDescription className="sr-only">{t("quickAdd.title")}</DialogDescription>
        {!project ? (
          <p className="text-caption text-muted-foreground" data-quick-add-none="true">{t("quickAdd.none")}</p>
        ) : made ? (
          <p className="flex min-w-0 items-center gap-sm text-body" role="status" data-quick-add-made={made}>
            <span className="min-w-0 truncate">{t(isMemoKey(made) ? "quickAdd.madeMemo" : "quickAdd.madeIssue", { project: project.label, token: itemToken(made, project) ?? "" })}</span>
            <Button size="sm" variant="ghost" onClick={view} data-quick-add-view="true">{t("quickAdd.view")}</Button>
          </p>
        ) : (
          <>
            <div className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
              <span className="min-w-0 truncate font-medium text-foreground" data-quick-add-project={project.id}>{project.label}</span>
              <span className="shrink-0" data-quick-add-reason={index === initial.index ? initial.reason : "cycled"}>{index === initial.index ? `· ${t(`quickAdd.reason.${initial.reason}`)}` : ""}</span>
              <span className="flex-1" />
              {initial.projects.length > 1 && (intent === null || final) ? <span className="shrink-0 text-micro">{t("quickAdd.cycle")}</span> : null}
            </div>
            <BacklogDestinationPicker project={project} value={target} disabled={intent !== null && !final} onChange={(value) => { setChosen(value); setTitle(""); }} />
            <input
              ref={input}
              autoFocus
              value={title}
              readOnly={intent !== null && !final}
              placeholder={t(destination === "local" ? "backlog.placeholderLocal" : "backlog.placeholderGithub", { project: project.label })}
              aria-label={t(destination === "local" ? "backlog.placeholderLocal" : "backlog.placeholderGithub", { project: project.label })}
              className="h-(--size-control-lg) w-full min-w-0 bg-transparent text-body text-foreground outline-none placeholder:text-muted-foreground"
              onChange={(event) => setTitle(backlogTitle(event.target.value))}
              onKeyDown={onKeyDown}
              data-quick-add-input="true"
            />
            <div className="flex min-w-0 items-center gap-xs text-micro text-muted-foreground">
              {sending ? (
                <span className="flex items-center gap-xxs" data-quick-add-creating="true"><LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />{t("quickAdd.creating")}</span>
              ) : item?.phase === "failed" ? (
                <span className="min-w-0 truncate text-destructive" role="alert" data-quick-add-failure={item.failure ?? "refused"}>{t(`backlog.failure.${item.failure ?? "refused"}`)}{item.retry ? ` · ${t("quickAdd.retry")}` : ""}</span>
              ) : (
                <span className="min-w-0 truncate" data-quick-add-destination={destination}>{t(`quickAdd.dest.${destination}`)}</span>
              )}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

