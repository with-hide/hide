import { PlusIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentAdapter } from "../agentAdapters";
import { catalogFor, providerLabel, takesModel, PROVIDER_KINDS, type ProviderKind } from "../agentPicker";
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
import { CANDIDATE_LIMIT, CAP_CHOICES, DEFAULT_CONDUCTOR, STRENGTHS, addCandidate, candidateAgent, effortsOf, replaceCandidate } from "../conductorSettings";
import type { BackgroundAi, ConductorCandidate, ConductorSettings, MissionScope, MissionStrength } from "../snapshot";
import type { CoreError } from "../coreError";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";
import { composingKey } from "../keys";

// The Conductor defaults' controls (PRD software-factory-conductor D-31, B48,
// B49), shared by Settings › Conductor and the Mission tool's defaults (PRD
// mission-tool D-03, B9): one value, the same edit events and the same
// refusal words, each place laying the controls out at its own width. Every
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

/** A refused edit, in Settings › Conductor's words, where it was made. */
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

/** Whether a checker comes from another company than the node's worker. */
export function OtherVendorControl({ settings, send }: Pick<ConductorEdit, "settings" | "send">) {
  const { t } = useInterfaceTranslation();
  return <Switch checked={settings.other_vendor} aria-label={t("conductor.otherVendor")} onCheckedChange={(other_vendor) => send({ other_vendor })} data-conductor-other-vendor={String(settings.other_vendor)} />;
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

/** Every candidate's row, each edit sending the whole list as Settings › Conductor does. */
export function CandidateRows({ settings, send, layout }: Pick<ConductorEdit, "settings" | "send"> & { layout: CandidateLayout }) {
  return (
    <>
      {settings.candidates.map((candidate, index) => (
        <CandidateRow key={index} candidate={candidate} layout={layout} onChange={(next) => send({ candidates: replaceCandidate(settings, index, next).candidates })} />
      ))}
    </>
  );
}

/** A candidate's cells as one row of the settings page's grid, or stacked on lines of their own in a narrow tool column. */
export type CandidateLayout = "grid" | "stack";

/** One candidate: the agent, its model where it takes one, its effort automatic or fixed, and what it is for. */
function CandidateRow({ candidate, layout, onChange }: { candidate: ConductorCandidate; layout: CandidateLayout; onChange: (next: ConductorCandidate | null) => void }) {
  const { t } = useInterfaceTranslation();
  const kind = candidateAgent(candidate.agent);
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const installed = useShellStore((s) => {
    const id = agentAdapter(candidate.agent)?.id;
    const row = s.rest?.navigator?.devices?.find((device) => device.kind !== "remote")?.kit?.agents?.find((agent) => agent.id === id);
    return row ? row.availability === "available" : null;
  });
  const [purpose, setPurpose] = useState(candidate.purpose);
  useEffect(() => setPurpose(candidate.purpose), [candidate.purpose]);
  const efforts = effortsOf(candidate.agent);
  const catalog = modelCatalog(ai, kind, t);
  const label = kind ? providerLabel(kind) : candidate.agent;
  const agent = (
    <Select value={kind ?? candidate.agent} onValueChange={(value) => onChange({ agent: value, purpose: candidate.purpose })}>
      <SelectTrigger aria-label={t("conductor.column.agent")} className="w-full min-w-0">
        <SelectValue>
          <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={kind ?? candidate.agent} /><span className="truncate">{label}</span></span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {PROVIDER_KINDS.map((option) => (
          <SelectItem key={option} value={option} data-conductor-agent-option={option}>
            <AgentMark kind={option} />{providerLabel(option)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
  const remove = <Button variant="ghost" size="icon-sm" aria-label={t("conductor.remove", { agent: label })} onClick={() => onChange(null)} data-candidate-remove="true"><XIcon /></Button>;
  const missing = <span className={cn("text-caption text-muted-foreground", layout === "grid" && "col-span-3")} data-candidate-missing="true">{t("conductor.notInstalled")}</span>;
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
  const data = { "data-conductor-candidate": candidate.agent, "data-candidate-installed": installed === false ? "false" : undefined };
  if (layout === "stack") {
    return (
      <div className={cn("flex min-w-0 flex-col gap-xs border-t border-border py-sm", installed === false && "opacity-(--opacity-dimmed)")} {...data}>
        <div className="flex min-w-0 items-center gap-xs"><div className="min-w-0 flex-1">{agent}</div>{remove}</div>
        {installed === false ? missing : (
          <>
            <div className="flex min-w-0 flex-wrap items-center gap-xs"><div className="min-w-0 flex-1">{model}</div>{effort}</div>
            {purposeInput}
          </>
        )}
      </div>
    );
  }
  return (
    <div className={cn("col-span-full grid grid-cols-subgrid items-center border-t border-border px-md py-xs", installed === false && "opacity-(--opacity-dimmed)")} {...data}>
      {agent}
      {installed === false ? missing : <>{model}{effort}{purposeInput}</>}
      {remove}
    </div>
  );
}

/** The start panel's models for an agent that takes one, with why it cannot list them; null for an agent that takes none. */
function modelCatalog(ai: BackgroundAi | undefined, kind: ProviderKind | null, t: (key: string, values?: Record<string, string>) => string) {
  const catalog = kind && takesModel(kind) ? catalogFor(ai, kind) : null;
  if (!catalog) return null;
  return {
    models: catalog.state === "ready" ? catalog.models : [],
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

/** The start panel's model list for the agent (B49), disabled with why when it cannot be read. */
function ModelMenu({ kind, value, models, reason, loading, onChange }: { kind: ProviderKind; value: string | null; models: readonly string[]; reason: string | null; loading: boolean; onChange: (model: string | null) => void }) {
  const { t } = useInterfaceTranslation();
  const listed = value && !models.includes(value) ? [value, ...models] : models;
  const menu = (
    <Select value={value ?? DEFAULT_MODEL} disabled={reason !== null || loading} onValueChange={(next) => onChange(next === DEFAULT_MODEL ? null : next)}>
      <SelectTrigger aria-label={t("conductor.column.model")} className="w-full min-w-0" data-candidate-model={value ?? ""} data-candidate-model-kind={kind}><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT_MODEL}>{t("conductor.defaultModel")}</SelectItem>
        {listed.map((model) => <SelectItem key={model} value={model}>{model}</SelectItem>)}
      </SelectContent>
    </Select>
  );
  return reason ? <Hint label={reason}><span className="min-w-0" data-candidate-models-unavailable="true">{menu}</span></Hint> : menu;
}
