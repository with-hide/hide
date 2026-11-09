import type { AskedTurn, LetterLine, LettersSince } from "./snapshot";

/** One line of the turn card's later half: a letter or a started agent, or a question the agent put to the operator. */
export type CardLine = { kind: "letter"; at: number; letter: LetterLine } | { kind: "asked"; at: number; asked: AskedTurn };

/**
 * The card's lines since the request in time order: the newest letters the
 * core sent and the questions later turns put to the operator, each in its
 * place. A question that ended the newest turn is left out while the card's
 * last line already shows it as the ask (`askShown`).
 */
export function cardLines(letters: LettersSince | undefined, asked: readonly AskedTurn[] | undefined, askShown: boolean): CardLine[] {
  const lines: CardLine[] = [
    ...(letters?.recent ?? []).map((letter): CardLine => ({ kind: "letter", at: letter.at_unix_ms, letter })),
    ...(asked ?? []).filter((one) => !(one.current && askShown)).map((one): CardLine => ({ kind: "asked", at: one.at_unix_ms, asked: one })),
  ];
  // A stable sort keeps a letter before a question written the same millisecond.
  return lines.sort((a, b) => a.at - b.at);
}

/** How many letters went since the request, the folded interim reports included. */
export function letterCount(letters: LettersSince | undefined): number {
  return letters ? letters.total + letters.progress : 0;
}

/** Letters the card counts but does not list: older than the newest it carries. */
export function lettersBefore(letters: LettersSince | undefined): number {
  return letters ? Math.max(0, letters.total - letters.recent.length) : 0;
}
