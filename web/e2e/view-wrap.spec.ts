// A file or diff view opens with long lines wrapped, and its Wrap toggle turns
// that off for the one tab, through an isolated pinned Herdr, hided and
// browser. The fixture is disposable and never uses an operator checkout,
// socket or app.

import { expect, test, type Locator } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { showExplorer, showProjectRow, showTool, screenshot } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 90_000 });

/** Whether the scroller's content runs past its width, which only an unwrapped long line does. */
function overflows(scroller: Locator): Promise<boolean> {
  return scroller.evaluate((element) => element.scrollWidth > element.clientWidth + 1);
}

test("file and diff views open wrapped and Wrap turns it off per tab", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repoDir = path.join(herdr.root, "wrap-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    const file = path.join(repo, "long.txt");
    const line = (tag: string) => Array.from({ length: 80 }, (_, index) => `${tag}${index}`).join(" ");
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(file, `${line("base")}\n`);
    execFileSync("git", ["add", "-A"], { cwd: repo });
    execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "base"], { cwd: repo });
    fs.writeFileSync(file, `${line("base")}\n${line("added")}\n`);
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "wrap-repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    daemon = await startHided(herdr, "view-wrap");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "wrap-repo" }).locator("[data-checkout]").first())).click();

    await showExplorer(page);
    await page.locator(`[data-explorer-row="${toPage(file)}"]`).click();
    const fileView = page.locator('[data-editor-kind="file"]');
    const fileScroller = fileView.locator("[data-editor-codemirror] .cm-scroller");
    await expect(fileView.locator(".cm-content")).toContainText("added79");
    await expect(fileView.locator("[data-editor-wrap]")).toHaveAttribute("data-editor-wrap", "true");
    await expect.poll(() => overflows(fileScroller)).toBe(false);
    await screenshot(page, "view-wrap-file");
    await fileView.locator("[data-editor-wrap]").click();
    await expect(fileView.locator("[data-editor-wrap]")).toHaveAttribute("data-editor-wrap", "false");
    await expect.poll(() => overflows(fileScroller)).toBe(true);

    await showTool(page, "changes");
    await page.locator('[data-history-group="working"][data-history-path="long.txt"]').click();
    const diffView = page.locator('[data-editor-kind="diff"]');
    const diffScroller = diffView.locator("[data-patch-view] .cm-scroller");
    await expect(diffView.locator("[data-patch-view] .cm-content")).toContainText("added79");
    await expect(diffView.locator("[data-editor-wrap]")).toHaveAttribute("data-editor-wrap", "true");
    await expect.poll(() => overflows(diffScroller)).toBe(false);
    await screenshot(page, "view-wrap-diff");
    await diffView.locator("[data-editor-wrap]").click();
    await expect(diffView.locator("[data-editor-wrap]")).toHaveAttribute("data-editor-wrap", "false");
    await expect.poll(() => overflows(diffScroller)).toBe(true);
    // An unwrapped added row keeps its band past the viewport.
    const band = await diffView.locator(".cm-patch-added").evaluate((row) => row.scrollWidth);
    expect(band).toBeGreaterThan(await diffScroller.evaluate((element) => element.clientWidth));
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
