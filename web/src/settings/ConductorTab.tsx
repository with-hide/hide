import { PlusIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentAdapter } from "../agentAdapters";
import { catalogFor, providerLabel, takesModel, PROVIDER_KINDS, type ProviderKind } from "../agentPicker";
import { Group, Note, Row } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { CANDIDATE_LIMIT, CAP_CHOICES, CHECK_COUNT_CHOICES, DEFAULT_CHECKERS, DEFAULT_CONDUCTOR, addCandidate, candidateAgent, effortsOf, replaceCandidate, replaceCheck, type CheckKind } from "../conductorSettings";
import type { BackgroundAi, ConductorCandidate, ConductorCheckDefault, ConductorSettings, MissionCheckRule, MissionScope } from "../snapshot";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

// Settings › Conductor (PRD software-factory-conductor D-31, B48, B49; PRD
// mission-map-git-criteria B36): how many agents a mission runs at once, how
// far a mission goes when it does not say, the candidates a Conductor chooses
// its workers from, and who reviews and verifies a node when its plan does
// not say. Every edit sends the field it changes; the core checks the result
// and keeps it, and a mission already planned keeps its own.

/** Radix items cannot carry an empty value; this stands for the CLI's own default model. */
const DEFAULT_MODEL = "__cli_default";
/** And this for a checker agent the Conductor chooses. */
const ANY_AGENT = "__conductor";

export function ConductorTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const settings = useShellStore((s) => s.rest?.ui_state?.conductor_settings) ?? DEFAULT_CONDUCTOR;
  const checkers = settings.checkers ?? DEFAULT_CHECKERS;
  const [sentAt, setSentAt] = useState<number | null>(null);
  const error = useErrorSince(sentAt, ["conductor_settings."]);
  const live = useShellStore((s) => s.connection === "live");
  // The model menus are the start panel's lists, read while this page shows.
  useEffect(() => (live ? actions.observeCatalog() : undefined), [actions, live]);
  const send = (patch: Partial<ConductorSettings>) => {
    setSentAt(Date.now());
    actions.setConductorSettings(patch);
  };
  return (
    <div data-settings-conductor="true">
      <Group>
        <Row label={<RowLabel title={t("conductor.cap")} hint={t("conductor.capHint")} />}>
          <Select value={String(settings.concurrency)} onValueChange={(value) => send({ concurrency: Number(value) })}>
            <SelectTrigger aria-label={t("conductor.cap")} className="w-auto" data-conductor-cap={settings.concurrency}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CAP_CHOICES.map((cap) => <SelectItem key={cap} value={String(cap)}>{cap}</SelectItem>)}
            </SelectContent>
          </Select>
        </Row>
        <Row label={<RowLabel title={t("conductor.scope")} hint={t("conductor.scopeHint")} />}>
          <RadioGroup value={settings.default_scope} onValueChange={(value) => send({ default_scope: value as MissionScope })} aria-label={t("conductor.scope")} className="flex flex-wrap gap-md" data-conductor-scope={settings.default_scope}>
            {(["pr", "merge"] as const).map((scope) => (
              <label key={scope} className="flex items-center gap-xs text-subhead">
                <RadioGroupItem value={scope} data-conductor-scope-option={scope} />
                {t(`conductor.scope.${scope}`)}
              </label>
            ))}
          </RadioGroup>
        </Row>
      </Group>
      {error ? <Note tone="error" data-conductor-error="true" data-core-error={error.kind}>{coreErrorText(error, t, "coreError.settings.notSaved")}</Note> : null}
      <Group
        title={t("conductor.candidates")}
        action={
          <Hint label={settings.candidates.length >= CANDIDATE_LIMIT ? t("conductor.limit", { count: CANDIDATE_LIMIT }) : t("conductor.add")}>
            <span>
              <Button size="sm" variant="ghost" disabled={settings.candidates.length >= CANDIDATE_LIMIT} onClick={() => send({ candidates: addCandidate(settings).candidates })} data-conductor-add="true">
                <PlusIcon />{t("conductor.add")}
              </Button>
            </span>
          </Hint>
        }
        note={t("conductor.footer")}
        data-conductor-candidates={String(settings.candidates.length)}
      >
        {settings.candidates.length > 0 ? (
          <div className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.3fr)_minmax(0,1.3fr)_minmax(0,1.4fr)_auto] items-center gap-x-sm px-md py-xs text-caption text-muted-foreground">
            <span>{t("conductor.column.agent")}</span>
            <span>{t("conductor.column.model")}</span>
            <span>{t("conductor.column.effort")}</span>
            <span>{t("conductor.column.purpose")}</span>
            <span />
          </div>
        ) : null}
        {settings.candidates.map((candidate, index) => (
          <CandidateRow key={index} candidate={candidate} onChange={(next) => send({ candidates: replaceCandidate(settings, index, next).candidates })} />
        ))}
      </Group>
      {(["review", "verify"] as const).map((kind) => (
        <CheckGroup key={kind} kind={kind} check={checkers[kind]} last={kind === "verify"} onChange={(next) => send({ checkers: replaceCheck(checkers, kind, next) })} />
      ))}
    </div>
  );
}

