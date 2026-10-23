import { projectBaseBranch } from "./projectSettings";
// The project and checkout management dialogs (PRD S5 B11-B17): a worktree
// creation, a purpose edit and a worktree deletion. Each sends the one core
// event that owns the change and then reads the core's own receipt for it
// (`task_operation`, `worktree_removal`); a dialog never decides that its
// request succeeded, and a receipt for another target is never its answer.

import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { rememberedSelection, modelToSend, type AgentSelection } from "./agentPicker";
import { AlertDialog, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { AgentPicker } from "./components/agent-picker";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { DiskCleanupSheet } from "./DiskCleanupSheet";
import { agentFailure, endedSentence } from "./startDraft";
import { QuotedLine } from "./QuotedLine";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Note, Status } from "./components/settings-rows";
import { SubtreeList } from "./components/subtree-list";
import { closeScope, subtreeOf, type Subtree } from "./close";
import { StartIssueDialog } from "./IssueDialogs";
import { PrDelegateDialog, PrLinkDialog, PrNewIssueDialog } from "./PrDialogs";
import { translate, useInterfaceTranslation } from "./i18n/client";
import { coreErrorText } from "./coreError";
import { useErrorSince } from "./errorSince";
import { localDeviceId, type Checkout, type Workspace } from "./snapshot";
import { machineName, windowDeviceId } from "./screenMachine";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import {
  PURPOSE_HARD_LIMIT,
  branchProblem,
  deletionFacts,
  discardConfirmationKey,
  factsLine,
  normalizePurpose,
  projectRemovalFacts,
  purposeCountLabel,
  purposeIsLong,
  purposeScope,
  removalFor,
  taskFor,
} from "./workspaceManage";

/** The row a dialog names, among this machine's projects or a connected device's workspaces (a remote purpose). */
function findTarget(workspaceId: string, checkoutId?: string): { workspace: Workspace; checkout: Checkout | null } | null {
  const rest = useShellStore.getState().rest;
  const workspace =
    rest?.navigator?.workspaces?.find((row) => row.id === workspaceId) ??
    rest?.status?.remote?.flatMap((row) => row.session?.workspaces ?? []).find((row) => row.id === workspaceId);
  if (!workspace) return null;
  return { workspace, checkout: checkoutId ? (workspace.checkouts.find((row) => row.id === checkoutId) ?? null) : null };
}

/** The machine a workspace belongs to, as this window names it, or null for the window's own. */
function deviceLabel(workspace: Workspace): string | null {
  const devices = useShellStore.getState().rest?.navigator?.devices;
  const device = workspace.remote_target_id ?? workspace.device_id;
  return device === windowDeviceId(devices) ? null : machineName(devices, device, translate);
}

