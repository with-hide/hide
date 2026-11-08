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
 * where the palette answers as it is typed and ⏎ stays the palette's. A pane's
 * words may instead be a catalog key (`words`), read when they are typed, so
 * the example is in the learner's language.
 */
export type PracticeTyping = { pane: string; text: string } | { pane: string; words: string } | { input: string; text: string };

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
  /** A command whose chord the step only shows, as a look names a shortcut: ⏎ does not press it. */
  shows?: CommandId;
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
  /**
   * With `done`: the step is over only once the palette its result opened has closed again, ⏎ on a row or Esc. While
   * it is up the guide stays on this step and says only how to use the palette, never the next step's card.
   */
  afterClose?: true;
  /** Done by the world on its own (`PracticeWorld.later`), an agent finishing its turn: ⏎ does nothing and the button says it waits. */
  wait?: true;
  /**
   * How long, in ms, the screen plays alone before the guide comes in: no dim,
   * ring or bubble until then, so a scene the learner meets first is seen
   * before anything covers it. ⏎ brings the guide in at once.
   */
  scene?: number;
};

/** The move a chapter's card plays on the practice list (`PracticePreview.tsx`). */
export type PracticePreviewKind = "mission" | "zones" | "split" | "switch" | "rows" | "children" | "search" | "lane" | "checklist" | "devices";

export type PracticeChapter<M = unknown> = {
  /** The catalog prefix (`practice.<id>.title`, `practice.<id>.<step>.title`) and the chapter's file name. */
  id: string;
  /** The flow's number as the list shows it; the next flow is the next number there is. */
  number: number;
  /** The flow its end card leads on to when that is not the next one in the list (the mission, where a newcomer starts, leads to the first flow). */
  next?: string;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  world: PracticeWorld<M>;
  steps: PracticeStep<M>[];
  /** Whether the quest card lists the steps; a look-only tour has nothing to tick. */
  quest: boolean;
  /** The move its card plays on the list. */
  preview: PracticePreviewKind;
};

/** Types a chapter against its own world, then lets the list hold chapters of every world. */
export const chapter = <M,>(spec: PracticeChapter<M>) => spec as unknown as PracticeChapter;
