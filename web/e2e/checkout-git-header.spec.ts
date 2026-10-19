import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { checkoutGitFixture } from "./checkout-git-fixture";
import { git } from "./work-prs-fixture";
import { enterWorkspace, screenshot } from "./wire";

test("checkout header reads working files with Changes closed, current-SHA statuses, and idempotent Changes", async ({ page }) => {
  test.setTimeout(90_000);
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const fixture = checkoutGitFixture(herdr);
    daemon = await startHided(herdr, "checkout-header");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const files = page.locator("[data-checkout-files]");
    await expect(page.locator("[data-checkout-commit]")).toHaveAttribute("data-checkout-commit", fixture.sha, { timeout: 15_000 });
    await expect(files).toHaveCount(0);
    await expect(page.locator('[data-checkout-ci="failed"]')).toBeVisible({ timeout: 15_000 });
    const requests = fs.readFileSync(fixture.log, "utf8");
    expect(requests).toContain(`/commits/${fixture.sha}/check-runs?per_page=100`);
    expect(requests).toContain(`/commits/${fixture.sha}/status`);
    await expect(page.locator('aside[data-lane="changes"]')).toHaveCount(0);
    fs.writeFileSync(path.join(fixture.root, "tracked.txt"), "edited\n");
    await expect(files).toHaveText("1", { timeout: 5_000 });
    fs.writeFileSync(path.join(fixture.root, "new.txt"), "new\n");
    await expect(files).toHaveText("2", { timeout: 5_000 });
    fs.unlinkSync(path.join(fixture.root, "new.txt"));
    await expect(files).toHaveText("1", { timeout: 5_000 });
    git(fixture.root, ["add", "tracked.txt"]);
    git(fixture.root, ["commit", "-m", "second HEAD"]);
    const next = git(fixture.root, ["rev-parse", "HEAD"]).trim();
    await expect(page.locator("[data-checkout-commit]")).toHaveAttribute("data-checkout-commit", next, { timeout: 5_000 });
    await expect(files).toHaveCount(0, { timeout: 5_000 });
    await expect(page.locator('[data-checkout-ci="none"]')).toBeVisible({ timeout: 15_000 });
    fs.unlinkSync(path.join(fixture.root, "tracked.txt"));
    await expect(files).toHaveText("1", { timeout: 5_000 });
    const terminal = page.locator(`[data-terminal-host="${herdr.panes[0]}"] textarea`);
    await terminal.focus();
    await files.click();
    await expect(page.locator('aside[data-lane="changes"]')).toBeVisible();
    await expect(terminal).toBeFocused();
    await files.click();
    await expect(page.locator('aside[data-lane="changes"]')).toBeVisible();
    const commit = page.locator("[data-checkout-commit]");
    await commit.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByText(next.slice(0, 12), { exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(commit).toBeFocused();
    await screenshot(page, "checkout-header-browser");
  } finally {
    daemon?.stop(); herdr.stop();
  }
});
