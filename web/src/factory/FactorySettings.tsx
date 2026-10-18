import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { BanIcon, ChevronRightIcon, PauseIcon, PlayIcon, PlusIcon, SparklesIcon, SquareTerminalIcon, UserIcon, XIcon } from "lucide-react";
import type { Actions } from "../actions";
import { agentAdapter, agentProductName } from "../agentAdapters";
import { PROVIDER_KINDS } from "../agentPicker";
import { AgentPicker } from "../components/agent-picker";
import { formatElapsed } from "../components/elapsed";
import { Disclosure, Group, Note, Row } from "../components/settings-rows";
import { Button } from "../components/ui/button";
import { Checkbox } from "../components/ui/checkbox";
import { Input } from "../components/ui/input";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Switch } from "../components/ui/switch";
import { CLI_DEFAULT, modelChoices, providerById } from "../hideAi";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import type { MessageKey } from "../i18n/catalogs";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { FactoryCommand } from "./commands";
import { MODE_LABEL, MODE_LINE } from "./labels";
import { OBSERVER_MODES, type FactoryAi, type FactoryView, type ObserverMode, type WorkerCandidate } from "./model";
import { Refusal } from "./Decisions";
import { useFactoryRequest } from "./request";

/** A Factory's settings as `hide factory config` answers them (docs/factory.md, Settings). */
export type FactoryConfig = {
  verification: { kind: "ci"; checks: string[] } | { kind: "commands"; commands: string[] } | { kind: "none" };
  merge_mode: "auto" | "manual";
  merge_method: "merge" | "squash" | "rebase";
  quick_check: string | null;
  question_deadline_ms: number;
  stall_ms: number;
  no_report_ms: number;
  watch_interval_ms: number;
  watch_daily_limit: number;
  outside_read_ms: number;
  cancel_keep_ms: number;
  done_fold_ms: number;
  archive_fold_ms: number;
  new_task_limit: number;
  verify_failure_limit: number;
  verify_timeout_ms: number;
  disk_floor_bytes: number;
  /** The first candidate's agent; the only candidate while `workers` is empty (D-42). */
  default_runtime: string;
  workers: WorkerCandidate[];
  observer_mode: ObserverMode;
  observer_daily_limit: number;
  factory_ai: FactoryAi | null;
  harness: { name: string; instructions: string } | null;
  autonomy: { id: string; description: string; enabled: boolean }[];
  autonomy_diff_limit: number;
  recovery: string[];
  risk_paths: string[];
  checks: { at: "intake" | "after_done" | "periodic"; instruction: string }[];
  prd_in_issue: boolean;
  macos_notifications: boolean;
  worker_args: Record<string, string[]>;
};

type ConfigAnswer = { config: FactoryConfig; machine: { max_workers: number } };

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const GIB = 1024 * 1024 * 1024;
/** The most worker candidates a Factory keeps (D-41). */
const WORKER_LIMIT = 5;

/** A number the engine keeps, how to read it from the config, and the words for its unit after the field. */
type NumberSetting = { key: string; read: (answer: ConfigAnswer) => number; unit: MessageKey };

const per = (unit: number, field: keyof FactoryConfig) => (answer: ConfigAnswer) => Math.round((answer.config[field] as number) / unit);
const plain = (field: keyof FactoryConfig) => (answer: ConfigAnswer) => answer.config[field] as number;

const DEADLINE: NumberSetting = { key: "question_deadline_hours", read: per(HOUR, "question_deadline_ms"), unit: "factory.settings.adv.hours" };
const STALL: NumberSetting = { key: "stall_minutes", read: per(MINUTE, "stall_ms"), unit: "factory.settings.adv.quiet" };
const NO_REPORT: NumberSetting = { key: "no_report_minutes", read: per(MINUTE, "no_report_ms"), unit: "factory.settings.adv.noReport" };
const OBSERVER_LIMIT: NumberSetting = { key: "observer_daily_limit", read: plain("observer_daily_limit"), unit: "factory.settings.adv.perDay" };
const WATCH_INTERVAL: NumberSetting = { key: "watch_interval_minutes", read: per(MINUTE, "watch_interval_ms"), unit: "factory.settings.adv.every" };
const WATCH_LIMIT: NumberSetting = { key: "watch_daily_limit", read: plain("watch_daily_limit"), unit: "factory.settings.adv.perDay" };
const CANCEL_KEEP: NumberSetting = { key: "cancel_keep_days", read: per(DAY, "cancel_keep_ms"), unit: "factory.settings.adv.cancelDays" };
const DONE_FOLD: NumberSetting = { key: "done_fold_days", read: per(DAY, "done_fold_ms"), unit: "factory.settings.adv.doneDays" };
const ARCHIVE_FOLD: NumberSetting = { key: "archive_fold_days", read: per(DAY, "archive_fold_ms"), unit: "factory.settings.adv.archiveDays" };

/** A number whose label names its unit, as the rows under 고급 설정's subheads read (B36). */
type LabelledNumber = { key: string; label: MessageKey; read: (answer: ConfigAnswer) => number };

