import { useEffect, useRef, useState } from "react";
import { useInterfaceTranslation } from "./i18n/client";
import { startAnswer, startRequestId, START_ANSWER_TIMEOUT_MS } from "./startAnswer";
import { coreErrorKey } from "./coreError";
import { startReasonText } from "./startReason";
import { agentFailure } from "./startDraft";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { resumeProvider } from "./linkPanel";
import type { Actions } from "./actions";
import type { TFunction } from "i18next";

/** The sentence for a resume the core refused, by its kind. */
function startRefusalKey(kind: string) {
  return coreErrorKey(kind) ?? "workspace.agentNotStarted";
}

/** A resume in flight: the request it sent, then the core's refusal if it came. */
type Resume = { request: string | null; failure: string | null; uncertain: boolean };
type Send = (request: string, confirmOnly: boolean) => void;

/** The session a resume continues, and the file its agent reads it from when Hide knows it. */
export type ResumedSession = { device_id: string; agent: string; session_id: string; path: string | null };

/**
 * The one resume request every session line sends (B12): the session's own
 * agent, id and file, in `checkout` on its own device; null when its agent
 * cannot resume or no checkout takes it.
 */
export function resumeRequest(startAgent: Actions["startAgent"], session: ResumedSession, checkout: { path: string } | null, node: string): Send | null {
  const provider = resumeProvider(session.agent);
  if (!checkout || !provider) return null;
  return (requestId, confirmOnly) => startAgent({ target: { checkoutPath: checkout.path }, deviceId: session.device_id === node ? undefined : session.device_id, provider, resumeSessionId: session.session_id, resumeSessionPath: session.path ?? undefined, requestId, confirmOnly });
}

/** What a resume that did not open says, in the words every session line uses; null while none failed. */
export function resumeFailureText(resume: Resume, t: TFunction<"translation">): string | null {
  if (resume.failure === null) return null;
  if (resume.uncertain) return resume.failure;
  return resume.failure ? t("links.resumeFailed", { reason: resume.failure }) : t("links.resumeFailedPlain");
}

/**
 * Follows one resume to its answer (B12, B14): a tab that opened moves the
 * screen to its pane, or goes where `opened` takes it (Desk opens it beside
 * itself, frame K), and a refusal or a failure stays on the line with Retry.
 */
export function useSessionResume(opened?: (paneId: string) => void): { resume: Resume; start: (send: Send) => void } {
  const { t } = useInterfaceTranslation();
  const [resume, setResume] = useState<Resume>({ request: null, failure: null, uncertain: false });
  const sending = useRef(false);
  const goTo = useRef(opened);
  goTo.current = opened;
  const confirming = useRef(false);
  // A retry keeps the original identity and destination, including after a timeout.
  const intent = useRef<{ id: string; send: Send } | null>(null);
  const before = useRef<{ operation: unknown; error: unknown } | null>(null);
  const request = resume.request;
  const operation = useShellStore((s) => s.rest?.task_operation);
  const error = useShellStore((s) => s.rest?.status?.last_error);
  useEffect(() => {
    if (!request) return;
    const freshError = error?.request_id === request && before.current?.error !== error;
    const freshOperation = operation?.request_id === request && before.current?.operation !== operation;
    if (!freshError && !freshOperation) return;
    const rest = useShellStore.getState().rest;
    // An unrelated error cannot make an old receipt this attempt's answer.
    const answer = startAnswer(!freshError && rest ? { ...rest, status: { ...rest.status, last_error: null } } : rest, request);
    if (answer.phase === "ready") {
      if (answer.agentPhase === "starting") return;
      sending.current = false;
      if (answer.agentPhase === "unknown") {
        setResume({ request, failure: null, uncertain: true });
        return;
      }
      const ui = useUiStore.getState();
      if (answer.agentPhase !== "failed") {
        intent.current = null;
        if (answer.agentPhase) ui.setWatchedTask(answer.taskId);
        if (answer.paneId) {
          if (goTo.current) goTo.current(answer.paneId);
          else ui.setFocusWhenListed(answer.paneId);
        }
      }
      const failure = agentFailure(answer.agentReason, null);
      const reason = "key" in failure ? t(failure.key) : failure.message;
      setResume({ request: null, failure: answer.agentPhase === "failed" ? reason : null, uncertain: false });
    } else if (answer.phase === "refused" || answer.phase === "failed") {
      sending.current = false;
      // Refusing an inspection says nothing about the original launch.
      const uncertain = confirming.current && answer.phase === "refused";
      const failure = answer.phase === "refused" ? t(startRefusalKey(answer.kind)) : answer.message ? startReasonText(answer.message, t) : "";
      setResume({ request: uncertain ? request : null, failure, uncertain });
    }
  }, [request, operation, error, t]);
  useEffect(() => {
    if (!request || resume.uncertain) return undefined;
    const timer = window.setTimeout(() => { sending.current = false; setResume({ request, failure: null, uncertain: true }); }, START_ANSWER_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [request, resume.uncertain]);
  return {
    resume,
    start: (send) => {
      if (sending.current) return;
      sending.current = true;
      confirming.current = resume.uncertain;
      const original = intent.current ?? { id: startRequestId(), send };
      intent.current = original;
      const rest = useShellStore.getState().rest;
      before.current = { operation: rest?.task_operation, error: rest?.status?.last_error };
      setResume({ request: original.id, failure: null, uncertain: false });
      original.send(original.id, resume.uncertain);
    },
  };
}
