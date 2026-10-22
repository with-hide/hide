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
    const fixture = checkoutGitFixture(herdr, { historyCount: 6 });
    daemon = await startHided(herdr, "checkout-header");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const files = page.locator("[data-checkout-files]");
    await expect(page.locator("[data-checkout-commit]")).toHaveAttribute("data-checkout-commit", fixture.sha, { timeout: 15_000 });
    await expect(files).toHaveCount(0);
    await expect(page.locator('[data-checkout-ci="failed"]')).toBeVisible({ timeout: 15_000 });
    const requests = fs.readFileSync(fixture.log, "utf8");
    expect(requests).toContain("HideCheckoutChecks");
    await page.locator("[data-checkout-commit]").click();
    const history = page.locator("[data-commit-history]");
    await expect(history).toBeVisible();
    await expect(page.locator('[data-slot="popover-content"]')).toBeFocused();
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await expect(history.locator("[data-commit-history-item]")).toHaveCount(5);
    const failedRow = history.locator(`[data-commit-history-item="${fixture.sha}"]`);
    await expect(failedRow.locator('[data-commit-ci="failed"]')).toBeVisible();
    const placement = await failedRow.evaluate(row => {
      const title = row.querySelector("p")!.getBoundingClientRect();
      const time = row.querySelector("time")!.getBoundingClientRect();
      const ci = row.querySelector("[data-commit-ci]")!.getBoundingClientRect();
      return { titleBottom: title.bottom, timeTop: time.top, timeRight: time.right, ciTop: ci.top, ciLeft: ci.left };
    });
    expect(placement.ciTop).toBeGreaterThanOrEqual(placement.titleBottom);
    expect(placement.ciLeft).toBeGreaterThanOrEqual(placement.timeRight);
    await page.keyboard.press("Tab");
    await expect(failedRow.locator('[data-commit-ci="failed"]')).toBeFocused();
    await expect(page.getByRole("tooltip")).toBeVisible();
    await page.keyboard.press("Escape");
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
    await expect(page.locator("[data-checkout-ci]")).toHaveCount(0, { timeout: 15_000 });
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
    await expect(page.getByText(next.slice(0, 7), { exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(commit).toBeFocused();
    await screenshot(page, "checkout-header-browser");
  } finally {
    daemon?.stop(); herdr.stop();
  }
});

test("a commit pushed to origin shows as one to receive, with its row and GitHub page, without a manual fetch", async ({ page }) => {
  test.setTimeout(90_000);
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    // A GitHub-named origin whose fetch is rewritten to a local bare repository, so the header
    // makes commit pages while the fetch stays on this machine (the fixture allows `file` only).
    const root = fs.realpathSync(path.join(herdr.root, "fixture"));
    const bare = path.join(herdr.root, "origin.git");
    const origin = "https://github.com/example/freshness.git";
    const local = `file://${bare.replaceAll("\\", "/")}`;
    git(herdr.root, ["init", "--bare", bare]);
    git(root, ["init"]);
    git(root, ["commit", "--allow-empty", "-m", "base"]);
    git(root, ["remote", "add", "origin", origin]);
    git(root, ["config", `url.${local}.insteadOf`, origin]);
    git(root, ["push", "-u", "origin", "main"]);
    const theirs = path.join(herdr.root, "theirs");
    git(herdr.root, ["clone", local, theirs]);
    git(theirs, ["commit", "--allow-empty", "-m", "Pushed elsewhere"]);
    git(theirs, ["push", "origin", "main"]);
    const incoming = git(theirs, ["rev-parse", "HEAD"]).trim();
    daemon = await startHided(herdr, "checkout-freshness");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const upstream = page.locator('[data-checkout-upstream="tracked"]');
    await expect(upstream).toHaveText("1", { timeout: 30_000 });
    await upstream.click();
    await expect(page.locator("[data-checkout-remote-checked]")).toHaveText(/^Remote checked /);
    await page.keyboard.press("Escape");
    await page.locator("[data-checkout-commit]").click();
    const row = page.locator(`[data-commit-history-item="${incoming}"]`);
    await expect(row).toHaveAttribute("data-commit-incoming", "true");
    await expect(row.locator("[data-commit-link]")).toHaveAttribute("aria-label", "Pushed elsewhere · Open commit on GitHub");
    await screenshot(page, "checkout-freshness-browser");
    // The fetch moved only the remote-tracking ref: the branch and the files are the operator's.
    expect(git(root, ["rev-parse", "HEAD"]).trim()).not.toBe(incoming);
    expect(git(root, ["rev-parse", "origin/main"]).trim()).toBe(incoming);
  } finally {
    daemon?.stop(); herdr.stop();
  }
});
