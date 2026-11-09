// What the first-run practice is: a few cards on a window with work in it, each
// pointing at a part of the screen, the last one handing over to the real
// Hide. Its words live in the practice catalog under `practice.<chapter>.<step>.*`,
// so it reads in the operator's language.

import type { CommandId } from "../shortcuts";
import type { PracticeWorld } from "./world";

export type PracticeStep = {
  id: string;
  /** What the spotlight rings: a selector into the real shell; every match is ringed as one box. */
  target?: string;
  /** The three parts of the screen, drawn as labeled zones. */
  zones?: true;
  /** The command whose chord the card names, in this host's keys and the operator's own. */
  command?: CommandId;
  /** The step whose button leaves the practice for a real mission: the Start panel with the sentence waiting (`channel.ts`). */
  mission?: true;
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
