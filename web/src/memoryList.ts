import { formatRelativeTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import type { MemoryQuestion, MemoryRow, ProjectMemory } from "./snapshot";

/** How many cards the list shows before `그 외 N` (design v7). */
export const MEMORY_LIST_HEAD = 5;

/** A card's short name, as agents and the CLI write it. */
export function cardName(number: number): string {
  return `m${number}`;
}

/**
 * The list under `기억`: the search's answer in its order while one runs,
 * else every memory the tab's questions do not already show. A question's
 * memory is drawn once, in `확인할 것`; one on its way into the documents
 * stays in the list with its line, and its row is in 제안.
 */
export function memoryCards(project: ProjectMemory): MemoryRow[] {
  const asked = new Set(memoryTabQuestions(project).map((question) => question.number));
  if (project.search) {
    const byNumber = new Map(project.rows.map((row) => [row.number, row]));
    return project.search.numbers.flatMap((number) => byNumber.get(number) ?? []);
  }
  return project.rows.filter((row) => !asked.has(row.number));
}

/**
 * The cards on screen and how many are folded under `그 외 N`: the first
 * few, plus the card shown in full and every card on its way into the
 * repository wherever they sit, so neither a card opened from a
 * conversation nor one just promoted is hidden; a search shows every match.
 */
export function visibleCards(cards: MemoryRow[], expanded: number | null, all: boolean): { shown: MemoryRow[]; folded: number } {
  if (all || cards.length <= MEMORY_LIST_HEAD) return { shown: cards, folded: 0 };
  const moving = (row: MemoryRow) => row.promotion !== null && row.promotion.state !== "merged";
  const shown = cards.filter((row, index) => index < MEMORY_LIST_HEAD || row.number === expanded || moving(row));
  return { shown, folded: cards.length - shown.length };
}

/** A row's time: minutes or hours ago today, yesterday, then the date. */
export function memoryTime(unixMs: number, language: InterfaceLanguage, now = Date.now()): string {
  const elapsed = Math.max(0, now - unixMs);
  // Under a minute is "now": zero minutes would read "this minute" (`현재 분`).
  if (elapsed < 60_000) return formatRelativeTime(language, 0, "second");
  if (elapsed < 60 * 60_000) return formatRelativeTime(language, -Math.floor(elapsed / 60_000), "minute");
  const then = new Date(unixMs);
  const today = new Date(now);
  const days = Math.round((startOfDay(today) - startOfDay(then)) / 86_400_000);
  if (days === 0) return formatRelativeTime(language, -Math.floor(elapsed / 3_600_000), "hour");
  if (days === 1) return formatRelativeTime(language, -1, "day");
  const day = `${pad(then.getMonth() + 1)}-${pad(then.getDate())}`;
  return then.getFullYear() === today.getFullYear() ? day : `${then.getFullYear()}-${day}`;
}

/** A source's time: the clock today, yesterday with the clock, then the date with the clock. */
export function sourceTime(unixMs: number, language: InterfaceLanguage, now = Date.now()): string {
  const then = new Date(unixMs);
  const today = new Date(now);
  const clock = `${pad(then.getHours())}:${pad(then.getMinutes())}`;
  const days = Math.round((startOfDay(today) - startOfDay(then)) / 86_400_000);
  if (days === 0) return clock;
  if (days === 1) return `${formatRelativeTime(language, -1, "day")} ${clock}`;
  const day = `${pad(then.getMonth() + 1)}-${pad(then.getDate())}`;
  return `${then.getFullYear() === today.getFullYear() ? day : `${then.getFullYear()}-${day}`} ${clock}`;
}

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/**
 * What the Memory tab asks: whether a memory or the document it disagrees
 * with is right (PRD hide-ai-suggestions D-02). A memory's way into the
 * documents, its promotion and a promotion that failed or waits for a build
 * are rows of the 제안 tab now.
 */
export function memoryTabQuestions(project: ProjectMemory): MemoryQuestion[] {
  return project.questions.filter((question) => question.kind === "docs_disagree");
}

/** Most corrections one `N개 문서에 반영` takes; the core's `PROMOTION_BATCH_LIMIT`. */
export const MEMORY_BATCH_LIMIT = 5;

export type DocumentQuestion = Extract<MemoryQuestion, { kind: "docs_disagree" }>;

/**
 * The confirmed corrections waiting for a document update, the one the
 * operator spoke most recently first; the batch takes these. A comparison
 * still `ask` is answered first and is not among them.
 */
export function confirmedCorrections(project: ProjectMemory): DocumentQuestion[] {
  return memoryTabQuestions(project)
    .filter((question): question is DocumentQuestion => question.kind === "docs_disagree" && question.document.authority === "card_newer")
    .sort((a, b) => (b.document.operator_at_unix_ms ?? 0) - (a.document.operator_at_unix_ms ?? 0) || b.number - a.number);
}

/**
 * The corrections the next `N개 문서에 반영` takes: every confirmed one is
 * checked from the start, the most recent {@link MEMORY_BATCH_LIMIT}; an
 * `overrides` entry is the operator's own check or uncheck of one number.
 */
export function checkedCorrections(project: ProjectMemory, overrides: Readonly<Record<number, boolean>>): number[] {
  return confirmedCorrections(project)
    .filter((question, index) => overrides[question.number] ?? index < MEMORY_BATCH_LIMIT)
    .slice(0, MEMORY_BATCH_LIMIT)
    .map((question) => question.number);
}
