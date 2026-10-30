import { FolderIcon, HomeIcon, ServerIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { rememberedSelection, modelToSend, selectKind, type AgentSelection } from "./agentPicker";
import { AgentPicker } from "./components/agent-picker";
import { Button } from "./components/ui/button";
import { useEscapeLayer } from "./components/ui/layer";
import { Kbd } from "./components/ui/kbd";
import { Select, SelectContent, SelectGroup, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./components/ui/select";
import { START_ANSWER_TIMEOUT_MS, startAnswer, startRequestId } from "./startAnswer";
import { agentFailure, useStartPanel, startFailure, startRefusal } from "./startDraft";
import { QuotedLine } from "./QuotedLine";
import { resolveTarget, startTargets, type StartTarget } from "./startTargets";
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
      // A start's core result owns selection and ordered Herdr focus.
      // Opening it again here would undo a newer visit made while it waited;
      // only a device's worktree is opened from the page once listed.
      if (answer.paneId && requestKind === "worktree_create") followStartedPane(answer.paneId);
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

/** A Home is this window's own machine's (the house) or another machine's (the server), the core's included on a node's window. */
function TargetIcon({ target, own }: { target: StartTarget; own: string }) {
  if (target.kind !== "home") return <FolderIcon aria-hidden="true" />;
  return target.deviceId === own ? <HomeIcon aria-hidden="true" /> : <ServerIcon aria-hidden="true" />;
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
  const target = resolveTarget(targets, chosen);
  const worktreeProject = target?.kind === "worktree" ? catalogWorkspaces(rest).find((workspace) => workspace.device_id === target.deviceId && workspace.path === target.repositoryRoot) ?? null : null;
  const initialBase = worktreeProject ? projectBaseBranch(rest, worktreeProject) : null;
  const [worktreeBranch, setWorktreeBranch] = useState("");
  const [worktreeBase, setWorktreeBase] = useState<{ project: string; branch: string } | null>(null);
  const base = worktreeBase && worktreeBase.project === target?.key ? worktreeBase.branch : initialBase?.branch ?? "";
  const [selection, setSelection] = useState<AgentSelection>(() => {
    const start = useShellStore.getState().rest?.ui_state?.agent_start;
    return preset?.kind ? selectKind(preset.kind, start) : rememberedSelection(start);
  });
  const surface = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLInputElement>(null);
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

  const canStart = target !== null && selection.kind !== "terminal" && !working && (target.kind !== "worktree" || (worktreeProject !== null && branchProblem(worktreeBranch) === null));
  const start = () => {
    if (!canStart || !target || selection.kind === "terminal") return;
    const id = startRequestId();
    useStartPanel.getState().begin(id, target.kind === "worktree" ? "worktree_create" : "agent_start");
    if (target.kind === "worktree") {
      actions.createWorktree({ requestId: id, deviceId: target.deviceId, repositoryRoot: target.repositoryRoot, branch: worktreeBranch.trim(), baseBranch: base || null, agentKind: selection.kind, model: modelToSend(selection), prompt: text.trim() || null, purpose: null });
      return;
    }
    actions.startAgent({
      target: target.kind === "home" ? { home: true } : { checkoutPath: target.checkoutPath },
      deviceId: target.deviceId,
      provider: selection.kind,
      model: modelToSend(selection),
      prompt: text.trim() || null,
      requestId: id,
    });
  };

  return (
    <div
      ref={surface}
      role="dialog"
      aria-label={t("commands.start_agent")}
      data-start-panel="true"
      className="fixed left-1/2 top-(--size-settings-sheet-window-inset) z-50 flex w-(--size-search-sheet-w) max-w-[calc(100%-var(--spacing-xxl))] -translate-x-1/2 flex-col overflow-hidden rounded-lg border border-border bg-popover text-body text-popover-foreground shadow-lg"
    >
      <input
        ref={field}
        type="text"
        value={text}
        autoComplete="off"
        spellCheck={false}
        aria-label={t("shell.firstInstruction")}
        placeholder={t("shell.instructionPrompt")}
        data-start-text="true"
        className="h-(--size-control-lg) w-full bg-transparent px-md pt-md pb-md text-body text-foreground outline-none placeholder:text-muted-foreground"
        onChange={(event) => useStartPanel.getState().setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== "Enter" || composingKey(event)) return;
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
      {target?.kind === "worktree" ? <div className="flex flex-col gap-sm px-md py-sm" data-start-worktree="true">
        <p className="text-caption text-muted-foreground">{t("projectSettings.confirmWorktree")}</p>
        <label className="text-body">{t("workspace.branch")}<Input value={worktreeBranch} disabled={working} onChange={(event) => setWorktreeBranch(event.target.value)} aria-label={t("workspace.branch")} /></label>
        {worktreeBranch && branchProblem(worktreeBranch) ? <p role="alert" className="text-caption text-destructive">{t(branchProblem(worktreeBranch)!)}</p> : null}
        <Select value={base} disabled={working || !worktreeProject} onValueChange={(branch) => setWorktreeBase({ project: target.key, branch })}><SelectTrigger aria-label={t("projectSettings.base")}><SelectValue /></SelectTrigger><SelectContent>{worktreeProject?.branches?.map((branch) => <SelectItem key={branch} value={branch}>{branch}</SelectItem>)}</SelectContent></Select>
        {initialBase?.missing ? <p className="text-caption text-warning">{t("projectSettings.branchMissing", { branch: initialBase.missing, fallback: initialBase.branch })}</p> : null}
      </div> : null}
      <div className="flex items-center gap-xs border-t border-border px-sm py-sm">
        <Select value={target?.key ?? ""} disabled={working || targets.groups.length === 0} onValueChange={(key) => useStartPanel.getState().setTarget(key)}>
          <SelectTrigger aria-label={t("common.target")} className="min-w-0 flex-1" data-start-target={target?.key ?? ""}>
            <SelectValue placeholder={t("shell.noTarget")} />
          </SelectTrigger>
          <SelectContent>
            {targets.groups.map((group, index) => (
              <SelectGroup key={group.deviceId} data-start-target-group={group.deviceId}>
                {index > 0 ? <SelectSeparator className="bg-secondary" /> : null}
                {group.items.map((item) => (
                  <SelectItem key={item.key} value={item.key} disabled={item.disabled !== null} data-start-target-option={item.key}>
                    <TargetIcon target={item} own={windowDeviceId(rest?.navigator?.devices)} />
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate">{item.label}</span>
                      {item.disabled ? <span className="text-caption text-muted-foreground">{t("devices.rail.notConnected")}</span> : null}
                    </span>
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>
        <AgentPicker
          actions={actions}
          value={selection}
          onChange={setSelection}
          disabled={working}
          kindNote={
            // The provider probe describes the core machine, never a remote target's login (B23, B37).
            target?.deviceId === localDeviceId(rest) && notSignedIn(selection.kind, rest?.status?.background_ai?.providers) ? (
              <span className="flex shrink-0 items-center [&>span]:text-caption" data-start-not-signed-in={selection.kind}>
                <Status tone="warn">{t("firstRun.tile.notSignedIn")}</Status>
              </span>
            ) : null
          }
        />
        <Kbd aria-hidden="true">⏎</Kbd>
        <Button size="default" disabled={!canStart} onClick={start} data-start-submit="true">
          {working ? t("shell.starting") : t("common.start")}
        </Button>
      </div>
    </div>
  );
}
