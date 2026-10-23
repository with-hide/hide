// A private real core, native candidate window and scripted provider/GitHub facts.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { countSent, sendEvent, setLanePinned, showProjectMemory, showProjectRow } from "../../web/e2e/wire";
import { fitWindow, isolate, launch, screenshot, test } from "./fixture";
import { seedReconciliation } from "./memory-reconcile-fixture";
import { placedProgressBatch, progressGithub } from "./memory-progress-fixture";
import { installSpawnProvider } from "./agent-spawn-fixture";

/** Click the text's actual location, allowing the row's sibling disclosure to receive it. */
async function clickText(row: Locator, text: Locator) {
  const bounds = await row.boundingBox(), target = await text.boundingBox();
  if (!bounds || !target) throw new Error("Memory row/text has no rendered bounds");
  await row.click({ position: { x: target.x - bounds.x + target.width / 2, y: target.y - bounds.y + target.height / 2 } });
}

async function captureMemory(page: Page, tab: Locator, name: string) {
  await screenshot(page, name);
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (dir) await tab.screenshot({ path: path.join(dir, `${name}-panel.png`), animations: "disabled" });
}

test("Memory progress expands whole rows and follows the live promotion agent through its PR", async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "memory-progress");
  const github = progressGithub(herdr);
  // The generic echo shim declares no native session. Use the established
  // caller shim so the real spawn ledger can bind this promotion's pane.
  installSpawnProvider(herdr, run.root);
  let app: ElectronApplication | null = null;
  try {
    const repo = path.join(run.env.HOME!, "projects", "sample");
    fs.mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
    fs.writeFileSync(path.join(repo, "AGENTS.md"), "Use one unpooled connection per write.\n");
    execFileSync("git", ["add", "AGENTS.md"], { cwd: repo });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Initial guide"], { cwd: repo });
    execFileSync("git", ["remote", "add", "origin", "https://github.com/acme/memory-fixture.git"], { cwd: repo });
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "sample", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page, sent = new Map<string, Record<string, unknown>>();
    const candidate = await app.evaluate(({ BrowserWindow }) => ({ pid: process.pid, executable: process.execPath, windows: BrowserWindow.getAllWindows().length }));
    expect(candidate.windows).toBe(1);
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.writeFileSync(path.join(evidence, "memory-todo-candidate.json"), JSON.stringify({ candidate, home: run.env.HOME, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, fixture: "synthetic card/GitHub/agent data", capture: "candidate-window renderer through its own CDP" }, null, 2));
    countSent(page, sent);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "sample" }).locator("[data-checkout]").first())).click();
    await showProjectMemory(page);
    await page.locator('[data-memory-switch="off"]').click();
    await expect(page.locator('[data-memory="on"]')).toBeVisible({ timeout: 30_000 });
    seedReconciliation(run, "acme/memory-fixture");
    await page.reload();
    await expect(page.locator('[data-workspace-screen]')).toBeVisible();
    await expect(page.locator('[data-connection]')).toHaveCount(0);
    await showProjectMemory(page);
    const tab = page.locator('[data-lane="memory"]');
    const quiet = tab.locator('[data-memory-pr-group="72"]');
    await expect(quiet).toContainText("Awaiting review");
    await expect(quiet.locator("[data-memory-agent]")).toHaveCount(0);
    await captureMemory(page, tab, "memory-todo-rest");
    const question = tab.locator('[data-memory-card="1"]');
    await clickText(question, question.locator('[title="Run checks before merging"]'));
    await expect(question.locator('[data-memory-detail="1"]')).toContainText("Run checks before merging");
    await captureMemory(page, tab, "memory-todo-expanded-title");
    await question.locator('[data-memory-row-toggle="1"]').press("Space");
    await expect(question.locator('[data-memory-detail="1"]')).toHaveCount(0);
    await question.locator('[data-memory-row-toggle="1"]').press("Enter");
    await expect(question.locator('[data-memory-detail="1"]')).toBeVisible();
    await question.locator('[data-memory-row-toggle="1"]').click();
    await tab.locator('[data-memory-select="1"]').click();
    await tab.locator('[data-memory-select="2"]').click();
    await expect(question.locator('[data-memory-detail="1"]')).toHaveCount(0);
    await tab.locator("[data-memory-open-batch]").click();
    const progress = tab.locator('[data-memory-progress="true"][data-memory-card="1"]');
    await expect(progress.locator('[data-memory-agent="1"]')).toBeVisible({ timeout: 30_000 });
    await expect(progress).toContainText("Writing PR");
    await expect(progress).toContainText("m1 · m2");
    await expect(progress).toContainText("+1");
    await expect(progress.locator("[data-memory-elapsed]")).toBeVisible();
    const placed = placedProgressBatch(run);
    github.open(placed.branch);
    const url = new URL(page.url());
    const daemon = { origin: url.origin, token: new URLSearchParams(url.hash.slice(1)).get("token")! };
    const refresh = () => sendEvent(page, daemon, "github_request", { workspace_id: sent.get("memory_open")!.workspace_id, refresh: true });
    await refresh();
    await expect(progress).toHaveAttribute("data-memory-pr-group", "73", { timeout: 30_000 });
    await expect(progress).toContainText("CI failed");
    await expect(progress.locator('[data-memory-agent="1"]')).toBeVisible();
    await captureMemory(page, tab, "memory-todo-ci-failed-agent");
    await clickText(progress, progress.locator('[title="Run checks before merging"]'));
    await expect(progress.locator("[data-memory-preview]")).toHaveCount(2);
    await expect(progress.locator('[data-memory-preview="2"]')).toContainText("Use a pooled connection for writes");
    await progress.locator('[data-memory-agent="1"]').click();
    await expect(page.locator(`[data-pane-view="${placed.pane}"]`)).toHaveAttribute("data-focused", "true");
    expect(sent.get("focus_pane")?.pane_id).toBe(placed.pane);
    await showProjectMemory(page);
    await progress.locator('[data-memory-row-toggle="1"]').press("Space");
    await expect(progress.locator("[data-memory-progress-detail]")).toHaveCount(0);
    await fitWindow(app, { width: 1440, height: 900 });
    await setLanePinned(page, true);
    const divider = page.locator('[data-lane-divider="lane"]');
    for (let step = 0; step < 6; step++) await divider.press("ArrowRight");
    await expect.poll(() => page.locator("[data-lane-frame]").evaluate((element) => Math.round(element.getBoundingClientRect().width))).toBe(360);
    for (const language of ["en", "ko", "zh-CN", "ja"]) {
      await page.locator("[data-open-settings]").click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.keyboard.press("Escape");
      await showProjectMemory(page);
      await expect.poll(() => tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
      await captureMemory(page, tab, `memory-todo-narrow-${language}`);
    }
    github.settle(73, "MERGED");
    github.settle(72, "CLOSED");
    await refresh();
    await expect(tab.locator("[data-memory-pr-group]")).toHaveCount(0, { timeout: 30_000 });
    await expect(tab.locator('[data-memory-cards] [data-memory-card="1"]')).toContainText("AGENTS.md");
    await tab.locator("[data-memory-more]").click();
    await expect(tab.locator('[data-memory-cards] [data-memory-card="9"]')).toBeVisible();
  } finally {
    if (app) await app.close();
    run.cleanup();
    herdr.stop();
  }
});
