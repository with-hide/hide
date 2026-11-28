// A file change as the conversation draws it: the changed lines with one line of context, and how many lines it adds
// and takes away, which a tool line and its run's gray line count.

import type { ThreadLine } from "../../desk";

export type DiffLine = { sign: "+" | "-" | " "; text: string };

/** A file's change as lines: the unchanged ends trimmed to one line of context each. */
export function diffLines(oldText: string | null | undefined, newText: string): DiffLine[] {
  const before = oldText ? oldText.split("\n") : [];
  const after = newText.split("\n");
  let head = 0;
  while (head < before.length && head < after.length && before[head] === after[head]) head += 1;
  let tail = 0;
  while (tail < before.length - head && tail < after.length - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail += 1;
  return [
    ...(head > 0 ? [{ sign: " " as const, text: before[head - 1]! }] : []),
    ...before.slice(head, before.length - tail).map((text) => ({ sign: "-" as const, text })),
    ...after.slice(head, after.length - tail).map((text) => ({ sign: "+" as const, text })),
    ...(tail > 0 ? [{ sign: " " as const, text: after[after.length - tail]! }] : []),
  ];
}

/** The lines a call's diffs add and take away. */
export function diffCounts(diffs: Extract<ThreadLine, { kind: "tool" }>["diffs"]): { added: number; removed: number } {
  const lines = diffs.flatMap((diff) => diffLines(diff.oldText, diff.newText));
  return { added: lines.filter((one) => one.sign === "+").length, removed: lines.filter((one) => one.sign === "-").length };
}
