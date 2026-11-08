// Account windows at the sidebar foot. The core owns reads and preferences;
// this view formats typed outcomes and sends visibility hints.
import { memo, useEffect, useRef, useState } from "react";
import { ChartNoAxesCombinedIcon, SettingsIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { AGENT_ADAPTERS, agentProductName } from "../agentAdapters";
import type { AgentUsage, LimitCell, UsagePreferences, WindowReading } from "../generated/hided-ws";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import { hasLimits, percentLabel, resetCountdown, SHORT_WINDOW, usageChips, usageTone, windowLabel, windowPercent, WINDOW_ORDER, type UsageTone } from "../usage";
import { UsageHistory, type UsageOutline } from "./usage-history";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from "./ui/popover";

const NO_ROWS: AgentUsage[] = [];
const NO_PREFS: UsagePreferences = { pinned: [], hidden: [] };
const TEXT_TONE: Record<UsageTone, string> = { success: "text-success", warning: "text-warning", danger: "text-destructive", muted: "text-muted-foreground" };
const FILL_TONE: Record<UsageTone, string> = { success: "bg-success", warning: "bg-warning", danger: "bg-destructive", muted: "bg-muted-foreground" };

export function useUsageWindowHint(actions: Actions) {
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (!live) return;
    const report = () => actions.observeUsage({ usage_window_visible: document.visibilityState === "visible" });
    report();
    document.addEventListener("visibilitychange", report);
    return () => document.removeEventListener("visibilitychange", report);
  }, [actions, live]);
}

export const Usage = memo(function Usage({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rows = useShellStore((s) => s.usage?.nodes[0]?.agents ?? NO_ROWS);
  const prefs = useShellStore((s) => s.rest?.ui_state?.usage ?? NO_PREFS);
  const node = useShellStore((s) => s.usage?.nodes[0]);
  const [outline, setOutline] = useState<UsageOutline>(null);
  const [open, setOpen] = useState(false);
  // An action that opens another layer runs once the popover has closed: while it is still open, Overview refuses to open over it.
  const afterClose = useRef<(() => void) | null>(null);
  // No closed-popover minute clock. Core snapshots expire chip readings.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!open) return;
    actions.observeUsage({ usage_popover_open: true });
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => { window.clearInterval(timer); actions.observeUsage({ usage_popover_open: false }); };
  }, [actions, open]);
  const chips = usageChips(rows, prefs, now);
  const visible = AGENT_ADAPTERS.filter((adapter) => hasLimits(adapter) && !prefs.hidden.includes(adapter.herdr_kind));
  if (!chips.length && !node?.history) return null;
  return <Popover open={open} onOpenChange={(next) => { setNow(Date.now()); setOpen(next); }}>
    <PopoverAnchor asChild><div className="flex min-w-0 flex-wrap gap-xs" data-usage-chips="true">
      {!chips.length ? <PopoverTrigger aria-label={t("usage.title")} data-usage-trigger="true" className="rounded-sm p-xs text-muted-foreground hover:bg-accent"><ChartNoAxesCombinedIcon className="size-(--size-icon)" /></PopoverTrigger> : null}
      {chips.map((chip) => {
        const label = agentProductName(chip.agent);
        const value = chip.percent !== null ? percentLabel(chip.percent) : chip.window ? t(chip.state === "not_started" ? "usage.notStarted" : "usage.expired") : chip.state === "loading" ? "…" : t("common.unavailable");
        return <PopoverTrigger key={`${chip.agent}-${chip.window ?? "none"}`} data-usage-trigger="true" data-usage-chip={chip.agent} data-usage-window={chip.window} data-usage-state={chip.state}
          aria-label={[label, chip.window ? windowLabel(chip.window, t) : "", value].filter(Boolean).join(", ")}
          className={cn("flex h-(--size-control-sm) shrink-0 items-center gap-xxs rounded-sm bg-secondary px-xs font-mono text-micro font-semibold outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-accent", TEXT_TONE[usageTone(chip.percent)])}>
          <AgentMark kind={chip.agent} className={chip.percent === null ? "opacity-(--opacity-dimmed) grayscale" : ""} />
          {chip.named && chip.window ? <span className="text-muted-foreground">{SHORT_WINDOW[chip.window]}</span> : null}
          {chip.percent !== null ? <span>{percentLabel(chip.percent)}</span> : null}
        </PopoverTrigger>;
      })}
    </div></PopoverAnchor>
    <PopoverContent side="top" align="start" className="flex max-h-(--radix-popover-content-available-height) w-(--size-usage-popover) flex-col gap-sm overflow-y-auto p-lg" data-usage-popover="true"
      onCloseAutoFocus={(event) => {
        const action = afterClose.current;
        afterClose.current = null;
        if (!action) return;
        event.preventDefault();
        action();
      }}>
      <div className="flex flex-wrap items-center gap-sm">
        <span className="min-w-0 flex-1 text-subhead font-semibold">{t("usage.title")}</span>
        <Hint label={t("usage.chooseChips")}><Button variant="ghost" size="icon-sm" onClick={() => { setOpen(false); actions.openSettings("usage"); }}><SettingsIcon /></Button></Hint>
      </div>
      {node ? <UsageHistory node={node} surface="popover" outline={outline} onHistory={() => { afterClose.current = () => actions.openUsageHistory(); setOpen(false); }}>
        <div className="usage-limits flex flex-col gap-md">
        {visible.map((adapter) => <div key={adapter.id} className="flex flex-col gap-xs" data-usage-row={adapter.herdr_kind}>
        <div className="flex items-center gap-xs text-caption font-semibold"><AgentMark kind={adapter.herdr_kind} />{adapter.label}</div>
        <UsageWindows agent={adapter.herdr_kind} cell={rows.find((row) => row.agent === adapter.herdr_kind)?.limits} now={now} onOutline={setOutline} compact />
      </div>)}
        </div>
      </UsageHistory> : null}
    </PopoverContent>
  </Popover>;
});

