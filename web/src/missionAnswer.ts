// What a decision's answer reads as, wherever it is asked (PRD
// mission-plan-visibility B26-B30, B39; PRD mobile-companion-ui B34, B35): the
// desktop's banner and the phone's mission page draw this one value, so the
// same mission is never answerable on one screen and refused on the other,
// and a refusal carries the same reason in both.

import type { MessageKey } from "./i18n/catalogs";
import type { Mission, MissionAnswerFailure, MissionAnswerState } from "./snapshot";

/** The longest written answer the core takes (`missions::ANSWER_LIMIT`), counted in UTF-16 units so the field's own cap never lets past what the core refuses. */
export const ANSWER_LIMIT = 500;

/** How long a pressed answer reads as sent before the core's own answer arrives; past it the screen draws the core's state again. */
export const PRESSED_MS = 5_000;

/** What a screen needs of a mission to ask its decision. */
export type AnswerSource = Pick<Mission, "decision" | "question" | "answer" | "answer_failed" | "stale"> & { conductor: Pick<Mission["conductor"], "present" | "takes_answers"> };

/** An answer this screen sent and has not yet seen the core keep, refuse or replace. `failed` is the failure shown when it was pressed, so only a new one ends it. */
export type PressedAnswer = { key: number; text: string; failed: MissionAnswerFailure | null; resend: boolean };

/** Why the operator cannot answer now, or why the last answer did not arrive (B28, B39). */
export type AnswerReason = { kind: "offline" } | { kind: "failed"; failure: MissionAnswerFailure } | { kind: "no_conductor" } | { kind: "unsupported" };

export type AnswerView = {
  /** A decision from before questions is answered under key 0, as the core keeps it. */
  key: number;
  options: string[];
  recommend: number | null;
  /** The answer as it stands: the core's, or the one just pressed; null while the question is open. */
  sent: { text: string; state: MissionAnswerState } | null;
  /** Sending an unconfirmed answer again shows it sending until the core moves it on. */
  resending: boolean;
  /** Nothing can be pressed: it cannot be taken, or it was already sent. */
  closed: boolean;
  /** No Conductor, a device that does not answer, or an agent that takes no banner answer. */
  unreachable: boolean;
  reason: AnswerReason | null;
};

export function answerView(mission: AnswerSource, device: string | null, pressed: PressedAnswer | null): AnswerView {
  const question = mission.question;
  const key = question?.key ?? 0;
  const answer = mission.answer?.key === key ? mission.answer : null;
  const resending = pressed?.resend === true && answer?.state === "unconfirmed";
  const sent: AnswerView["sent"] = answer && !resending ? { text: answer.text, state: answer.state } : pressed ? { text: pressed.text, state: "sending" } : null;
  const gone = !mission.conductor.present;
  const offline = mission.stale;
  const unsupported = !mission.conductor.takes_answers;
  const unreachable = gone || offline || unsupported;
  const reason: AnswerReason | null = offline && device
    ? { kind: "offline" }
    : !sent && mission.answer_failed
      ? { kind: "failed", failure: mission.answer_failed }
      : gone
        ? { kind: "no_conductor" }
        : unsupported
          ? { kind: "unsupported" }
          : null;
  return { key, options: question?.options ?? [], recommend: question?.recommend ?? null, sent, resending, closed: unreachable || sent !== null, unreachable, reason };
}

/** Whether the pressed answer still shows: not once the core answers for it, refuses it differently, or asks something else (B26). */
export function pressedShown(mission: AnswerSource, pressed: PressedAnswer, view: AnswerView): boolean {
  const answered = mission.answer?.key === view.key && !view.resending;
  return !answered && pressed.key === view.key && pressed.failed === mission.answer_failed;
}

/** The reason's catalog key; `offline` also takes `{ device }`. */
export function reasonKey(reason: AnswerReason): MessageKey {
  switch (reason.kind) {
    case "offline":
      return "mission.banner.offline";
    case "failed":
      return `mission.banner.failed.${reason.failure}`;
    case "no_conductor":
      return "mission.noConductor";
    case "unsupported":
      return "mission.banner.unsupported";
  }
}

/** A written answer as the core takes it: one line, so a tab or another control character becomes a space. */
export function writtenAnswer(draft: string): string {
  return draft.replace(/\p{Cc}/gu, " ").trim();
}
