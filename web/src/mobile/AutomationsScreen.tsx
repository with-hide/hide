// The phone's automations tab (PRD automations B34, B35, D-20): every
// automation this computer's core runs, each row with its unseen dot, name,
// schedule and next run, and [▶ 지금 실행] [⏸ 멈춤], or [▶ 다시 켜기] on a
// paused one; a row opens its page with the runs. Making, editing and
// removing stay with an agent, so the tab has no button for them. An
// automation that stopped for a person is also its own line of the agent
// list's 내 차례 (D-12, B23), drawn here too. How a schedule and a next run
// read is `automationWords.ts`, shared with the desktop.

import { Loader2Icon, PauseIcon, PlayIcon, RepeatIcon } from "lucide-react";
import { nextRun, nextRunText, pauseReasonText, scheduleText } from "../automationWords";
import { Button } from "../components/ui/button";
import { RowOverlayButton } from "../components/ui/row-button";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { pressAutomation } from "./connection";
import { OfflineText } from "./OfflineText";
import { noticeText, type AutomationAction, type PhoneAutomation } from "./protocol";
import { TabBar } from "./TabBar";
import { automationOf, openAutomation, usePhone } from "./store";

export function AutomationsScreen() {
  const { t } = useInterfaceTranslation();
  const automations = usePhone((s) => s.automations);
  const omitted = usePhone((s) => s.automationsOmitted);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const unreadable = usePhone((s) => s.automationsUnreadable);
  // The last list stays, dimmed, under the unreachable line, as on the other tabs (B35).
  const dim = unreachable && !connected;
  return (
    <main className="flex min-h-full flex-col" data-phone-automations="true">
      <header className="phone-safe-top px-xl pb-sm">
        <h1 className="pt-lg text-headline font-semibold text-foreground">{t("mobile.automations.title")}</h1>
      </header>
      {dim ? (
        <div role="status" className="flex items-center gap-sm px-xl py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      <div className={cn("flex-1", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim}>
        {unreadable ? <p role="status" className="break-keep px-xl py-sm text-body text-warning wrap-anywhere" data-phone-automations-unreadable="true">{t("mobile.automations.unreadable")}</p> : null}
        {automations && automations.length === 0 ? (
          <div className="flex flex-col gap-xs px-xl py-xxl text-center" data-phone-automations-state="empty">
            <p className="text-title text-muted-foreground">{t("mobile.automations.empty")}</p>
            <p className="text-body text-subtle-foreground">{t("mobile.automations.byAgent")}</p>
          </div>
        ) : automations ? (
          <ul className="mx-xl divide-y divide-border border-b border-border">
            {automations.map((automation) => (
              <li key={automation.id}>
                <Row automation={automation} connected={connected} />
              </li>
            ))}
          </ul>
        ) : null}
        {omitted > 0 ? (
          <p className="px-xl py-md text-body text-muted-foreground" data-phone-automations-omitted={omitted}>{t("mobile.automations.omitted", { count: omitted })}</p>
        ) : null}
      </div>
      <TabBar active="automations" />
    </main>
  );
}

/** One automation (B34): its unseen dot and name, then its schedule and next run, and its buttons; a paused one is dimmed. */
function Row({ automation, connected }: { automation: PhoneAutomation; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="relative" data-phone-automation={automation.id} data-phone-automation-state={automation.running ? "running" : automation.paused ? "paused" : "on"}>
      <RowOverlayButton aria-label={t("mobile.automations.open", { name: automation.name })} hover={false} className="active:bg-accent" onClick={() => openAutomation(automation.id)} />
      <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center gap-sm py-md">
        <div className={cn("flex min-w-0 flex-1 flex-col gap-xxs", automation.paused && "opacity-(--opacity-dimmed)")}>
          <span className="flex min-w-0 items-center gap-sm">
            {automation.unseen > 0 ? <span role="img" aria-label={t("mobile.automations.unseen")} className="size-(--size-status-mark) shrink-0 rounded-full bg-foreground" data-phone-automation-unseen={automation.unseen} /> : null}
            <span className="min-w-0 break-keep text-title font-medium text-foreground wrap-anywhere">{automation.name}</span>
          </span>
          <ScheduleLine automation={automation} />
        </div>
        <RowButtons automation={automation} connected={connected} />
      </div>
      <RowNotice id={automation.id} />
    </div>
  );
}

/** The schedule as it reads, then the next-run cell: `● 도는 중`, `⏸ <why>`, or when it runs next in the computer's zone (B3, B25). */
export function ScheduleLine({ automation, className }: { automation: PhoneAutomation; className?: string }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const zone = usePhone((s) => s.automationsZone);
  const next = nextRun(automation);
  const paused = automation.paused;
  return (
    <span className={cn("break-keep text-body text-muted-foreground wrap-anywhere", className)} data-phone-automation-next={next.kind}>
      {scheduleText(automation.schedule, language, t)}
      {" · "}
      {next.kind === "running" ? (
        <span className="whitespace-nowrap text-agent-working">● {nextRunText(automation, Date.now(), zone, language, t)}</span>
      ) : paused ? (
        // Only the operator's own pause is quiet; the rest are a person's to fix (D-12).
        <span className={paused.reason === "operator" ? undefined : "text-warning"} data-phone-automation-paused={paused.reason}><PauseText text={pauseReasonText(paused.reason, t)} /></span>
      ) : (
        <span>{nextRunText(automation, Date.now(), zone, language, t)}</span>
      )}
    </span>
  );
}

/** `⏸ <why>`: the mark stays with the reason's first word, and a long reason wraps after it. */
function PauseText({ text }: { text: string }) {
  const space = text.indexOf(" ");
  const head = space < 0 ? text : text.slice(0, space);
  return (
    <>
      <span className="whitespace-nowrap">⏸ {head}</span>
      {space < 0 ? null : text.slice(space)}
    </>
  );
}

/** Why a press on this row did not reach the computer, one line under it. */
function RowNotice({ id }: { id: string }) {
  const { t } = useInterfaceTranslation();
  const notice = usePhone((s) => (s.automationNotice?.id === id ? s.automationNotice.notice : null));
  if (!notice) return null;
  return <p role="alert" className="relative pb-md text-body text-warning" data-phone-automation-notice="true">{noticeText(t, notice)}</p>;
}

/** The automations the core counts as the operator's turn, oldest pause first, each one line of 내 차례: its name, `<why> · 고치고` and [다시 켜기], the press the tab's row sends (D-12, B23); the line opens its page. */
export function AutomationAskRows() {
  const { t } = useInterfaceTranslation();
  const automations = usePhone((s) => s.automations);
  const asks = usePhone((s) => s.automationAsks);
  const connected = usePhone((s) => s.connected);
  const asking = asks.flatMap((id) => {
    const automation = automationOf(automations, id);
    return automation?.paused ? [{ automation, paused: automation.paused }] : [];
  });
  if (asking.length === 0) return null;
  return (
    <ul className="divide-y divide-border border-b border-border" data-phone-automation-asks={asking.length}>
      {asking.map(({ automation, paused }) => (
        <li key={automation.id} className="relative" data-phone-automation-ask={automation.id}>
          <RowOverlayButton aria-label={t("mobile.automations.open", { name: automation.name })} hover={false} className="active:bg-accent" onClick={() => openAutomation(automation.id)} />
          <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center gap-sm py-md">
            <RepeatIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-warning" />
            <span className="min-w-0 flex-1 break-keep text-body wrap-anywhere">
              <span className="font-medium text-foreground">{automation.name}</span>
              <span className="text-warning">{" · "}</span>
              {/* `<why> · 고치고` wraps as one piece, so 고치고 never stands alone on a line. */}
              <span className="inline-block text-warning" data-phone-automation-ask-line={paused.reason}>{t("mobile.automations.ask", { reason: pauseReasonText(paused.reason, t) })}</span>
            </span>
            <ResumeButton automation={automation} connected={connected} />
          </div>
          <RowNotice id={automation.id} />
        </li>
      ))}
    </ul>
  );
}

/** [▶ 다시 켜기] in words, as the desktop's screen draws it, so it is not read as [지금 실행] (B25). */
function ResumeButton({ automation, connected }: { automation: PhoneAutomation; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const pending = usePhone((s) => s.automationPending);
  const waiting = pending?.id === automation.id && pending.action === "resume";
  return (
    <Button
      variant="secondary"
      className="pointer-events-auto min-h-(--size-touch-target) shrink-0 text-body"
      disabled={!connected || pending !== null}
      data-phone-automation-press="resume"
      onClick={() => pressAutomation(automation.id, "resume")}
    >
      {waiting ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : <PlayIcon aria-hidden="true" className="size-(--size-icon)" />}
      {t("mobile.automations.resume")}
    </Button>
  );
}

/** A row's own presses (mockup `phone`): [▶] [⏸] while it is on, [▶ 다시 켜기] in words while it is paused. */
function RowButtons({ automation, connected }: { automation: PhoneAutomation; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const pending = usePhone((s) => s.automationPending);
  const busy = !connected || pending !== null;
  const press = (action: AutomationAction) => pressAutomation(automation.id, action);
  const waiting = (action: AutomationAction) => pending?.id === automation.id && pending.action === action;
  const icon = (action: AutomationAction, Icon: typeof PlayIcon) =>
    waiting(action) ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : <Icon aria-hidden="true" className="size-(--size-icon)" />;
  const round = "pointer-events-auto size-(--size-touch-target) rounded-full";
  if (automation.paused) return <ResumeButton automation={automation} connected={connected} />;
  return (
    <span className="flex shrink-0 gap-sm">
      <Button variant="secondary" size="icon" className={round} aria-label={t("mobile.automations.run")} disabled={busy || automation.running} data-phone-automation-press="run" onClick={() => press("run")}>
        {icon("run", PlayIcon)}
      </Button>
      <Button variant="secondary" size="icon" className={round} aria-label={t("mobile.automations.pause")} disabled={busy} data-phone-automation-press="pause" onClick={() => press("pause")}>
        {icon("pause", PauseIcon)}
      </Button>
    </span>
  );
}
