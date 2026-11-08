// How a person's turn an agent declared reads (PRD ask-human): one sentence
// for every surface, the rows, the pane's head, the bell and the mission map.

import type { TFunction } from "i18next";
import type { HumanKind } from "./snapshot";

/** A person's turn as a line says it: its kind's word and where, `인증 · github.com/login` (PRD ask-human D-09, D-21). */
export function humanTurnText(t: TFunction<"translation">, turn: { kind: HumanKind; place: string }): string {
  return `${t(`agents.human.${turn.kind}`)} · ${turn.place}`;
}
