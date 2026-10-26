// What a practice chapter is: what each step points at, which command or
// gesture performs it, and the world state that means it is done. Its words
// live in the practice catalog under `practice.<chapter>.<step>.*`, so a
// chapter reads in the operator's language. A step names a command, never a
// chord, so the key follows the host, the system and the operator's bindings.

import type { CommandId } from "../shortcuts";
import type { PracticeWorld } from "./world";

/** A pane header dragged onto `edge` of the pane `to` names. */
export type PracticeDrag = { from: string; to: string; edge: "left" | "right" | "up" | "down" };

/**
 * What the learner would type: `text` into the terminal of the pane `pane`,
 * then ⏎, or `text` into the field `input` marks (a palette's search box),
 * where the palette answers as it is typed and ⏎ stays the palette's.
 */
export type PracticeTyping = { pane: string; text: string } | { input: string; text: string };

/** A menu opened by right-clicking what `at` marks, then each of `path` pressed in turn: a submenu, then its item. */
export type PracticeMenu = { at: string; path: string[] };

/** A control `at` marks pressed, then each of `path` pressed in turn: a popover's list, then its choice. */
export type PracticePick = { at: string; path: string[] };

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
  /** What ⏎ does in the learner's place: press the target, drag a pane, type, choose from a right-click menu, or press a control and choose from its list. */
  gesture?: { click: true } | { drag: PracticeDrag } | { type: PracticeTyping } | { menu: PracticeMenu } | { pick: PracticePick };
  /** Done when the world says so; a step without it is a look, done when the learner presses on. */
  done?: (model: M) => boolean;
  /**
   * Done once this selector is on screen: a card, a popover or a page the
   * shell opens with no core event, so the world cannot say so. Once done it
   * stays done, however the learner closes it afterwards.
   */
  shown?: string;
};

export type PracticeChapter<M = unknown> = {
  /** The catalog prefix (`practice.<id>.title`, `practice.<id>.<step>.title`) and the chapter's file name. */
  id: string;
  /** Where the chapter stands in the list; the next chapter is the next number there is. */
  number: number;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  world: PracticeWorld<M>;
  steps: PracticeStep<M>[];
  /** Whether the quest card lists the steps; a look-only tour has nothing to tick. */
  quest: boolean;
};

/** Types a chapter against its own world, then lets the list hold chapters of every world. */
export const chapter = <M,>(spec: PracticeChapter<M>) => spec as unknown as PracticeChapter;
