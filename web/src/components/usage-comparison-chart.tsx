import { useState, type CSSProperties, type KeyboardEvent } from "react";
import type { UsageDay } from "../generated/hided-ws";
import { AgentMark, HideMark } from "../AgentMark";
import { AGENT_ADAPTERS, agentProductName } from "../agentAdapters";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { HIDE_AI_USAGE } from "../overviewUsage";
import { compactTokens, usageDate } from "../usageHistory";
import { Button } from "./ui/button";

function series(day: UsageDay, hideOnly: boolean): { agent: string; tokens: number }[] {
  if (hideOnly) return [{ agent: HIDE_AI_USAGE, tokens: day.hide_ai?.amount.new_tokens ?? 0 }];
  return [...day.agents.map((row) => ({ agent: row.agent, tokens: row.new_tokens })),
    ...(day.hide_ai ? [{ agent: HIDE_AI_USAGE, tokens: day.hide_ai.amount.new_tokens }] : [])];
}

function usageSeriesIndex(agent: string): number {
  return agent === HIDE_AI_USAGE ? AGENT_ADAPTERS.length : Math.max(0, AGENT_ADAPTERS.findIndex((row) => row.herdr_kind === agent));
}

export function UsageComparisonChart({ days, selected, hideOnly, onSelect }: { days: UsageDay[]; selected: string | null; hideOnly: boolean; onSelect: (date: string | null) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [preview, setPreview] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  if (!days.length) return null;
  const maximum = Math.max(1, ...days.map((day) => day.amount.new_tokens));
  const active = days.find((day) => day.date === preview);
  const caption = active ? active.pending || !active.amount.calls
    ? `${usageDate(active.date, language)} · ${t(active.pending ? "usage.history.pending" : "usage.history.noData")}`
    : t("usage.history.cell", { date: usageDate(active.date, language), value: compactTokens(active.amount.new_tokens, language) })
    : selected ? usageDate(selected, language, { year: "numeric", month: "short", day: "numeric" }) : null;
  const focusDate = days.some((day) => day.date === cursor) ? cursor : selected ?? days.at(-1)!.date;
  const agents = [...new Set(days.flatMap((day) => series(day, hideOnly).filter((row) => row.tokens > 0).map((row) => row.agent)))];
  const move = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = event.key === "Home" ? 0 : event.key === "End" ? days.length - 1 : event.key === "ArrowLeft" ? Math.max(0, index - 1) : event.key === "ArrowRight" ? Math.min(days.length - 1, index + 1) : null;
    if (next === null) return;
    event.preventDefault();
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-usage-day="${days[next]!.date}"]`)?.focus();
  };
  return <div className="usage-comparison-chart flex min-w-0 flex-col gap-sm">
    <div className="flex flex-wrap items-center gap-sm text-caption"><h3 className="font-medium">{t("usage.history.calendar")}</h3><span className="text-muted-foreground" data-usage-day-caption="true">{caption}</span>{selected ? <Button variant="ghost" size="sm" onClick={() => onSelect(null)} data-usage-clear-day="true">{t("overview.usage.clearDay")}</Button> : null}</div>
    <div className="usage-comparison-plot" role="group" aria-label={t("usage.history.calendar")} onMouseLeave={() => setPreview(null)} style={{ "--usage-day-count": days.length } as CSSProperties}>
      {days.map((day, index) => <button type="button" key={day.date} className="usage-comparison-bar" data-usage-day={day.date} data-selected={selected === day.date} data-pending={day.pending} aria-pressed={selected === day.date} tabIndex={focusDate === day.date ? 0 : -1}
        aria-label={day.pending || !day.amount.calls ? `${usageDate(day.date, language)} · ${t(day.pending ? "usage.history.pending" : "usage.history.noData")}` : t("usage.history.cell", { date: usageDate(day.date, language), value: compactTokens(day.amount.new_tokens, language) })}
        onMouseEnter={() => setPreview(day.date)} onFocus={() => { setCursor(day.date); setPreview(day.date); }} onBlur={() => setPreview(null)} onClick={() => onSelect(selected === day.date ? null : day.date)} onKeyDown={(event) => move(event, index)}>
        {series(day, hideOnly).map((row) => <span key={row.agent} className="usage-series" data-series={usageSeriesIndex(row.agent)} style={{ height: `${row.tokens / maximum * 100}%` }} />)}
      </button>)}
    </div>
    <div className="flex justify-between text-micro text-muted-foreground" aria-hidden="true"><span>{usageDate(days[0]!.date, language)}</span><span>{usageDate(days.at(-1)!.date, language)}</span></div>
    <div className="flex flex-wrap items-center gap-md text-micro text-muted-foreground">{agents.map((agent) => <span key={agent} className="inline-flex items-center gap-xs"><i className="usage-series usage-series-key" data-series={usageSeriesIndex(agent)} />{agent === HIDE_AI_USAGE ? <HideMark className="size-(--size-icon-sm)" /> : <AgentMark kind={agent} className="size-(--size-icon-sm)" />}{agent === HIDE_AI_USAGE ? t("usage.hideAi.name") : agentProductName(agent)}</span>)}</div>
  </div>;
}
