import { Fragment, useMemo, useState, type ReactNode } from "react";
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import type { NodeUsage, UsageAmount, UsageDay } from "../generated/hided-ws";
import { AgentMark, HideMark } from "../AgentMark";
import { AGENT_ADAPTERS, agentProductName } from "../agentAdapters";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { allTokens, compactTokens, hideAiShare, selectedUsage, sumHideAiDays, sumUsageDays, usageDate, usagePeriodDays } from "../usageHistory";
import { UsageCost, UsageStatus, UsageSummary } from "./usage-amount";
import { BoundedTokens, HIDE_AI_FEATURES } from "./usage-hide-ai";
import { Button } from "./ui/button";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";
import { UsageComparisonChart } from "./usage-comparison-chart";
import { HIDE_AI_USAGE, usageModelTotals } from "../overviewUsage";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./ui/select";

const PAGE_DAYS = 30;
const HEADINGS = ["usage.history.dateAgent", "usage.history.costHeading", "usage.history.newTokens", "usage.history.inputHeading", "usage.history.outputHeading", "usage.history.cacheWriteHeading", "usage.history.cacheReadHeading", "usage.history.totalTokens"] as const;
/** The filter value for Hide AI's own rows; no agent is called this. */
const HIDE_AI = HIDE_AI_USAGE;
const featureKey = (feature: string) => HIDE_AI_FEATURES[feature as keyof typeof HIDE_AI_FEATURES] ?? HIDE_AI_FEATURES.other;

