import type { TFunction } from "i18next";
import { useEffect, type ReactNode } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { catalogFor, modelLabel, modelToSend, selectKind, providerLabel, takesModel, PROVIDER_KINDS, type AgentKind, type AgentSelection, type ModelCatalog, type ProviderKind } from "../agentPicker";
import { itemName, profileItem, type StartItem } from "../agentProfiles";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { cn } from "../lib/utils";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "./ui/select";
import { Hint } from "./ui/tooltip";

// The one kind and model control every start surface shares (PRD
// home-device-rail D-18, D-20): ⌘N's panel, New worktree, Start from an issue
// and Delegate. The kind menu carries the provider marks; the model menu is the
// chosen kind's catalog. Both are controlled by the surface, which starts
// from the remembered choice (`rememberedSelection`) and sends what is
// chosen. While any picker shows, the catalog is observed.

/** Radix items cannot carry an empty value; this stands for the CLI's own default. */
const DEFAULT_ITEM = "__cli_default";

/**
 * The agent menu as items (PRD agent-profiles-phase2 D-01): each agent with
 * its machine's profiles, `profile` the chosen item's profile id for the
 * chosen kind, `showLeft` whether each item shows its account's percent left.
 */
export type ItemMenu = { list: readonly StartItem[]; profile: string; showLeft: boolean; onItem: (item: StartItem) => void; defaultOpen?: boolean; /** Why the list is the built-in items alone, worded; drawn under the items as the menu's quiet description. */ note?: string | null };

const itemKey = (kind: ProviderKind | "terminal", profile: string) => `${kind}/${profile}`;

/** Why the model menu has no list to open, for its tooltip; null while there is one. */
function catalogReason(catalog: ModelCatalog, t: TFunction<"translation">): string | null {
  if (catalog.state === "loading") return t("agentPicker.catalogLoading");
  if (catalog.state === "unavailable") return t("agentPicker.catalogUnavailable", { reason: catalog.reason ?? t("agentPicker.catalogEmpty") });
  return null;
}

/** Observes the catalog while the caller shows a picker, again after a reconnect drops hided's side of it. */
function useCatalogObservation(actions: Actions) {
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (!live) return undefined;
    return actions.observeCatalog();
  }, [actions, live]);
}

export function AgentPicker({
  actions,
  value,
  onChange,
  items,
  withTerminal = false,
  disabled = false,
  kindNote,
  variant,
  className,
}: {
  actions: Actions;
  value: AgentSelection;
  onChange: (next: AgentSelection) => void;
  /** The start panel's agent menu as items (D-01); the other surfaces list the agents alone. */
  items?: ItemMenu;
  /** New worktree offers a tab with no agent as the kind menu's first item; it is never remembered. */
  withTerminal?: boolean;
  disabled?: boolean;
  /** A word about the chosen agent, drawn right after its menu (the start panel's sign-in mark). */
  kindNote?: ReactNode;
  /** `ghost` draws both menus as quiet words, for a composer's row (the start panel). */
  variant?: "default" | "ghost";
  className?: string;
}) {
  useCatalogObservation(actions);
  const start = useShellStore((s) => s.rest?.ui_state?.agent_start);
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  return (
    <AgentPickerView
      value={value}
      catalog={value.kind === "terminal" || !takesModel(value.kind) ? null : catalogFor(ai, value.kind, items ? profileItem(items.list, value.kind, items.profile) : null)}
      items={items}
      onKind={(kind) => onChange(selectKind(kind, start))}
      onModel={(model) => onChange({ kind: value.kind, model })}
      withTerminal={withTerminal}
      disabled={disabled}
      kindNote={kindNote}
      variant={variant}
      className={className}
    />
  );
}

