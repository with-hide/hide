import { describe, expect, it } from "vitest";
import { createInterfaceI18n } from "./i18n/instance";
import {
  BACKOFF_MAX_MS,
  BACKOFF_START_MS,
  badgeText,
  connectionAfterHealthFails,
  nextBackoff,
} from "./connection";

describe("connection machine", () => {
  it("caps backoff at 30s", () => {
    let value = BACKOFF_START_MS;
    for (let i = 0; i < 12; i += 1) value = nextBackoff(value);
    expect(value).toBe(BACKOFF_MAX_MS);
  });

  it("goes gone after three health failures", () => {
    expect(connectionAfterHealthFails(2)).toBe("reconnecting");
    expect(connectionAfterHealthFails(3)).toBe("gone");
  });

  it("hides the badge while live", async () => {
    const { t } = await createInterfaceI18n("en");
    expect(badgeText("live", false, t)).toBe("");
    expect(badgeText("reconnecting", false, t)).toBe("reconnecting");
    expect(badgeText("gone", false, t)).toContain("hide");
    expect(badgeText("gone", true, t)).toContain("refused");
  });

  it("keeps the shipped Korean wording for a refused connection", async () => {
    const { t } = await createInterfaceI18n("ko");
    expect(badgeText("gone", true, t)).toBe("연결 거부됨 - hide 명령을 다시 실행하세요");
  });
});
