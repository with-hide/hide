// Work that starts from a pull request on the PRs tab (PRD
// overview-lenses-prs): linking it to an issue, which for a GitHub issue
// writes `Closes #N` into its body, and handing it to an agent on its branch.
// Each sends the one core event that owns the change after the operator's one
// confirmation, and reads the core's own answer for it (`pr_work`,
// `task_operation`); a dialog never decides that its request succeeded.

import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { rememberedSelection, modelToSend, type AgentSelection } from "./agentPicker";
import { AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Input } from "./components/ui/input";
import { Kbd } from "./components/ui/kbd";
import { useInterfaceTranslation } from "./i18n/client";
import type { TFunction } from "i18next";
import { Note, Status } from "./components/settings-rows";
import { AgentField, Field, submitOnCommandEnter, TextArea } from "./IssueDialogs";
import { delegatePrompt } from "./prDelegate";
import { localDeviceId, type PrFeedback, type PrLink, type PullRequest, type Task, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { taskFor } from "./workspaceManage";
import { coreErrorKey, coreErrorText, type CoreError } from "./coreError";
import { issueFailureText } from "./issueFailure";
import { useErrorSince } from "./errorSince";
import { fieldLabel } from "./shortcutLabels";

/** A request this dialog sent, by the id the core answers under. */
type Sent = { id: string; at: number };

function requestId(kind: string, number: number): string {
  return `${kind}-${number}-${Date.now()}`;
}

/**
 * The core's answer to this dialog's own link request and a refusal since it
 * was sent; a link that reached `ready` closes the dialog.
 */
function useLinkAnswer(sent: Sent | null, onClose: () => void): { answer: PrLink | null; refused: CoreError | null; working: boolean; failed: boolean } {
  const link = useShellStore((s) => s.rest?.pr_work?.link ?? null);
  const refused = useErrorSince(sent?.at ?? null, ["pr_link."]);
  const answer = sent && link?.request_id === sent.id ? link : null;
  useEffect(() => {
    if (answer?.phase === "ready") onClose();
  }, [answer?.phase, onClose]);
  return {
    answer,
    refused,
    working: sent !== null && refused === null && (answer === null || answer.phase === "working"),
    failed: refused !== null || answer?.phase === "failed",
  };
}

/** The core's read of a pull request's body, checks and reviews under `readId`, asked for once per id. */
function useFeedbackRead(actions: Actions, readId: string, workspaceId: string, number: number): PrFeedback | null {
  const read = useShellStore((s) => (s.rest?.pr_work?.feedback?.request_id === readId ? s.rest.pr_work.feedback : null));
  useEffect(() => {
    actions.readPullRequestFeedback(readId, workspaceId, number);
  }, [actions, readId, workspaceId, number]);
  return read;
}

/** The number `Closes #N` names for a GitHub issue of this repository (`github:owner/repo#12`). */
function issueNumber(task: Task): string | null {
  return task.key.match(/#(\d+)$/)?.[1] ?? null;
}

/** What failed, in the operator's words; why is the core's one line, the rest is in the log (design 13). */
function failureHeadline(answer: PrLink, t: TFunction<"translation">): string {
  if (answer.step === "body") return answer.created ? t("prWork.createdBodyFailed", { issue: answer.issue_id ?? "" }) : t("prWork.linkedBodyFailed");
  if (answer.step === "create") return t("prWork.createFailed");
  return t("prWork.linkFailed");
}

/** The retry names what it does again: only the body once the issue is made and linked (B10, B13). */
function retryLabel(answer: PrLink | null, created: string, t: TFunction<"translation">): string {
  if (answer?.step === "body") return t("prWork.retryBody");
  if (answer?.step === "create") return created;
  return t("prWork.retryLink");
}

function LinkFailure({ answer, refused }: { answer: PrLink | null; refused: CoreError | null }) {
  const { t } = useInterfaceTranslation();
  const headline = answer ? failureHeadline(answer, t) : t("prWork.linkFailed");
  // A refusal under the headline says only what this build words; the headline already says it failed.
  const known = refused ? coreErrorKey(refused.kind) : null;
  const reason = issueFailureText(answer?.failure, t) ?? (known ? t(known) : null);
  return (
    <Note tone="error" data-pr-link-failure={answer?.step ?? "refused"} data-core-error={answer?.failure?.code ?? refused?.kind}>
      <span className="block">{headline}</span>
      {reason ? <span className="line-clamp-2 block break-words text-caption text-muted-foreground">{reason}</span> : null}
    </Note>
  );
}

/**
 * An issue of the project's source chosen for a pull request (B10, B11,
 * B14). A GitHub issue asks once, `Add "Closes #M" to the body of PR #N`, with
 * Cancel linking the default; confirming leaves Hide's link and writes the body. A
 * Local issue is Hide's link alone and asks nothing, so the dialog appears only
 * if that link fails. A failed body write keeps the link and offers to write
 * the body again, which the core finds already written if it was.
 */
export function PrLinkDialog({ actions, workspace, pr, task, onClose }: { actions: Actions; workspace: Workspace; pr: PullRequest; task: Task; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const local = task.source === "local";
  const [sent, setSent] = useState<Sent | null>(null);
  const { answer, refused, working, failed } = useLinkAnswer(sent, onClose);
  const send = (key: string) => {
    const id = requestId("pr-link", pr.number);
    setSent({ id, at: Date.now() });
    actions.linkPullRequestIssue(id, workspace.id, pr.number, { key });
  };
  // A Local issue asks nothing: the link goes out as the dialog opens.
  const autoSent = useRef(false);
  useEffect(() => {
    if (!local || autoSent.current) return;
    autoSent.current = true;
    send(task.key);
  });
  if (local && !failed) return null;
  const number = issueNumber(task);
  return (
    <AlertDialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <AlertDialogContent initialFocus="cancel" data-pr-link={pr.number} data-pr-link-issue={task.key}>
        <AlertDialogHeader>
          <AlertDialogTitle className="break-words">{t("prWork.linkTitle", { number: String(pr.number), issue: task.id ?? task.title })}</AlertDialogTitle>
          <AlertDialogDescription className="break-words" data-pr-link-confirm="true">
            {local ? t("prWork.confirmLocal", { issue: task.id ?? task.title }) : t("prWork.confirmGitHub", { number: String(pr.number), issueNumber: number ?? "" })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {failed ? <LinkFailure answer={answer} refused={refused} /> : null}
        {working ? <Status tone="pending">{t("prWork.writingBody")}</Status> : null}
        <AlertDialogFooter>
          <AlertDialogCancel data-pr-link-cancel="true">{t(failed ? "common.close" : working ? "issue.hide" : "prWork.stop")}</AlertDialogCancel>
          {!working ? (
            <Button onClick={() => send(answer?.issue_key ?? task.key)} data-pr-link-write={failed ? "retry" : "write"}>
              {failed ? retryLabel(answer, t("prWork.retryWrite"), t) : t("prWork.writeBody")}
            </Button>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * `Create new issue` for a pull request (B12, B13): the title and body start as
 * the pull request's own, and one confirmation makes the issue, then writes
 * `Closes #(new number)` into the body (a Local source's issue is only linked).
 * If the body write fails the issue stays made and linked, and the retry
 * writes only the body.
 */
export function PrNewIssueDialog({ actions, workspace, pr, onClose }: { actions: Actions; workspace: Workspace; pr: PullRequest; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const github = workspace.tasks?.source?.kind === "github";
  const [title, setTitle] = useState(pr.title);
  const [body, setBody] = useState("");
  const bodyEdited = useRef(false);
  const readId = useMemo(() => requestId("pr-body", pr.number), [pr.number]);
  const read = useFeedbackRead(actions, readId, workspace.id, pr.number);
  useEffect(() => {
    if (read?.phase === "ready" && !bodyEdited.current) setBody(read.body ?? "");
  }, [read?.phase, read?.body]);
  const [sent, setSent] = useState<Sent | null>(null);
  const { answer, refused, working, failed } = useLinkAnswer(sent, onClose);
  const submit = () => {
    if (working) return;
    const id = requestId("pr-link", pr.number);
    setSent({ id, at: Date.now() });
    // Once the issue exists, only its link and the body are asked for again.
    if (answer?.issue_key) actions.linkPullRequestIssue(id, workspace.id, pr.number, { key: answer.issue_key });
    else actions.linkPullRequestIssue(id, workspace.id, pr.number, { title: title.trim(), body });
  };
  const made = answer?.issue_key != null;
  const confirm = t(github ? "prWork.confirmCreateGitHub" : "prWork.confirmCreateLocal", { number: String(pr.number) });
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-pr-new-issue={pr.number}>
        <form className="flex min-h-0 flex-1 flex-col" onKeyDown={submitOnCommandEnter} onSubmit={(event) => { event.preventDefault(); submit(); }}>
          <DialogHeader>
            <DialogTitle>{t("prWork.newIssueTitle", { number: String(pr.number) })}</DialogTitle>
            <DialogDescription className="break-words" data-pr-new-issue-confirm="true">{confirm}</DialogDescription>
          </DialogHeader>
          <DialogBody className="space-y-sm">
            <Field label={t("issue.title")}>
              <Input value={title} autoFocus disabled={working || made} maxLength={256} onChange={(event) => setTitle(event.target.value.replace(/[\r\n]+/g, " "))} data-pr-new-issue-title="true" />
            </Field>
            <Field label={t("issue.content")} aside={read?.phase === "reading" ? <span className="text-muted-foreground">{t("prWork.readingBody")}</span> : null}>
              <TextArea
                value={body}
                rows={5}
                disabled={working || made}
                onChange={(event) => {
                  bodyEdited.current = true;
                  setBody(event.target.value);
                }}
                data-pr-new-issue-body="true"
              />
            </Field>
            {read?.phase === "failed" ? <Note tone="warn" data-pr-new-issue-body-failed="true">{t("prWork.bodyReadFailed")}</Note> : null}
            {failed ? <LinkFailure answer={answer} refused={refused} /> : null}
            {working ? <Status tone="pending">{t(answer?.step === "body" ? "prWork.writingBody" : "issue.creating")}</Status> : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={onClose} data-pr-new-issue-cancel="true">{t(failed ? "common.close" : working ? "issue.hide" : "prWork.cancelCreate")}</Button>
            {!working ? (
              <Button type="submit" disabled={!title.trim()} data-pr-new-issue-submit={failed ? "retry" : "create"}>
                {failed ? retryLabel(answer, t(github ? "prWork.retryCreateWrite" : "prWork.retryCreate"), t) : t(github ? "prWork.createWrite" : "prWork.createLink")}
                <Kbd>{fieldLabel("Enter")}</Kbd>
              </Button>
            ) : null}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * `▷ Assign` (B15, B16, B18): the Start dialog on the pull request's branch.
 * The worktree is the branch, shown and fixed; with no checkout of it here
 * the start makes the worktree of that branch first and says so. The first
 * prompt is what the core read of the failed checks and change requests,
 * editable; a failed read leaves it empty with why and `Read again`, and never
 * blocks the start. Started, the screen goes to the agent's pane.
 */
export function PrDelegateDialog({ actions, workspace, pr, intent, onClose }: { actions: Actions; workspace: Workspace; pr: PullRequest; intent?: "review"; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const branch = pr.head_branch ?? "";
  // The checkout the core holds this pull request for, which is what the start reuses.
  const checkout = workspace.checkouts.find((row) => row.exists && row.pull_request?.url === pr.url) ?? null;
  // A pull request is handed to an agent, never a bare terminal: the remembered kind, else Claude.
  const [agent, setAgent] = useState<AgentSelection>(() => rememberedSelection(useShellStore.getState().rest?.ui_state?.agent_start));
  // A review asks the agent to read the pull request and report to the operator, never to post to GitHub (B35).
  const reviewing = intent === "review";
  const [prompt, setPrompt] = useState(() => (reviewing ? t("work.card.reviewPrompt", { number: pr.number, branch, title: pr.title }) : ""));
  const promptEdited = useRef(reviewing);
  const [readId, setReadId] = useState(() => requestId("pr-feedback", pr.number));
  const read = useFeedbackRead(actions, readId, workspace.id, pr.number);
  useEffect(() => {
    if (read?.phase !== "ready" || promptEdited.current) return;
    setPrompt(delegatePrompt({ number: pr.number, title: pr.title, branch }, read, t));
  }, [read, pr.number, pr.title, branch, t]);
  // Whether the start makes the branch's worktree is fixed when it is asked
  // for: the worktree it makes joins the checkouts while its agent is still
  // starting, and reading `checkout` then would wait for the wrong task.
  const [request, setRequest] = useState<{ afterId: number; at: number; newWorktree: boolean } | null>(null);
  const newWorktree = request ? request.newWorktree : checkout === null;
  const operation = useShellStore((s) => s.rest?.task_operation);
  const started = taskFor(
    operation,
    request ? (request.newWorktree ? { kind: "worktree_create", afterId: request.afterId, repositoryRoot: workspace.path, branch } : { kind: "agent_start", afterId: request.afterId, repositoryRoot: workspace.path }) : null,
    localDeviceId(useShellStore.getState().rest),
  );
  const refused = useErrorSince(request?.at ?? null, ["pr_delegate.", "worktree.create", "task_operation.", "agent_start.", "overview.unknown_checkout"]);
  const working = request !== null && refused === null && (started === null || started.phase === "working");
  const failure = started?.phase === "failed" ? (started.failure ? coreErrorText({ kind: started.failure }, t, "prWork.startFailed") : t("prWork.startFailed")) : refused ? coreErrorText(refused, t, "prWork.startFailed") : null;
  useEffect(() => {
    if (started?.phase !== "ready") return;
    if (started.agent_phase) useUiStore.getState().setWatchedTask(started.id);
    if (started.pane_id) useUiStore.getState().setFocusWhenListed(started.pane_id);
    onClose();
  }, [started, onClose]);
  const submit = () => {
    if (working || !branch || agent.kind === "terminal") return;
    setRequest({ afterId: actions.taskIdNow(), at: Date.now(), newWorktree: checkout === null });
    actions.delegatePullRequest(workspace.id, pr.number, agent.kind, modelToSend(agent), prompt);
  };
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-pr-delegate={pr.number}>
        <form className="flex min-h-0 flex-1 flex-col" onKeyDown={submitOnCommandEnter} onSubmit={(event) => { event.preventDefault(); submit(); }}>
          <DialogHeader>
            <DialogTitle>{t("prWork.delegateTitle", { number: String(pr.number) })}</DialogTitle>
            <DialogDescription className="line-clamp-2 break-words">{pr.title}</DialogDescription>
          </DialogHeader>
          <DialogBody className="space-y-sm">
            <Field label={t("issue.worktreeBranch")}>
              <Input value={branch} mono disabled readOnly data-pr-delegate-branch="true" />
            </Field>
            {newWorktree ? (
              <Note tone="muted" data-pr-delegate-new-worktree="true">
                {t("prWork.newWorktree")}
              </Note>
            ) : null}
            <AgentField agent={agent} actions={actions} disabled={working} onChange={setAgent} />
            <Field label={t("issue.firstPrompt")} aside={reviewing ? undefined : <span className="text-muted-foreground">{t(read?.phase === "reading" || !read ? "prWork.readingFeedback" : "prWork.feedbackPrefilled")}</span>}>
              <TextArea
                value={prompt}
                rows={7}
                disabled={working}
                onChange={(event) => {
                  promptEdited.current = true;
                  setPrompt(event.target.value);
                }}
                data-pr-delegate-prompt="true"
              />
            </Field>
            {!reviewing && read?.phase === "failed" ? (
              <Note tone="warn" data-pr-delegate-read-failed="true">
                <span className="flex items-center gap-sm">
                  <span className="min-w-0 flex-1 break-words">{read.failure === "project_settings_repository_changed" ? t("prWork.feedbackReadFailed", { message: t("projectSettings.repositoryChanged") }) : t("githubReason.feedbackReadFailed")}</span>
                  <Button type="button" variant="secondary" size="sm" onClick={() => setReadId(requestId("pr-feedback", pr.number))} data-pr-delegate-reread="true">
                    {t("prWork.readAgain")}
                  </Button>
                </span>
              </Note>
            ) : null}
            {failure ? <Note tone="error" data-pr-delegate-error="true" data-core-error={refused?.kind}>{failure}</Note> : null}
            {working ? <Status tone="pending">{t(newWorktree ? "issue.creatingWorktree" : "prWork.startingAgent")}</Status> : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={onClose}>{t(working ? "issue.hide" : "common.cancel")}</Button>
            <Button type="submit" disabled={working || !branch} data-pr-delegate-submit="true">
              {t(working ? "issue.starting" : "common.start")}
              <Kbd>{fieldLabel("Enter")}</Kbd>
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
