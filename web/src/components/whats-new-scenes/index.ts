// The What's new scenes by the name a release's highlight gives as its
// `drawing`. A main feature's pull request adds its scene here
// (CONTRIBUTING.md, A main feature brings its practice, its scene and its measure).

import { ANSWER_LOOP_MS, ANSWER_STILL_MS, AnswerScene } from "./answer";
import { ATTACHMENTS_LOOP_MS, ATTACHMENTS_STILL_MS, AttachmentsScene } from "./attachments";
import { AUTOMATION_LOOP_MS, AutomationScene } from "./automation";
import { BOTS_LOOP_MS, BOTS_STILL_MS, BotsScene } from "./bots";
import { CONNECT_LOOP_MS, CONNECT_STILL_MS, ConnectScene } from "./connect";
import { PAIR_LOOP_MS, PairScene } from "./pair";
import { SKILLS_LOOP_MS, SkillsScene } from "./skills";
import type { Scene } from "./scene";

export const SCENES = {
  answer: { Draw: AnswerScene, loopMs: ANSWER_LOOP_MS, stillAt: ANSWER_STILL_MS },
  attachments: { Draw: AttachmentsScene, loopMs: ATTACHMENTS_LOOP_MS, stillAt: ATTACHMENTS_STILL_MS },
  automation: { Draw: AutomationScene, loopMs: AUTOMATION_LOOP_MS, stillAt: 9_000 },
  bots: { Draw: BotsScene, loopMs: BOTS_LOOP_MS, stillAt: BOTS_STILL_MS },
  connect: { Draw: ConnectScene, loopMs: CONNECT_LOOP_MS, stillAt: CONNECT_STILL_MS },
  pair: { Draw: PairScene, loopMs: PAIR_LOOP_MS, stillAt: 4_800 },
  skills: { Draw: SkillsScene, loopMs: SKILLS_LOOP_MS, stillAt: SKILLS_LOOP_MS - 1 },
} as const satisfies Record<string, Scene>;

export type SceneName = keyof typeof SCENES;