/** The two menus over what the store says; the connected picker above supplies the catalog and the remembered models. */
export function AgentPickerView({
  value,
  catalog,
  items,
  onKind,
  onModel,
  withTerminal = false,
  disabled = false,
  kindNote,
  variant,
  className,
}: {
  value: AgentSelection;
  /** The chosen kind's catalog; null for a terminal, which has no model. */
  catalog: ModelCatalog | null;
  items?: ItemMenu;
  onKind: (kind: AgentKind) => void;
  onModel: (model: string | null) => void;
  withTerminal?: boolean;
  disabled?: boolean;
  kindNote?: ReactNode;
  variant?: "default" | "ghost";
  className?: string;
}) {
  const { t } = useInterfaceTranslation();
  const kinds: readonly AgentKind[] = withTerminal ? ["terminal", ...PROVIDER_KINDS] : PROVIDER_KINDS;
  return (
    <div className={cn("flex min-w-0 gap-xs", className)} data-agent-picker="true">
      {items ? (
        <Select
          value={itemKey(value.kind, items.profile)}
          defaultOpen={items.defaultOpen}
          disabled={disabled}
          onValueChange={(next) => {
            const item = items.list.find((row) => itemKey(row.kind, row.profile) === next);
            if (item) items.onItem(item);
          }}
        >
          <SelectTrigger variant={variant} aria-label={t("agentPicker.kind")} className="w-auto min-w-0" data-agent-kind={value.kind} data-agent-item={itemKey(value.kind, items.profile)}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {items.list.map((item) => {
              const key = itemKey(item.kind, item.profile);
              return (
                <SelectItem key={key} value={key} trailing={items.showLeft ? (item.left === null ? "—" : t("agentProfiles.state.left", { percent: item.left })) : undefined} data-agent-kind-option={item.kind} data-agent-item-option={key} data-agent-item-left={item.left ?? ""}>
                  <AgentMark kind={item.kind} profile={item.name} />
                  {itemName(t, items.showLeft ? agentProductName(item.kind) : providerLabel(item.kind), item.name)}
                </SelectItem>
              );
            })}
            {items.note ? (
              <SelectGroup>
                <SelectLabel data-agent-item-note="true" className="max-w-(--size-picker-popover) whitespace-normal break-keep font-normal">{items.note}</SelectLabel>
              </SelectGroup>
            ) : null}
          </SelectContent>
        </Select>
      ) : (
        <Select value={value.kind} disabled={disabled} onValueChange={(next) => onKind(next as AgentKind)}>
          <SelectTrigger variant={variant} aria-label={t("agentPicker.kind")} className="w-auto min-w-0" data-agent-kind={value.kind}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {kinds.map((kind) => (
              <SelectItem key={kind} value={kind} data-agent-kind-option={kind}>
                <AgentMark kind={kind === "terminal" ? null : kind} />
                {kind === "terminal" ? t("agentPicker.terminalOnly") : providerLabel(kind)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      {kindNote}
      {value.kind === "terminal" || catalog === null ? null : (
        <ModelSelect kind={value.kind} catalog={catalog} disabled={disabled} model={modelToSend(value)} onChange={onModel} variant={variant} />
      )}
    </div>
  );
}

function ModelSelect({
  kind,
  catalog,
  disabled: pickerDisabled,
  model: shown,
  onChange,
  variant,
}: {
  kind: ProviderKind;
  catalog: ModelCatalog;
  disabled: boolean;
  /** The model a start sends, or null for the CLI default. */
  model: string | null;
  onChange: (model: string | null) => void;
  variant?: "default" | "ghost";
}) {
  const { t } = useInterfaceTranslation();
  const reason = catalogReason(catalog, t);
  const disabled = pickerDisabled || reason !== null;
  const models = catalog.state === "ready" ? catalog.models : [];
  const trigger = (
    <SelectTrigger variant={variant} aria-label={t("common.model")} className="w-auto min-w-0" data-agent-model={shown ?? ""} data-agent-model-kind={kind}>
      <SelectValue />
    </SelectTrigger>
  );
  const select = (
    <Select value={shown ?? DEFAULT_ITEM} disabled={disabled} onValueChange={(next) => onChange(next === DEFAULT_ITEM ? null : next)}>
      {trigger}
      <SelectContent>
        <SelectItem value={DEFAULT_ITEM} data-agent-model-option="">
          {t("agentPicker.cliDefault")}
        </SelectItem>
        {models.map((id) => (
          <SelectItem key={id} value={id} data-agent-model-option={id}>
            {modelLabel(catalog, id)}
          </SelectItem>
        ))}
        {/* A remembered model the list does not have stays a selectable value, so the trigger names what a start sends. */}
        {shown !== null && !models.includes(shown) ? (
          <SelectItem value={shown} data-agent-model-option={shown}>
            {shown}
          </SelectItem>
        ) : null}
      </SelectContent>
    </Select>
  );
  if (reason === null) return select;
  // A disabled control takes no pointer events; its wrapper carries the tooltip.
  return (
    <Hint label={reason} reveals>
      <span className="flex min-w-0" data-agent-model-reason="true" tabIndex={0}>
        {select}
      </span>
    </Hint>
  );
}