function RowLabel({ title, hint }: { title: string; hint: string }) {
  return (
    <span className="flex flex-col">
      <span>{title}</span>
      <span className="text-caption text-muted-foreground">{hint}</span>
    </span>
  );
}

/** One candidate: the agent, its model where it takes one, its effort automatic or fixed, and what it is for. */
function CandidateRow({ candidate, onChange }: { candidate: ConductorCandidate; onChange: (next: ConductorCandidate | null) => void }) {
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
  return (
    <div className={cn("grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.3fr)_minmax(0,1.3fr)_minmax(0,1.4fr)_auto] items-center gap-x-sm px-md py-xs", installed === false && "opacity-(--opacity-dimmed)")} data-conductor-candidate={candidate.agent} data-candidate-installed={installed === false ? "false" : undefined}>
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
      {installed === false ? (
        <span className="col-span-3 text-caption text-muted-foreground" data-candidate-missing="true">{t("conductor.notInstalled")}</span>
      ) : (
        <>
          {catalog === null ? (
            <span className="truncate text-caption text-muted-foreground" data-candidate-no-model="true">{t("conductor.noModel")}</span>
          ) : (
            <ModelMenu kind={kind!} value={candidate.model ?? null} {...catalog} onChange={(model) => onChange({ ...candidate, model })} />
          )}
          {efforts.length === 0 ? (
            <span className="text-caption text-muted-foreground">-</span>
          ) : (
            <EffortChoice efforts={efforts} value={candidate.effort ?? null} onChange={(effort) => onChange({ ...candidate, effort })} data-candidate-effort={candidate.effort ?? "auto"} />
          )}
          <Input value={purpose} placeholder={t("conductor.purposePlaceholder")} aria-label={t("conductor.column.purpose")} className="min-w-0" onChange={(event) => setPurpose(event.target.value)}
            onBlur={() => purpose !== candidate.purpose && onChange({ ...candidate, purpose })}
            onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
            data-candidate-purpose="true" />
        </>
      )}
      <Button variant="ghost" size="icon-sm" aria-label={t("conductor.remove", { agent: label })} onClick={() => onChange(null)} data-candidate-remove="true"><XIcon /></Button>
    </div>
  );
}

/**
 * Who reviews or verifies a node its plan leaves open (B36): the Conductor's
 * judgement, nobody, always one, or several deciding by a rule; the agent,
 * model and effort they start with or the Conductor's choice; and whether a
 * model of another company than the worker's comes first. Only the rows the
 * chosen mode uses show.
 */
