import { useState } from "react";
import type { Actions } from "../actions";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Group, Note, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { SLEEP_AFTER_CHOICES, sleepAfterChoice, sleepAfterLabel, sleepingCount } from "../settings";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

/**
 * Sleep idle agents (PRD agent-sleep B1-B3): one choice, and how many of this
 * machine's agents sleep now so the effect of the choice is visible.
 */
export function IdleAgentsGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const choice = useShellStore((s) => sleepAfterChoice(s.rest?.ui_state?.agent_sleep_after_hours));
  const sleeping = useShellStore((s) => sleepingCount(s.rest?.navigator?.agents));
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const error = useErrorSince(changedAt, ["agent_sleep."]);
  return (
    <Group title={t("settings.idleAgents")} note={t("settings.sleepingNow", { count: sleeping })} data-settings-group="idle-agents">
      <Row label={t("settings.sleepAfter")} detail={error ? <Note tone="error" data-agent-sleep-error="true" data-core-error={error.kind}>{coreErrorText(error, t, "coreError.settings.notSaved")}</Note> : null}>
        <Select
          value={choice}
          onValueChange={(value) => {
            const next = SLEEP_AFTER_CHOICES.find((row) => row.id === value);
            if (!next || value === choice) return;
            setChangedAt(Date.now());
            actions.setAgentSleepAfter(next.hours);
          }}
        >
          <SelectTrigger aria-label={t("settings.sleepAfter")} data-agent-sleep-after={choice}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SLEEP_AFTER_CHOICES.map((row) => (
              <SelectItem key={row.id} value={row.id} data-agent-sleep-option={row.id}>
                {sleepAfterLabel(row, t)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Row>
    </Group>
  );
}
