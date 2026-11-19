// One automation's page on the phone (PRD automations B34, mockup `phone`):
// its name, schedule and next run, [▶ 지금 실행] and [⏸ 멈춤] or [다시 켜기],
// and its runs, newest first, each with its mark, time, tags and result.
// The rest of a run (its terminal, its conversation) stays on the desktop.

import { ArrowLeftIcon, CheckIcon, Loader2Icon, MinusIcon, PauseIcon, PlayIcon, SquareIcon, XIcon } from "lucide-react";
import { recordText, skippedFolded, spendText, whenText, type RecordTone } from "../automationWords";
import { Button } from "../components/ui/button";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { ScheduleLine } from "./AutomationsScreen";
import { pressAutomation } from "./connection";
import { OfflineText } from "./OfflineText";
import { noticeText, type PhoneAutomation, type PhoneAutomationRecord } from "./protocol";
import { automationOf, closeAutomation, usePhone } from "./store";

export function AutomationDetail() {
  const automation = usePhone((s) => automationOf(s.automations, s.automationOpen));
  // An automation that left the list while its page was open is gone with it; the store closes the page on the next frame.
  if (!automation) return null;
  return <Page automation={automation} />;
}

function Page({ automation }: { automation: PhoneAutomation }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const dim = unreachable && !connected;
  const older = automation.records_omitted ?? 0;
  return (
    <main className="flex h-full flex-col" data-phone-automation-page={automation.id}>
      <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-md">
        <div className="pt-md">
          <Button variant="text" size="inline" className="min-h-(--size-touch-target) gap-xs text-body" data-phone-back="automations" onClick={closeAutomation}>
            <ArrowLeftIcon aria-hidden="true" className="size-(--size-icon)" />
            {t("mobile.automations.title")}
          </Button>
        </div>
        <h1 className="mt-sm break-keep text-headline font-semibold text-foreground wrap-anywhere">{automation.name}</h1>
        <p className="mt-xs">
          <ScheduleLine automation={automation} />
        </p>
        <Buttons automation={automation} connected={connected} />
      </header>
      {dim ? (
        <div role="status" className="flex items-center gap-sm px-lg py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      <section className={cn("min-h-0 flex-1 overflow-auto overscroll-contain px-lg pb-lg", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim} data-phone-automation-records={automation.records.length}>
        <h2 className="pb-xs pt-lg text-body text-muted-foreground">{t("mobile.automations.runs")}</h2>
        {automation.records.length === 0 ? (
          <p className="py-sm text-body text-subtle-foreground" data-phone-automation-records-state="empty">{t("mobile.automations.noRuns")}</p>
        ) : (
          <ul className="divide-y divide-border border-b border-border">
            {automation.records.map((record) => (
              <li key={recordKey(record)}>
                <Record record={record} />
              </li>
            ))}
          </ul>
        )}
        {older > 0 ? <p className="py-md text-body text-muted-foreground" data-phone-automation-older={older}>{t("mobile.automations.olderRuns", { count: older })}</p> : null}
      </section>
    </main>
  );
}

function recordKey(record: PhoneAutomationRecord): string {
  switch (record.kind) {
    case "run":
      return `run:${record.number}`;
    case "skipped":
      return `skipped:${record.at_unix_ms}`;
    case "missed":
      return `missed:${record.first_unix_ms}`;
  }
}

/** [▶ 지금 실행] off while a run goes (B19), and [⏸ 멈춤] that turns into [다시 켜기] (B25); a refusal is one line under them. */
function Buttons({ automation, connected }: { automation: PhoneAutomation; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const pending = usePhone((s) => (s.automationPending?.id === automation.id ? s.automationPending.action : null));
  const other = usePhone((s) => s.automationPending !== null);
  const notice = usePhone((s) => (s.automationNotice?.id === automation.id ? s.automationNotice.notice : null));
  const busy = !connected || other;
  const toggle = automation.paused ? "resume" : "pause";
  return (
    <div className="mt-md flex flex-col gap-sm">
      <div className="flex gap-sm">
        <Button
          variant="secondary"
          className="h-(--size-touch-target) flex-1 rounded-lg text-title"
          disabled={busy || automation.running}
          data-phone-automation-press="run"
          onClick={() => pressAutomation(automation.id, "run")}
        >
          {pending === "run" ? <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin" /> : <PlayIcon aria-hidden="true" className="size-(--size-icon-lg)" />}
          {t("mobile.automations.run")}
        </Button>
        <Button
          variant="secondary"
          className="h-(--size-touch-target) flex-1 rounded-lg text-title"
          disabled={busy}
          data-phone-automation-press={toggle}
          onClick={() => pressAutomation(automation.id, toggle)}
        >
          {pending === toggle ? <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin" /> : toggle === "pause" ? <PauseIcon aria-hidden="true" className="size-(--size-icon-lg)" /> : <PlayIcon aria-hidden="true" className="size-(--size-icon-lg)" />}
          {t(toggle === "pause" ? "mobile.automations.pause" : "mobile.automations.resume")}
        </Button>
      </div>
      {notice ? <p role="alert" className="text-body text-warning" data-phone-automation-notice="true">{noticeText(t, notice)}</p> : null}
    </div>
  );
}

const TONE: Record<RecordTone, { icon: typeof CheckIcon; className: string }> = {
  ok: { icon: CheckIcon, className: "text-success" },
  failed: { icon: XIcon, className: "text-destructive" },
  running: { icon: Loader2Icon, className: "animate-spin text-agent-working" },
  skipped: { icon: MinusIcon, className: "text-muted-foreground" },
  stopped: { icon: SquareIcon, className: "text-muted-foreground" },
};

/** When a record happened, in the computer's zone; a missed line and a skipped line of several times say their own range. */
function recordAt(record: PhoneAutomationRecord): number | null {
  switch (record.kind) {
    case "run":
      return record.started_at_unix_ms;
    case "skipped":
      return skippedFolded(record) ? null : record.at_unix_ms;
    case "missed":
      return null;
  }
}

/** One record (B19-B22): its mark, when it started with its tags and unseen dot, and its result. */
function Record({ record }: { record: PhoneAutomationRecord }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const zone = usePhone((s) => s.automationsZone);
  const now = Date.now();
  const line = recordText(record, now, zone, language, t);
  const { icon: Icon, className } = TONE[line.tone];
  const at = recordAt(record);
  const tag = record.kind === "run" && record.trigger !== "scheduled" ? t(record.trigger === "manual" ? "mobile.automations.manual" : "mobile.automations.missedRun") : null;
  const unseen = record.kind === "run" && !record.seen && line.tone !== "running";
  return (
    <div className="flex min-h-(--size-touch-target) items-start gap-sm py-md" data-phone-automation-record={record.kind} data-phone-automation-record-tone={line.tone}>
      <Icon aria-hidden="true" className={cn("mt-xxs size-(--size-icon) shrink-0", className)} />
      <div className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className="flex min-w-0 flex-wrap items-center gap-sm text-title text-foreground">
          {unseen ? <span role="img" aria-label={t("mobile.automations.unseen")} className="size-(--size-status-mark) shrink-0 rounded-full bg-foreground" /> : null}
          {at !== null ? <span>{whenText(at, now, zone, language, t)}</span> : null}
          {tag ? <span className="rounded-sm bg-secondary px-xs text-caption text-subtle-foreground" data-phone-automation-tag={record.kind === "run" ? record.trigger : undefined}>{tag}</span> : null}
        </span>
        <span className="break-keep text-body text-subtle-foreground wrap-anywhere" data-phone-automation-result="true">{line.text}</span>
        {record.kind === "run" && record.spend ? <span className="text-caption tabular-nums text-muted-foreground" data-phone-automation-spend="true">{spendText(record.spend, language, t)}</span> : null}
      </div>
    </div>
  );
}