const VERIFY_LIMITS: LabelledNumber[] = [
  { key: "verify_failure_limit", label: "factory.settings.verifyFailureLimit", read: plain("verify_failure_limit") },
  { key: "verify_timeout_minutes", label: "factory.settings.verifyTimeout", read: per(MINUTE, "verify_timeout_ms") },
];
const NEW_TASK_LIMIT: LabelledNumber = { key: "new_task_limit", label: "factory.settings.newTaskLimit", read: plain("new_task_limit") };
const DISK_FLOOR: LabelledNumber = { key: "disk_floor_gb", label: "factory.settings.diskFloor", read: per(GIB, "disk_floor_bytes") };
const OUTSIDE_READ: LabelledNumber = { key: "outside_read_minutes", label: "factory.settings.outsideRead", read: per(MINUTE, "outside_read_ms") };
const AUTONOMY_DIFF: LabelledNumber = { key: "autonomy_diff_limit", label: "factory.settings.autonomyDiffLimit", read: plain("autonomy_diff_limit") };

const CHECK_POINTS = ["intake", "after_done", "periodic"] as const;
const CHECK_LABEL: Record<(typeof CHECK_POINTS)[number], MessageKey> = {
  intake: "factory.settings.checkAt.intake",
  after_done: "factory.settings.checkAt.after_done",
  periodic: "factory.settings.checkAt.periodic",
};

const RECOVERY: readonly { id: string; label: MessageKey }[] = [
  { id: "remove_finished_worktrees", label: "factory.settings.recovery.remove_finished_worktrees" },
  { id: "restart_worker", label: "factory.settings.recovery.restart_worker" },
  { id: "sleep_wake_worker", label: "factory.settings.recovery.sleep_wake_worker" },
  { id: "switch_runtime", label: "factory.settings.recovery.switch_runtime" },
  { id: "retry_reads_and_reconnect", label: "factory.settings.recovery.retry_reads_and_reconnect" },
];

/**
 * What each choice hands to Factory AI and what stays the person's (D-03,
 * D-14, D-21, D-32): the engine's mode table in the words the settings show.
 */
const MODE_TABLE: Record<ObserverMode, { me: MessageKey[]; ai: MessageKey[] }> = {
  manual: {
    me: ["factory.settings.decide.technical", "factory.settings.decide.product", "factory.settings.decide.cardFix", "factory.settings.decide.permission", "factory.settings.decide.riskMerge"],
    ai: ["factory.settings.decide.answered"],
  },
  assist: {
    me: ["factory.settings.decide.product", "factory.settings.decide.cardFixProposal", "factory.settings.decide.permission", "factory.settings.decide.riskMerge"],
    ai: ["factory.settings.decide.technical", "factory.settings.decide.answered"],
  },
  autonomous: {
    me: ["factory.settings.decide.permission"],
    ai: ["factory.settings.decide.technical", "factory.settings.decide.product", "factory.settings.decide.cardFix", "factory.settings.decide.riskMerge", "factory.settings.decide.answered"],
  },
};

/**
 * The settings tab (PRD factory-observer B33-B36): with every project shown,
 * the list of Factories and what this Mac runs at once; with one, its four
 * groups (AI에게 맡기기, 작업자, 머지, 그 밖) and the rest folded under 고급 설정.
 * Every value is read from and written through the `config` command, so a
 * change applies from the engine's next judgment.
 */
export function FactorySettings({ factories, filtered, actions }: { factories: FactoryView[]; filtered: boolean; actions: Actions }) {
  if (factories.length === 0) return null;
  return (
    <div className="flex max-w-(--size-settings-sheet-w) flex-col gap-sm px-lg pb-xl" data-factory-settings={filtered ? factories[0]!.id : "all"}>
      {filtered ? <SettingsBody key={factories[0]!.id} factory={factories[0]!} actions={actions} /> : <FactoryList factories={factories} actions={actions} />}
    </div>
  );
}

