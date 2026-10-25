// The Memory tool on a real core through an isolated pinned
// Herdr, hided and browser (PRD project-memory-v2 D-17, B19). The fixture is
// disposable and never uses an operator checkout, socket, app or Memory store.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { countSent, showProjectMemory, showProjectRow } from "./wire";

test.describe.configure({ timeout: 90_000 });

test("Memory opens the front checkout's Project and shows it off until the operator turns it on", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repoDir = path.join(herdr.root, "memory-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "memory-repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    daemon = await startHided(herdr, "memory-tab");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "memory-repo" }).locator("[data-checkout]").first())).click();
    await showProjectMemory(page);

    // The tab names its Project; the core answers with the Project's switch,
    // off for a Project that never turned Memory on, and its disclosure.
    await expect.poll(() => last.get("memory_open")?.workspace_id).toBeTruthy();
    const tab = page.locator('[data-lane="memory"]');
    await expect(tab.locator('[data-memory="off"]')).toBeVisible({ timeout: 15_000 });
    await expect(tab.locator('[data-memory-switch="off"]')).toBeEnabled();
    await expect(tab).toContainText("Use Auto memory");
    // Nothing to ask and no cards: no dot, no card list, no data to delete.
    await expect(page.locator('[data-lane-tool="memory"] [data-lane-todo-count]')).toHaveCount(0);
    await expect(tab.locator("[data-memory-cards]")).toHaveCount(0);
    await expect(tab.locator('[data-memory-delete="link"]')).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