function CheckGroup({ kind, check, last, onChange }: { kind: CheckKind; check: ConductorCheckDefault; last: boolean; onChange: (next: ConductorCheckDefault) => void }) {
  const { t } = useInterfaceTranslation();
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const agent = check.agent ? candidateAgent(check.agent) : null;
  const catalog = modelCatalog(ai, agent, t);
  const efforts = check.agent ? effortsOf(check.agent) : [];
  return (
    <Group title={t(`conductor.${kind}`)} note={last ? t("conductor.check.footer") : undefined} data-conductor-check={kind} data-check-mode={check.mode}>
      <Row label={t("conductor.check.mode")}>
        <Select value={check.mode} onValueChange={(mode) => onChange({ ...check, mode: mode as ConductorCheckDefault["mode"] })}>
          <SelectTrigger aria-label={t("conductor.check.mode")} className="w-auto" data-check-field="mode"><SelectValue /></SelectTrigger>
          <SelectContent>
            {(["conductor", "none", "one", "many"] as const).map((mode) => <SelectItem key={mode} value={mode}>{t(`conductor.check.mode.${mode}`)}</SelectItem>)}
          </SelectContent>
        </Select>
      </Row>
      {check.mode === "many" ? (
        <Row label={t("conductor.check.count")}>
          <Select value={String(check.count)} onValueChange={(count) => onChange({ ...check, count: Number(count) })}>
            <SelectTrigger aria-label={t("conductor.check.count")} className="w-auto" data-check-field="count"><SelectValue /></SelectTrigger>
            <SelectContent>{CHECK_COUNT_CHOICES.map((count) => <SelectItem key={count} value={String(count)}>{count}</SelectItem>)}</SelectContent>
          </Select>
          <Select value={check.rule} onValueChange={(rule) => onChange({ ...check, rule: rule as MissionCheckRule })}>
            <SelectTrigger aria-label={t("conductor.check.rule")} className="w-auto" data-check-field="rule"><SelectValue /></SelectTrigger>
            <SelectContent>
              {(["all", "majority"] as const).map((rule) => <SelectItem key={rule} value={rule}>{t(`conductor.check.rule.${rule}`)}</SelectItem>)}
            </SelectContent>
          </Select>
        </Row>
      ) : null}
      {check.mode === "none" ? null : (
        <>
          <Row label={t("conductor.column.agent")}>
            <Select value={check.agent ?? ANY_AGENT} onValueChange={(value) => onChange({ ...check, agent: value === ANY_AGENT ? null : value })}>
              <SelectTrigger aria-label={t("conductor.column.agent")} className="w-auto min-w-0" data-check-field="agent">
                <SelectValue>
                  {check.agent ? <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={agent ?? check.agent} /><span className="truncate">{agent ? providerLabel(agent) : check.agent}</span></span> : t("conductor.check.agentAny")}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ANY_AGENT}>{t("conductor.check.agentAny")}</SelectItem>
                {PROVIDER_KINDS.map((option) => <SelectItem key={option} value={option}><AgentMark kind={option} />{providerLabel(option)}</SelectItem>)}
              </SelectContent>
            </Select>
            {agent && catalog ? <ModelMenu kind={agent} value={check.model} {...catalog} onChange={(model) => onChange({ ...check, model })} /> : null}
            {efforts.length > 0 ? <EffortChoice efforts={efforts} value={check.effort} onChange={(effort) => onChange({ ...check, effort })} data-check-field="effort" /> : null}
          </Row>
          <Row label={t("conductor.check.otherVendor")}>
            <Switch checked={check.other_vendor} aria-label={t("conductor.check.otherVendor")} onCheckedChange={(other_vendor) => onChange({ ...check, other_vendor })} data-check-field="other_vendor" />
          </Row>
        </>
      )}
    </Group>
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
          <SelectTrigger aria-label={t("conductor.column.effort")} className="w-auto min-w-0" size="sm"><SelectValue /></SelectTrigger>
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
