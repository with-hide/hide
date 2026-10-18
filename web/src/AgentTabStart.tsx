import { BriefcaseBusinessIcon, TerminalIcon, XIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import type { AgentStartTarget } from "./agentLayout";
import { rememberedSelection, modelToSend, selectKind, type AgentKind } from "./agentPicker";
import { AgentPicker } from "./components/agent-picker";
import { Button } from "./components/ui/button";
import { deviceConnected } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { startRequestId } from "./startAnswer";
import { inlineStartKey, startFailure, useInlineStarts, type InlineStartDraft } from "./startDraft";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { catalogWorkspaces } from "./snapshot";
import { failedReceiptMoved, inlineReceipts, inlineStartAnswer, receiptTarget } from "./inlineStart";
import type { ViewWorkspace } from "./viewLayout";

const ANSWER_TIMEOUT_MS = 90_000;

/** One existing snapshot subscription also follows starts whose group is hidden. */
export function InlineStartHost() {
  const drafts = useInlineStarts((state) => state.drafts);
  useEffect(() => {
    const settle = () => {
      const store = useInlineStarts.getState();
      const rest = useShellStore.getState().rest;
      const receipts = inlineReceipts(rest);
      store.retainRetired(new Set(receipts.map(({ receipt }) => receipt.request_id)));
      const movedFailures = new Set(receipts.filter(({ device, path, receipt }) => useShellStore.getState().connection === "live" && deviceConnected(rest, device) && failedReceiptMoved(rest, device, path, receipt)).map(({ receipt }) => receipt.request_id));
      for (const [request, close] of Object.entries(store.closing)) {
        if (rest?.status?.last_error?.request_id === request) { store.settleClose(request, false); continue; }
        const groups = catalogWorkspaces(rest).find((workspace) => workspace.device_id === close.device && workspace.checkouts.some((checkout) => checkout.path === close.path))?.checkouts.find((checkout) => checkout.path === close.path)?.agent_groups;
        if (groups && !groups.drafts.includes(close.tab)) store.settleClose(request, true);
      }
      for (const { device, path, receipt } of receipts) {
        if (useInlineStarts.getState().retired.has(receipt.request_id) || movedFailures.has(receipt.request_id)) continue;
        if (receipt.state === "ready" && (!receipt.agent_phase || receipt.agent_phase === "started")) continue;
        const target = receiptTarget(receipt);
        const key = inlineStartKey(device, path, target);
        if (store.retarget(device, path, receipt.request_id, target) || useInlineStarts.getState().drafts[key]) continue;
        const selection = selectKind((receipt.agent_kind ?? "terminal") as AgentKind, rest?.ui_state?.agent_start);
        store.put(key, { text: "", selection, request: { id: receipt.request_id, sentAt: 0, provider: selection.kind, model: selection.model, prompt: null, target, restored: true }, phase: "unknown", taskId: null, failure: null });
      }
      for (const [key, draft] of Object.entries(useInlineStarts.getState().drafts)) {
        if (!draft.request) continue;
        const receipt = receipts.find((row) => row.receipt.request_id === draft.request!.id)?.receipt;
        const answer = inlineStartAnswer(rest, draft.request.id, receipt);
        if (answer.phase === "pending" || ((draft.phase === "pending" || draft.phase === "unknown") && draft.request.beforeAnswer === JSON.stringify(answer))) continue;
        if (answer.phase === "ready" && answer.agentPhase) useUiStore.getState().setWatchedTask(answer.taskId);
        if (answer.phase === "ready" && (answer.agentPhase === "starting" || answer.agentPhase === "unknown")) {
          const phase = answer.agentPhase === "unknown" || Date.now() - draft.request.sentAt >= ANSWER_TIMEOUT_MS ? "unknown" : "pending";
          if (draft.phase !== phase || draft.taskId !== answer.taskId) store.put(key, { ...draft, phase, taskId: answer.taskId });
        } else if (answer.phase === "ready" && answer.agentPhase !== "failed") {
          store.drop(key);
        } else if (answer.phase === "ready") {
          if (draft.phase !== "agent_failed") store.put(key, { ...draft, phase: "agent_failed", taskId: answer.taskId, failure: startFailure(answer.agentMessage) });
        } else if (draft.phase !== "failed") {
          store.put(key, { ...draft, phase: "failed", request: answer.phase === "refused" ? null : draft.request, failure: startFailure(answer.message) });
        }
      }
      for (const requestId of movedFailures) store.retireFailed(requestId);
    };
    settle();
    const unsubscribe = useShellStore.subscribe((state, previous) => {
      // Terminal output does not rescan forms. Only catalog/receipt changes can settle a start.
      if (state.connection !== previous.connection || state.rest?.workspace_view?.agent_layout !== previous.rest?.workspace_view?.agent_layout || state.rest?.navigator?.workspaces !== previous.rest?.navigator?.workspaces || state.rest?.status?.remote !== previous.rest?.status?.remote || state.rest?.task_operation !== previous.rest?.task_operation || state.rest?.status?.last_error !== previous.rest?.status?.last_error) settle();
    });
    return unsubscribe;
  }, []);
  useEffect(() => {
    const pending = Object.values(drafts).filter((draft) => draft.phase === "pending" && draft.request);
    const delay = pending.length ? Math.max(0, Math.min(...pending.map((draft) => draft.request!.sentAt + ANSWER_TIMEOUT_MS - Date.now()))) : null;
    const timer = delay === null ? null : window.setTimeout(() => {
      for (const [key, draft] of Object.entries(useInlineStarts.getState().drafts)) {
        if (draft.phase === "pending" && draft.request && Date.now() - draft.request.sentAt >= ANSWER_TIMEOUT_MS) useInlineStarts.getState().put(key, { ...draft, phase: "unknown" });
      }
    }, delay);
    return () => { if (timer !== null) window.clearTimeout(timer); };
  }, [drafts]);
  return null;
}

export function AgentTabStart({ workspace, target, groupName, actions, onCancel }: { workspace: ViewWorkspace; target: AgentStartTarget; groupName: string; actions: Actions; onCancel?: () => void }) {
  const { t } = useInterfaceTranslation();
  const key = inlineStartKey(workspace.device_id, workspace.path, target);
  const held = useInlineStarts((state) => state.drafts[key]);
  const [initial] = useState<InlineStartDraft>(() => ({ text: "", selection: rememberedSelection(useShellStore.getState().rest?.ui_state?.agent_start), request: null, phase: "editing", taskId: null, failure: null }));
  const draft = held ?? initial;
  const [full, setFull] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const sent = useRef(false);
  const connected = useShellStore((state) => state.connection === "live" && deviceConnected(state.rest, workspace.device_id));
  const busy = draft.phase === "pending" || draft.phase === "unknown";
  const locked = draft.request !== null;
  const terminalIntent = draft.request?.provider === "terminal";
  const terminalRetry = terminalIntent && draft.phase === "failed";
  const editingRestored = draft.request?.restored && !terminalIntent && (draft.phase === "failed" || draft.phase === "agent_failed");
  const update = (next: InlineStartDraft) => { const ok = useInlineStarts.getState().put(key, next); setFull(!ok); return ok; };
  useEffect(() => {
    if (held?.request) sent.current = true;
    else if (!held && sent.current) onCancel?.();
  }, [held, onCancel]);
  useEffect(() => {
    if (!onCancel || locked) return;
    field.current?.focus();
    return useUiStore.getState().pushEscape(onCancel);
  }, [onCancel, locked]);
  const start = (terminal: boolean, confirmOnly = false) => {
    const latest = useInlineStarts.getState().drafts[key] ?? draft;
    if (!connected || latest.phase === "pending" || (latest.phase === "unknown" && !confirmOnly)) return;
    if (!confirmOnly && latest.request && (latest.request.provider === "terminal") !== terminal) return;
    const beforeAnswer = latest.request ? JSON.stringify(inlineStartAnswer(useShellStore.getState().rest, latest.request.id, inlineReceipts(useShellStore.getState().rest).find((row) => row.receipt.request_id === latest.request!.id)?.receipt)) : undefined;
    if (latest.phase === "agent_failed" && latest.taskId !== null && !latest.request?.restored) {
      if (update({ ...latest, request: latest.request ? { ...latest.request, sentAt: Date.now(), beforeAnswer } : null, phase: "pending", failure: null })) actions.retryTaskAgent(latest.taskId);
      return;
    }
    const request = latest.request ? { ...latest.request, sentAt: Date.now(), beforeAnswer, ...(latest.request.restored && !confirmOnly ? { prompt: latest.request.provider === "terminal" ? null : latest.text.trim() || null, model: latest.request.provider === "terminal" ? null : modelToSend(latest.selection), restored: false } : {}) } : { id: startRequestId(), sentAt: Date.now(), provider: terminal ? "terminal" : latest.selection.kind, model: terminal ? null : modelToSend(latest.selection), prompt: terminal ? null : latest.text.trim() || null, target };
    if (!update({ ...latest, request, phase: "pending", failure: null })) return;
    actions.startAgent({ target: { checkoutPath: workspace.path }, deviceId: workspace.device_id, provider: request.provider, model: request.model, prompt: request.prompt, requestId: request.id, tabTarget: request.target, confirmOnly });
  };
  const failure = draft.failure ? "key" in draft.failure ? t(draft.failure.key) : draft.failure.message : null;
  return <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-lg" data-agent-tab-start={target.tab_id ?? "empty"}>
    <div className="flex w-full max-w-(--size-worktree-dialog) flex-col gap-md">
      <div className="flex items-start justify-between gap-sm"><div className="min-w-0"><p className="flex items-center gap-xs text-caption text-muted-foreground" title={groupName}><BriefcaseBusinessIcon className="size-(--size-icon-sm) shrink-0" /><span className="truncate">{groupName}</span></p><h2 className="mt-xs text-title font-semibold">{t(target.add_pane ? "groups.addAgent" : target.tab_id ? "groups.draftTitle" : "groups.firstTitle")}</h2><p className="mt-xs text-caption text-muted-foreground">{t("groups.startHint")}</p></div>{onCancel && !locked ? <Button variant="ghost" size="icon-sm" aria-label={t("common.cancel")} onClick={onCancel}><XIcon /></Button> : null}</div>
      <div className="overflow-hidden rounded-lg border border-input bg-background focus-within:ring-1 focus-within:ring-ring">
        <textarea ref={field} rows={3} value={draft.text} readOnly={locked && !editingRestored} aria-label={t("shell.firstInstruction")} placeholder={t("groups.prompt")} className="block w-full resize-none bg-transparent p-md text-body text-foreground outline-none placeholder:text-muted-foreground" onChange={(event) => update({ ...draft, text: event.target.value, phase: editingRestored ? "unknown" : "editing", failure: null })} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && event.keyCode !== 229) { event.preventDefault(); if (!busy && !terminalIntent) start(false); } }} />
        <div className="flex flex-wrap items-center justify-between gap-sm px-sm pb-sm"><AgentPicker actions={actions} value={draft.selection} onChange={(selection) => update({ ...draft, selection, phase: "editing", failure: null })} disabled={locked} /><Button disabled={!connected || busy || terminalIntent || draft.selection.kind === "terminal"} onClick={() => start(false)} data-group-start-agent="true">{t(busy ? "shell.starting" : "common.start")}</Button></div>
      </div>
      <Button variant="secondary" className="w-full" disabled={!connected || (locked && !terminalRetry)} onClick={() => start(true)} data-group-start-terminal="true"><TerminalIcon />{t(terminalRetry ? "groups.retryTerminal" : "groups.terminal")}</Button>
      <p className="truncate text-center text-caption text-muted-foreground" title={workspace.path}>{workspace.path}</p>
      {!connected ? <p role="status" className="text-caption text-muted-foreground">{t("groups.notConnected")}</p> : null}
      {failure ? <p role="alert" className="whitespace-pre-line text-caption text-destructive">{failure}</p> : null}
      {draft.phase === "unknown" ? <div className="flex flex-col items-start gap-xs"><p role="status" className="text-caption text-muted-foreground">{t("groups.unknown")}</p><Button variant="outline" disabled={!connected} onClick={() => start(false, true)}>{t("groups.confirm")}</Button></div> : null}
      {full ? <p role="alert" className="text-caption text-destructive">{t("groups.draftLimit")}</p> : null}
    </div>
  </div>;
}
