// The phone's hand-over of a pull request to an agent (PRD mobile-companion-ui
// B44, D-13, doop final board F10): the branch, the agent kind and model the
// desktop remembers, and the first instruction the desktop card would start
// with (`prDelegate.ts`, called and never copied), editable. A hand-over made
// here starts the agent where the desktop would start it and leaves the
// desktop's screen and keyboard where they are. The prompt and the choice
// stay until an agent started, so a refusal or a lost connection never costs
// what was edited.

import { Loader2Icon } from "lucide-react";
import { useEffect } from "react";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../components/ui/sheet";
import { Textarea } from "../components/ui/textarea";
import { useInterfaceTranslation } from "../i18n/translator";
import { delegatePrompt } from "../prDelegate";
import { closeDelegate, editDelegate, fillDelegatePrompt, readFeedbackAgain, submitDelegate } from "./connection";
import { AgentChoice } from "./AgentChoice";
import { OfflineText } from "./OfflineText";
import { Field } from "./parts";
import { prOf } from "./prModel";
import { noticeText } from "./protocol";
import { selectionOf, startFailure, startProblem } from "./start";
import { usePhone } from "./store";

export function PrDelegateSheet() {
  const { t } = useInterfaceTranslation();
  const sheet = usePhone((s) => s.prDelegate);
  const pr = usePhone((s) => (s.prDelegate ? prOf(s.prs?.items ?? null, s.prDelegate.key) : null));
  const feedback = usePhone((s) => s.prs?.feedback ?? null);
  const catalog = usePhone((s) => s.startCatalog);
  const connected = usePhone((s) => s.connected);
  // The read this sheet asked for, not an older one the core still holds.
  const read = sheet && feedback && sheet.feedbackId === feedback.request_id ? feedback : null;
  const number = pr?.number ?? 0;
  const title = pr?.title ?? "";
  const branch = pr?.head_branch ?? "";
  useEffect(() => {
    if (read?.phase !== "ready") return;
    fillDelegatePrompt(delegatePrompt({ number, title, branch }, read, t));
  }, [read, number, title, branch, t]);
  const open = sheet !== null && pr !== null;
  const selection = selectionOf(catalog, sheet?.choice ?? { target: null, kind: null, model: undefined });
  const problem = startProblem(sheet?.prompt ?? "");
  const tooLong = problem === "too_long";
  const shown = tooLong ? startFailure("too_long") : (sheet?.error ?? null);
  const pending = sheet?.pending != null;
  const ready = connected && !pending && catalog !== null && problem === null;
  const reading = read === null ? sheet?.feedbackId !== null : read.phase === "reading";
  return (
    <Sheet open={open} onOpenChange={(next) => !next && closeDelegate()}>
      <SheetContent side="bottom" className="phone-safe-bottom max-h-full gap-md overflow-y-auto rounded-t-xl px-lg pt-lg pb-lg" data-phone-pr-delegate-sheet={pr?.key} onOpenAutoFocus={(event) => event.preventDefault()}>
        <SheetTitle className="text-headline">{t("prWork.delegateTitle", { number: String(number) })}</SheetTitle>
        <SheetDescription className="line-clamp-2 break-words text-body text-muted-foreground">{title}</SheetDescription>
        {!connected ? (
          <p role="status" className="flex items-center gap-md rounded-lg bg-secondary px-md py-sm text-subhead text-subtle-foreground" data-phone-unreachable="true">
            <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
            <span><OfflineText /></span>
          </p>
        ) : null}
        <Field label={t("issue.worktreeBranch")}>
          <Input
            mono
            value={branch}
            readOnly
            disabled
            data-phone-pr-delegate-branch="true"
            className="h-(--size-touch-target) truncate rounded-lg border-border bg-input px-md text-title disabled:opacity-100"
          />
        </Field>
        {pr && !pr.checkout ? <p className="text-body text-muted-foreground" data-phone-pr-delegate-new-worktree="true">{t("prWork.newWorktree")}</p> : null}
        <AgentChoice selection={selection} choice={sheet?.choice ?? { target: null, kind: null, model: undefined }} disabled={pending} onChange={(choice) => editDelegate({ choice })} name="pr-delegate" />
        <Field label={t("issue.firstPrompt")}>
          <Textarea
            value={sheet?.prompt ?? ""}
            onChange={(event) => editDelegate({ prompt: event.target.value })}
            rows={6}
            disabled={pending}
            autoCorrect="off"
            data-phone-pr-delegate-prompt="true"
            aria-invalid={tooLong || undefined}
            className="min-h-(--size-touch-target) rounded-lg border-border bg-input px-md py-sm text-title"
          />
        </Field>
        {read?.phase === "failed" ? (
          <p role="status" className="flex flex-wrap items-center gap-sm text-body text-warning" data-phone-pr-delegate-read-failed="true">
            <span className="min-w-0 break-keep wrap-anywhere">{t("githubReason.feedbackReadFailed")}</span>
            <Button variant="secondary" size="sm" className="min-h-(--size-touch-target)" disabled={!connected} onClick={readFeedbackAgain}>{t("prWork.readAgain")}</Button>
          </p>
        ) : (
          <p className="text-body text-muted-foreground" data-phone-pr-delegate-read={reading ? "reading" : "ready"}>{t(reading ? "prWork.readingFeedback" : "prWork.feedbackPrefilled")}</p>
        )}
        <p className="text-body text-muted-foreground" data-phone-pr-delegate-stays="true">{t("mobile.prs.delegate.stays")}</p>
        {shown ? <p role="alert" className="text-body text-destructive" data-phone-pr-delegate-error="true">{noticeText(t, shown)}</p> : null}
        <Button disabled={!ready} onClick={() => submitDelegate()} className="h-(--size-touch-target) w-full rounded-lg text-title font-semibold" data-phone-pr-delegate-submit="true">
          {pending ? <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin" /> : null}
          {t("mobile.prs.delegate.submit")}
        </Button>
      </SheetContent>
    </Sheet>
  );
}
