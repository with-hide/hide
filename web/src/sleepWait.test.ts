// PRD core-wait D-15, B18: the line under a sleeping agent's row while it waits.
import { expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { sleepWaitWords } from "./sleepWait";

const t = () => initializeInterfaceI18n("ko").t;
const hour = 3_600_000;
const now = 1_000 * hour;

it("names what a sleeping agent waits on as its code host writes a pull request, led by the hours left under a day", () => {
  expect(sleepWaitWords({ target: { ci: 1232 }, until_unix_ms: now + 5 * hour - 60_000, reference: { label: "!1232", url: "https://gitlab.com/a/b/-/merge_requests/1232" } }, now, t())).toBe("5시간 남음 · !1232 CI 기다림");
  expect(sleepWaitWords({ target: { pr: 7 }, until_unix_ms: now + 3 * 24 * hour }, now, t())).toBe("#7 머지 기다림");
  expect(sleepWaitWords({ target: { mission: "mission-91" }, until_unix_ms: now + 3 * 24 * hour }, now, t())).toBe("mission-91 끝남 기다림");
  expect(sleepWaitWords({ target: { node: { mission: "mission-91", node: "gitlab-write" } }, until_unix_ms: now + 3 * 24 * hour }, now, t())).toBe("mission-91 · gitlab-write 기다림");
  expect(sleepWaitWords({ target: { branch: "feat/x" }, until_unix_ms: now - 1 }, now, t())).toBe("feat/x main 반영 기다림");
});
