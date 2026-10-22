// Display periods reuse the core's prepared daily amounts and intensity levels.
import { useMemo, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { ArrowUpRightIcon } from "lucide-react";
import type { NodeUsage, UsageDay } from "../generated/hided-ws";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { compactTokens, sumUsageDays, usageDate, usagePeriodDays } from "../usageHistory";
import { UsageCost, UsageStatus, UsageSummary } from "./usage-amount";
import { BoundedTokens } from "./usage-hide-ai";
import { UsageHistoryTable } from "./usage-history-table";
import { Button } from "./ui/button";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";
import "./usage-history.css";

export type UsageOutline = "five_hour" | "weekly" | null;

function DayChart({ days, period, selected, outline, onPreview, onSelect }: { days: UsageDay[]; period: 7 | 30; selected: string; outline: UsageOutline; onPreview: (date: string | null) => void; onSelect: (date: string) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [cursor, setCursor] = useState<string | null>(null);
  const tabDate = days.some((day) => day.date === cursor) ? cursor : selected;
  const bars = period === 7;
  const maximum = Math.max(1, ...days.map((day) => day.amount.new_tokens));
  const key = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const offsets: Record<string, number> = bars ? { ArrowLeft: -1, ArrowRight: 1 } : { ArrowUp: -1, ArrowDown: 1, ArrowLeft: -7, ArrowRight: 7 };
    const next = event.key === "Home" ? 0 : event.key === "End" ? days.length - 1 : offsets[event.key] !== undefined ? index + offsets[event.key]! : null;
    if (next === null) return;
    event.preventDefault();
    const date = days[Math.max(0, Math.min(next, days.length - 1))]!.date;
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-usage-day="${date}"]`)?.focus();
  };
  return <div className={bars ? "usage-bars" : "usage-grass"} role="group" aria-label={t("usage.history.calendar")} onMouseLeave={() => onPreview(null)} style={{ "--usage-weeks": days.at(-1)!.week + 1 } as CSSProperties}>
    {days.map((day, index) => <button type="button" key={day.date} className={bars ? "usage-bar" : "usage-cell"} data-usage-day={day.date} data-level={day.level} data-pending={day.pending} data-today={day.today} data-selected={day.date === selected} data-outline={outline === "weekly" ? day.recent_week : outline === "five_hour" ? day.today : false}
      tabIndex={day.date === tabDate ? 0 : -1} style={bars ? { "--usage-bar-height": `${day.amount.new_tokens / maximum * 100}%` } as CSSProperties : { gridColumn: day.week + 1, gridRow: day.weekday + 1 }} aria-pressed={day.date === selected}
      aria-label={day.pending ? `${usageDate(day.date, language)} · ${t("usage.history.pending")}` : t("usage.history.cell", { date: usageDate(day.date, language), value: compactTokens(day.amount.new_tokens, language) })}
      onMouseEnter={() => onPreview(day.date)} onFocus={() => { setCursor(day.date); onPreview(day.date); }} onBlur={() => { setCursor(null); onPreview(null); }} onClick={() => onSelect(day.date)} onKeyDown={(event) => key(event, index)}>
      {bars ? <><span className="usage-bar-plot" aria-hidden="true"><span className="usage-bar-fill" /></span><span aria-hidden="true">{day.today ? t("usage.history.today") : usageDate(day.date, language, { weekday: "short" })}</span></> : null}
    </button>)}
  </div>;
}

function CompactHistory({ node, outline, children, onHistory }: { node: NodeUsage; outline: UsageOutline; children?: ReactNode; onHistory?: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const range = node.history?.popover.range;
  const [period, setPeriod] = useState<7 | 30>(30);
  const days = useMemo(() => range ? usagePeriodDays(range, period) : [], [range, period]);
  const total = useMemo(() => sumUsageDays(days), [days]);
  const totalLowerBound = days.some((day) => (day.hide_ai?.unknown_calls ?? 0) > 0);
  const [pinned, setPinned] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  if (!range) return children;
  const selected = days.find((day) => day.date === pinned) ?? days.at(-1)!;
  const day = days.find((day) => day.date === preview) ?? selected;
  const fmt = (n: number) => compactTokens(n, language);
  return <section className="usage-history flex min-w-0 flex-col gap-md" data-usage-history="popover" data-surface="popover">
    <div data-usage-detail="true" className="flex flex-col gap-sm">
      <div className="flex items-center justify-between gap-xs text-caption text-muted-foreground"><span data-usage-date="true">{day.today ? t("usage.history.today") : usageDate(day.date, language, { month: "short", day: "numeric", weekday: "short" })}</span>{!selected.today ? <Button variant="ghost" size="sm" onClick={() => { setPinned(null); setPreview(null); }}>{t("usage.history.today")}</Button> : null}</div>
      <UsageSummary amount={day.amount} pending={day.pending} tokensLowerBound={(day.hide_ai?.unknown_calls ?? 0) > 0} announce />
      <details className="text-micro text-muted-foreground" data-usage-breakdown="true"><summary className="cursor-pointer">{t("usage.history.details")}</summary><div className="usage-token-detail"><span>{t("usage.history.input", { value: fmt(day.amount.tokens.input) })}</span><span>{t("usage.history.output", { value: fmt(day.amount.tokens.output) })}</span><span>{t("usage.history.cacheWrite", { value: fmt(day.amount.tokens.cache_write) })}</span><span>{t("usage.history.cacheRead", { value: fmt(day.amount.tokens.cache_read) })}</span></div></details>
    </div>
    <div className="usage-grass-section">
      <div className="flex items-center justify-between gap-xs text-micro text-muted-foreground"><span>{t("usage.history.newTokens")}</span>
        <ToggleGroup type="single" value={String(period)} aria-label={t("usage.history.period")} onValueChange={(value) => {
          if (value !== "7" && value !== "30") return;
          const next = Number(value) as 7 | 30;
          if (!usagePeriodDays(range, next).some((day) => day.date === pinned)) setPinned(null);
          setPreview(null);
          setPeriod(next);
        }}>
          <ToggleGroupItem value="7" size="sm" aria-label={t("usage.history.last7")}>{t("usage.history.days7")}</ToggleGroupItem>
          <ToggleGroupItem value="30" size="sm" aria-label={t("usage.history.last30")}>{t("usage.history.days30")}</ToggleGroupItem>
        </ToggleGroup>
      </div>
      <div className={period === 7 ? "flex flex-col gap-sm" : "flex items-center gap-lg"}>
        <DayChart days={days} period={period} selected={selected.date} outline={outline} onPreview={setPreview} onSelect={setPinned} />
        <div data-usage-period-total="true" aria-label={t("usage.history.periodTotal")} className={period === 7 ? "flex min-w-0 items-center justify-between gap-xs" : "flex min-w-0 flex-col gap-xs"}>
          <span className="font-mono text-subhead font-semibold"><UsageCost amount={total} /></span><span className="font-mono text-caption text-muted-foreground" data-usage-period-tokens="true"><BoundedTokens text={fmt(total.new_tokens)} bounded={totalLowerBound} /></span>
          {period === 30 ? <span className="usage-legend" aria-hidden="true">{t("usage.history.less")}{[0, 1, 2, 3, 4].map((level) => <span key={level} className="usage-cell" data-level={level} />)}{t("usage.history.more")}</span> : null}
        </div>
      </div>
    </div>
    <UsageStatus range={range} />
    {children}
    {onHistory ? <Button variant="ghost" size="sm" className="w-full justify-between" onClick={onHistory} data-usage-history-link="true">{t("usage.history.history")}<ArrowUpRightIcon /></Button> : null}
  </section>;
}

export function UsageHistory({ node, surface, outline, onHistory, children }: { node: NodeUsage; surface: "popover" | "year"; outline: UsageOutline; onHistory?: () => void; children?: ReactNode }) {
  return surface === "popover" ? <CompactHistory node={node} outline={outline} onHistory={onHistory}>{children}</CompactHistory> : <UsageHistoryTable node={node} />;
}
