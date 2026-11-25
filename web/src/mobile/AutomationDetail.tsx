// One automation's page on the phone (PRD automations B34, mockup `phone`):
// its name, schedule and next run, [▶ 지금 실행] and [⏸ 멈춤] or [다시 켜기],
// and its runs, newest first, each with its mark, time, tags and result.
// The rest of a run (its terminal, its conversation) stays on the desktop.

import { CheckIcon, Loader2Icon, MinusIcon, SquareIcon, XIcon } from "lucide-react";
import { lineTime, recordText, spendText, whenText, type RecordTone } from "../automationWords";
import { Button } from "../components/ui/button";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { ScheduleLine } from "./AutomationsScreen";
import { PageHead, TwoLineRow } from "./layout";
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
      <PageHead
        back={{ label: t("mobile.automations.title"), onClick: closeAutomation, tag: "automations" }}
        title={automation.name}
        line={<ScheduleLine automation={automation} page />}
      >
        <Buttons automation={automation} connected={connected} />
      </PageHead>
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
          <ul className="-mx-lg divide-y divide-border border-b border-border [&>li]:px-lg">
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
    <div className="mt-sm flex flex-col gap-sm">
      <div className="flex gap-sm">
        <Button
          variant="secondary"
          className="h-(--size-touch-target) rounded-lg px-lg text-body"
          disabled={busy || automation.running}
          data-phone-automation-press="run"
          onClick={() => pressAutomation(automation.id, "run")}
        >
          {pending === "run" ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : null}
          {t("mobile.automations.run")}
        </Button>
        {/* A done once automation has no time left to pause for (mission-104 S-3); it is made again on the computer. */}
        {automation.done && !automation.paused ? null : (
          <Button
            variant="secondary"
            className="h-(--size-touch-target) rounded-lg px-lg text-body"
            disabled={busy}
            data-phone-automation-press={toggle}
            onClick={() => pressAutomation(automation.id, toggle)}
          >
            {pending === toggle ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : null}
            {t(toggle === "pause" ? "mobile.automations.pause" : "mobile.automations.resume")}
          </Button>
        )}
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

/** When a record happened, in the computer's zone: a run's start, a skipped or missed line's one time; a line of several times says its own span. */
function recordAt(record: PhoneAutomationRecord): number | null {
  return record.kind === "run" ? record.started_at_unix_ms : lineTime(record);
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
  const spend = record.kind === "run" && record.spend ? spendText(record.spend, language, t) : null;
  return (
    <div className="flex min-h-(--size-touch-target) items-center py-md" data-phone-automation-record={record.kind} data-phone-automation-record-tone={line.tone}>
      <TwoLineRow
        lead={<Icon aria-hidden="true" className={cn("size-(--size-icon) shrink-0", className)} />}
        head={
          <>
            {at !== null ? <span className="shrink-0 text-title text-foreground">{whenText(at, now, zone, language, t)}</span> : null}
            {unseen ? <span role="img" aria-label={t("mobile.automations.unseen")} className="size-(--size-status-mark) shrink-0 rounded-full bg-agent-working" /> : null}
            {tag ? <span className="shrink-0 rounded-sm bg-secondary px-xs text-caption text-subtle-foreground" data-phone-automation-tag={record.kind === "run" ? record.trigger : undefined}>{tag}</span> : null}
            {at === null ? <span className="min-w-0 break-keep text-title text-foreground wrap-anywhere" data-phone-automation-result="true">{line.text}</span> : null}
          </>
        }
        line={at !== null ? (
          <span className={cn("min-w-0 break-keep wrap-anywhere", line.tone === "failed" ? "text-destructive" : "text-subtle-foreground")}>
            <span data-phone-automation-result="true">{line.text}</span>
            {spend ? <> · <span className="tabular-nums" data-phone-automation-spend="true">{spend}</span></> : null}
          </span>
        ) : null}
      />
    </div>
  );
}