export function UsageHistoryTable({ node }: { node: NodeUsage }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [agent, setAgent] = useState("all");
  const [period, setPeriod] = useState("30");
  const [page, setPage] = useState(0);
  const [exact, setExact] = useState(false);
  const [view, setView] = useState("daily");
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [folds, setFolds] = useState<Record<string, boolean>>({});
  const [hideFolds, setHideFolds] = useState<Record<string, boolean>>({});
  const hideOnly = agent === HIDE_AI;
  const withHide = agent === "all" || hideOnly;
  const range = selectedUsage(node, "year", withHide ? null : agent);
  const selectedCell = withHide ? null : node.agents.find((row) => row.agent === agent)?.consumption;
  const periodDays = useMemo(() => {
    if (!range) return [];
    const available = Math.round((Date.parse(range.calendar_end) - Date.parse(range.calendar_start)) / 86_400_000) + 1;
    const days = usagePeriodDays(range, period === "year" ? available : Number(period));
    return hideOnly ? days.map((day) => ({ ...day, amount: sumHideAiDays([day]).amount })) : days;
  }, [range, period, hideOnly]);
  // A newer snapshot can roll an old selection out of the chosen period.
  const selected = periodDays.some((day) => day.date === selectedDate) ? selectedDate : null;
  const selectedDays = useMemo(() => selected ? periodDays.filter((day) => day.date === selected) : periodDays, [periodDays, selected]);
  const days = useMemo(() => selectedDays.filter((day) => day.amount.calls > 0 || (withHide && (day.hide_ai?.unknown_calls ?? 0) > 0)).reverse(), [selectedDays, withHide]);
  const total = useMemo(() => sumUsageDays(selectedDays), [selectedDays]);
  const hideTotal = useMemo(() => sumHideAiDays(selectedDays), [selectedDays]);
  const lowerBound = withHide && hideTotal.unknown > 0;
  const models = useMemo(() => usageModelTotals(node.history?.models ?? [], new Set(selectedDays.map((day) => day.date)), agent), [node.history?.models, selectedDays, agent]);
  const byAgent = useMemo(() => node.agents.flatMap((row) => row.consumption.state === "reading" ? [{ agent: row.agent, days: new Map(row.consumption.value.year.days.map((day) => [day.date, day])) }] : []), [node.agents]);
  const pages = Math.max(1, Math.ceil((view === "daily" ? days.length : models.length) / PAGE_DAYS));
  const current = Math.min(page, pages - 1);
  const shown = days.slice(current * PAGE_DAYS, (current + 1) * PAGE_DAYS);
  const shownModels = models.slice(current * PAGE_DAYS, (current + 1) * PAGE_DAYS);
  const numbers = new Intl.NumberFormat(language);
  const fmt = (value: number) => exact ? numbers.format(value) : compactTokens(value, language);
  const reset = () => { setPage(0); setFolds({}); setHideFolds({}); };
  const chooseDay = (date: string | null) => { setSelectedDate(date); reset(); if (date) setFolds({ [date]: true }); };
  const cells = (amount: UsageAmount, bounded = false) => <><td className="usage-table-cost"><UsageCost amount={amount} /></td>{[amount.new_tokens, amount.tokens.input, amount.tokens.output, amount.tokens.cache_write, amount.tokens.cache_read, allTokens(amount)].map((value, index) => <td key={index} title={amount.calls ? numbers.format(value) : undefined}>{!amount.calls ? t("usage.history.noData") : index === 0 ? <BoundedTokens text={fmt(value)} bounded={bounded} /> : fmt(value)}</td>)}</>;
  return <section className="usage-history usage-records flex min-w-0 flex-col gap-lg" data-usage-history="year" data-surface="year">
    <div className="flex flex-wrap items-center justify-between gap-sm"><h2 className="text-subhead font-semibold">{t("usage.history.history")}</h2><div className="flex flex-wrap items-center gap-sm">
      <Select value={agent} onValueChange={(next) => { setAgent(next); reset(); }}><SelectTrigger size="sm" className="w-auto" aria-label={t("usage.history.agent")} data-usage-agent-select="true"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">{t("usage.history.allAgents")}</SelectItem><SelectItem value={HIDE_AI}><span className="inline-flex items-center gap-xs"><HideMark className="size-(--size-icon)" />{t("usage.hideAi.filter")}</span></SelectItem>{AGENT_ADAPTERS.filter((adapter) => adapter.usage.tokens.support === "supported").map((adapter) => <SelectItem key={adapter.herdr_kind} value={adapter.herdr_kind}><span className="inline-flex items-center gap-xs"><AgentMark kind={adapter.herdr_kind} />{adapter.label}</span></SelectItem>)}</SelectContent></Select>
      <Select value={period} onValueChange={(next) => { setPeriod(next); chooseDay(null); }}><SelectTrigger size="sm" className="w-auto" aria-label={t("usage.history.period")} data-usage-period-select="true"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="7">{t("usage.history.last7")}</SelectItem><SelectItem value="30">{t("usage.history.last30")}</SelectItem><SelectItem value="90">{t("overview.usage.last90")}</SelectItem><SelectItem value="year">{t("usage.history.lastYear")}</SelectItem></SelectContent></Select>
    </div></div>
    <div className="usage-comparison-summary"><UsageSummary amount={total} exact={exact} tokensLowerBound={lowerBound} /><div className="usage-comparison-cache"><div className="text-micro text-muted-foreground">{t("usage.history.cacheReadHeading")}</div><div className="usage-summary-value" data-usage-cache-read="true">{total.calls ? <BoundedTokens text={fmt(total.tokens.cache_read)} bounded={lowerBound} /> : t("usage.history.noData")}</div><span className="text-micro text-muted-foreground">{t("overview.usage.cacheReadNote")}</span></div></div>
    {range ? <UsageStatus range={range} /> : null}
    {selectedCell?.state === "loading" ? <span role="status" className="text-micro text-muted-foreground">{t("overview.usage.loading")}</span> : selectedCell?.state === "unavailable" && selectedCell.cause !== "no_records" ? <span role="status" className="text-micro text-muted-foreground">{agentProductName(agent)} · {t("common.unavailable")}</span> : null}
    <UsageComparisonChart days={periodDays} selected={selected} hideOnly={hideOnly} onSelect={chooseDay} />
    <div className="flex flex-wrap items-center justify-between gap-sm"><div className="flex flex-wrap items-center gap-sm"><h3 className="text-caption font-medium">{selected ? usageDate(selected, language, { year: "numeric", month: "short", day: "numeric" }) : t("overview.usage.breakdown")}</h3><ToggleGroup type="single" value={view} aria-label={t("overview.usage.breakdown")} onValueChange={(next) => { if (next) { setView(next); reset(); } }}><ToggleGroupItem size="sm" value="daily" data-usage-view="daily">{t("overview.usage.daily")}</ToggleGroupItem><ToggleGroupItem size="sm" value="model" data-usage-view="model">{t("overview.usage.models")}</ToggleGroupItem></ToggleGroup></div><Button variant={exact ? "secondary" : "ghost"} size="sm" aria-pressed={exact} onClick={() => setExact((value) => !value)} data-usage-exact="true">{t("usage.history.exact")}</Button></div>
    {view === "model" && node.history?.models_limited ? <span role="status" className="text-micro text-muted-foreground" data-usage-model-limited="true">{t("overview.usage.modelsLimited")}</span> : null}
    <div className="usage-table-scroll" tabIndex={0} role="region" aria-label={t("usage.history.history")}>
      <table className="usage-table" data-usage-table="true" data-usage-breakdown-view={view}><thead><tr>{HEADINGS.map((key, index) => <th key={key} scope="col">{t(index === 0 && view === "model" ? "overview.usage.agentModel" : key)}</th>)}</tr></thead><tbody>
        <tr className="usage-total-row" data-usage-period-total-row="true"><th scope="row">{t(selected ? "overview.usage.selectedDay" : "usage.history.periodTotal")}</th>{cells(total, lowerBound)}</tr>
        {view === "model" ? shownModels.map((row) => <tr key={row.key} data-usage-model-row={row.key}><th scope="row"><div className="usage-model-name">{row.model ?? t("overview.usage.unknownModel")}</div><span className="inline-flex items-center gap-xs text-micro text-muted-foreground">{row.hideAi ? <HideMark className="size-(--size-icon-sm)" /> : <AgentMark kind={row.agent} className="size-(--size-icon-sm)" />}{row.hideAi ? t("usage.hideAi.name") : agentProductName(row.agent)}{total.new_tokens > 0 ? <span>· {t("overview.usage.share", { share: hideAiShare(Math.floor(row.amount.new_tokens / total.new_tokens * 10_000), language) })}</span> : null}</span></th>{cells(row.amount)}</tr>) : shown.map((day) => {
          const expanded = folds[day.date] ?? (day.today || hideOnly);
          const hide = day.hide_ai;
          const hideExpanded = hideFolds[day.date] ?? false;
          const rows = hideOnly ? [] : byAgent.flatMap((row) => { const detail = row.days.get(day.date); return detail?.amount.calls && (agent === "all" || agent === row.agent) ? [{ agent: row.agent, amount: detail.amount }] : []; });
          const dayBounded = withHide && (hide?.unknown_calls ?? 0) > 0;
          return <Fragment key={day.date}><tr data-usage-date-row={day.date}><th scope="row"><button type="button" className="usage-date-toggle" aria-expanded={expanded} aria-label={t("usage.history.expandDay", { date: usageDate(day.date, language, { year: "numeric", month: "short", day: "numeric" }) })} onClick={() => setFolds((before) => ({ ...Object.fromEntries(shown.filter((item) => item.date in before).map((item) => [item.date, before[item.date]!])), [day.date]: !expanded }))}>{expanded ? <ChevronDownIcon /> : <ChevronRightIcon />}<span>{usageDate(day.date, language, { ...(period === "year" ? { year: "numeric" as const } : {}), month: "short", day: "numeric", weekday: "short" })}</span>{day.today ? <span className="text-micro text-muted-foreground">{t("usage.history.today")}</span> : null}</button></th>{cells(day.amount, dayBounded)}</tr>
            {expanded ? rows.map((row) => <tr key={row.agent} data-usage-agent-row={row.agent} className="usage-agent-row"><th scope="row"><span className="inline-flex items-center gap-xs"><AgentMark kind={row.agent} />{agentProductName(row.agent)}</span></th>{cells(row.amount)}</tr>) : null}
            {expanded && withHide && hide ? <HideAiRows day={day} open={hideExpanded} onToggle={() => setHideFolds((before) => ({ ...before, [day.date]: !hideExpanded }))} cells={cells} bare={hideOnly} /> : null}
          </Fragment>;
        })}
        {(view === "model" ? !shownModels.length : !shown.length) ? <tr><td colSpan={8} className="usage-empty">{t(view === "model" && (node.history?.models === undefined || node.history.models_limited && total.calls > 0) ? "overview.usage.modelsUnavailable" : range?.reading || selectedCell?.state === "loading" ? "usage.history.pending" : selectedCell?.state === "unavailable" && selectedCell.cause !== "no_records" ? "common.unavailable" : "usage.history.noMatches")}</td></tr> : null}
      </tbody></table>
    </div>
    <div className="flex flex-wrap items-start justify-between gap-sm text-micro text-muted-foreground"><div className="flex min-w-0 flex-col gap-xs"><span>{t("usage.history.apiEquivalent")}</span><span>{t("overview.usage.newTokensNote")}</span>{withHide ? <span>{t("overview.usage.included")}</span> : null}{lowerBound ? <span data-usage-hide-bound="true">{t("usage.hideAi.lowerBound")}</span> : null}{withHide ? <span data-usage-hide-since="true">{range?.hide_ai_since ? t("usage.hideAi.since", { date: usageDate(range.hide_ai_since, language, { year: "numeric", month: "long", day: "numeric" }) }) : t("usage.hideAi.sinceNone")}</span> : null}{withHide && hideTotal.unknown ? <span data-usage-unknown-calls="true">{t("usage.hideAi.unknownCalls")} · {t("usage.hideAi.calls", { count: hideTotal.unknown })}</span> : null}</div><div className="flex items-center gap-sm"><Button variant="ghost" size="sm" disabled={current === 0} onClick={() => { setPage(current - 1); setFolds({}); setHideFolds({}); }}>{t("usage.history.previous")}</Button><span>{t("usage.history.page", { page: current + 1, pages })}</span><Button variant="ghost" size="sm" disabled={current === pages - 1} onClick={() => { setPage(current + 1); setFolds({}); setHideFolds({}); }}>{t("usage.history.next")}</Button></div></div>
  </section>;
}

