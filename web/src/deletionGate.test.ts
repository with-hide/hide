import { describe, expect, it } from "vitest";
import { blockedText, branchLossText, deleteButtonText, discardText, warningText } from "./deletionGate";
import { initializeInterfaceI18n } from "./i18n/instance";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("the Delete worktree confirmation words the core's deletion gate", () => {
  it("names every loss the Discard checkbox accepts in one line", () => {
    const discard = { changed_files: 2, nested: true, status_unknown: false, ignored_repositories: ["target/a", "vendor/b"] };
    expect(discardText(discard, english)).toBe("Discard 2 changed files, the worktree inside it, ignored repositories target/a, vendor/b");
    expect(discardText({ ...discard, nested: false, ignored_repositories: [] }, korean)).toBe("버리기: 변경된 파일 2개");
  });

  it("says what the branch loses and how many panes the button closes", () => {
    expect(branchLossText({ code: "commits", count: 1, base: "main" }, english)).toBe("1 commit not on main is lost with it");
    expect(branchLossText({ code: "commits", count: 3, base: null }, english)).toBe("3 commits not on the base branch are lost with it");
    expect(deleteButtonText({ blocked: null, warnings: [], pane_count: 2, can_delete_branch: true, branch_loss: null, discard: null }, english)).toBe("Close 2 panes and delete");
  });

  it("keeps Git's lock reason as the operator wrote it", () => {
    expect(blockedText({ code: "locked", name: "feature", reason: "검토 중" }, english)).toContain("Worktree feature is locked: 검토 중.");
    expect(blockedText({ code: "locked", name: "feature", reason: "" }, english)).toContain("Worktree feature is locked. Unlock");
  });

  it("reads a code this build does not know as its group's general line", () => {
    expect(warningText({ code: "from_a_newer_core" }, english)).toBe("Check before deleting");
    expect(blockedText({ code: "from_a_newer_core" }, english)).toBe("This worktree cannot be deleted right now.");
    expect(branchLossText({ code: "from_a_newer_core" }, english)).toBe("Commits on this branch may be lost with it.");
  });
});