export function WorkspaceDialogs({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const dialog = useUiStore((s) => s.workspaceDialog);
  // Re-read on every snapshot so a dialog follows its row (a purpose saved
  // elsewhere, a gate that changed) rather than a copy from when it opened.
  useShellStore((s) => s.rest?.navigator?.workspaces);
  useShellStore((s) => s.rest?.status?.remote);
  // A deleted worktree or a removed project leaves the catalog while its
  // dialog still reports the result, so the dialog keeps the last row it saw.
  const lastSeen = useRef<{ key: string; target: { workspace: Workspace; checkout: Checkout | null } } | null>(null);
  const close = () => useUiStore.getState().setWorkspaceDialog(null);
  if (!dialog) return null;
  const key = JSON.stringify(dialog);
  const found = findTarget(dialog.workspaceId, "checkoutId" in dialog ? dialog.checkoutId : undefined);
  if (found && (!("checkoutId" in dialog) || found.checkout)) lastSeen.current = { key, target: found };
  const keepsLastSeen = dialog.kind === "delete_worktree" || dialog.kind === "remove_project";
  const target = found && (found.checkout || !("checkoutId" in dialog)) ? found : keepsLastSeen && lastSeen.current?.key === key ? lastSeen.current.target : found;
  if (!target || ("checkoutId" in dialog && !target.checkout)) {
    return (
      <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
        <DialogContent aria-label={t("common.unavailable")}>
          <DialogBody>
            <Note tone="warn">{t("workspace.unavailableProject")}</Note>
          </DialogBody>
          <DialogFooter>
            <Button onClick={close}>{t("common.close")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
  if (dialog.kind === "new_worktree") return <NewWorktreeDialog actions={actions} workspace={target.workspace} onClose={close} />;
  if (dialog.kind === "start_issue") {
    const task = target.workspace.tasks?.tasks.find((row) => row.key === dialog.taskKey);
    if (task) return <StartIssueDialog key={JSON.stringify([target.workspace.device_id, target.workspace.id, task.key])} actions={actions} workspace={target.workspace} task={task} onClose={close} />;
    return (
      <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
        <DialogContent aria-label={t("common.unavailable")}>
          <DialogBody>
            <Note tone="warn">{t("workspace.unavailableIssue")}</Note>
          </DialogBody>
          <DialogFooter>
            <Button onClick={close}>{t("common.close")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
  if (dialog.kind === "pr_link" || dialog.kind === "pr_new_issue" || dialog.kind === "pr_delegate") {
    const pr = target.workspace.pull_requests?.find((row) => row.number === dialog.prNumber);
    const task = dialog.kind === "pr_link" ? target.workspace.tasks?.tasks.find((row) => row.key === dialog.issueKey) : undefined;
    if (pr && dialog.kind === "pr_delegate") return <PrDelegateDialog key={`${pr.number}:${dialog.intent ?? "fix"}`} actions={actions} workspace={target.workspace} pr={pr} intent={dialog.intent} onClose={close} />;
    if (pr && dialog.kind === "pr_new_issue") return <PrNewIssueDialog key={pr.number} actions={actions} workspace={target.workspace} pr={pr} onClose={close} />;
    if (pr && task) return <PrLinkDialog key={`${pr.number}:${task.key}`} actions={actions} workspace={target.workspace} pr={pr} task={task} onClose={close} />;
    return (
      <Dialog open onOpenChange={(next) => { if (!next) close(); }}>
        <DialogContent aria-label={t("common.unavailable")}>
          <DialogBody>
            <Note tone="warn">{t("workspace.unavailablePr")}</Note>
          </DialogBody>
          <DialogFooter>
            <Button onClick={close}>{t("common.close")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
  if (dialog.kind === "disk_cleanup") return <DiskCleanupSheet key={dialog.workspaceId} actions={actions} workspace={target.workspace} filter={dialog.filter} onClose={close} />;
  if (dialog.kind === "remove_project") return <RemoveProjectDialog actions={actions} workspace={target.workspace} listed={found !== null} onClose={close} />;
  if (dialog.kind === "purpose" && target.checkout) return <PurposeDialog actions={actions} checkout={target.checkout} deviceLabel={deviceLabel(target.workspace)} onClose={close} />;
  if (dialog.kind === "delete_worktree" && target.checkout) return <DeleteWorktreeDialog actions={actions} deviceId={target.workspace.device_id} checkout={target.checkout} listed={!!found?.checkout} onClose={close} />;
  return null;
}

/**
 * The agents outside what a removal takes, spawned from the agents inside
 * it (PRD close-agent-subtree D-34): re-read on every snapshot, so a status
 * check or an agent that closed shows at once. Null when there are none,
 * which leaves the dialog as it was.
 */
function useOutsideSubtree(actions: Actions, inside: string[], listed: boolean): Subtree | null {
  const rest = useShellStore((s) => s.rest);
  useShellStore((s) => s.agents);
  // A retained result row is no longer a live removal target.
  return listed ? subtreeOf(closeScope(rest, inside), actions.everyAgent()) : null;
}

/**
 * The part of a removal dialog that names the agents outside it: a short
 * heading, the list and, while one of them is unknown, the status check the
 * subtree choice waits for (D-37, D-42).
 */
function OutsideAgents({ actions, subtree, what, targetDevice }: { actions: Actions; subtree: Subtree; what: "project" | "worktree"; targetDevice: string | undefined }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-removal-subtree="true">
      <p className="break-words text-caption font-medium text-subtle-foreground">
        {t("workspace.outsideAgents", { count: subtree.ids.length, scope: t(`workspace.scope.${what}`) })}
      </p>
      <SubtreeList subtree={subtree} targetDevice={targetDevice} scope="removal" />
      {subtree.unknown ? (
        <p className="flex flex-wrap items-center gap-xs text-caption text-subtle-foreground" data-subtree-blocked="true">
          <span className="min-w-0 break-words">{t("workspace.outsideUnknown")}</span>
          <Button size="sm" variant="secondary" onClick={() => actions.refreshStatus()} data-subtree-check-status="true">
            {t("workspace.checkStatus")}
          </Button>
        </p>
      ) : null}
    </div>
  );
}

/**
 * `Remove project…` on any device: the panes the core counted close first,
 * then only the registration goes. The row leaving the registrations is the
 * answer; a refusal or a close that failed is the core's error. A row Herdr
 * shows without a registration has only its panes to close, so its answer is
 * the row leaving the list with Herdr's workspace (PRD sidebar-context-menus D-14).
 */
function RemoveProjectDialog({ actions, workspace, listed, onClose }: { actions: Actions; workspace: Workspace; listed: boolean; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const [at, setAt] = useState<number | null>(null);
  const registered = useShellStore((s) => s.rest?.ui_state?.workspace_registrations?.some((row) => row.id === workspace.id) ?? false);
  const refused = useErrorSince(at, ["workspace.remove", "workspace.create_in_flight"]);
  const removed = at !== null && (workspace.registered ? !registered : !listed);
  const working = at !== null && !removed && refused === null;
  const panes = workspace.removal?.pane_count ?? 0;
  const inside = workspace.checkouts.flatMap((checkout) => checkout.tabs.flatMap((tab) => tab.panes.map((pane) => pane.id)));
  const subtree = useOutsideSubtree(actions, inside, listed);
  // What the operator chose, so Try again repeats it on what is left (B22).
  const [withOutside, setWithOutside] = useState(false);
  const remove = (outside: boolean) => {
    setWithOutside(outside);
    setAt(Date.now());
    actions.removeWorkspace(workspace.id, outside ? (subtree?.ids ?? []) : []);
  };
  const offering = at === null || (refused !== null && !removed);
  const closingOutside = working && withOutside && subtree !== null;
  return (
    <AlertDialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <AlertDialogContent data-remove-project={workspace.id}>
        <AlertDialogHeader>
          <AlertDialogTitle className="break-words">{t("workspace.removeProjectTitle", { name: workspace.label })}</AlertDialogTitle>
          <AlertDialogDescription className="break-all font-mono text-caption text-muted-foreground">{workspace.path}</AlertDialogDescription>
        </AlertDialogHeader>
        {at === null ? (
          <p className="break-words text-body text-subtle-foreground" data-remove-consequences="true">
            {factsLine(projectRemovalFacts(workspace, t))}
          </p>
        ) : null}
        {subtree && (offering || closingOutside) ? <OutsideAgents actions={actions} subtree={subtree} what="project" targetDevice={workspace.device_id} /> : null}
        {working ? (
          <Status tone="pending" data-remove-phase={closingOutside ? "closing_outside" : "closing"}>
            {closingOutside ? t("workspace.closingOutsideProject", { count: subtree.ids.length }) : panes > 0 ? t("workspace.closingProject") : t("workspace.removingRegistration")}
          </Status>
        ) : null}
        {refused ? (
          <Note tone="error" data-remove-result="failed" data-core-error={refused.kind}>
            {coreErrorText(refused, t, "coreError.project.removeFailed")}
          </Note>
        ) : null}
        {removed ? (
          <Note tone="ok" data-remove-result="finished">
            {t("workspace.projectRemoved")}
          </Note>
        ) : null}
        <AlertDialogFooter>
          <Button variant="secondary" onClick={onClose} data-remove-cancel="true">
            {removed || refused ? t("common.close") : working ? t("workspace.hide") : t("workspace.keepProject")}
          </Button>
          {/* A refusal is retried from the same button (S5.5 B45). Plain
              Button, not AlertDialogAction: Radix closes on an Action's
              click, but this one has to stay open through the async removal. */}
          {offering && subtree ? (
            <>
              <Button variant="secondary" onClick={() => remove(false)} data-remove-confirm="only">
                {t("workspace.removeOnly")}
              </Button>
              <Button variant="destructive" disabled={subtree.unknown} onClick={() => remove(true)} data-remove-confirm="with-outside">
                {t("workspace.closeAgentsRemove", { count: subtree.ids.length })}
              </Button>
            </>
          ) : offering ? (
            <Button variant="destructive" onClick={() => remove(withOutside)} data-remove-confirm="true">
              {refused !== null ? t("workspace.tryAgain") : panes > 0 ? t("workspace.closePanesRemove", { count: panes }) : t("workspace.removeProject")}
            </Button>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** A Dialog's title plus an optional monospace path beneath it. */
function DialogIntro({ title, detail }: { title: string; detail?: string }) {
  return (
    <DialogHeader>
      <DialogTitle className="break-words">{title}</DialogTitle>
      {detail ? <DialogDescription className="break-all font-mono text-caption text-muted-foreground">{detail}</DialogDescription> : null}
    </DialogHeader>
  );
}

function NewWorktreeDialog({ actions, workspace, onClose }: { actions: Actions; workspace: Workspace; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const branches = workspace.branches ?? [];
  const [branch, setBranch] = useState("");
  const [base, setBase] = useState(() => projectBaseBranch(useShellStore.getState().rest, workspace).branch);
  const missingBase = projectBaseBranch(useShellStore((state) => state.rest), workspace).missing;
  // The remembered kind and model, not a terminal (PRD home-device-rail B36); Terminal only is the menu's first item.
  const [agent, setAgent] = useState<AgentSelection>(() => rememberedSelection(useShellStore.getState().rest?.ui_state?.agent_start));
  const [purpose, setPurpose] = useState("");
  const [request, setRequest] = useState<{ afterId: number; branch: string; at: number } | null>(null);
  const operation = useShellStore((s) => s.rest?.task_operation);
  const task = taskFor(operation, request ? { kind: "worktree_create", afterId: request.afterId, deviceId: workspace.device_id, repositoryRoot: workspace.path, branch: request.branch } : null, localDeviceId(useShellStore.getState().rest));
  const refused = useErrorSince(request?.at ?? null, ["worktree.create", "task_operation."]);
  const working = request !== null && refused === null && (task === null || task.phase === "working");
  const problem = branch ? branchProblem(branch) : null;

  useEffect(() => {
    if (task?.phase !== "ready") return;
    // The core focused the created pane; the agent's answer, if one was
    // chosen, is reported by the task notice after this dialog is gone.
    if (task.agent_phase) useUiStore.getState().setWatchedTask(task.id);
    if (task.pane_id) useUiStore.getState().setFocusWhenListed(task.pane_id);
    onClose();
  }, [task, onClose]);

  const submit = () => {
    const name = branch.trim();
    if (branchProblem(name) || working) return;
    setRequest({ afterId: actions.taskIdNow(), branch: name, at: Date.now() });
    actions.createWorktree({
      deviceId: workspace.device_id,
      repositoryRoot: workspace.path,
      branch: name,
      baseBranch: base || null,
      agentKind: agent.kind === "terminal" ? null : agent.kind,
      model: modelToSend(agent),
      purpose: purpose.trim() ? normalizePurpose(purpose.trim()) : null,
    });
  };
  const failure = task?.phase === "failed" ? (task.message ?? t("workspace.createFailed")) : refused ? coreErrorText(refused, t, "workspace.createFailed") : null;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-new-worktree={workspace.id}>
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <DialogIntro title={t("workspace.newWorktreeTitle", { name: workspace.label })} detail={workspace.path} />
          <DialogBody className="space-y-sm">
            <label className="block text-body text-subtle-foreground">
              {t("workspace.branch")}
              <Input value={branch} disabled={working} autoComplete="off" spellCheck={false} placeholder="feature/name" className="mt-xxs" mono onChange={(event) => setBranch(event.target.value)} data-worktree-branch="true" />
            </label>
            {problem ? <Note tone="warn">{t(problem)}</Note> : null}
            {missingBase ? <Note tone="warn">{t("projectSettings.branchMissing", { branch: missingBase, fallback: base })}</Note> : null}
            <label className="block text-body text-subtle-foreground">
              {t("workspace.base")}
              <Select value={base || undefined} disabled={working || branches.length === 0} onValueChange={(value) => setBase(value)}>
                <SelectTrigger data-worktree-base="true" aria-label={t("workspace.baseBranch")} className="mt-xxs w-full">
                  <SelectValue placeholder={t("workspace.noBranches")} />
                </SelectTrigger>
                <SelectContent>
                  {branches.map((name) => (
                    <SelectItem key={name} value={name}>
                      {name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <div className="text-body text-subtle-foreground">
              {t("workspace.startInPane")}
              <AgentPicker actions={actions} value={agent} onChange={setAgent} withTerminal disabled={working} className="mt-xxs" />
            </div>
            <label className="block text-body text-subtle-foreground">
              {t("workspace.optionalPurpose")}
              <Input value={purpose} disabled={working} maxLength={PURPOSE_HARD_LIMIT * 2} className="mt-xxs" onChange={(event) => setPurpose(normalizePurpose(event.target.value))} data-worktree-purpose="true" />
            </label>
            {purpose ? (
              <p className={`text-caption ${purposeIsLong(purpose) ? "text-warning" : "text-muted-foreground"}`}>{purposeCountLabel(purpose, t)}{purposeIsLong(purpose) ? ` · ${t("workspace.purposeLong")}` : ""}</p>
            ) : null}
            {failure ? <Note tone="error" data-worktree-error="true" data-core-error={refused?.kind}>{failure}</Note> : null}
            {working ? <Status tone="pending">{t("workspace.creatingWorktree")}</Status> : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={onClose}>{working ? t("workspace.hide") : t("common.cancel")}</Button>
            <Button type="submit" disabled={working || !branch.trim() || problem !== null} data-worktree-create="true">
              {working ? t("workspace.creating") : t("workspace.createWorktree")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function PurposeDialog({ actions, checkout, deviceLabel, onClose }: { actions: Actions; checkout: Checkout; deviceLabel: string | null; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  // Only a purpose someone wrote is the field's value; a title the row falls
  // back to is shown as the placeholder, so saving never stores a guess.
  const written = checkout.purpose && (checkout.purpose.origin === "token" || checkout.purpose.origin === "branch_description") ? checkout.purpose.text : "";
  const [text, setText] = useState(written);
  const [request, setRequest] = useState<{ afterId: number; at: number; text: string } | null>(null);
  const operation = useShellStore((s) => s.rest?.task_operation);
  const task = taskFor(operation, request ? { kind: "checkout_purpose", afterId: request.afterId, path: checkout.path } : null, localDeviceId(useShellStore.getState().rest));
  const refused = useErrorSince(request?.at ?? null, ["checkout_purpose.", "task_operation."]);
  const working = request !== null && refused === null && (task === null || task.phase === "working");
  const [saved, setSaved] = useState<string | null>(null);
  useEffect(() => {
    if (task?.phase === "ready" && request) setSaved(request.text);
  }, [task, request]);
  const send = (value: string) => {
    setSaved(null);
    setRequest({ afterId: actions.taskIdNow(), at: Date.now(), text: value });
    actions.setPurpose(checkout.id, value);
  };
  const failure = task?.phase === "failed" ? (task.message ?? t("workspace.purposeFailed")) : refused ? coreErrorText(refused, t, "workspace.purposeFailed") : null;
  const label = checkout.branch ?? checkout.label;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-purpose-dialog={checkout.id}>
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            if (!working) send(normalizePurpose(text).trim());
          }}
        >
          <DialogIntro title={t("workspace.purposeTitle", { name: label })} detail={checkout.path} />
          <DialogBody className="space-y-sm">
            <Input
              value={text}
              disabled={working}
              aria-label={t("workspace.purpose")}
              placeholder={checkout.purpose && !written ? checkout.purpose.text : t("workspace.purposePlaceholder")}
              onChange={(event) => {
                setSaved(null);
                setText(normalizePurpose(event.target.value));
              }}
              data-purpose-field="true"
            />
            <p className={`text-caption ${purposeIsLong(text) ? "text-warning" : "text-muted-foreground"}`} data-purpose-count="true">
              {purposeCountLabel(text, t)}
              {purposeIsLong(text) ? ` · ${t("workspace.purposeLong")}` : ""}
            </p>
            <p className="text-caption text-muted-foreground" data-purpose-scope="true">
              {purposeScope(deviceLabel, checkout.branch ?? null, t)}
            </p>
            {failure ? (
              <Note tone="error" data-purpose-error="true" data-core-error={refused?.kind}>
                {t("workspace.purposeFailure", { message: failure })}
              </Note>
            ) : null}
            {saved !== null ? (
              <Status tone="ok" data-purpose-saved="true">
                {saved ? t("workspace.saved") : t("workspace.cleared")}
              </Status>
            ) : null}
            {working ? <Status tone="pending">{t("workspace.saving")}</Status> : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={onClose}>{saved !== null ? t("workspace.done") : t("common.cancel")}</Button>
            <Button variant="secondary" disabled={working || (!written && !text)} onClick={() => send("")} data-purpose-clear="true">
              {t("workspace.clear")}
            </Button>
            <Button type="submit" disabled={working} data-purpose-save="true">
              {working ? t("workspace.saving") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteWorktreeDialog({ actions, deviceId, checkout, listed, onClose }: { actions: Actions; deviceId: string; checkout: Checkout; listed: boolean; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const row = checkout.worktree;
  const gate = row?.deletion_gate;
  const paneCount = checkout.tabs.reduce((count, tab) => count + tab.panes.length, 0);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const discardKey = discardConfirmationKey(checkout);
  const [discardSelection, setDiscardSelection] = useState({ key: discardKey, accepted: false });
  // Every observed facts transition retires the previous consent, including
  // a return to the same names after a lock or unavailable scan is resolved.
  if (discardSelection.key !== discardKey) setDiscardSelection({ key: discardKey, accepted: false });
  const discard = discardSelection.key === discardKey && discardSelection.accepted;
  const [request, setRequest] = useState<{ afterId: number; at: number } | null>(null);
  const current = useShellStore((s) => s.rest?.worktree_removal);
  const removal = request ? removalFor(current, deviceId, checkout.path, request.afterId, localDeviceId(useShellStore.getState().rest)) : null;
  const refused = useErrorSince(request?.at ?? null, ["worktree.remove"]);
  const inFlight = request !== null && refused === null && (removal === null || removal.phase === "checking" || removal.phase === "closing" || removal.phase === "removing");
  const finished = removal?.phase === "finished" ? removal : null;
  // A refusal or a failed removal keeps the worktree, so the choices come
  // back under the reason and Delete tries again on the row as it is now.
  const failure = refused ? coreErrorText(refused, t, "workspace.deleteFailed") : removal?.phase === "failed" ? (removal.message ?? t("workspace.deleteFailed")) : null;
  const choosing = !!row && !!gate && !gate.blocked_reason && !inFlight && !finished;
  const branch = row?.branch ?? checkout.branch;
  const needsDiscard = !!gate?.discard_label;
  const inside = checkout.tabs.flatMap((tab) => tab.panes.map((pane) => pane.id));
  const subtree = useOutsideSubtree(actions, inside, listed);
  const [withOutside, setWithOutside] = useState(false);
  const closingOutside = inFlight && withOutside && subtree !== null && removal?.phase === "closing";
  const confirm = (outside: boolean) => {
    const afterId = useShellStore.getState().rest?.worktree_removal?.id ?? 0;
    setWithOutside(outside);
    setRequest({ afterId, at: Date.now() });
    useUiStore.getState().setWatchedRemoval({ deviceId, path: checkout.path, afterId });
    actions.removeWorktree(deviceId, checkout.path, deleteBranch && !!gate?.can_delete_branch, discard && needsDiscard, outside ? (subtree?.ids ?? []) : [], row?.ignored_repositories ?? []);
  };
  const hide = () => {
    // A removal the operator stopped watching still reports its end through
    // the notice; one that already ended needs nothing more.
    if (!inFlight) useUiStore.getState().setWatchedRemoval(null);
    onClose();
  };
  return (
    <AlertDialog open onOpenChange={(next) => { if (!next) hide(); }}>
      <AlertDialogContent data-delete-worktree={checkout.id}>
        <AlertDialogHeader>
          <AlertDialogTitle className="break-words">{t("workspace.deleteTitle", { name: branch ?? checkout.label })}</AlertDialogTitle>
          <AlertDialogDescription className="break-all font-mono text-caption text-muted-foreground">{checkout.path}</AlertDialogDescription>
        </AlertDialogHeader>
        {!row || !gate ? (
          <Status tone="pending" data-delete-reading="true">
            {t("workspace.readingGit")}
          </Status>
        ) : null}
        {gate?.blocked_reason ? (
          <Note tone="warn" data-delete-blocked="true">
            {gate.blocked_reason}
          </Note>
        ) : null}
        {failure ? (
          <Note tone="error" data-delete-result="failed" data-core-error={refused?.kind}>
            {failure}
          </Note>
        ) : null}
        {choosing && gate ? (
          <>
            <div className="flex min-w-0 flex-col gap-xs" data-delete-consequences="true">
              <p className="break-words text-body text-subtle-foreground">{factsLine(deletionFacts(checkout, paneCount, t))}</p>
              {gate.warnings.length > 0 ? (
                <ul className="flex flex-wrap gap-xs" aria-label={t("workspace.warnings")}>
                  {gate.warnings.map((warning) => (
                    <li key={warning}>
                      <Badge variant="secondary" data-delete-warning={warning}>
                        {warning}
                      </Badge>
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>
            {branch ? (
              <label className={`flex items-start gap-xs text-body ${gate.can_delete_branch ? "text-foreground" : "text-muted-foreground"}`}>
                <Checkbox checked={deleteBranch && gate.can_delete_branch} disabled={!gate.can_delete_branch} onCheckedChange={(checked) => setDeleteBranch(checked === true)} data-delete-branch="true" className="mt-xxs" />
                <span className="min-w-0 break-words">
                  {t("workspace.deleteBranch", { branch })}
                  {!gate.can_delete_branch ? <span> ({t("workspace.branchNotOffered")})</span> : null}
                  {gate.can_delete_branch && gate.branch_warning ? (
                    <span className="block text-caption text-subtle-foreground" data-delete-branch-warning="true">
                      {gate.branch_warning}
                    </span>
                  ) : null}
                </span>
              </label>
            ) : null}
            {gate.discard_label ? (
              <label className="flex items-start gap-xs text-body text-foreground">
                <Checkbox checked={discard} onCheckedChange={(checked) => setDiscardSelection({ key: discardKey, accepted: checked === true })} data-delete-discard="true" className="mt-xxs" />
                <span className="min-w-0 break-words">
                  {gate.discard_label}
                  <span className="block text-caption text-muted-foreground">{t("workspace.discardRequired")}</span>
                </span>
              </label>
            ) : null}
          </>
        ) : null}
        {subtree && (choosing || closingOutside) ? <OutsideAgents actions={actions} subtree={subtree} what="worktree" targetDevice={deviceId} /> : null}
        {inFlight ? (
          <Status tone="pending" data-delete-phase={closingOutside ? "closing_outside" : (removal?.phase ?? "requested")}>
            {removal === null || removal.phase === "checking"
              ? t("workspace.recheckingGit")
              : removal.phase === "removing"
              ? t("workspace.removingFolder")
              : closingOutside
                ? t("workspace.closingOutside", { count: subtree.ids.length, panes: t("workspace.panes", { count: paneCount }) })
                : t("workspace.closingPanes", { count: paneCount })}
          </Status>
        ) : null}
        {finished ? (
          <Note tone="ok" data-delete-result="finished">
            {finished.message ?? t("workspace.worktreeRemoved")}
          </Note>
        ) : null}
        <AlertDialogFooter>
          <Button variant="secondary" onClick={hide} data-delete-cancel="true">
            {finished ? t("common.close") : inFlight ? t("workspace.hide") : t("workspace.keepWorktree")}
          </Button>
          {/* Plain Button, not AlertDialogAction: it has to stay open through
              the async removal instead of closing on the first click. */}
          {choosing && gate && subtree ? (
            <>
              <Button variant="secondary" disabled={needsDiscard && !discard} onClick={() => confirm(false)} data-delete-confirm="only">
                {t("workspace.deleteOnly")}
              </Button>
              <Button variant="destructive" disabled={(needsDiscard && !discard) || subtree.unknown} onClick={() => confirm(true)} data-delete-confirm="with-outside">
                {t("workspace.closeAgentsDelete", { count: subtree.ids.length })}
              </Button>
            </>
          ) : choosing && gate ? (
            <Button variant="destructive" disabled={needsDiscard && !discard} onClick={() => confirm(false)} data-delete-confirm="true">
              {gate.button_label || t("workspace.deleteWorktree")}
            </Button>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * The answers that arrive after their dialog is gone: the chosen agent's start
 * in a created pane, and a removal the operator hid while it ran. Only the
 * request this page made is reported, by its id or its checkout.
 */
export function WorkspaceNotices({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const watchedTask = useUiStore((s) => s.watchedTask);
  const watchedRemoval = useUiStore((s) => s.watchedRemoval);
  const operation = useShellStore((s) => s.rest?.task_operation);
  const removal = useShellStore((s) => s.rest?.worktree_removal);
  const dialogOpen = useUiStore((s) => s.workspaceDialog?.kind === "delete_worktree");
  const task = operation && operation.id === watchedTask ? operation : null;

  useEffect(() => {
    if (!watchedRemoval || dialogOpen) return;
    const answer = removalFor(removal, watchedRemoval.deviceId, watchedRemoval.path, watchedRemoval.afterId, localDeviceId(useShellStore.getState().rest));
    if (!answer || (answer.phase !== "finished" && answer.phase !== "failed")) return;
    useUiStore.getState().setNotice({ text: answer.message ?? (answer.phase === "finished" ? translate("workspace.worktreeRemoved") : translate("workspace.deleteFailed")), refreshable: false });
    useUiStore.getState().setWatchedRemoval(null);
  }, [watchedRemoval, removal, dialogOpen]);

  useEffect(() => {
    if (watchedTask !== null && operation && operation.id !== watchedTask) useUiStore.getState().setWatchedTask(null);
  }, [watchedTask, operation]);

  // The created worktree's pane is where the operator goes next (B13). The
  // core selects it when the creation lands, but only a focus request is
  // tracked until Herdr confirms it, so the move is made explicit once the
  // pane is listed: a late focus event for the pane left behind cannot undo it.
  // It is opened as an agent row opens its pane, so a worktree started from
  // an Overview (Start, New agent) brings its Workspace to the front.
  const focusWhenListed = useUiStore((s) => s.focusWhenListed);
  const listed = useShellStore((s) =>
    focusWhenListed !== null &&
    [...(s.rest?.navigator?.workspaces ?? []), ...(s.rest?.status?.remote ?? []).flatMap((row) => row.session?.workspaces ?? [])].some((workspace) =>
      workspace.checkouts.some((checkout) => checkout.tabs.some((tab) => tab.panes.some((pane) => pane.id === focusWhenListed))),
    ),
  );
  useEffect(() => {
    if (!focusWhenListed || !listed) return;
    useUiStore.getState().setFocusWhenListed(null);
    actions.openAgent(focusWhenListed);
  }, [focusWhenListed, listed, actions]);

  if (!task || !task.agent_phase || task.agent_phase === "started") return null;
  const dismiss = () => useUiStore.getState().setWatchedTask(null);
  const failure = agentFailure(task.agent_reason, task.agent_line);
  return (
    <div role="status" data-task-agent={task.agent_phase} data-core-error={"code" in failure ? failure.code : undefined} className="flex flex-wrap items-center gap-md border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground">
      <span className="min-w-0 flex-1 break-words">
        {task.agent_phase === "starting"
          ? t("workspace.startingAgent", { kind: task.agent_kind ?? t("common.agent") })
          : `${endedSentence("key" in failure ? t(failure.key) : failure.message)} ${task.kind === "worktree_create" ? t("workspace.worktreePaneKept") : t("workspace.paneKept")}`}
        {task.agent_phase !== "starting" && "line" in failure && failure.line ? <QuotedLine line={failure.line} /> : null}
      </span>
      {task.agent_phase === "failed" ? (
        <button type="button" className="text-foreground underline" onClick={() => actions.retryTaskAgent(task.id)} data-task-agent-retry="true">
          {t("workspace.retryAgent")}
        </button>
      ) : null}
      {task.pane_id ? (
        <button type="button" className="text-foreground underline" onClick={() => actions.focusPane(task.pane_id as string)}>
          {t("workspace.showPane")}
        </button>
      ) : null}
      <button type="button" className="text-muted-foreground" aria-label={t("workspace.dismiss")} onClick={dismiss}>
        ×
      </button>
    </div>
  );
}
