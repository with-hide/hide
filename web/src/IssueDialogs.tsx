// Work that starts from an issue (the issue-first Overview, 2026-09-28): the
// Start dialog that turns an issue into a worktree with an agent and its
// first prompt. Each sends the one core event that owns
// the change and reads the core's own answer for it (`issue_work`,
// `task_operation`); a dialog never decides that its request succeeded.

import { FileTextIcon, CircleDotIcon, RotateCcwIcon, SparklesIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ComponentProps, type ReactNode } from "react";
import type { Actions } from "./actions";
import { rememberedSelection, modelToSend, type AgentSelection } from "./agentPicker";
import { AgentPicker } from "./components/agent-picker";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { Kbd } from "./components/ui/kbd";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { Note, Status } from "./components/settings-rows";
import { defaultWorktreeName, firstPrompt, namePrefix } from "./issueStart";
import { cn } from "./lib/utils";
import { localDeviceId, type IssueSettings, type Task, type Workspace } from "./snapshot";
import { hideAiCanAnswer } from "./hideAi";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { branchProblem, taskFor } from "./workspaceManage";
import { coreErrorText } from "./coreError";
import { issueBodyFailure } from "./issueFailure";
import { useErrorSince } from "./errorSince";
import { fieldLabel } from "./shortcutLabels";
import { originOf, useBacklogOrigin } from "./backlogOrigin";
import { useIssueDetail } from "./issueDetails";
import { remoteRequestId } from "./remote";
import { followStartedPane } from "./opening";

export const DEFAULT_SETTINGS: IssueSettings = { ai_worktree_name: true, closes_instruction: true };

/** A multi-line field in the text field's look; the issue body and the first prompt. */
export function TextArea({ className, ...props }: ComponentProps<"textarea">) {
  return (
    <textarea
      className={cn(
        "min-h-(--size-overview-group-header) w-full min-w-0 resize-y rounded-sm border border-input bg-background px-sm py-xs text-body text-foreground outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-(--opacity-disabled)",
        className,
      )}
      {...props}
    />
  );
}

export function Field({ label, aside, children }: { label: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <label className="block text-body text-subtle-foreground">
      <span className="flex items-center gap-xs">
        <span>{label}</span>
        {aside ? <span className="ml-auto text-caption">{aside}</span> : null}
      </span>
      <span className="mt-xxs block">{children}</span>
    </label>
  );
}

/** ⌘↵ (Ctrl+Enter off macOS) sends the form from any field, the way the footer's keycap says. */
export function submitOnCommandEnter(event: React.KeyboardEvent<HTMLFormElement>) {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    event.currentTarget.requestSubmit();
  }
}

/** The name field's note: who wrote the name, and the way back to the AI's. */
function NameNote({
  ai,
  answer,
  edited,
  onRestore,
}: {
  ai: boolean;
  answer: { phase: string; name: string | null; message: string | null } | null;
  edited: boolean;
  onRestore: () => void;
}) {
  const { t } = useInterfaceTranslation();
  if (!ai) return null;
  if (!answer || answer.phase === "working") {
    return (
      <span className="text-muted-foreground" data-name-state="working">
        {t("issue.name.working")}
      </span>
    );
  }
  if (answer.phase === "failed") {
    return (
      <Hint label={answer.message ?? t("issue.name.unanswered")}>
        <span className="text-muted-foreground" tabIndex={0} data-name-state="failed">
          {t("issue.name.defaultFallback")}
        </span>
      </Hint>
    );
  }
  if (edited && answer.name) {
    return (
      <button type="button" onClick={onRestore} className="inline-flex items-center gap-xxs rounded-xs text-primary outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-name-state="edited">
        {t("issue.name.edited")} <RotateCcwIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        {t("issue.name.restore")}
      </button>
    );
  }
  return (
    <span className="inline-flex items-center gap-xxs text-muted-foreground" data-name-state="ai">
      <SparklesIcon aria-hidden="true" className="size-(--size-icon-sm) text-primary" />
      {t("issue.name.aiEditable")}
    </span>
  );
}

/**
 * Start from an issue: one worktree named for the issue (the dialog's name
 * first, the background AI's once it answers unless the operator has typed),
 * its base, the agent, and the first prompt drawn from the issue's body. The
 * core creates the worktree, writes the link into it, starts the agent and
 * sends it the prompt once it is ready. A folder project has no worktree:
 * the agent starts in the folder with the prompt.
 */
