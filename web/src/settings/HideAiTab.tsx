import { useState } from "react";
import type { Actions } from "../actions";
import { AgentLogo } from "../components/agent-logo";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { Group, Note, Row, Status } from "../components/settings-rows";
import {
  CLI_DEFAULT,
  NO_USAGE_LIMIT,
  firstEnabledAgent,
  modelChoices,
  modelToValue,
  modelsFailed,
  nobodySignedIn,
  parseRowKey,
  providerById,
  rowKey,
  runsOnChoices,
  usageLimitChoices,
  usageLimitOf,
  usageLimitToValue,
  valueToModel,
  valueToUsageLimit,
} from "../hideAi";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { HideAiFallback } from "./HideAiFallback";
import { ProviderState, useHideAiWords } from "./hideAiParts";
import { useAgentsDemand } from "./useAgentsDemand";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

/**
 * Hide AI: the agent Hide uses behind the scenes and the features that use it
 * (PRD settings-cleanup D-14 to D-18, D-24). Everything shown is the core's
 * `background_ai` answer: which agents may be chosen, their sign-in state and
 * models, the fallback list and who is answering now. The "Worktree names"
 * value keeps its owner, the issue settings; this tab reads and writes it.
 */
export function HideAiTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { name, refusal, signIn } = useHideAiWords();
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const kitAgents = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.kit?.agents);
  const issueSettings = useShellStore((s) => s.rest?.ui_state?.issue_settings);
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const aiError = useErrorSince(changedAt, ["ai_settings."]);
  const [namesAt, setNamesAt] = useState<number | null>(null);
  const namesError = useErrorSince(namesAt, ["issue_settings."]);
  useAgentsDemand(actions, false);
  const changed = () => setChangedAt(Date.now());
  const enabled = ai?.enabled ?? true;
  const selected = providerById(ai, ai?.provider);
  const answering = ai?.refusal?.using ? providerById(ai, ai.refusal.using) : null;
  const refused = ai?.refusal ? providerById(ai, ai.refusal.provider) : null;
  const choices = ai ? runsOnChoices(ai) : [];
  const modelValues = selected ? modelChoices(selected) : [];
  const usageLimit = usageLimitOf(ai);
  // Every agent Hide AI would ask is past the limit: the line names the limit, not one agent (B5).
  const listed = [selected, ...(ai?.fallback ?? []).map((entry) => providerById(ai, entry.provider))];
  const everyAgentPast = listed.length > 1 && listed.every((provider) => provider?.past_limit === true);
  const agentLines = [
    aiError ? (
      <Note key="error" tone="error" data-ai-error="true" data-core-error={aiError.kind}>
        {coreErrorText(aiError, t, "hideAi.notSaved")}
      </Note>
    ) : null,
    ai?.refusal && !ai.refusal.using && refused ? (
      <Note key="paused" tone="warn" data-ai-paused="true">
        {ai.refusal.reason === "past_limit" && everyAgentPast
          ? t("hideAi.pausedAllPast", { limit: ai.refusal.limit ?? "" })
          : `${refusal(refused, ai.refusal)} · ${t("hideAi.paused", { agent: name(refused) })}`}
      </Note>
    ) : null,
    ai && !selected && nobodySignedIn(ai) ? (
      <Note key="signin" tone="warn" data-ai-sign-in="true">
        {(() => {
          const agent = firstEnabledAgent(kitAgents);
          return agent ? signIn(agent) : t("hideAi.turnOnAgent");
        })()}
      </Note>
    ) : null,
  ].filter(Boolean);
  return (
    <div data-hide-ai-tab="true" data-ai-enabled={String(enabled)}>
      <Group data-settings-group="hide-ai-use">
        <Row
          label={
            <span className="flex min-w-0 flex-col gap-xxs">
              <span>{t("hideAi.use")}</span>
              <span className="text-body text-muted-foreground">{t("hideAi.useDescription")}</span>
            </span>
          }
        >
          <Switch
            checked={enabled}
            disabled={!ai}
            onCheckedChange={(next) => {
              changed();
              actions.setHideAiEnabled(next);
            }}
            aria-label={t("hideAi.use")}
            data-ai-use={String(enabled)}
          />
        </Row>
      </Group>
      {!ai ? <Note tone="pending">{t("hideAi.reading")}</Note> : null}
      {/* Off dims the rest and takes it out of reach; the values stay stored and come back as they were (B33). */}
      <div inert={!ai || !enabled} className={!ai || !enabled ? "opacity-(--opacity-dimmed)" : undefined} data-hide-ai-body="true">
        <Group title={t("hideAi.runsOn")} note={t("hideAi.runsOnNote")} data-settings-group="hide-ai-runs-on">
          <Row label={t("hideAi.agent")} detail={agentLines.length > 0 ? <div className="flex flex-col gap-xs">{agentLines}</div> : null}>
            <Select
              value={selected ? rowKey(selected) : undefined}
              disabled={choices.length === 0}
              onValueChange={(value) => {
                changed();
                const { provider, device } = parseRowKey(value);
                actions.chooseAi(provider, undefined, device);
              }}
            >
              <SelectTrigger aria-label={t("hideAi.agentAria")} data-ai-provider="true">
                <SelectValue placeholder={t("hideAi.choose")} />
              </SelectTrigger>
              <SelectContent>
                {choices.map((provider) => (
                  <SelectItem key={rowKey(provider)} value={rowKey(provider)}>
                    <AgentLogo agent={provider.agent} label={provider.label} />
                    {name(provider)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {selected ? <ProviderState provider={selected} /> : null}
          </Row>
          {selected ? (
            <Row label={t("hideAi.model")} detail={modelsFailed(selected) ? <Note data-ai-models-failed="true">{t("hideAi.modelsFailed")}</Note> : null}>
              <Select
                value={modelToValue(selected.model)}
                disabled={modelValues.length < 2}
                onValueChange={(value) => {
                  changed();
                  actions.chooseAi(selected.id, valueToModel(value));
                }}
              >
                <SelectTrigger aria-label={t("hideAi.modelAria")} data-ai-model="true">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {modelValues.map((value) => (
                    <SelectItem key={value} value={value}>
                      {value === CLI_DEFAULT ? t("hideAi.cliDefault") : value}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {selected.models_fixed ? <Status tone="muted" data-ai-models-fixed="true">{t("hideAi.modelFixed")}</Status> : null}
            </Row>
          ) : null}
          {answering && refused && ai?.refusal ? (
            <Row
              label={
                <Status tone="warn" data-ai-using={answering.id}>
                  {t("hideAi.using", { using: name(answering), reason: refusal(refused, ai.refusal) })}
                </Status>
              }
            />
          ) : null}
        </Group>
        {ai ? <HideAiFallback ai={ai} actions={actions} onChange={changed} /> : null}
        <Group data-settings-group="hide-ai-usage-limit">
          <Row
            label={
              <span className="flex min-w-0 flex-col gap-xxs">
                <span>{t("hideAi.usageLimit")}</span>
                <span className="text-body text-muted-foreground" data-ai-usage-limit-summary="true">
                  {usageLimit === null ? t("hideAi.usageLimitNoneSummary") : t("hideAi.usageLimitSummary", { limit: usageLimit })}
                </span>
              </span>
            }
            detail={<Note>{t("hideAi.usageLimitNote")}</Note>}
          >
            {/* The shown value is the core's: a pick stays as it was until the core accepts it, so a refused change never looks saved. */}
            <Select
              value={usageLimitToValue(usageLimit)}
              disabled={!ai}
              onValueChange={(value) => {
                changed();
                actions.setAiUsageLimit(valueToUsageLimit(value));
              }}
            >
              <SelectTrigger aria-label={t("hideAi.usageLimit")} data-ai-usage-limit={usageLimitToValue(usageLimit)}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {usageLimitChoices(usageLimit).map((value) => (
                  <SelectItem key={value} value={value}>
                    {value === NO_USAGE_LIMIT ? t("hideAi.usageLimitNone") : `${value}%`}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
        </Group>
        <Group title={t("hideAi.features")} data-settings-group="hide-ai-features">
          <Row label={t("hideAi.agentSummaries")} detail={<Note>{t("hideAi.agentSummariesDescription")}</Note>}>
            <Switch
              checked={ai?.agent_summary ?? true}
              disabled={!ai}
              onCheckedChange={(checked) => {
                changed();
                actions.setAgentSummary(checked);
              }}
              aria-label={t("hideAi.agentSummaries")}
              data-ai-agent-summary={String(ai?.agent_summary ?? true)}
            />
          </Row>
          <Row
            label={t("hideAi.worktreeNames")}
            detail={
              namesError ? (
                <Note tone="error" data-issue-settings-error="true" data-core-error={namesError.kind}>
                  {coreErrorText(namesError, t, "coreError.settings.notSaved")}
                </Note>
              ) : (
                <Note>{t("hideAi.worktreeNamesDescription")}</Note>
              )
            }
          >
            <Switch
              checked={issueSettings?.ai_worktree_name ?? true}
              disabled={!issueSettings}
              onCheckedChange={(checked) => {
                setNamesAt(Date.now());
                actions.setIssueSettings({ ai_worktree_name: checked });
              }}
              aria-label={t("hideAi.worktreeNames")}
              data-issue-ai-worktree-name={String(issueSettings?.ai_worktree_name ?? true)}
            />
          </Row>
        </Group>
      </div>
    </div>
  );
}
