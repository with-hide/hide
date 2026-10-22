// S4's checkout-scoped History and read-only diff through an isolated pinned
// Herdr, hided and browser. The fixture is disposable and never uses an
// operator checkout, socket or app.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { countSent, registerFolder, screenshot, showExplorer, showProjectRow, showTool } from "./wire";
import { chord } from "./chords";
import { toPage } from "../../desktop/src/main/wirePath";
import { onDisk } from "./wait";

test.describe.configure({ timeout: 90_000 });

test("History opens a scoped patch, then updates after editing the original file", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repoDir = path.join(herdr.root, "history-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    const file = path.join(repo, "한글 notes.txt");
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(file, "first\n");
    execFileSync("git", ["add", "-A"], { cwd: repo });
    execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "base"], { cwd: repo });
    // The worktree reader takes the default from origin/HEAD, never guesses
    // it from the main worktree's current branch.
    execFileSync("git", ["update-ref", "refs/remotes/origin/main", "main"], { cwd: repo });
    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"], { cwd: repo });
    execFileSync("git", ["switch", "-q", "-c", "feature"], { cwd: repo });
    fs.writeFileSync(path.join(repo, "branch.txt"), "branch\n");
    execFileSync("git", ["add", "-A"], { cwd: repo });
    execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "branch"], { cwd: repo });
    fs.writeFileSync(file, "first\nsecond <script>window.__executed = true</script>\n");
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "history-repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    daemon = await startHided(herdr, "s4");
    const lastSent = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, lastSent);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "history-repo" }).locator("[data-checkout]").first())).click();
    // The tool column holds one tool; Changes swaps the Explorer out (PRD
    // lane-two-columns B14).
    await showExplorer(page);
    await showTool(page, "changes");
    await expect(page.locator("[data-explorer]")).toHaveCount(0);
    await expect(page.locator('[data-history-group-section="working"]')).toBeVisible();
    await expect(page.locator('[data-history-group-section="committed"]')).toBeVisible();
    const row = page.locator('[data-history-group="working"][data-history-path="한글 notes.txt"]');
    await expect(row).toHaveAttribute("aria-label", /Uncommitted: 한글 notes.txt, Modified/);
    await row.click();
    await expect(page.locator('[data-editor-kind="diff"]')).toBeVisible();
    await expect(page.locator('[data-patch-view] .cm-content')).toContainText("second <script>window.__executed = true</script>");
    expect(await page.evaluate(() => (window as Window & { __executed?: boolean }).__executed ?? false)).toBe(false);
    await expect(page.locator('[data-tab-kind="diff"]')).toHaveAttribute("data-preview", "true");
    // The click hands the keyboard to the diff it opened, out of History.
    await expect(page.locator('[data-patch-view] .cm-content')).toBeFocused();
    expect(sent.get("file_draft") ?? 0).toBe(0);
    expect(sent.get("file_save") ?? 0).toBe(0);
    await screenshot(page, "s4-working-diff");

    // An opened diff shows in the viewing column and Changes stays in the
    // tool column for the next row (PRD lane-two-columns B7).
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "documents");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-shows", "changes");
    await page.locator('[data-history-group="committed"][data-history-path="branch.txt"]').click();
    await expect(page.locator('[data-diff-group="committed"] [data-patch-view] .cm-content')).toContainText("+branch");
    await row.click();
    await expect(page.locator('[data-diff-group="working"]')).toBeVisible();
    // ⌘E closes the lane that shows, and again brings both columns back (B15).
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator('[data-lane="changes"]')).toBeVisible();
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "documents");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await showExplorer(page);

    const fileRow = page.locator(`[data-explorer-row="${toPage(file)}"]`);
    await expect(fileRow).toBeVisible();
    await fileRow.click();
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("first\nthird\n");
    await expect.poll(() => onDisk(file)).toBe("first\nthird\n");
    await showTool(page, "changes");
    await row.click();
    await expect(page.locator('[data-patch-view] .cm-content')).toContainText("+third");
    await expect(page.locator('[data-patch-view] .cm-content')).not.toContainText("second <script>");
    await expect(page.locator('[data-tab-kind="file"]')).toHaveAttribute("data-preview", "false"); // edited preview remains open
    const diffTab = page.locator('[data-tab-kind="diff"]');
    const diffDisplayId = await diffTab.getAttribute("data-display");
    await page.locator('[data-tab-kind="file"]').click();
    await expect(diffTab).toHaveAttribute("aria-selected", "false");
    await diffTab.dblclick();
    await expect(diffTab).toHaveAttribute("data-preview", "false");
    expect(lastSent.get("view_layout.keep_open")).toMatchObject({ action: "keep_open", display_id: diffDisplayId });
    expect(sent.get("file_draft")).toBeGreaterThanOrEqual(1);
    expect(sent.get("file_save")).toBeGreaterThanOrEqual(1);
    await screenshot(page, "s4-updated-diff");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("registered subfolder History opens inside patches and hides sibling changes", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const sent = countSent(page);
    const repo = path.join(fs.realpathSync(herdr.root), "fixture");
    const registered = path.join(repo, "registered");
    fs.mkdirSync(registered, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(path.join(registered, "inside.txt"), "inside base\n");
    fs.writeFileSync(path.join(repo, "outside.txt"), "outside base\n");
    execFileSync("git", ["add", "-A"], { cwd: repo });
    execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "base"], { cwd: repo });
    execFileSync("git", ["update-ref", "refs/remotes/origin/main", "main"], { cwd: repo });
    execFileSync("git", ["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"], { cwd: repo });
    execFileSync("git", ["switch", "-q", "-c", "feature"], { cwd: repo });
    fs.writeFileSync(path.join(registered, "branch.txt"), "inside committed\n");
    fs.writeFileSync(path.join(repo, "outside-branch.txt"), "outside committed\n");
    execFileSync("git", ["add", "-A"], { cwd: repo });
    execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "feature"], { cwd: repo });
    fs.writeFileSync(path.join(registered, "inside.txt"), "inside base\ninside working\n");
    fs.writeFileSync(path.join(repo, "outside.txt"), "outside base\noutside working\n");
    daemon = await startHided(herdr, "s4-nested", herdr.root);

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await registerFolder(page, daemon, registered);
    await expect.poll(() => sent.get("create_workspace") ?? 0).toBe(1);
    const project = await showProjectRow(page, page.locator("[data-project]", { hasText: "registered" }));
    await project.locator("[data-checkout]").first().click();
    await showTool(page, "changes");
    const history = page.locator('[data-history-root]');
    await expect(history).toHaveAttribute("data-history-root", toPage(registered));
    await expect(page.locator('[data-history-path="inside.txt"]')).toBeVisible();
    await expect(page.locator('[data-history-path="branch.txt"]')).toBeVisible();
    await expect(page.locator('[data-history-path="outside.txt"]')).toHaveCount(0);
    await expect(page.locator('[data-history-path="outside-branch.txt"]')).toHaveCount(0);

    await page.locator('[data-history-group="working"][data-history-path="inside.txt"]').click();
    await expect(page.locator('[data-patch-view] .cm-content')).toContainText("+inside working");
    await showTool(page, "changes");
    await page.locator('[data-history-group="committed"][data-history-path="branch.txt"]').click();
    await expect(page.locator('[data-patch-view] .cm-content')).toContainText("+inside committed");

    await page.locator('[data-view-new-tab]').click();
    const diff = page.locator('[data-new-tab-page]').getByRole("button", { name: "Diff", exact: true });
    await expect(diff).toBeVisible();
    const emptyId = await page.locator('[data-browser-display]').getAttribute("data-browser-display");
    await diff.click();
    await expect(page.locator('[data-palette-row$="/registered/inside.txt"]')).toBeVisible();
    await expect(page.locator('[data-palette-row$="/outside.txt"]')).toHaveCount(0);
    await page.locator('[data-palette-row$="/registered/inside.txt"]').click();
    await expect(page.locator(`[role="tab"][data-display="${emptyId}"]`)).toHaveAttribute("data-tab-kind", "diff");
    await expect(page.locator('[data-patch-view] .cm-content')).toContainText("+inside working");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
