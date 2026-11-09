// What the first-run practice is: a few cards on a window with work in it, each
// pointing at a part of the screen, the last one leaving for the real Hide. Its words live in the practice catalog under `practice.<chapter>.<step>.*`,
// so it reads in the operator's language.

import type { CommandId } from "../shortcuts";
import type { PracticeWorld } from "./world";

/**
 * What a card sets on the window when it opens (no quest: the card does it, and
 * a later card that says nothing keeps it): the list the left sidebar shows, what
 * the right lane holds, and the pane that holds the keyboard.
 */
export type PracticeScene = {
  lens?: "projects" | "tasks";
  /** The lane closed, the Documents, the Documents with the Explorer beside them, or the 보기 slot holding the mission's map. */
  lane?: "closed" | "documents" | "explorer" | "mission";
  /** The pane that holds the keyboard; null for none. */
  focus?: string | null;
  /** The one pane the tab shows alone, for a card whose lane needs the room; null for the four. */
  solo?: string | null;
};

export type PracticeStep = {
  id: string;
  /** What the window shows while this card is up. */
  scene?: PracticeScene;
  /** What the spotlight rings: a selector into the real shell, or several for parts that are apart; the matches of one selector are ringed as one box. */
  target?: string | string[];
  /** Parts of the screen the card talks about without ringing them: the card floats clear of them too. */
  keepClear?: string[];
  /** A button the card presses when it opens and again when it is left, so the popover it holds is open exactly while the card talks about it. */
  opens?: string;
  /** The three parts of the screen, drawn as labeled zones. */
  zones?: true;
  /** The commands whose chords the card names, in this host's keys and the operator's own. */
  commands?: CommandId[];
};

/** The move a flow's card plays on the practice list (`PracticePreview.tsx`). */
export type PracticePreviewKind = "mission" | "zones" | "split" | "switch" | "rows" | "children" | "search" | "devices" | "overview" | "tree" | "waits" | "asleep";

export type PracticeChapter = {
  /** The catalog prefix (`practice.<id>.title`, `practice.<id>.<step>.title`) and the chapter's file name. */
  id: string;
  /** The flow's number as the list shows it. */
  number: number;
  /** Bumped when the steps change, so a learner's progress on the old steps reads as changed. */
  version: number;
  world: PracticeWorld<unknown>;
  steps: PracticeStep[];
  /** The move its card plays on the list. */
  preview: PracticePreviewKind;
};

/** Types a chapter against its own world, then lets the list hold chapters of every world. */
export const chapter = <M,>(spec: Omit<PracticeChapter, "world"> & { world: PracticeWorld<M> }) => spec as unknown as PracticeChapter;
