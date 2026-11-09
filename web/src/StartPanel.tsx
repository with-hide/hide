import { CheckIcon, ChevronDownIcon, FolderIcon, GitBranchIcon, PlusIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { rememberedSelection, modelToSend, selectKind, type AgentSelection } from "./agentPicker";
import { AgentPicker } from "./components/agent-picker";
import { ProfilePicker } from "./components/agent-profiles";
import { startProfiles } from "./agentProfiles";
import { Button } from "./components/ui/button";
import { useEscapeLayer } from "./components/ui/layer";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from "./components/ui/command";
import { Textarea } from "./components/ui/textarea";
import { Hint } from "./components/ui/tooltip";
import { DeviceChip } from "./components/device-chip";
import { START_ANSWER_TIMEOUT_MS, startAnswer, startRequestId } from "./startAnswer";
import { agentFailure, useStartPanel, startFailure, startRefusal } from "./startDraft";
import { QuotedLine } from "./QuotedLine";
import { projectOf, resolveTarget, startTargets, type StartProject, type StartTarget, type StartTargets } from "./startTargets";
import { windowDeviceId } from "./screenMachine";
import { useShellStore } from "./store";
import { restoreFocus } from "./terminals";
import { useUiStore } from "./ui";
import { useInterfaceTranslation } from "./i18n/client";
import { notSignedIn } from "./firstRun";
import { Status } from "./components/settings-rows";
import { Input } from "./components/ui/input";
import { catalogWorkspaces, localDeviceId } from "./snapshot";
import { projectBaseBranch } from "./projectSettings";
import { branchProblem } from "./workspaceManage";
import { composingKey } from "./keys";
import { followStartedPane } from "./opening";

// The start panel (PRD home-device-rail D-17..D-21, B23-B31): ⌘K's position
// and width, no backdrop, the keyboard in the text box. What is written is the
// new agent's first prompt; Enter or Start sends one `agent_start_in_checkout`
// and the panel then follows only the answer that carries its request id.
// Over the text, one quiet row says where (the device when it is another one,
// the project › the branch); under it, one says who (the agent and its model)
// and starts.

/**
 * Follows the answer to the request the panel sent, whether or not the panel
 * is still open: a start that lands after Esc still spends the draft and
 * moves the center to its pane.
 */
function useStartAnswer() {
  const request = useStartPanel((s) => s.request);
  const requestKind = useStartPanel((s) => s.requestKind);
  useEffect(() => {
    if (!request) return undefined;
    const settle = (): boolean => {
      const answer = startAnswer(useShellStore.getState().rest, request, requestKind);
      const panel = useStartPanel.getState();
      if (answer.phase === "pending") return false;
      if (answer.phase === "refused" || answer.phase === "failed") {
        panel.fail(answer.phase === "refused" ? startRefusal(answer.kind) : startFailure(answer.message));
        return true;
      }
      const ui = useUiStore.getState();
      // A watched task reports the agent's own start after the panel is gone (the notice bar).
      if (answer.agentPhase) ui.setWatchedTask(answer.taskId);
      // A start's core result owns selection and ordered engine focus.
      // Opening it again here would undo a newer visit made while it waited;
      // only a device's worktree is opened from the page once listed.
      if (answer.paneId && requestKind === "worktree_create") followStartedPane(answer.paneId, answer.deviceId);
      if (answer.agentPhase === "failed") panel.fail(agentFailure(answer.agentReason, answer.agentLine));
      else panel.finish(answer.agentPhase === "starting" ? answer.taskId : null);
      return true;
    };
    if (settle()) return undefined;
    const unsubscribe = useShellStore.subscribe(() => {
      if (settle()) unsubscribe();
    });
    const timer = window.setTimeout(() => {
      unsubscribe();
      useStartPanel.getState().fail({ key: "shell.startTimeout" });
    }, START_ANSWER_TIMEOUT_MS);
    return () => {
      unsubscribe();
      window.clearTimeout(timer);
    };
  }, [request, requestKind]);
}

/**
 * Follows a start whose tab opened while its agent was still starting: an
 * agent that then fails to start puts its text back in the draft with the
 * reason, so the next ⌘N shows both and nothing written is lost (B31).
 */
function useSpentStart() {
  const spent = useStartPanel((s) => s.spent);
  const operation = useShellStore((s) => s.rest?.task_operation);
  useEffect(() => {
    if (!spent) return;
    const panel = useStartPanel.getState();
    if (!operation || operation.id !== spent.taskId) return panel.settle();
    if (operation.agent_phase === "failed" || operation.agent_phase === "unknown") panel.restore(agentFailure(operation.agent_reason, operation.agent_line));
    else if (operation.agent_phase !== "starting") panel.settle();
  }, [spent, operation]);
}

export function StartPanelHost({ actions }: { actions: Actions }) {
  useStartAnswer();
  useSpentStart();
  const open = useStartPanel((s) => s.isOpen);
  // A palette or a dialog opening over the panel takes the place; the draft stays.
  const covered = useUiStore((s) => s.overlay !== "none" || s.workspaceDialog !== null);
  useEffect(() => {
    if (covered) useStartPanel.getState().close();
  }, [covered]);
  return open ? <StartPanel actions={actions} /> : null;
}

/** A chip of the where row: a quiet word that names its value and opens its menu. */
const CHIP = "h-(--size-control-sm) max-w-full min-w-0 gap-xxs rounded-sm px-xs text-caption font-normal";

/**
 * The project menu: each device's projects, the window's own device first. Choosing one starts on its main checkout; a device that is not
 * connected is listed and cannot be chosen.
 */
function ProjectPicker({ targets, project, target, disabled, onPick }: { targets: StartTargets; project: StartProject | null; target: StartTarget | null; disabled: boolean; onPick: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className={CHIP} disabled={disabled} aria-label={t("startPanel.project")} data-start-target={target?.key ?? ""} data-start-project={project?.key ?? ""}>
          {project ? <FolderIcon aria-hidden="true" /> : null}
          <span className="truncate font-medium">{project?.label ?? t("shell.noTarget")}</span>
          <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-(--size-picker-popover) p-none" data-start-project-menu="true">
        <Command>
          <CommandInput placeholder={t("startPanel.findProject")} />
          <CommandList>
            <CommandEmpty>{t("startPanel.noMatch")}</CommandEmpty>
            {targets.groups.map((group, index) => (
              <div key={group.deviceId} data-start-target-group={group.deviceId}>
                {index > 0 ? <CommandSeparator /> : null}
                <CommandGroup heading={group.connected ? group.deviceLabel : t("startPanel.deviceOffline", { device: group.deviceLabel })}>
                  {group.projects.map((row) => (
                    <CommandItem
                      key={row.key}
                      value={`${group.deviceLabel} ${row.label} ${row.key}`}
                      disabled={row.disabled !== null}
                      onSelect={() => {
                        onPick(row.branches[0]!.key);
                        setOpen(false);
                      }}
                      data-start-target-option={row.branches[0]!.key}
                    >
                      <FolderIcon aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate">{row.label}</span>
                      {row.key === project?.key ? <CheckIcon aria-hidden="true" className="text-primary" /> : null}
                    </CommandItem>
                  ))}
                </CommandGroup>
              </div>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

/**
 * The branch menu of the chosen project: its main checkout (`기본`), the one in
 * front (`지금`), its other checkouts, and a new branch Hide makes a worktree for.
 */
function BranchPicker({ project, target, currentKey, disabled, onPick }: { project: StartProject; target: StartTarget; currentKey: string | null; disabled: boolean; onPick: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const shown = target.kind === "checkout" ? target.branch : "";
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className={CHIP} disabled={disabled} aria-label={t("startPanel.branch")} data-start-branch={target.key}>
          <GitBranchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="truncate">{shown}</span>
          <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-(--size-picker-popover) p-none" data-start-branch-menu="true">
        <Command>
          <CommandInput placeholder={t("startPanel.findBranch")} />
          <CommandList>
            <CommandEmpty>{t("startPanel.noMatch")}</CommandEmpty>
            <CommandGroup>
              {project.branches.map((row) => (
                <CommandItem
                  key={row.key}
                  value={`${row.kind === "checkout" ? row.branch : row.label} ${row.key}`}
                  onSelect={() => {
                    onPick(row.key);
                    setOpen(false);
                  }}
                  data-start-target-option={row.key}
                >
                  <GitBranchIcon aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate">{row.kind === "checkout" ? row.branch : row.label}</span>
                  {row.kind === "checkout" && row.primary ? <span className="text-caption text-muted-foreground">{t("startPanel.mainCheckout")}</span> : null}
                  {row.key === currentKey ? <span className="text-caption text-muted-foreground">{t("startPanel.current")}</span> : null}
                  {row.key === target.key ? <CheckIcon aria-hidden="true" className="text-primary" /> : null}
                </CommandItem>
              ))}
            </CommandGroup>
            {project.worktree ? (
              <>
                <CommandSeparator />
                <CommandGroup forceMount>
                  <CommandItem
                    forceMount
                    value={`new ${project.worktree.key}`}
                    onSelect={() => {
                      onPick(project.worktree!.key);
                      setOpen(false);
                    }}
                    data-start-new-branch="true"
                  >
                    <PlusIcon aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate">{t("startPanel.newBranch")}</span>
                  </CommandItem>
                </CommandGroup>
              </>
            ) : null}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function StartPanel({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const text = useStartPanel((s) => s.text);
  const chosen = useStartPanel((s) => s.target);
  const request = useStartPanel((s) => s.request);
  const failure = useStartPanel((s) => s.failure);
  const rest = useShellStore((s) => s.rest);
  const screen = useUiStore((s) => s.screen);
  const overSettings = useStartPanel((s) => s.overSettings);
  const preset = useStartPanel((s) => s.preset);
  // What is in front is read again on every open; the panel itself lives only while open.
  const frontProjectId = useUiStore((s) => s.overviewOpen || s.screen?.kind === "main" ? s.overview.projectId : null);
  const overviewProjectId = preset?.projectId ?? frontProjectId;
  const targets = useMemo(() => startTargets(rest, screen, t, overSettings, overviewProjectId), [rest, screen, t, overSettings, overviewProjectId]);
  // A pick whose device stops answering drops out of that device's list; the
  // panel keeps showing it, disabled, rather than moving the start elsewhere.
  const held = useRef<{ target: StartTarget; project: StartProject | null } | null>(null);
  const listed = resolveTarget(targets, chosen);
  const lost = chosen !== null && listed?.key !== chosen && held.current?.target.key === chosen && targets.groups.find((group) => group.deviceId === held.current!.target.deviceId)?.connected === false;
  const target: StartTarget | null = lost ? { ...held.current!.target, disabled: "not_connected" } : listed;
  const project = lost ? held.current!.project : projectOf(targets, target);
  useEffect(() => {
    if (!lost && target) held.current = { target, project };
  });
  const own = windowDeviceId(rest?.navigator?.devices);
  const offline = target?.disabled === "not_connected" ? targets.groups.find((group) => group.deviceId === target.deviceId) ?? null : null;
  const worktreeProject = target?.kind === "worktree" ? catalogWorkspaces(rest).find((workspace) => workspace.device_id === target.deviceId && workspace.path === target.repositoryRoot) ?? null : null;
  const initialBase = worktreeProject ? projectBaseBranch(rest, worktreeProject) : null;
  const [worktreeBranch, setWorktreeBranch] = useState("");
  const [worktreeBase, setWorktreeBase] = useState<{ project: string; branch: string } | null>(null);
  const base = worktreeBase && worktreeBase.project === target?.key ? worktreeBase.branch : initialBase?.branch ?? "";
  const [selection, setSelection] = useState<AgentSelection>(() => {
    const start = useShellStore.getState().rest?.ui_state?.agent_start;
    return preset?.kind ? selectKind(preset.kind, start) : rememberedSelection(start);
  });
  // The profile menu (PRD agent-profiles B14): what the core would pick is
  // shown chosen, and only a profile the operator picked is sent.
  const profiles = target && selection.kind !== "terminal" ? startProfiles(rest, target.deviceId, target.workspaceId, selection.kind) : null;
  const profileKey = target && profiles ? `${target.deviceId}:${target.workspaceId}:${profiles.view.agent}` : null;
  const [profilePick, setProfilePick] = useState<{ key: string; id: string } | null>(null);
  const pickedProfile = profilePick && profilePick.key === profileKey && profiles?.view.profiles.some((row) => row.id === profilePick.id) ? profilePick.id : null;
  const surface = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const previous = useRef<HTMLElement | null>(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const working = request !== null;

  const dismiss = () => {
    useStartPanel.getState().close();
    restoreFocus(previous.current);
  };
  useEscapeLayer(true, dismiss);
  useEffect(() => {
    field.current?.focus();
  }, []);
  // A press outside closes, except inside the menus the panel opened: they render in their own portal.
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const at = event.target;
      if (!(at instanceof Element)) return;
      if (surface.current?.contains(at) || at.closest("[data-radix-popper-content-wrapper]")) return;
      useStartPanel.getState().close();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, []);

  const canStart = target !== null && target.disabled === null && selection.kind !== "terminal" && !working && (target.kind !== "worktree" || (worktreeProject !== null && branchProblem(worktreeBranch) === null));
  // A device with no project has nothing to start in: Add a project takes the start's place.
  const addProject = () => {
    useStartPanel.getState().close();
    actions.openAddProject(targets.frontDeviceId);
  };
  const start = () => {
    if (!canStart || !target || selection.kind === "terminal") return;
    const id = startRequestId();
    useStartPanel.getState().begin(id, target.kind === "worktree" ? "worktree_create" : "agent_start");
    if (target.kind === "worktree") {
      actions.createWorktree({ requestId: id, deviceId: target.deviceId, repositoryRoot: target.repositoryRoot, branch: worktreeBranch.trim(), baseBranch: base || null, agentKind: selection.kind, model: modelToSend(selection), prompt: text.trim() || null, purpose: null, profile: pickedProfile });
      return;
    }
    actions.startAgent({
      target: { checkoutPath: target.checkoutPath },
      deviceId: target.deviceId,
      provider: selection.kind,
      model: modelToSend(selection),
      prompt: text.trim() || null,
      requestId: id,
      profile: pickedProfile,
    });
  };

  return (
    <div
      ref={surface}
      role="dialog"
      aria-label={t("commands.start_agent")}
      data-start-panel="true"
      className="fixed left-1/2 top-(--size-settings-sheet-window-inset) z-50 flex w-(--size-search-sheet-w) max-w-[calc(100%-var(--spacing-xxl))] -translate-x-1/2 flex-col overflow-hidden rounded-lg border border-border bg-dialog text-body text-popover-foreground [--accent:var(--dialog-raised)] [--secondary:var(--dialog-raised)] shadow-lg"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-xxs px-sm pt-sm" data-start-where="true">
        {target && target.deviceId !== own ? <DeviceChip variant="remote" label={targets.groups.find((group) => group.deviceId === target.deviceId)?.deviceLabel ?? ""} /> : null}
        <ProjectPicker targets={targets} project={project} target={target} disabled={working || targets.groups.length === 0} onPick={(key) => useStartPanel.getState().setTarget(key)} />
        {project && target ? <span aria-hidden="true" className="text-caption text-muted-foreground">›</span> : null}
        {project && target && target.kind !== "worktree" ? (
          <BranchPicker project={project} target={target} currentKey={targets.currentKey} disabled={working} onPick={(key) => useStartPanel.getState().setTarget(key)} />
        ) : null}
        {project && target?.kind === "worktree" ? (
          <span className="flex min-w-0 items-center gap-xs" data-start-worktree="true">
            <Input
              value={worktreeBranch}
              disabled={working}
              placeholder={t("startPanel.newBranchName")}
              onChange={(event) => setWorktreeBranch(event.target.value)}
              aria-label={t("workspace.branch")}
              aria-invalid={worktreeBranch !== "" && branchProblem(worktreeBranch) !== null}
              className="h-(--size-control-sm) w-(--size-start-branch-field) text-caption"
              data-start-branch-name="true"
            />
            <span className="text-caption text-muted-foreground">{t("startPanel.from")}</span>
            <Select value={base} disabled={working || !worktreeProject} onValueChange={(branch) => setWorktreeBase({ project: target.key, branch })}>
              <SelectTrigger size="sm" aria-label={t("projectSettings.base")} className="w-auto min-w-0"><SelectValue /></SelectTrigger>
              <SelectContent>{worktreeProject?.branches?.map((branch) => <SelectItem key={branch} value={branch}>{branch}</SelectItem>)}</SelectContent>
            </Select>
            <Hint label={t("startPanel.cancelNewBranch")}>
              <Button variant="ghost" size="icon-xs" disabled={working} aria-label={t("startPanel.cancelNewBranch")} onClick={() => useStartPanel.getState().setTarget(project.branches[0]!.key)} data-start-cancel-new-branch="true"><XIcon /></Button>
            </Hint>
          </span>
        ) : null}
      </div>
      {target?.kind === "worktree" && worktreeBranch && branchProblem(worktreeBranch) ? <p role="alert" className="px-md pt-xs text-caption text-destructive">{t(branchProblem(worktreeBranch)!)}</p> : null}
      {target?.kind === "worktree" && initialBase?.missing ? <p className="px-md pt-xs text-caption text-warning">{t("projectSettings.branchMissing", { branch: initialBase.missing, fallback: initialBase.branch })}</p> : null}
      <Textarea
        ref={field}
        bare
        rows={1}
        value={text}
        autoComplete="off"
        spellCheck={false}
        aria-label={t("shell.firstInstruction")}
        placeholder={t("shell.instructionPrompt")}
        data-start-text="true"
        className="field-sizing-content max-h-(--size-start-text-max) min-h-(--size-control-lg) overflow-y-auto px-md pt-xs pb-sm"
        onChange={(event) => useStartPanel.getState().setText(event.target.value)}
        onKeyDown={(event) => {
          // Enter starts; Shift+Enter is a new line, and an Enter that ends a composition is neither.
          if (event.key !== "Enter" || event.shiftKey || composingKey(event)) return;
          event.preventDefault();
          start();
        }}
      />
      {failure ? (
        <p role="alert" data-start-failure="true" data-core-error={"code" in failure ? failure.code : undefined} className="whitespace-pre-line break-words px-md pb-xs text-caption text-destructive">
          {"key" in failure ? t(failure.key) : failure.message}
          {"line" in failure && failure.line ? <QuotedLine line={failure.line} /> : null}
        </p>
      ) : null}
      {offline ? (
        <p role="status" data-start-offline={offline.deviceId} className="flex flex-wrap items-center gap-xs px-md pb-xs text-caption text-muted-foreground">
          {t("startPanel.deviceOffline", { device: offline.deviceLabel })}
          <Button variant="text" size="inline" onClick={() => actions.retryDevice(offline.deviceId)} data-start-reconnect={offline.deviceId}>{t("sidebar.reconnect")}</Button>
        </p>
      ) : null}
      <div className="flex min-w-0 items-center gap-xs px-sm pt-xs pb-sm" data-start-who="true">
        <AgentPicker
          variant="ghost"
          actions={actions}
          value={selection}
          onChange={setSelection}
          disabled={working}
          kindNote={
            <>
              {profiles && profileKey ? (
                <ProfilePicker view={profiles.view} value={pickedProfile ?? profiles.picked} projectDefault={profiles.projectDefault} disabled={working} onChange={(id) => setProfilePick({ key: profileKey, id })} />
              ) : null}
              {/* The provider probe describes the core machine, never a remote target's login (B23, B37). */}
              {target?.deviceId === localDeviceId(rest) && notSignedIn(selection.kind, rest?.status?.background_ai?.providers) ? (
                <span className="flex shrink-0 items-center [&>span]:text-caption" data-start-not-signed-in={selection.kind}>
                  <Status tone="warn">{t("firstRun.tile.notSignedIn")}</Status>
                </span>
              ) : null}
            </>
          }
        />
        <span className="flex-1" />
        {target === null && !working ? (
          <Button size="default" onClick={addProject} data-start-add-project="true">
            <PlusIcon aria-hidden="true" />
            {t("firstRun.home.addProject")}
          </Button>
        ) : (
          <Button size="default" disabled={!canStart} onClick={start} data-start-submit="true">
            {working ? t("shell.starting") : t("common.start")}
            {working ? null : <span aria-hidden="true" className="opacity-(--opacity-dimmed)">⏎</span>}
          </Button>
        )}
      </div>
    </div>
  );
}
