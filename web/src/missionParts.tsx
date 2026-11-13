import { GitBranchIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { useMemo, type ReactNode } from "react";
import { AgentMark } from "./AgentMark";
import { DeviceChip } from "./components/device-chip";
import { formatElapsed } from "./components/elapsed";
import { MissionPrMark } from "./components/mission-pr";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { checkerState, itemNames, launchSession, missionNumber, nodeTimes, nodeWorkWords, runWords, shortCommit, workerAsleep, type ItemNames, type NodeLook, type NodeResult, type RunProfile } from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import { machineName, windowDeviceId } from "./screenMachine";
import type { Mission, MissionChecker, MissionEvent, MissionNode, MissionWorker } from "./snapshot";
import { useShellStore } from "./store";

// The parts a mission's map, its node panel and the Conductor pane's popover
// draw alike: a node's result, its worker's live row, a look's word, a node's
// times and one checker of a review or verification.

/** A worker's device, only when it is away from the window reading the mission. */
export function useMissionDeviceName(machine: string | null | undefined): string | null {
  const { t } = useInterfaceTranslation();
  return useShellStore((s) => {
    const devices = s.rest?.navigator?.devices;
    return machine && machine !== windowDeviceId(devices) ? machineName(devices, machine, t) : null;
  });
}

/** A device in the worker's metadata, with the full name on hover; only a device away from the window is named, so it wears the other-device colour the workspace's header band and its pane border wear (PRD home-device-rail D-15). */
export function MissionDevice({ name, className }: { name: string | null; className?: string }) {
  return name ? <Hint label={name} reveals><span className={cn("flex min-w-0 max-w-full", className)}><DeviceChip label={name} variant="remote" className="max-w-full" /></span></Hint> : null;
}

/**
 * What a node left, by its kind, as the Conductor pane's popover lists it
 * (PRD mission-plan-visibility B11): `⑂ #n` in its pull request's state
 * colour, a direct change's branch or its commit on main, or a general
 * node's verdict or result line.
 */
export function ResultTail({ result, stale, full = false }: { result: NodeResult; stale: boolean; full?: boolean }) {
  const { t } = useInterfaceTranslation();
  switch (result.kind) {
    case "pr":
      return <MissionPrMark number={result.number} state={result.state} stale={stale} data-node-pr={result.number} />;
    case "commit":
      return <span className="shrink-0 font-mono text-caption text-mission-done" title={result.commit} data-node-commit={result.commit}>{t("mission.result.main", { commit: shortCommit(result.commit) })}</span>;
    case "verdict":
      return (
        <span className={cn("shrink-0 font-mono text-caption font-semibold", result.verdict === "pass" ? "text-mission-done" : "text-mission-failed")} data-node-verdict={result.verdict}>
          {t(result.verdict === "pass" ? "mission.result.pass" : "mission.result.fail")}
          {result.checks ? ` ${result.checks.passed}/${result.checks.total}` : ""}
        </span>
      );
    case "branch":
      return (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-subtle-foreground" data-node-branch={result.branch}>
          <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
          <span className="truncate font-mono" title={result.branch}>{result.branch}</span>
        </span>
      );
    case "line":
      return <span className={cn("min-w-0 text-caption text-muted-foreground", full ? "break-words" : "truncate")} title={result.line} data-node-result-line="true">{result.line}</span>;
  }
}

/** The worker's live row, the one the panel's terminal and record follow. */
export function useWorkerRow(node: MissionNode) {
  return useShellStore((s) => {
    if (!node.pane_id) return null;
    const rows = agentsByPane(allAgents(s.agents, s.rest?.status?.remote));
    return rows.get(node.pane_id) ?? null;
  });
}

/** The look's word, the same on the card's mark, its hover and its accessible name (B26). */
export function lookWord(look: NodeLook, t: TFunction<"translation">): string {
  return t(`mission.look.${look}`);
}

/**
 * A node mark's hover and accessible name (PRD mission-node-simple B1): its
 * look's word, for a node in progress what its worker does now as the core
 * reads it (`work`: 작업 중, 기다리는 중 · <what>, 멈춤 · <why>), and a sleeping
 * worker's sleep beside it, since the mark itself has no fourth shape.
 */
export function markWord(node: MissionNode, look: NodeLook, t: TFunction<"translation">): string {
  const word = look === "working" && node.work ? nodeWorkWords(node.work, t) : lookWord(look, t);
  return look === "working" && workerAsleep(node) ? `${word} · ${t("mission.worker.asleep")}` : word;
}

/** How long a node took, from its events (B28); nothing moves while its device does not answer (B35). */
export function useNodeTimes(mission: Mission, node: MissionNode, events: MissionEvent[] | null, end: number) {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const times = events ? nodeTimes(node, events, end) : null;
  if (!times || (times.running && mission.stale)) return null;
  return { took: times.took != null && times.took > 0 ? formatElapsed(language, times.took) : null, running: times.running };
}

const CHECKER_TONE = { pass: "text-muted-foreground", fail: "text-mission-failed", running: "text-mission-working", waiting: "text-muted-foreground" } as const;

/** One checker: its agent's face, the model and effort it runs, and where its check is. */
export function CheckerRow({ mission, kind, checker }: { mission: Mission; kind: "review" | "verify"; checker: MissionChecker }) {
  const { t } = useInterfaceTranslation();
  const state = checkerState(checker);
  const run = useRunWords(mission, checker.pane_id, checker.machine, checker.worker, checker.worker.agent);
  const device = useMissionDeviceName(checker.machine);
  const status = state === "pass" || state === "fail" ? t(`mission.checker.${state}`) : t(`mission.checker.${state}.${kind}`);
  return (
    <span className="flex min-w-0 flex-col gap-xxs" data-node-checker={checker.worker.agent} data-checker-state={state}>
      <span className="flex min-w-0 items-center gap-xs">
        <AgentMark kind={checker.worker.agent} profile={run?.profile} />
        {run ? <RunText run={run} className="flex-1" /> : <span className="flex-1" />}
        <span className={cn("shrink-0", CHECKER_TONE[state])}>
          {state === "pass" ? "✓ " : state === "fail" ? "✕ " : ""}
          {status}
        </span>
      </span>
      <MissionDevice name={device} />
    </span>
  );
}

/** What runs in a pane: its words, the mark a plan's words carry, the two as one line for a hover or a name, and the profile's name of the item that speaks. */
export type RunLabel = { text: string; words: string; mark: string | null; from: "launch" | "plan"; profile: string | null };

/** How a run's item reads on `machine` (`itemNames`): from the machine's profile list and the model lists the snapshot holds while a screen reads them. */
export function useItemNames(machine: string | null | undefined, agent: string | null | undefined, profile: RunProfile | null): ItemNames {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const ai = useShellStore((s) => s.rest?.status?.background_ai);
  const id = profile?.id ?? null;
  const name = profile?.name ?? null;
  return useMemo(() => itemNames(devices, ai, machine, agent, { id, name }, t("agentProfiles.itemGone")), [devices, ai, machine, agent, id, name, t]);
}

/** A planned worker's words as its item reads: its profile's name and its model's shown name. */
function plannedRun(plan: MissionWorker | null | undefined, names: ItemNames) {
  return plan ? { ...plan, model: plan.model ? names.model(plan.model) : null, profileName: names.profile } : null;
}

/**
 * What runs in a pane as Hide launched it, else what the plan asked marked
 * planned (B29); `from` says which, for a reader that tells them apart. The
 * launch names the profile the pane's agent runs with and its model by the
 * name that item's list shows; the plan names its own item the same way.
 */
export function useRunWords(mission: Mission, paneId: string | null | undefined, machine: string | null | undefined, plan: MissionWorker | null | undefined, agent: string | null | undefined): RunLabel | null {
  const { t } = useInterfaceTranslation();
  const session = useShellStore((s) => launchSession(s.rest, mission, paneId, machine));
  const runs = useShellStore((s) => (paneId ? agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).get(paneId)?.profile ?? null : null));
  const ran = useItemNames(machine ?? mission.device_id, agent, runs ? { name: runs } : null);
  const planned = useItemNames(mission.device_id, plan?.agent, plan?.profile ? { id: plan.profile } : null);
  // Neither value is no proof of a launch, so a profile alone does not make one speak.
  const proven = session && (session.launch_model || session.launch_effort);
  const launch = proven ? { launch_model: session.launch_model ? ran.model(session.launch_model) : null, launch_effort: session.launch_effort, profile: ran.profile } : null;
  return runLabel(runWords(launch, plannedRun(plan, planned), { auto: t("mission.auto"), defaultModel: t("conductor.defaultModel") }), t);
}

