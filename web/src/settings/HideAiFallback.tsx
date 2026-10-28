import { PlusIcon, XIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentLogo } from "../components/agent-logo";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Group, Row } from "../components/settings-rows";
import { CLI_DEFAULT, addableProviders, modelChoices, modelToValue, providerById, rowKey, unusableProviders, valueToModel } from "../hideAi";
import { useInterfaceTranslation } from "../i18n/client";
import type { AiProvider, BackgroundAi } from "../snapshot";
import { ProviderState, useHideAiWords } from "./hideAiParts";

/**
 * "If <Runs on> can't answer" (PRD settings-cleanup D-16, D-17, B38 to B40):
 * the agents the operator added, in the order they were added, each with its
 * own model and a remove; Add agent offers only agents Hide AI can use. There
 * is no switch and no reordering: an empty list is no fallback, and a different
 * order is a remove and an add.
 */
export function HideAiFallback({ ai, actions, onChange }: { ai: BackgroundAi; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name } = useHideAiWords();
  const chosen = providerById(ai, ai.provider);
  if (!chosen) return null;
  const entries = ai.fallback ?? [];
  const agent = name(chosen);
  return (
    <Group
      title={t("hideAi.fallbackTitle", { agent })}
      note={[t("hideAi.fallbackNote", { agent }), entries.length === 0 ? t("hideAi.fallbackEmpty", { agent }) : null].filter(Boolean).join(" ")}
      data-settings-group="hide-ai-fallback"
    >
      {entries.map((entry, index) => {
        const provider = providerById(ai, entry.provider);
        return provider ? <FallbackRow key={entry.provider} index={index} provider={provider} model={entry.model} actions={actions} onChange={onChange} /> : null;
      })}
      <AddAgent ai={ai} actions={actions} onChange={onChange} />
    </Group>
  );
}

function FallbackRow({ index, provider, model, actions, onChange }: { index: number; provider: AiProvider; model: string; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name } = useHideAiWords();
  const choices = modelChoices(provider, model);
  return (
    <Row
      data-ai-fallback={provider.id}
      label={
        <span className="flex min-w-0 items-center gap-sm">
          <span aria-hidden="true" className="w-md shrink-0 text-center font-mono text-body text-muted-foreground" data-ai-fallback-order="true">
            {index + 1}
          </span>
          <AgentLogo agent={provider.agent} label={provider.label} />
          <span className="min-w-0 break-words">{name(provider)}</span>
        </span>
      }
    >
      <ProviderState provider={provider} />
      <Select
        value={modelToValue(model)}
        disabled={choices.length < 2}
        onValueChange={(value) => {
          onChange();
          actions.chooseAi(provider.id, valueToModel(value));
        }}
      >
        <SelectTrigger aria-label={t("hideAi.fallbackModelAria", { agent: name(provider) })} data-ai-fallback-model={provider.id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {choices.map((choice) => (
            <SelectItem key={choice} value={choice}>
              {choice === CLI_DEFAULT ? t("hideAi.cliDefault") : choice}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t("hideAi.removeAria", { agent: name(provider) })}
        data-ai-fallback-remove={provider.id}
        onClick={() => {
          onChange();
          actions.removeAiFallback(provider.id);
        }}
      >
        <XIcon aria-hidden="true" />
      </Button>
    </Row>
  );
}

/**
 * The Add agent menu: agents Hide AI can use first, then, dimmed under their
 * own heading, the installed agents it cannot use yet with the reason (B38).
 * With nothing to show the button is disabled rather than opening an empty menu.
 */
function AddAgent({ ai, actions, onChange }: { ai: BackgroundAi; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name, state } = useHideAiWords();
  const addable = addableProviders(ai);
  const unusable = unusableProviders(ai);
  return (
    <Row label={<span />} className="py-xs">
      <DropdownMenu>
        <DropdownMenuTrigger asChild disabled={addable.length === 0 && unusable.length === 0}>
          <Button variant="ghost" size="sm" data-ai-add-agent="true">
            <PlusIcon aria-hidden="true" />
            {t("hideAi.addAgent")}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" data-ai-add-menu="true">
          {addable.map((provider) => (
            <DropdownMenuItem
              key={rowKey(provider)}
              data-ai-add-item={rowKey(provider)}
              onSelect={() => {
                onChange();
                actions.addAiFallback(provider.id, provider.device_id ?? null);
              }}
            >
              <AgentLogo agent={provider.agent} label={provider.label} />
              <span className="min-w-0 flex-1">{name(provider)}</span>
              <span className="text-caption text-muted-foreground">{state(provider).text}</span>
            </DropdownMenuItem>
          ))}
          {addable.length > 0 && unusable.length > 0 ? <DropdownMenuSeparator /> : null}
          {unusable.length > 0 ? <DropdownMenuLabel>{t("hideAi.cantUse")}</DropdownMenuLabel> : null}
          {unusable.map((provider) => (
            <DropdownMenuItem key={rowKey(provider)} disabled data-ai-unusable-item={rowKey(provider)}>
              <AgentLogo agent={provider.agent} label={provider.label} />
              <span className="min-w-0 flex-1">{name(provider)}</span>
              <span className="text-caption text-muted-foreground">{state(provider).text}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </Row>
  );
}
