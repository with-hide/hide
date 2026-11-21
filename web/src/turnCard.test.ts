import { expect, it } from "vitest";
import type { LettersSince } from "./snapshot";
import { lettersBefore, peerCount } from "./turnCard";

const letters: LettersSince = {
  total: 11,
  progress: 10,
  letters: 21,
  recent: [
    { direction: "sent", peer: { kind: "named", name: "together" }, line: "디스크 여유 5.3GB", at_unix_ms: 22 },
    { direction: "received", peer: { kind: "named", name: "review" }, mark: "report", line: "승인", at_unix_ms: 25 },
  ],
  peers: [
    { peer: { kind: "named", name: "review" }, mark: "report", line: "승인", at_unix_ms: 25 },
    { peer: { kind: "named", name: "together" }, mark: "waiting_answer", line: "디스크 여유 5.3GB", at_unix_ms: 22 },
  ],
  more_peers: 3,
};

it("counts the agents past the listed and the letters older than the listed", () => {
  expect(peerCount(letters)).toBe(5);
  expect(lettersBefore(letters)).toBe(9);
  expect(peerCount(undefined)).toBe(0);
});
