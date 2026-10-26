// The Hide AI tool on a real core through an isolated pinned Herdr, hided and
// browser (PRD hide-ai-suggestions B1, B3). The fixture is disposable and
// never uses an operator checkout, socket, app, Memory or suggestion store.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { countSent, showProjectMemory, showProjectRow } from "./wire";

test.describe.configure({ timeout: 90_000 });

test("Hide AI opens the front checkout's Project with its two tabs, and its head switch turns all of it off", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repoDir = path.join(herdr.root, "hide-ai-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "hide-ai-repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    daemon = await startHided(herdr, "hide-ai-tool");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "hide-ai-repo" }).locator("[data-checkout]").first())).click();
    await showProjectMemory(page);

    // One `memory_open` names the Project for both tabs; the head names it too.
    await expect.poll(() => last.get("memory_open")?.workspace_id).toBeTruthy();
    const workspaceId = last.get("memory_open")!.workspace_id;
    const tool = page.locator('aside[data-lane="memory"] [data-hide-ai-tool]');
    await expect(tool.locator("[data-lane-header]")).toContainText("Hide AI");
    await expect(tool.locator("[data-lane-header]")).toContainText("hide-ai-repo");

    // A Project that was never investigated is on, says when the first
    // investigation comes, and shows the one-time line once, which it marks seen.
    await expect(tool).toHaveAttribute("data-hide-ai", "on", { timeout: 15_000 });
    await expect(tool.locator('[data-hide-ai-switch="on"]')).toBeEnabled();
    await expect(tool.locator('[data-hide-ai-tool-tab="suggestions"]')).toHaveAttribute("aria-selected", "true");
    await expect(tool.locator('[data-suggestions-status="first_run_pending"]')).toBeVisible();
    await expect(tool.locator("[data-hide-ai-intro]")).toBeVisible();
    await expect.poll(() => sent.get("suggestions_intro_seen") ?? 0).toBe(1);
    // Nothing to do yet: no tab number and no rail number.
    await expect(tool.locator("[data-hide-ai-tool-tab-count]")).toHaveCount(0);
    await expect(page.locator('[data-lane-tool="memory"] [data-lane-todo-count]')).toHaveCount(0);

    // The Memory tab keeps Memory's own switch: a Project that never turned
    // Memory on shows its disclosure and the button that turns it on.
    await tool.locator('[data-hide-ai-tool-tab="memory"]').click();
    await expect(tool.locator('[data-memory="off"]')).toBeVisible({ timeout: 15_000 });
    await expect(tool.locator('[data-memory-switch="off"]')).toContainText("Turn on Memory");
    await expect(tool.locator("[data-memory-cards]")).toHaveCount(0);

    // The head switch is one `hide_ai_switch`; off, the tool says so and counts nothing.
    await tool.locator("[data-hide-ai-switch]").click();
    await expect.poll(() => last.get("hide_ai_switch")).toEqual({ workspace_id: workspaceId, on: false });
    await expect(tool.locator("[data-hide-ai-off]")).toBeVisible({ timeout: 15_000 });
    await expect(tool.locator('[data-hide-ai-switch="off"]')).toBeEnabled();
    await expect(page.locator('[data-lane-tool="memory"] [data-lane-todo-count]')).toHaveCount(0);

    // On again gives each tab back as it was.
    await tool.locator("[data-hide-ai-switch]").click();
    await expect.poll(() => last.get("hide_ai_switch")).toEqual({ workspace_id: workspaceId, on: true });
    await expect(tool).toHaveAttribute("data-hide-ai", "on", { timeout: 15_000 });
    await expect(tool.locator('[data-hide-ai-tool-tab="memory"]')).toBeVisible();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
