import { memo, useEffect, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { AGENT_ADAPTERS } from "./agentAdapters";
import { UsageWindows } from "./components/usage";
import { UsageHistory } from "./components/usage-history";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { useShellStore } from "./store";
import { hasLimits } from "./usage";
import { compactTokens, usageProvider } from "./usageHistory";
import { machineName } from "./screenMachine";

export const UsageOverview = memo(function UsageOverview({ actions }: { actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const node = useShellStore((s) => s.usage?.nodes[0]);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    actions.observeUsage({ usage_popover_open: true });
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => { window.clearInterval(timer); actions.observeUsage({ usage_popover_open: false }); };
  }, [actions]);
  if (!node) return <p className="p-lg text-caption text-muted-foreground" role="status">{t("overview.usage.loading")}</p>;
  const loading = node.agents.some((row) => row.consumption.state === "loading");
  const stale = node.agents.some((row) => row.consumption.state === "reading" && row.consumption.freshness === "stale");
  const unavailable = node.agents.some((row) => row.consumption.state === "unavailable" && row.consumption.cause !== "no_records");
  return <div className="usage-overview" data-usage-overview="true">
    <div className="flex flex-wrap items-center gap-sm text-caption text-muted-foreground" data-usage-global-scope="true"><span className="font-medium text-foreground" title={t("overview.usage.scope")}>{t("overview.usage.global", { device: machineName(devices, node.node, t) })}</span>{stale || unavailable ? <span role="status">{t(stale ? "overview.usage.stale" : "overview.usage.unavailable")}</span> : null}</div>
    {node.history ? <div className="usage-overview-history"><UsageHistory node={node} surface="year" outline={null} /></div> : <p className="text-caption text-muted-foreground" role="status">{t(loading ? "overview.usage.loading" : "overview.usage.noHistory")}</p>}
    <details className="usage-account-details"><summary className="cursor-pointer text-caption text-muted-foreground">{t("usage.history.accountLimits")}</summary><div className="usage-overview-limits flex flex-col gap-md">
      {AGENT_ADAPTERS.filter(hasLimits).map((adapter) => <div key={adapter.id} className="flex flex-col gap-sm py-sm" data-usage-row={adapter.herdr_kind}>
        <div className="flex items-center gap-sm text-subhead font-semibold"><AgentMark kind={adapter.herdr_kind} />{adapter.label}</div>
        {node.agents.some((row) => row.agent === adapter.herdr_kind && row.consumption.state === "reading" && row.consumption.value.year.limited) ? <span className="text-micro text-muted-foreground">{t("usage.history.limited")}</span> : null}
        <UsageWindows agent={adapter.herdr_kind} cell={node.agents.find((row) => row.agent === adapter.herdr_kind)?.limits} now={now} />
      </div>)}
      <h3 className="text-caption text-muted-foreground">{t("usage.noLimits")}</h3>
      {AGENT_ADAPTERS.filter((adapter) => !hasLimits(adapter) && adapter.usage.tokens.support === "supported").map((adapter) => {
        const cell = node.agents.find((row) => row.agent === adapter.herdr_kind)?.consumption;
        const value = cell?.state === "reading" ? cell.value : null;
        return <div key={adapter.id} className="flex flex-col gap-xxs text-caption" data-usage-token-agent={adapter.herdr_kind}>
          <div className="flex flex-wrap items-center gap-xs"><AgentMark kind={adapter.herdr_kind} /><span className="flex-1">{adapter.label}</span><span className="font-mono">{value?.year.amount.calls ? compactTokens(value.year.amount.new_tokens, language) : cell?.state === "loading" ? "…" : t("usage.history.noData")}</span>{value?.year.limited ? <span className="text-micro text-muted-foreground">{t("usage.history.limited")}</span> : null}</div>
          {value?.retained?.providers.map((provider) => {
            const display = usageProvider(provider);
            return display ? <span key={provider} className="pl-lg text-micro text-muted-foreground">{"account" in display ? t("usage.history.account", display) : t("usage.history.provider", display)}</span> : null;
          })}
        </div>;
      })}
    </div>
    </details>
  </div>;
});
