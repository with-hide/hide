import { PlusIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentAdapter, agentProductName } from "../agentAdapters";
import { catalogFor, modelLabel, providerLabel, takesModel, type ProfileItem, type ProviderKind } from "../agentPicker";
import { itemName, missingItemName, type AgentItem } from "../agentProfiles";
import { Note } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { CANDIDATE_LIMIT, CAP_CHOICES, DEFAULT_CONDUCTOR, STRENGTHS, addCandidate, candidateAgent, candidateGone, candidateItems, candidateProfile, candidateProfileItem, effortsOf, pickItem, replaceCandidate } from "../conductorSettings";
import type { BackgroundAi, ConductorCandidate, ConductorSettings, Device, MissionScope, MissionStrength } from "../snapshot";
import type { CoreError } from "../coreError";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";
import { composingKey } from "../keys";

// The Conductor defaults' controls (PRD software-factory-conductor D-31, B48,
// B49), laid out by Settings › Mission: one value, the same edit events and
// the same refusal words. Every
// edit sends the field it changes; the core checks the result and keeps it,
// and a mission already created keeps its own.

/** Radix items cannot carry an empty value; this stands for the CLI's own default model. */
const DEFAULT_MODEL = "__cli_default";

export type ConductorEdit = { settings: ConductorSettings; send: (patch: Partial<ConductorSettings>) => void; error: CoreError | null };

/** The core's defaults, the edit that sends one field, and the core's refusal of the last edit sent from here. */
export function useConductorEdit(actions: Actions): ConductorEdit {
  const settings = useShellStore((s) => s.rest?.ui_state?.conductor_settings) ?? DEFAULT_CONDUCTOR;
  const [sentAt, setSentAt] = useState<number | null>(null);
  const error = useErrorSince(sentAt, ["conductor_settings."]);
  const live = useShellStore((s) => s.connection === "live");
  // The model menus are the start panel's lists, read while the controls show.
  useEffect(() => (live ? actions.observeCatalog() : undefined), [actions, live]);
  const send = (patch: Partial<ConductorSettings>) => {
    setSentAt(Date.now());
    actions.setConductorSettings(patch);
  };
  return { settings, send, error };
}

/** A refused edit, in Settings › Mission's words, where it was made. */
export function ConductorError({ error }: { error: CoreError | null }) {
  const { t } = useInterfaceTranslation();
  if (!error) return null;
  return <Note tone="error" data-conductor-error="true" data-core-error={error.kind}>{coreErrorText(error, t, "coreError.settings.notSaved")}</Note>;
}

