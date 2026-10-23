import { describe, expect, it } from "vitest";
import { diffNoticeText } from "./diffNotice";
import { initializeInterfaceI18n } from "./i18n/instance";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("why a diff is short or absent, worded from the core's code", () => {
  it("names the limit that cut a diff and the folder History reads", () => {
    expect(diffNoticeText({ code: "truncated", limit_kb: 256 }, english)).toBe("This diff is larger than 256 KB and is shown cut short.");
    expect(diffNoticeText({ code: "outside", root: "/work/app" }, korean)).toContain("/work/app");
  });

  it("reads a code this build does not know as a diff Git could not show", () => {
    expect(diffNoticeText({ code: "from_a_newer_core" }, english)).toBe("Git could not show this diff.");
  });
});
