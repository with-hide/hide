import { expect, it } from "vitest";
import type { LettersSince } from "./snapshot";
import { cardLines, letterCount, lettersBefore } from "./turnCard";

const letters: LettersSince = {
  total: 11,
  progress: 10,
  recent: [
    { direction: "sent", peer: { kind: "named", name: "together" }, line: "디스크 여유 5.3GB", at_unix_ms: 22 },
    { direction: "received", peer: { kind: "named", name: "review" }, mark: "report", line: "승인", at_unix_ms: 25 },
  ],
};

// The questions later turns asked sit among the letters by time, and the
// one that ended the newest turn leaves while the card's last line asks it.
it("places each question among the letters by time and drops the one the last line asks", () => {
  const asked = [
    { line: "닫은 탭 결함 어떻게 할지", at_unix_ms: 24, current: false },
    { line: "머지할까요?", at_unix_ms: 30, current: true },
  ];
  expect(cardLines(letters, asked, false).map((line) => [line.kind, line.at])).toEqual([["letter", 22], ["asked", 24], ["letter", 25], ["asked", 30]]);
  expect(cardLines(letters, asked, true).map((line) => line.at)).toEqual([22, 24, 25]);
  expect(cardLines(undefined, undefined, false)).toEqual([]);
});

it("counts the folded reports with the letters and the ones older than the listed", () => {
  expect(letterCount(letters)).toBe(21);
  expect(lettersBefore(letters)).toBe(9);
  expect(letterCount(undefined)).toBe(0);
});
