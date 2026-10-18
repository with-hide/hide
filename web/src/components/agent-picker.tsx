import type { TFunction } from "i18next";
import { useEffect, type ReactNode } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { catalogFor, modelToSend, selectKind, providerLabel, takesModel, PROVIDER_KINDS, type AgentKind, type AgentSelection, type ModelCatalog, type ProviderKind } from "../agentPicker";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";
import { cn } from "../lib/utils";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./ui/select";
import { Hint } from "./ui/tooltip";

// The one kind and model control every start surface shares (PRD
// home-device-rail D-18, D-20): ⌘N's panel, New worktree, Start from an issue
// and Delegate. The kind menu carries the provider marks; the model menu is the
// chosen kind's catalog. Both are controlled by the surface, which starts
// from the remembered choice (`rememberedSelection`) and sends what is
// chosen. While any picker shows, the catalog is observed.

/** Radix items cannot carry an empty value; this stands for the CLI's own default. */
const DEFAULT_ITEM = "__cli_default";

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
  withTerminal = false,
  disabled = false,
  kindNote,
  className,
}: {
  actions: Actions;
  value: AgentSelection;
  onChange: (next: AgentSelection) => void;
  /** New worktree offers a tab with no agent as the kind menu's first item; it is never remembered. */
  withTerminal?: boolean;
  disabled?: boolean;
  /** A word about the chosen agent, drawn right after its menu (the start panel's sign-in mark). */
  kindNote?: ReactNode;
  className?: string;
}) {
  useCatalogObservation(actions);
  const start = useShellStore((s) => s.rest?.ui_state?.agent_start);
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  return (
    <AgentPickerView
      value={value}
      catalog={value.kind === "terminal" || !takesModel(value.kind) ? null : catalogFor(ai, value.kind)}
      onKind={(kind) => onChange(selectKind(kind, start))}
      onModel={(model) => onChange({ kind: value.kind, model })}
      withTerminal={withTerminal}
      disabled={disabled}
      kindNote={kindNote}
      className={className}
    />
  );
}

/** The two menus over what the store says; the connected picker above supplies the catalog and the remembered models. */
export function AgentPickerView({
  value,
  catalog,
  onKind,
  onModel,
  withTerminal = false,
  disabled = false,
  kindNote,
  className,
}: {
  value: AgentSelection;
  /** The chosen kind's catalog; null for a terminal, which has no model. */
  catalog: ModelCatalog | null;
  onKind: (kind: AgentKind) => void;
  onModel: (model: string | null) => void;
  withTerminal?: boolean;
  disabled?: boolean;
  kindNote?: ReactNode;
  className?: string;
}) {
  const { t } = useInterfaceTranslation();
  const kinds: readonly AgentKind[] = withTerminal ? ["terminal", ...PROVIDER_KINDS] : PROVIDER_KINDS;
  return (
    <div className={cn("flex min-w-0 gap-xs", className)} data-agent-picker="true">
      <Select value={value.kind} disabled={disabled} onValueChange={(next) => onKind(next as AgentKind)}>
        <SelectTrigger aria-label={t("agentPicker.kind")} className="w-auto min-w-0" data-agent-kind={value.kind}>
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
      {kindNote}
      {value.kind === "terminal" || catalog === null ? null : (
        <ModelSelect kind={value.kind} catalog={catalog} disabled={disabled} model={modelToSend(value)} onChange={onModel} />
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
}: {
  kind: ProviderKind;
  catalog: ModelCatalog;
  disabled: boolean;
  /** The model a start sends, or null for the CLI default. */
  model: string | null;
  onChange: (model: string | null) => void;
}) {
  const { t } = useInterfaceTranslation();
  const reason = catalogReason(catalog, t);
  const disabled = pickerDisabled || reason !== null;
  const models = catalog.state === "ready" ? catalog.models : [];
  const trigger = (
    <SelectTrigger aria-label={t("common.model")} className="w-auto min-w-0" data-agent-model={shown ?? ""} data-agent-model-kind={kind}>
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
            {id}
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