/** How many agents a mission runs at once. */
export function CapControl({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  return (
    <Select value={String(settings.concurrency)} onValueChange={(value) => send({ concurrency: Number(value) })}>
      <SelectTrigger aria-label={t("conductor.cap")} className="w-auto" data-conductor-cap={settings.concurrency}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {CAP_CHOICES.map((cap) => <SelectItem key={cap} value={String(cap)}>{cap}</SelectItem>)}
      </SelectContent>
    </Select>
  );
}

/** How far a mission goes when it does not say. */
export function ScopeControl({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  return (
    <RadioGroup value={settings.default_scope} onValueChange={(value) => send({ default_scope: value as MissionScope })} aria-label={t("conductor.scope")} className="flex flex-wrap gap-md" data-conductor-scope={settings.default_scope}>
      {(["pr", "merge"] as const).map((scope) => (
        <label key={scope} className="flex items-center gap-xs text-subhead">
          <RadioGroupItem value={scope} data-conductor-scope-option={scope} />
          {t(`conductor.scope.${scope}`)}
        </label>
      ))}
    </RadioGroup>
  );
}

/** How strongly a new mission is checked. */
export function StrengthControl({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  return (
    <ToggleGroup type="single" value={settings.default_strength} onValueChange={(value) => value && value !== settings.default_strength && send({ default_strength: value as MissionStrength })} aria-label={t("conductor.strength")} data-conductor-strength={settings.default_strength}>
      {STRENGTHS.map((strength) => <ToggleGroupItem key={strength} value={strength} data-conductor-strength-option={strength}>{t(`mission.strength.${strength}`)}</ToggleGroupItem>)}
    </ToggleGroup>
  );
}

/** Whether a done node's worktree is removed once its pull request merged, when it is clean and nothing runs in it. */
export function CleanMergedControl({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  return <Switch checked={settings.clean_merged} aria-label={t("conductor.cleanMerged")} onCheckedChange={(clean_merged) => send({ clean_merged })} data-conductor-clean-merged={String(settings.clean_merged)} />;
}

/** One more candidate, off at the limit with why. */
export function CandidateAdd({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  const full = settings.candidates.length >= CANDIDATE_LIMIT;
  return (
    <Hint label={full ? t("conductor.limit", { count: CANDIDATE_LIMIT }) : t("conductor.add")}>
      <span>
        <Button size="sm" variant="ghost" disabled={full} onClick={() => send({ candidates: addCandidate(settings).candidates })} data-conductor-add="true">
          <PlusIcon />{t("conductor.add")}
        </Button>
      </span>
    </Hint>
  );
}

/** Every candidate's row, each edit sending the whole list as Settings › Mission does. */
export function CandidateRows({ settings, send, layout }: Pick<ConductorEdit, "settings" | "send"> & { layout: CandidateLayout }) {
  // The candidates name this machine's profiles (PRD agent-profiles-phase2 D-08): its list is every row's agent cell.
  const device = useShellStore((s) => s.rest?.navigator?.devices?.find((row) => row.kind !== "remote"));
  const list = useMemo(() => candidateItems(device), [device]);
  return (
    <>
      {settings.candidates.map((candidate, index) => (
        <CandidateRow key={index} candidate={candidate} device={device} list={list} layout={layout} onChange={(next) => send({ candidates: replaceCandidate(settings, index, next).candidates })} />
      ))}
    </>
  );
}

/** A Radix item value for an agent item: its kind and profile id. */
const itemValue = (item: Pick<AgentItem, "kind" | "profile">) => JSON.stringify([item.kind, item.profile]);

/** Radix items cannot carry an empty value; this stands for a row whose item the list no longer holds. */
const NO_ITEM = "__no_item";

/** A candidate's cells as one row of the settings page's grid, or stacked on lines of their own in a narrow tool column. */
export type CandidateLayout = "grid" | "stack";

/**
 * One candidate: the agent item (an agent, or one of its profiles on this
 * machine), its model from that item's list where it takes one, its effort
 * automatic or fixed, and what it is for. A row whose profile this machine
 * no longer lists is dimmed as `없는 프로필` and its item is not in the list
 * to pick again (B15); picking another item mends it.
 */
function CandidateRow({ candidate, device, list, layout, onChange }: { candidate: ConductorCandidate; device: Device | undefined; list: { items: AgentItem[]; named: boolean }; layout: CandidateLayout; onChange: (next: ConductorCandidate | null) => void }) {
  const { t } = useInterfaceTranslation();
  const kind = candidateAgent(candidate.agent);
  const profile = candidateProfile(candidate);
  const gone = candidateGone(candidate, list.items);
  // The start panel's names: the agents' product names once any agent has a profile, as before with none (B1).
  const agentLabel = (of: ProviderKind) => (list.named ? agentProductName(of) : providerLabel(of));
  const named = (item: AgentItem) => itemName(t, agentLabel(item.kind), item.name);
  const item = list.items.find((row) => row.kind === kind && row.profile === profile) ?? null;
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const installed = useShellStore((s) => {
    const id = agentAdapter(candidate.agent)?.id;
    const row = s.rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.kit?.agents?.find((agent) => agent.id === id);
    return row ? row.availability === "available" : null;
  });
  const [purpose, setPurpose] = useState(candidate.purpose);
  useEffect(() => setPurpose(candidate.purpose), [candidate.purpose]);
  const efforts = effortsOf(candidate.agent);
  const catalog = modelCatalog(ai, kind, candidateProfileItem(device, profile), t);
  const label = !kind ? candidate.agent : gone ? missingItemName(t, agentLabel(kind)) : item ? named(item) : agentLabel(kind);
  const agent = (
    <Select value={item ? itemValue(item) : NO_ITEM} onValueChange={(value) => {
      const [nextKind, nextProfile] = JSON.parse(value) as [ProviderKind, string];
      onChange(pickItem(candidate, { kind: nextKind, profile: nextProfile }));
    }}>
      <SelectTrigger aria-label={t("conductor.column.agent")} className="w-full min-w-0" data-candidate-item={gone ? "gone" : profile}>
        <SelectValue>
          <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={kind ?? candidate.agent} /><span className="truncate">{label}</span></span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {list.items.map((option) => (
          <SelectItem key={itemValue(option)} value={itemValue(option)} data-conductor-agent-option={option.kind} data-conductor-item-option={option.profile}>
            <AgentMark kind={option.kind} />{named(option)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
  const remove = <Button variant="ghost" size="icon-sm" aria-label={t("conductor.remove", { agent: label })} onClick={() => onChange(null)} data-candidate-remove="true"><XIcon /></Button>;
  // A row that cannot start says why in place of its other cells: its agent is not installed, or its profile is gone.
  const unusable = gone ? "gone" : installed === false ? "missing" : null;
  const missing = <span className={cn("text-caption text-muted-foreground", layout === "grid" && "col-span-3")} data-candidate-missing={unusable ?? undefined}>{unusable === "gone" ? t("conductor.profileGone") : t("conductor.notInstalled")}</span>;
  const model = catalog === null ? (
    <span className="truncate text-caption text-muted-foreground" data-candidate-no-model="true">{t("conductor.noModel")}</span>
  ) : (
    <ModelMenu kind={kind!} value={candidate.model ?? null} {...catalog} onChange={(next) => onChange({ ...candidate, model: next })} />
  );
  const effort = efforts.length === 0 ? (
    <span className="text-caption text-muted-foreground">-</span>
  ) : (
    <EffortChoice efforts={efforts} value={candidate.effort ?? null} onChange={(next) => onChange({ ...candidate, effort: next })} data-candidate-effort={candidate.effort ?? "auto"} />
  );
  const purposeInput = (
    <Input value={purpose} placeholder={t("conductor.purposePlaceholder")} aria-label={t("conductor.column.purpose")} className="min-w-0" onChange={(event) => setPurpose(event.target.value)}
      onBlur={() => purpose !== candidate.purpose && onChange({ ...candidate, purpose })}
      onKeyDown={(event) => { if (event.key === "Enter" && !composingKey(event)) event.currentTarget.blur(); }}
      data-candidate-purpose="true" />
  );
  const data = { "data-conductor-candidate": candidate.agent, "data-candidate-installed": installed === false ? "false" : undefined, "data-candidate-gone": gone ? "true" : undefined };
  if (layout === "stack") {
    return (
      <div className={cn("flex min-w-0 flex-col gap-xs border-t border-border py-sm", unusable && "opacity-(--opacity-dimmed)")} {...data}>
        <div className="flex min-w-0 items-center gap-xs"><div className="min-w-0 flex-1">{agent}</div>{remove}</div>
        {unusable ? missing : (
          <>
            <div className="flex min-w-0 flex-wrap items-center gap-xs"><div className="min-w-0 flex-1">{model}</div>{effort}</div>
            {purposeInput}
          </>
        )}
      </div>
    );
  }
  return (
    <div className={cn("col-span-full grid grid-cols-subgrid items-center border-t border-border px-md py-xs", unusable && "opacity-(--opacity-dimmed)")} {...data}>
      {agent}
      {unusable ? missing : <>{model}{effort}{purposeInput}</>}
      {remove}
    </div>
  );
}

/** The start panel's models of an item whose agent takes one (its profile's own list, B7), with why it cannot list them; null for an agent that takes none. */
function modelCatalog(ai: BackgroundAi | undefined, kind: ProviderKind | null, profile: ProfileItem | null, t: (key: string, values?: Record<string, string>) => string) {
  const catalog = kind && takesModel(kind) ? catalogFor(ai, kind, profile) : null;
  if (!catalog) return null;
  return {
    models: catalog.state === "ready" ? catalog.models : [],
    label: (model: string) => modelLabel(catalog, model),
    reason: catalog.state === "unavailable" ? t("conductor.modelsUnavailable", { reason: catalog.reason ?? "-" }) : null,
    loading: catalog.state === "loading",
  };
}

/** An effort the Conductor chooses, or one fixed from the agent's own list; fixing one starts at medium where the agent has it. */
function EffortChoice({ efforts, value, onChange, ...data }: { efforts: readonly string[]; value: string | null; onChange: (effort: string | null) => void } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex min-w-0 items-center gap-xs">
      <ToggleGroup type="single" value={value ? "fixed" : "auto"} onValueChange={(next) => next && onChange(next === "fixed" ? (efforts.includes("medium") ? "medium" : efforts[0]!) : null)} aria-label={t("conductor.column.effort")} {...data}>
        <ToggleGroupItem size="sm" value="auto">{t("conductor.auto")}</ToggleGroupItem>
        <ToggleGroupItem size="sm" value="fixed">{t("conductor.fixed")}</ToggleGroupItem>
      </ToggleGroup>
      {value ? (
        <Select value={value} onValueChange={onChange}>
          <SelectTrigger aria-label={t("conductor.column.effort")} className="w-auto shrink-0" size="sm"><SelectValue /></SelectTrigger>
          <SelectContent>{efforts.map((effort) => <SelectItem key={effort} value={effort}>{effort}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
    </span>
  );
}

/** The start panel's model list for the item (B49), each model by the name its agent shows for it, disabled with why when it cannot be read. */
function ModelMenu({ kind, value, models, label, reason, loading, onChange }: { kind: ProviderKind; value: string | null; models: readonly string[]; label: (model: string) => string; reason: string | null; loading: boolean; onChange: (model: string | null) => void }) {
  const { t } = useInterfaceTranslation();
  const listed = value && !models.includes(value) ? [value, ...models] : models;
  const menu = (
    <Select value={value ?? DEFAULT_MODEL} disabled={reason !== null || loading} onValueChange={(next) => onChange(next === DEFAULT_MODEL ? null : next)}>
      <SelectTrigger aria-label={t("conductor.column.model")} className="w-full min-w-0" data-candidate-model={value ?? ""} data-candidate-model-kind={kind}><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT_MODEL}>{t("conductor.defaultModel")}</SelectItem>
        {listed.map((model) => <SelectItem key={model} value={model}>{label(model)}</SelectItem>)}
      </SelectContent>
    </Select>
  );
  return reason ? <Hint label={reason}><span className="min-w-0" data-candidate-models-unavailable="true">{menu}</span></Hint> : menu;
}
