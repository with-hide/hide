// The phone's automations tab (PRD automations B34, B35, D-20): every
// automation this computer's core runs, each a two-line row (Doop 15) with its
// unseen dot and name, then its schedule and next run; only a row that stopped
// for a person carries [다시 켜기], and a row opens its page, where running,
// pausing and the runs are. Making, editing and
// removing stay with an agent, so the tab has no button for them. An
// automation that stopped for a person is also its own line of the agent
// list's 내 차례 (D-12, B23), drawn here too. How a schedule and a next run
// read is `automationWords.ts`, shared with the desktop.

import { Loader2Icon, RepeatIcon } from "lucide-react";
import { nextRun, nextRunText, pauseReasonText, scheduleText } from "../automationWords";
import { Button } from "../components/ui/button";
import { RowOverlayButton } from "../components/ui/row-button";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { pressAutomation } from "./connection";
import { OfflineText } from "./OfflineText";
import { DeviceTag } from "./parts";
import { noticeText, type PhoneAutomation } from "./protocol";
import { TabBar } from "./TabBar";
import { ScreenHead, TwoLineRow } from "./layout";
import { automationOf, openAutomation, usePhone } from "./store";

export function AutomationsScreen() {
  const { t } = useInterfaceTranslation();
  const automations = usePhone((s) => s.automations);
  const omitted = usePhone((s) => s.automationsOmitted);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const unreadable = usePhone((s) => s.automationsUnreadable);
  const macName = usePhone((s) => s.macName);
  // The last list stays, dimmed, under the unreachable line, as on the other tabs (B35).
  const dim = unreachable && !connected;
  return (
    <main className="flex min-h-full flex-col" data-phone-automations="true">
      <ScreenHead title={t("mobile.automations.title")} line={automations && automations.length > 0 ? t("mobile.automations.summary", { name: macName || t("mobile.mac"), count: automations.length }) : null} />
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
          <ul className="mx-xl">
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

/**
 * One automation (B34, Doop 15): its unseen dot ahead of the name, then its schedule and what comes next in the state's colour; a
 * paused one is dimmed, and only one that stopped for a person to fix carries [다시 켜기]. Running it now and pausing it are on its
 * page.
 */
function Row({ automation, connected }: { automation: PhoneAutomation; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const asks = automation.paused !== null && automation.paused !== undefined && automation.paused.reason !== "operator";
  return (
    <div className="relative" data-phone-automation={automation.id} data-phone-automation-state={automation.running ? "running" : automation.paused ? "paused" : "on"}>
      <RowOverlayButton aria-label={t("mobile.automations.open", { name: automation.name })} hover={false} className="active:bg-accent" onClick={() => openAutomation(automation.id)} />
      <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center gap-sm py-md">
        <span className={cn("flex min-w-0 flex-1", automation.paused && !asks && "opacity-(--opacity-dimmed)")}>
          <TwoLineRow
            lead={automation.unseen > 0 ? <span role="img" aria-label={t("mobile.automations.unseen")} className="size-(--size-status-mark) shrink-0 rounded-full bg-agent-working" data-phone-automation-unseen={automation.unseen} /> : null}
            head={<span className="min-w-0 break-keep text-title font-medium text-foreground wrap-anywhere">{automation.name}</span>}
            line={<ScheduleLine automation={automation} />}
          />
        </span>
        {asks ? <ResumeButton automation={automation} connected={connected} /> : null}
      </div>
      <RowNotice id={automation.id} />
    </div>
  );
}

/** The schedule line's colour by what it says: a run going, a stop a person must fix, or the plain schedule. */
const LINE_TONE = { working: "text-agent-working", warning: "text-warning", quiet: "text-muted-foreground" } as const;

/**
 * The schedule as it reads and what comes next, the whole line in the state's colour (B3, B25, Doop 15): `· 도는 중` while a run
 * goes, `· <why> · 고치고` for what a person fixes, `· 멈춤` for the operator's own pause, else when it runs next in the
 * computer's zone, which the page says as `다음 <when>` (`page`).
 */
export function ScheduleLine({ automation, page = false, className }: { automation: PhoneAutomation; page?: boolean; className?: string }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const zone = usePhone((s) => s.automationsZone);
  const next = nextRun(automation);
  const paused = automation.paused;
  const when = nextRunText(automation, Date.now(), zone, language, t);
  const after = next.kind === "running" ? when
    : paused ? (paused.reason === "operator" ? pauseReasonText(paused.reason, t) : t("mobile.automations.ask", { reason: pauseReasonText(paused.reason, t) }))
      : page && next.kind === "at" ? t("mobile.automations.nextAt", { when }) : when;
  const tone = next.kind === "running" ? "working" : paused && paused.reason !== "operator" ? "warning" : "quiet";
  return (
    <span className={cn("break-keep text-body wrap-anywhere", LINE_TONE[tone], className)} data-phone-automation-next={next.kind} data-phone-automation-paused={paused?.reason} data-phone-automation-tone={tone}>
      {scheduleText(automation.schedule, language, t)}
      {" · "}
      {after}
    </span>
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
/** `computer` is the gathered view's: the name of the computer these automations run on, drawn under each one's line. */
export function AutomationAskRows({ computer }: { computer?: string } = {}) {
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
    <ul data-phone-automation-asks={asking.length}>
      {asking.map(({ automation, paused }) => (
        <li key={automation.id} className="relative" data-phone-automation-ask={automation.id}>
          <RowOverlayButton aria-label={t("mobile.automations.open", { name: automation.name })} hover={false} className="active:bg-accent" onClick={() => openAutomation(automation.id)} />
          {/* One line of 내 차례 (Doop 01): the repeat mark, the name, then `자동화 <why> · 고치고` and [다시 켜기]. */}
          <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center gap-sm py-md">
            <span className="flex min-w-0 flex-1">
              <TwoLineRow
                lead={<RepeatIcon aria-hidden="true" className="size-(--size-icon) text-warning" />}
                head={<span className="min-w-0 break-keep text-title font-medium text-foreground wrap-anywhere">{automation.name}</span>}
                line={
                  <span className="min-w-0 break-keep wrap-anywhere">
                    {computer ? <DeviceTag label={computer} computer /> : null}
                    <span className="text-muted-foreground">{t("mobile.tab.automations")} </span>
                    <span className="text-warning" data-phone-automation-ask-line={paused.reason}>{t("mobile.automations.ask", { reason: pauseReasonText(paused.reason, t) })}</span>
                  </span>
                }
              />
            </span>
            <ResumeButton automation={automation} connected={connected} />
          </div>
          <RowNotice id={automation.id} />
        </li>
      ))}
    </ul>
  );
}

/** An automation's name and why it stopped for a person, as one line of 내 차례; the gathered view draws other computers' with it. */
export function AskLine({ name, reason, grow = false }: { name: string; reason: NonNullable<PhoneAutomation["paused"]>["reason"]; grow?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("break-keep text-body wrap-anywhere", grow && "min-w-0 flex-1")}>
      <span className="font-medium text-foreground">{name}</span>
      <span className="text-warning">{" · "}</span>
      {/* `<why> · 고치고` wraps as one piece, so 고치고 never stands alone on a line. */}
      <span className="inline-block text-warning" data-phone-automation-ask-line={reason}>{t("mobile.automations.ask", { reason: pauseReasonText(reason, t) })}</span>
    </span>
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
      {waiting ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin" /> : null}
      {t("mobile.automations.resume")}
    </Button>
  );
}