function runLabel(run: ReturnType<typeof runWords>, t: TFunction<"translation">): RunLabel | null {
  if (!run) return null;
  return run.planned
    ? { text: t("mission.planned", { words: run.words }), words: run.words, mark: t("mission.plannedMark"), from: "plan", profile: run.profile }
    : { text: run.words, words: run.words, mark: null, from: "launch", profile: run.profile };
}

/**
 * A node's worker as its card and its panel's head draw it (operator,
 * 2026-11-01): the agent item, model and effort the core kept on the node for
 * its current worker (`launched`, proven by that worker's own conversation,
 * so it outlives the pane and is never another session's); without it, the
 * plan's, marked planned, while the node has not finished; otherwise
 * nothing, so finished work never shows the plan as what ran. Either names
 * its profile and its model by the name that item's list shows (PRD
 * agent-profiles-phase2 B12).
 */
export function useNodeRun(mission: Mission, node: MissionNode): { run: RunLabel; agent: string | null; model: string | null } | null {
  const { t } = useInterfaceTranslation();
  const launched = node.launched ?? null;
  const ran = useItemNames(node.machine ?? mission.device_id, launched?.agent, launched?.profile ?? null);
  const plan = useItemNames(mission.device_id, node.worker?.agent, node.worker?.profile ? { id: node.worker.profile } : null);
  const planned = node.status !== "done" && node.status !== "dropped";
  const launch = launched ? { launch_model: launched.model ? ran.model(launched.model) : null, launch_effort: launched.effort ?? null, profile: ran.profile } : null;
  const shownPlan = planned ? plannedRun(node.worker, plan) : null;
  const run = runLabel(runWords(launch, shownPlan, { auto: t("mission.auto"), defaultModel: t("conductor.defaultModel") }), t);
  if (!run) return null;
  // The card's worker row names the model alone (PRD mission-destination D-51): the launched one, the CLI's default when the launch named none, else the plan's.
  const model = run.from === "launch" ? (launch?.launch_model ?? t("conductor.defaultModel")) : (shownPlan?.model ?? null);
  return { run, agent: run.from === "launch" ? launched!.agent : (node.worker?.agent ?? null), model };
}

/** A run's words, which truncate, before its planned mark, which never does, so a planned model never reads as launched. */
export function RunText({ run, className, wordsClassName, children }: { run: RunLabel; className?: string; wordsClassName?: string; children?: ReactNode }) {
  return (
    <span className={cn("flex min-w-0 items-center gap-xxs", className)} title={run.text} data-run={run.from}>
      {/* A grid track that starts at nothing: the words ask no width to fit in, yet take their whole width when there is room. */}
      <span className={cn("grid min-w-0 grid-cols-[minmax(0,1fr)]", wordsClassName)} data-run-words="true"><span className="truncate">{run.words}</span></span>
      {run.mark ? <span className="shrink-0" data-run-planned="true">{run.mark}</span> : null}
      {children}
    </span>
  );
}

/** A mission's short number (`M39`) in muted monospace before its title; nothing for an id that carries none. */
export function MissionNumber({ mission, className }: { mission: Pick<Mission, "id">; className?: string }) {
  const number = missionNumber(mission.id);
  return number ? <span className={cn("shrink-0 font-mono tabular-nums text-muted-foreground", className)} data-mission-number={number}>{number}</span> : null;
}
