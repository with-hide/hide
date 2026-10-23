import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { issueFailureText } from "./issueFailure";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("an issue failure the core names by its code", () => {
  it("is worded in the viewer's language with the limit it names", () => {
    expect(issueFailureText({ code: "local_issue.title_too_long", limit: 200 }, korean)).toBe("제목은 최대 200자까지 쓸 수 있어요.");
    expect(issueFailureText({ code: "pr_link.no_worktree" }, english)).toBe("The PR's branch has no worktree. Create a worktree first.");
  });

  it("is nothing when this build does not word the code, so the headline speaks alone", () => {
    expect(issueFailureText({ code: "pr_link.from_the_future" }, english)).toBeNull();
    expect(issueFailureText(null, english)).toBeNull();
  });
});