export function UsageWindows({ agent, cell, now, onOutline, compact = false }: { agent: string; cell: LimitCell | undefined; now: number; onOutline?: (window: UsageOutline) => void; compact?: boolean }) {
  const { t } = useInterfaceTranslation();
  if (!cell || cell.state === "loading") return <span data-usage-value="true" className="text-caption text-muted-foreground">…</span>;
  if (cell.state !== "reading") return <div className="text-caption text-muted-foreground">
    <span data-usage-value="true">{t("common.unavailable")}</span>
    {cell.state === "unavailable" && cell.cause === "signed_out" ? <p>{t("usage.signedOut", { command: agent })}</p> : null}
    {cell.state === "unavailable" && cell.cause === "not_installed" ? <p>{t("usage.notInstalled", { command: agent })}</p> : null}
  </div>;
  const windows = WINDOW_ORDER.flatMap((kind) => cell.value.windows.filter((window) => window.kind === kind).sort((a, b) => Number(a.scope.kind === "model") - Number(b.scope.kind === "model")));
  // A model's window under its account window says its reset only where it
  // differs from the line above: the same countdown twice only crowds the row.
  const countdown = (window: WindowReading) => window.outcome.state === "open" ? resetCountdown(window.outcome.resets_at_s, now, t) : null;
  const parentReset = new Map(windows.filter((window) => window.scope.kind !== "model").map((window) => [window.kind, countdown(window)]));
  return windows.map((window) => {
    const reset = countdown(window);
    const shown = window.scope.kind === "model" && reset === parentReset.get(window.kind) ? null : reset;
    return <UsageLine key={`${window.kind}-${window.scope.kind === "model" ? window.scope.label : "all"}`} window={window} reset={shown} now={now} onOutline={onOutline} compact={compact} />;
  });
}

function UsageLine({ window, reset, now, onOutline, compact }: { window: WindowReading; reset: string | null; now: number; onOutline?: (window: UsageOutline) => void; compact: boolean }) {
  const { t } = useInterfaceTranslation();
  const percent = windowPercent(window, now);
  if (["expired", "not_offered", "unreadable"].includes(window.outcome.state) || (window.outcome.state === "open" && percent === null)) return null;
  const bucket = window.scope.kind === "model";
  return <div onMouseEnter={() => onOutline?.(window.kind === "monthly" ? null : window.kind)} onMouseLeave={() => onOutline?.(null)} className={cn("flex flex-col gap-xs", bucket && "pl-md")} data-usage-window-row={window.kind} data-usage-bucket={window.scope.kind === "model" ? window.scope.label : undefined}>
    <div className="flex items-baseline gap-xs">
      <span className="min-w-0 truncate text-caption font-medium text-subtle-foreground" title={window.scope.kind === "model" ? window.scope.label : undefined}>{window.scope.kind === "model" ? `└ ${window.scope.label}` : windowLabel(window.kind, t)}</span>
      {reset ? <span className="shrink-0 whitespace-nowrap font-mono text-micro text-muted-foreground" data-usage-reset="true">· {reset}</span> : null}
      {compact && percent !== null ? <div aria-hidden="true" className="ml-auto h-xs w-(--size-control-lg) shrink-0 overflow-hidden rounded-full bg-border"><div className={cn("h-full rounded-full", FILL_TONE[usageTone(percent)])} style={{ width: `${Math.min(Math.max(percent, 0), 100)}%` }} /></div> : null}
      <span className={cn(compact ? "shrink-0 font-mono text-caption font-semibold" : "ml-auto shrink-0 font-mono text-caption font-semibold", TEXT_TONE[usageTone(percent)])} data-usage-value="true">{percent !== null ? percentLabel(percent) : t("usage.notStarted")}</span>
    </div>
    {!compact && percent !== null ? <div aria-hidden="true" className="h-xs overflow-hidden rounded-full bg-border"><div className={cn("h-full rounded-full", FILL_TONE[usageTone(percent)])} style={{ width: `${Math.min(Math.max(percent, 0), 100)}%` }} /></div> : null}
  </div>;
}