export function StartIssueDialog({ actions, workspace, task, onClose }: { actions: Actions; workspace: Workspace; task: Task; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const settings = useShellStore((s) => s.rest?.ui_state?.issue_settings) ?? DEFAULT_SETTINGS;
  const hideAi = useShellStore((s) => s.rest?.status?.background_ai);
  // The AI names the worktree only when it is on and something can answer; otherwise the dialog keeps its default name and does not wait.
  const aiNames = settings.ai_worktree_name && hideAiCanAnswer(hideAi);
  const git = workspace.is_git === true;
  const branches = workspace.branches ?? [];
  const requestId = useMemo(() => JSON.stringify([workspace.device_id, workspace.id, task.key, remoteRequestId()]), [workspace.device_id, workspace.id, task.key]);
  const [name, setName] = useState(() => defaultWorktreeName(task));
  const [edited, setEdited] = useState(false);
  const [base, setBase] = useState(workspace.default_branch && branches.includes(workspace.default_branch) ? workspace.default_branch : (branches[0] ?? ""));
  const [agent, setAgent] = useState<AgentSelection>(() => rememberedSelection(useShellStore.getState().rest?.ui_state?.agent_start));
  const [prompt, setPrompt] = useState(() => firstPrompt(task, null, settings.closes_instruction, t));
  const promptEdited = useRef(false);
  const detail = useIssueDetail(actions, workspace, task.key);
  const origin = useShellStore((s) => originOf(s.rest, workspace, task.key));
  const nameAnswer = useShellStore((s) => (s.rest?.issue_work?.name?.request_id === requestId ? s.rest.issue_work.name : null));
  const asked = useRef(false);
  const [request, setRequest] = useState<{ afterId: number; branch: string; at: number } | null>(null);
  const operation = useShellStore((s) => s.rest?.task_operation);
  const created = taskFor(operation, request ? { kind: "worktree_create", afterId: request.afterId, deviceId: workspace.device_id, repositoryRoot: workspace.path, branch: request.branch } : null, localDeviceId(useShellStore.getState().rest));
  const refused = useErrorSince(request?.at ?? null, ["worktree.create", "task_operation.", "agent_start."]);
  const working = request !== null && refused === null && (created === null || created.phase === "working");

  // The body is read once per dialog; a GitHub issue's comes from `gh`.
  // So is where it came from: a missed line's request and answer join the prompt (B33).
  useBacklogOrigin(actions, workspace, task.key);
  const bodyKnown = detail !== null && detail.phase !== "reading";
  const originKnown = origin !== null && origin.phase !== "reading";
  useEffect(() => {
    if (!bodyKnown || !originKnown || promptEdited.current) return;
    setPrompt(firstPrompt(task, detail?.body ?? null, settings.closes_instruction, t, origin ?? undefined));
  }, [bodyKnown, originKnown, detail?.body, origin, task, settings.closes_instruction, t]);
  // The AI names the worktree once the body is known, so the name can use it.
  useEffect(() => {
    if (!git || !aiNames || !bodyKnown || asked.current) return;
    asked.current = true;
    actions.suggestWorktreeName(requestId, namePrefix(task), task.title, detail?.body ?? "");
  }, [actions, git, aiNames, bodyKnown, detail?.body, requestId, task]);
  useEffect(() => {
    if (nameAnswer?.phase === "ready" && nameAnswer.name && !edited) setName(nameAnswer.name);
  }, [nameAnswer, edited]);
  useEffect(() => {
    if (created?.phase !== "ready") return;
    if (created.agent_phase) useUiStore.getState().setWatchedTask(created.id);
    if (created.pane_id) followStartedPane(created.pane_id);
    onClose();
  }, [created, onClose]);

  const branch = name.trim();
  const problem = git ? branchProblem(branch) : null;
  const existing = git && branches.includes(branch) ? (workspace.checkouts.find((checkout) => checkout.branch === branch) ?? null) : null;
  const taken = git && branches.includes(branch);
  const failure = created?.phase === "failed" ? (created.failure ? coreErrorText({ kind: created.failure }, t, "issue.worktreeFailed") : t("issue.worktreeFailed")) : refused ? coreErrorText(refused, t, "issue.worktreeFailed") : null;

  const submit = () => {
    if (working) return;
    if (existing) {
      actions.openWorkspace(workspace.device_id, existing.workspace_id, existing.id);
      onClose();
      return;
    }
    const text = prompt.trim();
    const model = modelToSend(agent);
    if (!git) {
      const folder = workspace.checkouts[0];
      if (!folder) return;
      if (agent.kind === "terminal") return;
      actions.startAgent({ target: { checkoutPath: folder.path }, deviceId: workspace.device_id, provider: agent.kind, model, prompt: text || null });
      onClose();
      return;
    }
    if (problem || taken) return;
    setRequest({ afterId: actions.taskIdNow(), branch, at: Date.now() });
    actions.createWorktree({
      deviceId: workspace.device_id,
      repositoryRoot: workspace.path,
      branch,
      baseBranch: base || null,
      agentKind: agent.kind === "terminal" ? null : agent.kind,
      model,
      purpose: null,
      taskKey: task.key,
      prompt: text || null,
    });
  };
  const Glyph = task.source === "github" ? CircleDotIcon : FileTextIcon;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-start-issue={task.key}>
        <form className="flex min-h-0 flex-1 flex-col" onKeyDown={submitOnCommandEnter} onSubmit={(event) => { event.preventDefault(); submit(); }}>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-xs">
              <Glyph aria-hidden="true" className="size-(--size-icon) text-muted-foreground" />
              {t("issue.startTitle", { issue: task.id ?? t("issue.label") })}
            </DialogTitle>
            <DialogDescription className="line-clamp-2 break-words">{task.title}</DialogDescription>
          </DialogHeader>
          <DialogBody className="space-y-sm">
            {git ? (
              <>
                <Field
                  label={t("issue.worktreeBranch")}
                  aside={
                    <NameNote
                      ai={aiNames}
                      answer={nameAnswer}
                      edited={edited}
                      onRestore={() => {
                        if (nameAnswer?.name) setName(nameAnswer.name);
                        setEdited(false);
                      }}
                    />
                  }
                >
                  <Input
                    value={name}
                    mono
                    disabled={working}
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={taken || problem !== null}
                    onChange={(event) => {
                      setName(event.target.value);
                      setEdited(true);
                    }}
                    data-start-name="true"
                  />
                </Field>
                {taken ? <Note tone="error" data-start-name-taken="true">{t(existing ? "issue.branchTakenExisting" : "issue.branchTakenNoCheckout")}</Note> : problem ? <Note tone="warn">{problem}</Note> : null}
                <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-md">
                  <Field label={t("issue.base")}>
                    <Select value={base || undefined} disabled={working || branches.length === 0} onValueChange={setBase}>
                      <SelectTrigger className="w-full" aria-label={t("issue.baseBranch")} data-start-base="true">
                        <SelectValue placeholder={t("issue.branchesUnread")} />
                      </SelectTrigger>
                      <SelectContent>
                        {branches.map((row) => (
                          <SelectItem key={row} value={row}>
                            {row}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </Field>
                  <AgentField agent={agent} actions={actions} disabled={working} onChange={setAgent} />
                </div>
              </>
            ) : (
              <AgentField agent={agent} actions={actions} disabled={working} onChange={setAgent} />
            )}
            <Field label={t("issue.firstPrompt")} aside={<span className="text-muted-foreground">{t(!bodyKnown || !originKnown ? "issue.bodyReading" : "issue.bodyPrefilled")}</span>}>
                <TextArea
                  value={prompt}
                  rows={5}
                  disabled={working}
                  onChange={(event) => {
                    promptEdited.current = true;
                    setPrompt(event.target.value);
                  }}
                  data-start-prompt="true"
                />
              </Field>
            {detail?.phase === "failed" ? <Note tone="warn" data-issue-detail-failure={detail.failure ?? undefined}>{issueBodyFailure(detail.failure, t)}</Note> : null}
            {failure ? <Note tone="error" data-start-error="true" data-core-error={refused?.kind}>{failure}</Note> : null}
            {working ? <Status tone="pending">{t("issue.creatingWorktree")}</Status> : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={onClose}>{t(working ? "issue.hide" : "common.cancel")}</Button>
            <Button type="submit" disabled={working || (git && !existing && (taken || problem !== null))} data-start-submit={existing ? "open" : "start"}>
              {t(existing ? "issue.openExisting" : working ? "issue.starting" : "common.start")}
              <Kbd>{fieldLabel("Enter")}</Kbd>
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** The kind and model to start with, the same control ⌘N's panel has (PRD home-device-rail B35). */
export function AgentField({ agent, actions, disabled, onChange }: { agent: AgentSelection; actions: Actions; disabled: boolean; onChange: (agent: AgentSelection) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="text-body text-subtle-foreground">
      {t("common.agent")}
      <AgentPicker actions={actions} value={agent} onChange={onChange} disabled={disabled} className="mt-xxs" />
    </div>
  );
}