/** Every Factory on one line each, and the one number they share on this Mac (B36). */
function FactoryList({ factories, actions }: { factories: FactoryView[]; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const read = useFactoryRequest(actions);
  const write = useFactoryRequest(actions);
  const pause = useFactoryRequest(actions);
  const [machine, setMachine] = useState<number | null>(null);
  const first = factories[0]!.project;
  useEffect(() => {
    read.send({ verb: "config", project: first, set: [] });
  }, [first]);
  useEffect(() => {
    for (const state of [read.state, write.state]) if (state.phase === "taken") setMachine((state.answer as unknown as ConfigAnswer).machine.max_workers);
  }, [read.state, write.state]);
  return (
    <>
      <Group title={t("factory.settings.factories")} caption={t("factory.settings.factoriesCaption")} data-factory-settings-group="factories">
        {factories.map((view) => {
          const turn = view.my_turn;
          return (
            <div key={view.id} className="flex min-w-0 items-center gap-md px-md py-sm" data-factory-list-row={view.id}>
              <span className="w-[calc(var(--size-control-lg)*3)] min-w-0 shrink truncate text-subhead font-semibold">{view.project_name}</span>
              <span className="flex w-[calc(var(--size-control-lg)*3)] shrink-0 items-center gap-xxs text-caption text-subtle-foreground" data-factory-list-paused={view.paused ? "true" : "false"}>
                {view.paused ? <PauseIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <span aria-hidden="true" className="size-(--size-status-mark) rounded-full bg-agent-working" />}
                {view.paused ? t("factory.settings.paused") : t("factory.settings.running")}
              </span>
              <span className="flex w-[calc(var(--size-control-lg)*2)] shrink-0 items-center gap-xxs text-caption text-subtle-foreground">
                <SparklesIcon aria-hidden="true" className="size-(--size-icon-sm)" />
                {t(MODE_LABEL[view.observer_mode])}
              </span>
              <span className="flex shrink-0 items-center gap-xxs text-caption text-subtle-foreground">
                <SquareTerminalIcon aria-hidden="true" className="size-(--size-icon-sm)" />
                {t("factory.settings.workerCount", { count: view.workers.length })}
              </span>
              <span className="flex-1" />
              <span className={cn("shrink-0 text-caption", turn > 0 ? "text-warning" : "text-muted-foreground")}>{t("factory.settings.turnCount", { count: turn })}</span>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={view.paused ? t("factory.resume") : t("factory.pause")}
                data-factory-list-pause={view.id}
                disabled={pause.state.phase === "sending"}
                onClick={() => pause.send({ verb: view.paused ? "resume_factory" : "pause_factory", project: view.project })}
              >
                {view.paused ? <PlayIcon /> : <PauseIcon />}
              </Button>
              <Button variant="ghost" size="icon-sm" aria-label={view.project_name} data-factory-list-open={view.id} onClick={() => useUiStore.getState().setFactoryPlace({ factory: view.id })}>
                <ChevronRightIcon />
              </Button>
            </div>
          );
        })}
      </Group>
      <Refusal state={pause.state} />
      <Group title={t("factory.settings.machine")} data-factory-settings-group="machine">
        <Row label={t("factory.settings.machineWorkers")} detail={<Note>{t("factory.settings.machineWorkersDetail")}</Note>}>
          {machine === null ? null : <TextField reset={0} value={String(machine)} numeric valid={(text) => Number.isInteger(Number(text)) && Number(text) >= 1} onCommit={(value) => write.send({ verb: "config", project: first, set: [["max_workers", value]] })} data="max_workers" />}
        </Row>
      </Group>
      <Refusal state={write.state} />
    </>
  );
}

/** The agents whose start arguments the settings offer: every agent Factory can start, and any the config already names. */
function argAgents(config: FactoryConfig): string[] {
  return [...new Set([...PROVIDER_KINDS, ...Object.keys(config.worker_args)])];
}

/** A config's worker candidates; a Factory made before candidates reads as one of its default agent (D-42). */
function candidatesOf(config: FactoryConfig): WorkerCandidate[] {
  return config.workers.length > 0 ? config.workers : [{ agent: config.default_runtime, description: "" }];
}

function SettingsBody({ factory, actions }: { factory: FactoryView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const background = useShellStore((s) => s.rest?.status?.background_ai);
  const read = useFactoryRequest(actions);
  const write = useFactoryRequest(actions);
  const [answer, setAnswer] = useState<ConfigAnswer | null>(null);
  useEffect(() => {
    read.send({ verb: "config", project: factory.project, set: [] });
  }, [factory.project]);
  useEffect(() => {
    if (read.state.phase === "taken") setAnswer(read.state.answer as unknown as ConfigAnswer);
  }, [read.state]);
  const [refusals, setRefusals] = useState(0);
  const [writes, setWrites] = useState(0);
  useEffect(() => {
    if (write.state.phase === "refused") setRefusals((count) => count + 1);
    if (write.state.phase !== "taken") return;
    setWrites((count) => count + 1);
    // A config write answers with the config; a check or a close answers
    // with a message, so the config is read again.
    const taken = write.state.answer as Record<string, unknown>;
    if ("config" in taken) setAnswer(taken as unknown as ConfigAnswer);
    else read.send({ verb: "config", project: factory.project, set: [] });
  }, [write.state]);
  if (answer === null) {
    return read.state.phase === "refused" ? <Refusal state={read.state} /> : <div className="h-(--size-control-lg) rounded-md bg-muted" aria-busy="true" aria-label={t("factory.loading")} data-factory-settings-loading="true" />;
  }
  const config = answer.config;
  // With Hide AI off a risk path is always the person's to merge (B37).
  const aiOff = background?.enabled === false;
  const set = (key: string, value: string) => write.send({ verb: "config", project: factory.project, set: [[key, value]] });
  const send = (command: FactoryCommand) => write.send(command);
  // A number inside a sentence: the field, then the words for its unit (B36).
  const amount = (row: NumberSetting) => (
    <span className="flex items-center gap-xs">
      <TextField reset={refusals} value={String(row.read(answer))} numeric valid={(text) => Number.isInteger(Number(text)) && Number(text) >= 0} onCommit={(value) => set(row.key, value)} data={row.key} />
      <span className="text-caption text-muted-foreground">{t(row.unit)}</span>
    </span>
  );
  // A number whose label carries its unit: the rows under 고급 설정's subheads.
  const number = (row: LabelledNumber) => (
    <Row key={row.key} label={t(row.label)}>
      <TextField reset={refusals} value={String(row.read(answer))} numeric valid={(text) => Number.isInteger(Number(text)) && Number(text) >= 0} onCommit={(value) => set(row.key, value)} data={row.key} />
    </Row>
  );
  const verify = config.verification.kind === "commands" ? config.verification.commands.join(" &&& ") : "";
  // Closing follows the lifecycle states, independent of board presentation.
  const running = factory.columns.some((column) => column.cards.some((card) => !["drafting", "waiting", "done", "cancelled"].includes(card.state)));
  return (
    // The last write's phase, so a reader can wait for the engine's answer.
    <div className="contents" data-factory-settings-write={write.state.phase}>
      <Refusal state={write.state} />
      <ObserverGroup factory={factory} config={config} reset={refusals} set={set} actions={actions} />
      <WorkersGroup config={config} machine={answer.machine.max_workers} reset={refusals} set={set} actions={actions} />
      <Group title={t("factory.settings.merge")} data-factory-settings-group="merge">
        <Row label={t("factory.settings.autoMerge")} detail={<Note>{t("factory.settings.autoMergeDetail")}</Note>}>
          <Switch checked={config.merge_mode === "auto"} disabled={!factory.auto_merge_available && config.merge_mode !== "auto"} aria-label={t("factory.settings.autoMerge")} data-factory-setting="merge_mode" onCheckedChange={(on) => set("merge_mode", on ? "auto" : "manual")} />
        </Row>
        <Row label={t("factory.settings.verification")}>
          <span className="min-w-0 text-right text-body text-subtle-foreground [overflow-wrap:anywhere]" data-factory-settings-verification={config.verification.kind}>
            {config.verification.kind === "ci" ? t("factory.settings.verificationCi", { checks: config.verification.checks.join(", ") }) : config.verification.kind === "commands" ? t("factory.settings.verificationCommands", { commands: config.verification.commands.join(", ") }) : t("factory.create.none")}
          </span>
        </Row>
        <Row label={t("factory.settings.riskPaths")} detail={<Note data-factory-risk-note="true">{config.observer_mode === "autonomous" && !aiOff ? t("factory.settings.riskAi") : t("factory.settings.riskMine")}</Note>}>
          <TextField reset={refusals} value={config.risk_paths.join(", ")} mono onCommit={(value) => set("risk_paths", value)} data="risk_paths" />
        </Row>
      </Group>
      <Group title={t("factory.settings.other")} data-factory-settings-group="other">
        <Row label={t("factory.settings.macosNotifications")} detail={<Note>{t("factory.settings.macosNotificationsDetail")}</Note>}>
          <Switch checked={config.macos_notifications} aria-label={t("factory.settings.macosNotifications")} data-factory-setting="macos_notifications" onCheckedChange={(on) => set("macos_notifications", on ? "on" : "off")} />
        </Row>
        <Disclosure title={t("factory.settings.advanced")} summary={t("factory.settings.advancedSummary")} data-factory-settings-group="advanced">
          <Row label={t("factory.settings.adv.deadline")} detail={<Note>{t("factory.settings.adv.deadlineDetail")}</Note>}>
            {amount(DEADLINE)}
          </Row>
          <Row label={t("factory.settings.adv.stop")}>
            {amount(STALL)}
            {amount(NO_REPORT)}
          </Row>
          <Row label={t("factory.settings.adv.aiLimit")} detail={<Note>{t("factory.settings.adv.aiLimitDetail")}</Note>}>
            {amount(OBSERVER_LIMIT)}
          </Row>
          <Row label={t("factory.settings.adv.watch")}>
            {amount(WATCH_INTERVAL)}
            {amount(WATCH_LIMIT)}
          </Row>
          <Row label={t("factory.settings.adv.keep")}>
            {amount(CANCEL_KEEP)}
            {amount(DONE_FOLD)}
            {amount(ARCHIVE_FOLD)}
          </Row>
          <Row label={t("factory.settings.adv.recovery")} detail={
            <div className="flex flex-col gap-xs">
              {RECOVERY.map((action) => (
                <label key={action.id} className="flex items-center gap-sm text-body text-foreground">
                  <Checkbox checked={config.recovery.includes(action.id)} aria-label={t(action.label)} data-factory-setting={`recovery:${action.id}`} onCheckedChange={(on) => set("recovery", `${action.id}=${on === true ? "on" : "off"}`)} />
                  {t(action.label)}
                </label>
              ))}
            </div>
          }>
            <span className="text-caption text-muted-foreground">{t("factory.settings.adv.recoveryCaption")}</span>
          </Row>
          <Row label={t("factory.settings.adv.workerArgs")} detail={
            <div className="grid grid-cols-[auto_1fr] items-center gap-x-md gap-y-xs">
              {argAgents(config).map((agent) => (
                <Fragment key={agent}>
                  <span className="text-caption text-subtle-foreground">{agentProductName(agent)}</span>
                  <TextField reset={refusals} value={(config.worker_args[agent] ?? []).join(" ")} mono wide placeholder={t("factory.settings.adv.workerArgsFor", { agent: agentProductName(agent) })} onCommit={(value) => set("worker_args", `${agent}=${value}`)} data={`worker_args:${agent}`} />
                </Fragment>
              ))}
            </div>
          }>
            <span className="text-caption text-muted-foreground">{t("factory.settings.adv.workerArgsCaption")}</span>
          </Row>
          {/* Every other value keeps the control it had, grouped under small subheads (B36). */}
          <Subgroup title={t("factory.settings.verification")} data-factory-settings-subhead="verification">
            {config.verification.kind === "ci" ? (
              <Row label={t("factory.settings.ciChecks")}>
                <TextField reset={refusals} value={config.verification.checks.join(", ")} mono onCommit={(value) => set("ci", value)} data="ci" />
              </Row>
            ) : (
              <Row label={t("factory.settings.verifyCommands")} detail={<Note>{t("factory.settings.verifyCommandsDetail")}</Note>}>
                <TextField reset={refusals} value={verify} mono onCommit={(value) => set("verify", value)} data="verify" />
              </Row>
            )}
            {VERIFY_LIMITS.map(number)}
          </Subgroup>
          <Subgroup title={t("factory.settings.merge")} data-factory-settings-subhead="merge">
            <Row label={t("factory.settings.mergeMethod")}>
              <Choice value={config.merge_method} options={[["merge", t("factory.settings.method.merge")], ["squash", t("factory.settings.method.squash")], ["rebase", t("factory.settings.method.rebase")]]} onChange={(value) => set("merge_method", value)} data="merge_method" />
            </Row>
            <Row label={t("factory.settings.quickCheck")}>
              <TextField reset={refusals} value={config.quick_check ?? ""} mono onCommit={(value) => set("quick_check", value)} data="quick_check" />
            </Row>
          </Subgroup>
          <Subgroup title={t("factory.settings.run")} data-factory-settings-subhead="run">
            <Row label={t("factory.settings.harness")} detail={<Note>{t("factory.settings.harnessDetail")}</Note>}>
              <TextField reset={refusals} value={config.harness ? `${config.harness.name}: ${config.harness.instructions}` : ""} placeholder={t("factory.settings.harnessPlaceholder")} onCommit={(value) => set("harness", value)} data="harness" />
            </Row>
            {number(NEW_TASK_LIMIT)}
            {number(DISK_FLOOR)}
            <Row label={t("factory.settings.prdInIssue")}>
              <Switch checked={config.prd_in_issue} aria-label={t("factory.settings.prdInIssue")} data-factory-setting="prd_in_issue" onCheckedChange={(on) => set("prd_in_issue", on ? "on" : "off")} />
            </Row>
          </Subgroup>
          <Subgroup title={t("factory.settings.checks")} data-factory-settings-subhead="checks">
            {number(OUTSIDE_READ)}
            {config.checks.map((check, at) => (
              <Row key={at} label={t(CHECK_LABEL[check.at])}>
                <span className="min-w-0 text-body text-subtle-foreground [overflow-wrap:anywhere]">{check.instruction}</span>
              </Row>
            ))}
            <AddCheck written={writes} refused={refusals} sending={write.state.phase === "sending"} onAdd={(at, instruction) => send({ verb: "check", project: factory.project, at, instruction })} />
          </Subgroup>
          <Subgroup title={t("factory.settings.adv.autonomy")} data-factory-settings-subhead="autonomy">
            {config.autonomy.map((scope) => (
              <Row key={scope.id} label={scope.description}>
                <Switch checked={scope.enabled} aria-label={scope.description} data-factory-setting={`autonomy:${scope.id}`} onCheckedChange={(on) => set("autonomy", `${scope.id}=${on ? "on" : "off"}`)} />
              </Row>
            ))}
            {number(AUTONOMY_DIFF)}
          </Subgroup>
          <div className="px-md py-sm">
            <Note>{t("factory.settings.adv.rest")}</Note>
          </div>
        </Disclosure>
        <Row label={t("factory.settings.close")} detail={<Note>{running ? t("factory.settings.closeRunning") : t("factory.settings.closeDetail")}</Note>}>
          <Button variant="outline" size="sm" disabled={running || write.state.phase === "sending"} data-factory-close="true" onClick={() => send({ verb: "close", project: factory.project })}>
            {t("factory.settings.closeButton")}
          </Button>
        </Row>
      </Group>
    </div>
  );
}

/**
 * AI에게 맡기기 (B33): who answers, as three choices with what each hands to
 * Factory AI; the agent, model and effort Factory AI runs on; and today's
 * judgments against the daily cap. With Hide AI off every decision comes to
 * the person, so the choices dim and say so (B10).
 */
function ObserverGroup({ factory, config, reset, set, actions }: { factory: FactoryView; config: FactoryConfig; reset: number; set: (key: string, value: string) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const off = ai?.enabled === false;
  const mode = config.observer_mode;
  const full = factory.observer_today >= factory.observer_limit;
  return (
    <Group title={t("factory.settings.ai")} caption={t("factory.settings.aiCaption")} data-factory-settings-group="observer">
      <div className="flex flex-col gap-sm px-md py-sm">
        <RadioGroup value={mode} disabled={off} onValueChange={(value) => set("observer_mode", value)} className={cn("grid-cols-3", off && "opacity-(--opacity-dimmed)")} aria-label={t("factory.settings.ai")} data-factory-setting="observer_mode">
          {OBSERVER_MODES.map((choice) => (
            <label key={choice} className={cn("flex min-w-0 cursor-pointer items-start gap-sm rounded-md border px-md py-sm", choice === mode ? "border-foreground bg-accent" : "border-border")} data-factory-mode={choice}>
              <RadioGroupItem value={choice} className="mt-xxs" />
              <span className="flex min-w-0 flex-col">
                <span className="text-subhead font-semibold">{t(MODE_LABEL[choice])}</span>
                <span className="text-caption text-subtle-foreground">{t(MODE_LINE[choice])}</span>
              </span>
            </label>
          ))}
        </RadioGroup>
        {off ? (
          <Note data-factory-ai-off="true">
            <span className="flex items-center gap-xs">
              <BanIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
              {t("factory.settings.aiOff")}
            </span>
          </Note>
        ) : (
          <div className="flex flex-col gap-xs" data-factory-mode-table={mode}>
            <ModeChips icon={<UserIcon aria-hidden="true" className="size-(--size-icon-sm)" />} label={t("factory.settings.toMe")} keys={MODE_TABLE[mode].me} />
            <ModeChips icon={<SparklesIcon aria-hidden="true" className="size-(--size-icon-sm)" />} label={t("factory.settings.toAi")} keys={MODE_TABLE[mode].ai} />
          </div>
        )}
      </div>
      <Row label={t("factory.settings.aiAgent")}>
        {off ? <span className="text-body text-subtle-foreground" data-factory-setting="factory_ai">{t("factory.settings.aiNone")}</span> : <FactoryAiPicker value={config.factory_ai} reset={reset} set={set} actions={actions} />}
      </Row>
      {/* With Hide AI off nothing is judged, so there is no day's use to show. */}
      {off ? null : <Row label={t("factory.settings.aiToday")} detail={full ? <Note>{t("factory.settings.aiTodayFull")}</Note> : undefined}>
        <span className="flex items-center gap-sm" data-factory-ai-today={`${factory.observer_today}/${factory.observer_limit}`}>
          <span aria-hidden="true" className="h-xs w-(--size-settings-control-w) overflow-hidden rounded-full bg-border">
            <span className="block h-full rounded-full bg-primary" style={{ width: `${Math.min(100, (factory.observer_today / Math.max(1, factory.observer_limit)) * 100)}%` }} />
          </span>
          <span className={cn("font-mono text-caption", full ? "text-warning" : "text-subtle-foreground")}>
            {factory.observer_today} / {factory.observer_limit}
          </span>
        </span>
      </Row>}
      <Row label={t("factory.settings.metrics")} detail={<Note>{t("factory.settings.metricsHint")}</Note>}>
        <Metrics metrics={factory.metrics} />
      </Row>
    </Group>
  );
}

/**
 * The last seven local days in three numbers (D-45, B37): 결정 필요 items a
 * person moved per finished Task, the median from label to first start, and
 * the share of Factory AI's decisions a person changed; nothing to count reads '-'.
 */
function Metrics({ metrics }: { metrics: FactoryView["metrics"] }) {
  const { t, i18n } = useInterfaceTranslation();
  const perTask = metrics.person_items_tenths === null ? "-" : (metrics.person_items_tenths / 10).toLocaleString(i18n.language, { maximumFractionDigits: 1 });
  const median = metrics.start_median_ms === null ? "-" : formatElapsed(requireInterfaceLanguage(i18n.language), metrics.start_median_ms);
  const share = metrics.override_percent === null ? "-" : `${metrics.override_percent}%`;
  const cells: { key: string; label: string; value: string; sub: string }[] = [
    { key: "items", label: t("factory.settings.metrics.items"), value: perTask, sub: t("factory.settings.metrics.itemsSub", { count: metrics.finished }) },
    { key: "start", label: t("factory.settings.metrics.start"), value: median, sub: t("factory.settings.metrics.startSub", { count: metrics.started }) },
    { key: "override", label: t("factory.settings.metrics.override"), value: share, sub: t("factory.settings.metrics.overrideSub", { overridden: metrics.overridden, decided: metrics.ai_decisions }) },
  ];
  return (
    <span className="grid grid-cols-3 gap-lg" data-factory-metrics="true">
      {cells.map((cell) => (
        <span key={cell.key} className="flex min-w-0 flex-col" data-factory-metric={cell.key}>
          <span className="text-caption text-subtle-foreground">{cell.label}</span>
          <span className="font-mono text-subhead font-semibold" data-factory-metric-value={cell.value}>{cell.value}</span>
          <span className="text-caption text-muted-foreground">{cell.sub}</span>
        </span>
      ))}
    </span>
  );
}

function ModeChips({ icon, label, keys }: { icon: ReactNode; label: string; keys: MessageKey[] }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-xs">
      <span className="flex w-[calc(var(--size-control-lg)*3)] shrink-0 items-center gap-xxs text-caption text-subtle-foreground">
        {icon}
        {label}
      </span>
      {keys.map((key) => (
        <span key={key} className="rounded-sm bg-secondary px-sm py-xxs text-caption text-foreground">
          {t(key)}
        </span>
      ))}
    </div>
  );
}

/** Factory AI's agent among Hide AI's, or Hide AI's own choice; then its model and effort (D-40). */
function FactoryAiPicker({ value, set }: { value: FactoryAi | null; reset: number; set: (key: string, value: string) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const provider = providerById(ai, value?.provider);
  const providers = (ai?.providers ?? []).filter((row) => row.selectable || row.id === value?.provider);
  const efforts = provider ? (agentAdapter(provider.agent)?.efforts ?? []) : [];
  return (
    <>
      <Select value={value?.provider ?? "default"} onValueChange={(next) => set("factory_ai", next)}>
        <SelectTrigger size="sm" className="w-auto" aria-label={t("factory.settings.aiAgent")} data-factory-setting="factory_ai">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="default">{t("factory.settings.aiFollow")}</SelectItem>
          {providers.map((row) => (
            <SelectItem key={row.id} value={row.id}>
              {row.label}
            </SelectItem>
          ))}
          {/* An agent this Mac no longer offers still reads as the one chosen. */}
          {value && !providers.some((row) => row.id === value.provider) ? <SelectItem value={value.provider}>{value.provider}</SelectItem> : null}
        </SelectContent>
      </Select>
      {provider && value ? (
        <>
          <Select value={value.model ?? CLI_DEFAULT} onValueChange={(next) => set("factory_ai_model", next === CLI_DEFAULT ? "default" : next)}>
            <SelectTrigger size="sm" className="w-auto" aria-label={t("factory.settings.model")} data-factory-setting="factory_ai_model">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[CLI_DEFAULT, ...modelChoices(provider, value.model ?? "").filter((choice) => choice !== CLI_DEFAULT)].map((choice) => (
                <SelectItem key={choice} value={choice}>
                  {choice === CLI_DEFAULT ? t("factory.settings.cliDefault") : choice}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {efforts.length > 0 ? <EffortSelect value={value.effort ?? null} efforts={efforts} onChange={(next) => set("factory_ai_effort", next ?? "default")} data="factory_ai_effort" /> : null}
        </>
      ) : null}
    </>
  );
}

function EffortSelect({ value, efforts, onChange, data }: { value: string | null; efforts: readonly string[]; onChange: (effort: string | null) => void; data: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex items-center gap-xs">
      <span className="text-caption text-muted-foreground">{t("factory.settings.effort")}</span>
      <Select value={value ?? CLI_DEFAULT} onValueChange={(next) => onChange(next === CLI_DEFAULT ? null : next)}>
        <SelectTrigger size="sm" className="w-auto" aria-label={t("factory.settings.effort")} data-factory-setting={data}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={CLI_DEFAULT}>{t("factory.settings.cliDefault")}</SelectItem>
          {efforts.map((effort) => (
            <SelectItem key={effort} value={effort}>
              {effort}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </span>
  );
}

/**
 * 작업자 (B32): the candidates Factory AI picks from per Task, the first the
 * default, each an agent, model and effort with a line saying when to use it.
 * Every change sends the whole list, which the engine checks as one (D-41).
 */
function WorkersGroup({ config, machine, reset, set, actions }: { config: FactoryConfig; machine: number; reset: number; set: (key: string, value: string) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // With Hide AI off nothing picks per Task, so every start takes the default (D-42).
  const off = useShellStore((s) => s.rest?.status?.background_ai?.enabled === false);
  const workers = candidatesOf(config);
  const save = (next: WorkerCandidate[]) => set("workers", JSON.stringify(next));
  const change = (at: number, patch: Partial<WorkerCandidate>) => save(workers.map((candidate, index) => (index === at ? { ...candidate, ...patch } : candidate)));
  return (
    <Group title={t("factory.settings.workers")} caption={off ? t("factory.settings.workersCaptionOff") : t("factory.settings.workersCaption")} data-factory-settings-group="workers">
      {workers.map((candidate, at) => {
        const adapter = agentAdapter(candidate.agent);
        return (
          <div key={at} className="flex flex-col gap-xs px-md py-sm" data-factory-worker-candidate={at + 1}>
            <div className="flex min-w-0 flex-wrap items-center gap-sm">
              <span className="min-w-0 flex-1 text-subhead">{at === 0 ? t("factory.settings.workerDefault") : t("factory.settings.workerCandidate")}</span>
              <AgentPicker
                actions={actions}
                value={{ kind: candidate.agent as never, model: candidate.model ?? null }}
                onChange={(next) => change(at, next.kind === candidate.agent ? { model: next.model } : { agent: next.kind, model: null, effort: null })}
              />
              {adapter && adapter.efforts.length > 0 ? <EffortSelect value={candidate.effort ?? null} efforts={adapter.efforts} onChange={(effort) => change(at, { effort })} data={`worker_effort:${at + 1}`} /> : <span className="text-caption text-muted-foreground">{t("factory.settings.cliDefault")}</span>}
              {at > 0 ? (
                <Button variant="ghost" size="icon-sm" aria-label={t("factory.settings.removeWorker")} data-factory-worker-remove={at + 1} onClick={() => save(workers.filter((_, index) => index !== at))}>
                  <XIcon />
                </Button>
              ) : (
                <span className="size-(--size-control-sm)" />
              )}
            </div>
            <TextField reset={reset} value={candidate.description} placeholder={t("factory.settings.workerDescription")} wide onCommit={(description) => change(at, { description })} data={`worker_description:${at + 1}`} />
          </div>
        );
      })}
      <div className="flex min-w-0 flex-wrap items-center gap-sm px-md py-xs">
        <Button variant="ghost" size="sm" disabled={workers.length >= WORKER_LIMIT} data-factory-worker-add="true" onClick={() => save([...workers, { agent: workers[0]!.agent, description: "" }])}>
          <PlusIcon />
          {t("factory.settings.addWorker")}
        </Button>
        {workers.length >= WORKER_LIMIT ? <span className="text-caption text-muted-foreground">{t("factory.settings.workerLimit")}</span> : null}
        <span className="flex-1" />
        <Button variant="link" size="sm" className="text-caption text-muted-foreground" data-factory-machine-link="true" onClick={() => useUiStore.getState().setFactoryPlace({ factory: null })}>
          {t("factory.settings.maxWorkersNote", { count: machine })}
        </Button>
      </div>
    </Group>
  );
}

/** A small heading inside 고급 설정 and the rows it groups, hairlines between the rows only. */
function Subgroup({ title, children, ...data }: { title: string; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <section {...data}>
      <h4 className="px-md pt-md pb-xxs text-caption font-semibold text-subtle-foreground">{title}</h4>
      <div className="divide-y divide-border">{children}</div>
    </section>
  );
}

function Choice({ value, options, onChange, data }: { value: string; options: [string, ReactNode][]; onChange: (value: string) => void; data: string }) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger size="sm" className="w-auto" data-factory-setting={data}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map(([option, label]) => (
          <SelectItem key={option} value={option}>
            {label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** The instruction stays until the engine takes the check, so a refusal keeps what was typed. */
function AddCheck({ onAdd, written, refused, sending }: { onAdd: (at: (typeof CHECK_POINTS)[number], instruction: string) => void; written: number; refused: number; sending: boolean }) {
  const { t } = useInterfaceTranslation();
  const [at, setAt] = useState<(typeof CHECK_POINTS)[number]>("after_done");
  const [instruction, setInstruction] = useState("");
  const asked = useRef(false);
  useEffect(() => {
    if (!asked.current) return;
    asked.current = false;
    setInstruction("");
  }, [written]);
  // A refused check keeps its text, and a later write of another setting leaves it alone.
  useEffect(() => {
    asked.current = false;
  }, [refused]);
  return (
    <Row label={t("factory.settings.addCheck")}>
      <Choice value={at} options={CHECK_POINTS.map((point) => [point, t(CHECK_LABEL[point])])} onChange={(value) => setAt(value as (typeof CHECK_POINTS)[number])} data="check_at" />
      <Input value={instruction} className="w-(--size-settings-control-w)" placeholder={t("factory.settings.checkInstruction")} aria-label={t("factory.settings.checkInstruction")} data-factory-setting="check_instruction" onChange={(event) => setInstruction(event.target.value)} />
      <Button size="sm" variant="secondary" disabled={!instruction.trim() || sending} onClick={() => { asked.current = true; onAdd(at, instruction.trim()); }}>
        {t("factory.settings.add")}
      </Button>
    </Row>
  );
}

/** `reset` changes when the engine refuses a write, which leaves the config as it was, so the draft shows it again. */
function TextField({ value, onCommit, placeholder, numeric = false, mono = false, wide = false, valid, data, reset }: { value: string; onCommit: (value: string) => void; placeholder?: string; numeric?: boolean; mono?: boolean; wide?: boolean; valid?: (text: string) => boolean; data: string; reset: number }) {
  const [draft, setDraft] = useState(value);
  // The draft last sent, so the blur after Enter does not send it again.
  const sent = useRef<string | null>(null);
  useEffect(() => {
    setDraft(value);
    sent.current = null;
  }, [value, reset]);
  const commit = () => {
    // An emptied number would read as 0, and a value the field cannot take
    // would never reach the engine, so both go back to the saved value.
    if ((numeric && draft.trim() === "") || (valid && !valid(draft.trim()))) return setDraft(value);
    if (draft === value || draft === sent.current) return;
    sent.current = draft;
    onCommit(draft.trim());
  };
  return (
    <Input
      value={draft}
      type={numeric ? "number" : "text"}
      className={cn(numeric ? "w-[calc(var(--size-control-lg)*2)] text-right" : wide ? "w-full" : "w-(--size-settings-control-w)", mono && "font-mono")}
      placeholder={placeholder}
      aria-label={placeholder}
      data-factory-setting={data}
      onChange={(event) => {
        setDraft(event.target.value);
        sent.current = null;
      }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === "Enter") commit();
        if (event.key === "Escape") setDraft(value);
      }}
    />
  );
}
