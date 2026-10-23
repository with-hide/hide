import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { removalNoticeText } from "./removalNotice";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("a worktree removal's answer, worded from the core's codes", () => {
  it("says what became of the branch once the folder is gone", () => {
    expect(removalNoticeText({ code: "removed", branch: "deleted" }, "finished", english)).toBe("Worktree and its local branch removed.");
    expect(removalNoticeText({ code: "removed", branch: "kept" }, "finished", korean)).toBe("워크트리를 삭제했어요. 로컬 브랜치는 남겼어요.");
  });

  it("names why the host stopped it and whether panes were already closed", () => {
    const locked = { code: "stopped", stop: { code: "locked", name: "feature", reason: "release review" }, panes_closed: false };
    expect(removalNoticeText(locked, "failed", english)).toBe(
      "No panes were closed, and the worktree is kept. Worktree feature is locked: release review. Unlock it, then refresh its Git state before deleting: git worktree unlock",
    );
    expect(removalNoticeText({ code: "stopped", stop: { code: "changed", files: 2 }, panes_closed: true }, "failed", english)).toContain("2 files changed after you confirmed");
  });

  it("names the programs still holding the folder, and the link's reason the device could not be asked", () => {
    expect(removalNoticeText({ code: "still_running", programs: [{ name: "node", pid: 42 }] }, "failed", english)).toContain("node (pid 42)");
    expect(removalNoticeText({ code: "host_failed", panes_closed: false, reason: "link.not_connected" }, "failed", english)).toContain("The device is not connected");
  });

  it("reads a code this build does not know as the general line", () => {
    expect(removalNoticeText({ code: "from_a_newer_core" }, "failed", english)).toBe("Worktree removal failed.");
    expect(removalNoticeText({ code: "stopped", stop: { code: "from_a_newer_host" }, panes_closed: true }, "failed", english)).toContain("Hide could not finish the removal.");
  });
});
