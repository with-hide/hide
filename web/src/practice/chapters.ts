// The practice chapters as data: what each step points at, which command or
// gesture performs it, and the world state that means it is done. Its words
// live in the practice catalog under `practice.<chapter>.<step>.*`, so a
// chapter reads in the operator's language. A step names a command, never a
// chord, so the key follows the host, the system and the operator's bindings.

import type { CommandId } from "../shortcuts";
import { focusedPane, PRACTICE_WORLD, type PracticeModel } from "./practiceWorld";
import { TOUR_WORLD, type TourModel } from "./tourWorld";
import type { PracticeWorld } from "./world";

/** A pane header dragged onto `edge` of the pane `to` names. */
export type PracticeDrag = { from: string; to: string; edge: "left" | "right" | "up" | "down" };

export type PracticeStep<M = unknown> = {
  id: string;
  /** Listed on the quest card; a step without it is not a task of its own. */
  quest?: true;
  /** What the spotlight rings: a selector into the real shell; every match is ringed as one box. */
  target?: string;
  /** Where the bubble sits beside the target; `center` floats it low in the middle. */
  side: "right" | "left" | "bottom" | "inside" | "center";
  /** The three parts of the screen, drawn as labeled zones. */
  zones?: true;
  /** The command the step teaches; ⏎ presses its chord unless the step has a gesture. */
  command?: CommandId;
  /** What ⏎ does in the learner's place: press the target, or drag a pane. */
  gesture?: { click: true } | { drag: PracticeDrag };
  /** Done when the world says so; a step without it is a look, done when the learner presses on. */
  done?: (model: M) => boolean;
};

export type PracticeChapter<M = unknown> = {
  /** The catalog prefix: `practice.<id>.title`, `practice.<id>.<step>.title`. */
  id: string;
  number: number;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  world: PracticeWorld<M>;
  steps: PracticeStep<M>[];
  /** Whether the quest card lists the steps; a look-only tour has nothing to tick. */
  quest: boolean;
};

/** Types a chapter against its own world, then lets the list hold chapters of every world. */
const chapter = <M,>(spec: PracticeChapter<M>) => spec as unknown as PracticeChapter;

export const CHAPTER_OVERVIEW = chapter<TourModel>({
  id: "overview",
  number: 0,
  version: 1,
  world: TOUR_WORLD,
  quest: false,
  steps: [
    { id: "parts", zones: true, side: "center" },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]", side: "right" },
    { id: "lenses", target: "[data-sidebar-lens]", side: "right", command: "toggle_sidebar_lens", done: (model) => model.seen.length === 3 },
    { id: "search", target: "[data-sidebar-search]", side: "right" },
    { id: "center", target: "[data-agent-area]", side: "right" },
    { id: "lane", target: "[data-lane-slot]", side: "left" },
    { id: "todo", target: "[data-project-todo]", side: "left" },
    { id: "tools", target: "[data-lane-tool]", side: "left" },
    { id: "devices", target: "[data-device-rail]", side: "right" },
    { id: "end", zones: true, side: "center" },
  ],
});

const t1 = (model: PracticeModel) => model.tabs.find((tab) => tab.id === "t1")!;
const panesOf = (node: PracticeModel["tabs"][number]["root"]): string[] => (node.type === "pane" ? [node.pane_id] : [...panesOf(node.first), ...panesOf(node.second)]);
const SHELL = '[data-pane-view^="sh"]';

export const CHAPTER_SCREEN = chapter<PracticeModel>({
  id: "screen",
  number: 1,
  version: 3,
  world: PRACTICE_WORLD,
  quest: true,
  steps: [
    { id: "split", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "split_right", done: (model) => panesOf(t1(model).root).length === 2 },
    { id: "jump", quest: true, target: '[data-pane-view="lead"]', side: "inside", command: "select_agent_1", done: (model) => panesOf(t1(model).root).length === 2 && focusedPane(model) === "lead" },
    { id: "zoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => t1(model).zoomed },
    { id: "unzoom", quest: true, target: '[data-pane-view="lead"] [data-pane-zoom]', side: "bottom", command: "toggle_zoom", gesture: { click: true }, done: (model) => !t1(model).zoomed },
    { id: "move", quest: true, target: `${SHELL} header`, side: "bottom", gesture: { drag: { from: `${SHELL} header`, to: '[data-pane-view="lead"]', edge: "left" } }, done: (model) => panesOf(t1(model).root)[0]!.startsWith("sh") },
    { id: "tab", quest: true, target: "[data-new-agent-tab]", side: "bottom", command: "new_tab", done: (model) => model.tabs.length === 2 },
    { id: "end", target: "[data-checkout-agents-open]", side: "right" },
  ],
});

export const CHAPTERS: readonly PracticeChapter[] = [CHAPTER_OVERVIEW, CHAPTER_SCREEN];
