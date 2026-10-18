// The core move as a window sees it (PRD core-host-node-move B2 to B5, W1 to
// W3): the supervisor's `core_move` frame (`hided/src/core_move/control.rs`),
// which every role of the process sends, and the node's `core_link` frame
// while its screens wait for the core. Nothing here decides the move; the
// window asks with one `core_move` event and draws what the frames say.

import type { TFunction } from "i18next";
import { takesEuro } from "./i18n/koParticle";

export type MoveState =
  | "idle"
  | "checking"
  | "ready"
  | "checks_failed"
  | "stopping"
  | "copying"
  | "starting"
  | "linking"
  | "done"
  | "rolling_back"
  | "rolled_back"
  | "waiting"
  | "abandoned";

export type CheckId =
  | "connection"
  | "other_node"
  | "target_state"
  | "target_folder"
  | "herdr"
  | "identity"
  | "own_state"
  | "link"
  | "gh"
  | "gui_session"
  | "sleep"
  | "build"
  | "ai"
  | "ai_setup"
  | "dormant";

export type MoveStep = "check" | "stop_core" | "copy" | "start_target" | "reattach";

export type MoveView = {
  state: MoveState;
  direction: "forward" | "back" | null;
  device: string | null;
  /** The SSH alias the move reaches its other machine by: a window the move holds before it drew the core names that machine by it. */
  alias?: string;
  intent: string | null;
  sent: number;
  total: number;
  failed: { check: CheckId; detail: string }[];
  /** How many checks ran, so the dialog counts the ones that passed. */
  checked?: number;
  step: MoveStep | null;
  cause: { kind: string } | null;
  node: string | null;
  /** The move's wait on its other machine can be given up now (amendment 4): a forward move stops waiting, a move back takes the core here. */
  abandon?: boolean;
};

/** Whether the operator can give up the move's wait on its other machine now. */
export function giveUpOffered(view: MoveView | null): boolean {
  return view?.state === "waiting" && view.abandon === true;
}

/**
 * Where a node's link to its core stands while its screens wait (`core_link` frame); `disconnected` is the operator's
 * own end of it (B16). `machine` is the core machine's name once its hello gave it; `alias` is the SSH alias the node
 * dials it by, the name a window that has not drawn the core yet has.
 */
export type CoreLink = { phase: "connecting" | "updating" | "waiting" | "disconnected"; machine: string | null; alias: string };

/** The five steps the dialog and the strip name, in order. */
export const MOVE_STEPS: readonly MoveStep[] = ["check", "stop_core", "copy", "start_target", "reattach"];

/** The states in which the move is under way and the window cannot act on the core. */
const UNDER_WAY: ReadonlySet<MoveState> = new Set(["stopping", "copying", "starting", "linking", "rolling_back", "waiting"]);

export function moveUnderWay(state: MoveState): boolean {
  return UNDER_WAY.has(state);
}

/** The step the move is on: the frame's own, else the one its state stands for. */
export function currentStep(view: MoveView): MoveStep {
  if (view.step) return view.step;
  switch (view.state) {
    case "stopping":
      return "stop_core";
    case "copying":
      return "copy";
    case "starting":
      return "start_target";
    case "linking":
    case "done":
      return "reattach";
    default:
      return "check";
  }
}

export type StepMark = "done" | "run" | "todo" | "fail";

/** Each step's mark: those before the current one done, the current running (or failed), the rest to do. */
export function stepMarks(view: MoveView): { step: MoveStep; mark: StepMark }[] {
  const at = MOVE_STEPS.indexOf(currentStep(view));
  const failed = view.state === "rolling_back" || view.state === "rolled_back";
  const finished = view.state === "done";
  return MOVE_STEPS.map((step, index) => ({
    step,
    mark: finished || index < at ? "done" : index > at ? "todo" : failed ? "fail" : "run",
  }));
}

/**
 * The one command that fixes a failing check, where one exists; the other
 * checks name what to do in words. Commands are typed on the machine taking
 * the core and are never translated.
 */
export function checkCommand(check: CheckId): string | null {
  switch (check) {
    case "gh":
      return "gh auth login";
    case "sleep":
      return "sudo pmset -c sleep 0";
    default:
      return null;
  }
}

/** The machines a move names: the one whose core stops and the one that takes it. */
export type MoveMachines = { from: string; to: string };

/**
 * The machine a failing check is about: Hide AI's own setting is the moving
 * core's, so its row names the machine whose core stops; every other check is
 * about the machine taking the core.
 */
export function checkMachine(check: CheckId, machines: MoveMachines): string {
  return check === "ai_setup" ? machines.from : machines.to;
}

/** The interpolation values a move's copy takes. */
export type MoveCopy = { from: string; to: string };

export function moveCopy(machines: MoveMachines): MoveCopy {
  return { from: machines.from, to: machines.to };
}

/** The strip's text while the window waits for a move (W1). */
export function movingStripText(view: MoveView, machines: MoveMachines, t: TFunction<"translation">): string {
  const step = currentStep(view);
  const index = MOVE_STEPS.indexOf(step) + 1;
  const values = moveCopy(machines);
  const title =
    view.state === "rolling_back"
      ? t("coreMove.strip.rollingBack", values)
      : view.direction === "back"
        ? t("coreMove.strip.movingBack", values)
        : takesEuro(machines.to)
          ? t("coreMove.strip.movingEuro", values)
          : t("coreMove.strip.moving", values);
  return t("coreMove.strip.line", { title, index, total: MOVE_STEPS.length, step: t(`coreMove.step.${step}`, values) });
}
