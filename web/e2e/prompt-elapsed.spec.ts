// The row measures the latest actual prompt, independently of state and daemon lifetime.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { claudeProjects, continueFixtureTranscript, labelAgent, setFixtureLifecycle, setFixtureSession, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, showTasks } from "./wire";

test("prompt elapsed survives state changes and restart, advances only on a prompt, and clears for a new session", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | null = null;
  try {
    const pane = herdr.panes[0]!;
    const at = Date.now() - 3 * 3_600_000 - 60_000;
    const session = labelAgent(herdr, pane, { task: "마지막 요청 시간 확인", at });
    daemon = await startHided(herdr, "prompt-elapsed");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const projectTime = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"] [data-agent-elapsed]`);
    await expect(projectTime).toHaveText("3h", { timeout: 30_000 });
    await setFixtureLifecycle(herdr, pane, "working");
    await expect(projectTime).toHaveText("3h");
    const tasks = await showTasks(page);
    const taskTime = tasks.locator(`[data-task-row="${pane}"] [data-agent-elapsed]`);
    await expect(taskTime).toHaveText("3h");
    // A restart must follow the completed asynchronous save, not just its rendered label.
    await expect.poll(() => {
      const file = path.join(daemon!.stateDir, "labels.json");
      return fs.existsSync(file) ? fs.readFileSync(file, "utf8").includes(String(at)) : false;
    }).toBe(true);
    await page.goto("about:blank");
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(taskTime).toHaveText("3h", { timeout: 30_000 });
    continueFixtureTranscript(herdr, session, { task: "마지막 요청 시간 확인", at: Date.now() - 7 * 60_000 - 5_000 });
    await setFixtureLifecycle(herdr, pane, "blocked");
    await expect(taskTime).toHaveText("7m", { timeout: 30_000 });
    // Metadata alone cannot manufacture a prompt time or inherit the old session's.
    fs.writeFileSync(path.join(claudeProjects(herdr), "e2e", "no-prompt.jsonl"), `${JSON.stringify({ type: "system", sessionId: "no-prompt", timestamp: new Date().toISOString(), subtype: "init" })}\n`);
    setFixtureSession(herdr, pane, "no-prompt");
    await expect(tasks.locator(`[data-task-row="${pane}"]`)).toBeVisible();
    await expect(taskTime).toHaveCount(0, { timeout: 30_000 });
    continueFixtureTranscript(herdr, "no-prompt", { task: "새 세션 요청", at: Date.now() - 2 * 60_000 - 5_000 });
    await setFixtureLifecycle(herdr, pane, "working");
    await expect(taskTime).toHaveText("2m", { timeout: 30_000 });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