/** Hide AI's row on a date, at the layer of the agents' rows, and under it one row per feature and a line for calls that reported nothing. */
function HideAiRows({ day, open, onToggle, cells, bare }: { day: UsageDay; open: boolean; onToggle: () => void; cells: (amount: UsageAmount, bounded?: boolean) => ReactNode; bare: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const hide = day.hide_ai!;
  const bounded = hide.unknown_calls > 0;
  const date = usageDate(day.date, language, { year: "numeric", month: "short", day: "numeric" });
  return <>
    {bare ? null : <tr data-usage-hide-row={day.date} className="usage-agent-row"><th scope="row"><button type="button" className="usage-date-toggle" aria-expanded={open} aria-label={t("usage.hideAi.expand", { date })} onClick={onToggle}>{open ? <ChevronDownIcon /> : <ChevronRightIcon />}<HideMark className="size-(--size-icon)" /><span>{t("usage.hideAi.name")}</span></button></th>{cells(hide.amount, bounded)}</tr>}
    {open || bare ? hide.features.map((feature) => <tr key={feature.feature} data-usage-feature-row={feature.feature} className="usage-agent-row usage-feature-row"><th scope="row"><span className="inline-flex items-center gap-xs">{t(featureKey(feature.feature))}<span className="usage-calls">{t("usage.hideAi.calls", { count: feature.amount.calls })}</span>{feature.providers.map((provider) => <AgentMark key={provider} kind={provider} className="size-(--size-icon-sm)" />)}</span></th>{cells(feature.amount)}</tr>) : null}
    {(open || bare) && hide.unknown_calls > 0 ? <tr data-usage-unknown-row={day.date} className="usage-agent-row usage-feature-row"><th scope="row"><span className="inline-flex items-center gap-xs">{t("usage.hideAi.unknownCalls")}<span className="usage-calls">{t("usage.hideAi.calls", { count: hide.unknown_calls })}</span></span></th><td colSpan={7} className="usage-unknown-note">{t("usage.hideAi.unknownNote")}</td></tr> : null}
  </>;
}
