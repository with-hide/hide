// The What's new scenes by the name a release's highlight gives as its
// `drawing`. A main feature's pull request adds its scene here
// (CONTRIBUTING.md, A main feature brings its practice and its scene).

import { AUTOMATION_LOOP_MS, AutomationScene } from "./automation";
import type { Scene } from "./scene";

export const SCENES = {
  automation: { Draw: AutomationScene, loopMs: AUTOMATION_LOOP_MS, stillAt: AUTOMATION_LOOP_MS - 1 },
} as const satisfies Record<string, Scene>;

export type SceneName = keyof typeof SCENES;
