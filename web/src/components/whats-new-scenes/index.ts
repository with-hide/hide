// The What's new scenes by the name a release's highlight gives as its
// `drawing`. A main feature's pull request adds its scene here
// (CONTRIBUTING.md, A main feature brings its practice and its scene).

import { AUTOMATION_LOOP_MS, AutomationScene } from "./automation";
import { PAIR_LOOP_MS, PairScene } from "./pair";
import { SKILLS_LOOP_MS, SkillsScene } from "./skills";
import type { Scene } from "./scene";

export const SCENES = {
  automation: { Draw: AutomationScene, loopMs: AUTOMATION_LOOP_MS, stillAt: 9_000 },
  pair: { Draw: PairScene, loopMs: PAIR_LOOP_MS, stillAt: 4_800 },
  skills: { Draw: SkillsScene, loopMs: SKILLS_LOOP_MS, stillAt: SKILLS_LOOP_MS - 1 },
} as const satisfies Record<string, Scene>;

export type SceneName = keyof typeof SCENES;
