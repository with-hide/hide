import { PlusIcon, XIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentLogo } from "../components/agent-logo";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Group, Row } from "../components/settings-rows";
import { CLI_DEFAULT, addableItems, fallbackItems, itemKey, itemRef, modelChoices, modelToValue, runsOnItem, unusableItems, valueToModel, type AiItem, type ProfileLists } from "../hideAi";
import { useInterfaceTranslation } from "../i18n/client";
import type { BackgroundAi } from "../snapshot";
import { ProviderState, useHideAiWords } from "./hideAiParts";

/**
 * "If <Runs on> can't answer" (PRD settings-cleanup D-16, D-17, B38 to B40):
 * the items the operator added, in the order they were added, each with its
 * own model and a remove; Add agent offers only items Hide AI can use, another
 * profile of the Runs on agent among them (PRD agent-profiles-phase2 B16).
 * There is no switch and no reordering: an empty list is no fallback, and a
 * different order is a remove and an add.
 */
export function HideAiFallback({ ai, lists, actions, onChange }: { ai: BackgroundAi; lists: ProfileLists; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name } = useHideAiWords();
  const chosen = runsOnItem(ai, lists);
  if (!chosen) return null;
  const entries = fallbackItems(ai, lists);
  const agent = name(chosen);
  return (
    <Group
      title={t("hideAi.fallbackTitle", { agent })}
      note={[t("hideAi.fallbackNote", { agent }), entries.length === 0 ? t("hideAi.fallbackEmpty", { agent }) : null].filter(Boolean).join(" ")}
      data-settings-group="hide-ai-fallback"
    >
      {entries.map((entry, index) => (
        <FallbackRow key={itemKey(entry.item)} index={index} item={entry.item} model={entry.model} actions={actions} onChange={onChange} />
      ))}
      <AddAgent ai={ai} lists={lists} actions={actions} onChange={onChange} />
    </Group>
  );
}

function FallbackRow({ index, item, model, actions, onChange }: { index: number; item: AiItem; model: string; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name } = useHideAiWords();
  const choices = item.missing ? [] : modelChoices(item, model);
  const key = itemKey({ ...itemRef(item), device: null });
  return (
    <Row
      data-ai-fallback={key}
      data-ai-missing={item.missing ? "true" : undefined}
      label={
        // A missing profile's entry stays, dimmed, until the operator removes it: Hide never drops it (B21).
        <span className={item.missing ? "flex min-w-0 items-center gap-sm opacity-(--opacity-dimmed)" : "flex min-w-0 items-center gap-sm"}>
          <span aria-hidden="true" className="w-md shrink-0 text-center font-mono text-body text-muted-foreground" data-ai-fallback-order="true">
            {index + 1}
          </span>
          <AgentLogo agent={item.agent} label={item.label} />
          <span className="min-w-0 break-words">{name(item)}</span>
        </span>
      }
    >
      {item.missing ? null : <ProviderState provider={item} />}
      <Select
        value={modelToValue(model)}
        disabled={choices.length < 2}
        onValueChange={(value) => {
          onChange();
          actions.chooseAi(itemRef(item), valueToModel(value));
        }}
      >
        <SelectTrigger aria-label={t("hideAi.fallbackModelAria", { agent: name(item) })} data-ai-fallback-model={key}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {choices.map((choice) => (
            <SelectItem key={choice} value={choice}>
              {choice === CLI_DEFAULT ? t("hideAi.cliDefault") : (item.model_labels?.[choice] ?? choice)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t("hideAi.removeAria", { agent: name(item) })}
        data-ai-fallback-remove={key}
        onClick={() => {
          onChange();
          actions.removeAiFallback(itemRef(item));
        }}
      >
        <XIcon aria-hidden="true" />
      </Button>
    </Row>
  );
}

/**
 * The Add agent menu: items Hide AI can use first, then, dimmed under their
 * own heading, the installed items it cannot use yet with the reason (B38).
 * With nothing to show the button is disabled rather than opening an empty menu.
 */
function AddAgent({ ai, lists, actions, onChange }: { ai: BackgroundAi; lists: ProfileLists; actions: Actions; onChange: () => void }) {
  const { t } = useInterfaceTranslation();
  const { name, state } = useHideAiWords();
  const addable = addableItems(ai, lists);
  const unusable = unusableItems(ai, lists);
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
              key={itemKey(provider)}
              data-ai-add-item={itemKey(provider)}
              onSelect={() => {
                onChange();
                actions.addAiFallback(itemRef(provider));
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
            <DropdownMenuItem key={itemKey(provider)} disabled data-ai-unusable-item={itemKey(provider)}>
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
