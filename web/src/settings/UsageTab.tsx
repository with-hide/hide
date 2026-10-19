import { useState } from "react";
import type { Actions } from "../actions";
import { AGENT_ADAPTERS } from "../agentAdapters";
import { AgentMark } from "../AgentMark";
import { Group, Note, Row } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { absenceReason, accountWindows, hasLimits, limitValue, windowLabel } from "../usage";

/** The declaration supplies all seven rows; the core owns every saved edit. */
export function UsageTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const preferences = useShellStore((s) => s.rest?.ui_state?.usage);
  const agents = useShellStore((s) => s.usage?.nodes[0]?.agents);
  const [now] = useState(Date.now);
  return <div data-usage-settings="true">
    {AGENT_ADAPTERS.filter(hasLimits).map((adapter) => {
      const id = adapter.herdr_kind;
      const cell = agents?.find((row) => row.agent === id)?.limits;
      const hidden = preferences?.hidden.includes(id) ?? false;
      return <Group key={id} data-usage-agent={id}>
        <Row label={<span className="flex items-center gap-sm font-medium"><AgentMark kind={id} />{adapter.label}</span>}>
          {hidden ? <span className="text-caption text-muted-foreground">{t("usage.hidden")}</span> : null}
          <Button variant="ghost" size="sm" disabled={!preferences} onClick={() => actions.setUsageHidden({ agent: id, hidden: !hidden })} aria-label={t(hidden ? "usage.showAgent" : "usage.hideAgent", { agent: adapter.label })}>{t(hidden ? "usage.show" : "usage.hide")}</Button>
        </Row>
        {!hidden ? accountWindows(adapter, cell).map((kind) => {
          const checked = preferences?.pinned.some((pin) => pin.agent === id && pin.window === kind) ?? false;
          const full = !checked && (preferences?.pinned.length ?? 0) >= 3;
          const label = windowLabel(kind, t);
          const noteId = `usage-pin-limit-${id}-${kind}`;
          return <Row key={kind} data-usage-pin-row={kind} label={<span data-usage-pin-label="true">{label}</span>} value={<span className="font-mono text-body text-muted-foreground">{limitValue(cell, kind, now, t)}</span>} detail={full ? <Note data-usage-pin-limit="true"><span id={noteId}>{t("usage.pinLimit")}</span></Note> : undefined}>
            <Switch checked={checked} disabled={!preferences || full} aria-describedby={full ? noteId : undefined} aria-label={`${adapter.label} ${label}`} onCheckedChange={(enabled) => actions.setUsagePin({ agent: id, window: kind, enabled })} />
          </Row>;
        }) : null}
      </Group>;
    })}
    <p className="mb-xl text-body text-muted-foreground">{t("usage.note")}</p>
    <Group title={t("usage.noLimits")} data-usage-unsupported="true">
      {AGENT_ADAPTERS.filter((adapter) => !hasLimits(adapter)).map((adapter) => <Row key={adapter.id} label={<span className="flex items-center gap-sm"><AgentMark kind={adapter.herdr_kind} />{adapter.label}</span>} detail={<Note>{absenceReason(adapter, t)}</Note>} />)}
    </Group>
  </div>;
}
