// An agent's reference (UI_BEHAVIOR.md, the pane menu): the pane menu copies
// `@<name> (pane <id> · <task>)`, and a sidebar agent row dropped on another
// pane's terminal types that same reference and a space into the agent's
// PTY there, never Enter, and focuses it; a drop on the agent's own pane types
// nothing. A capture of the menu lands in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { quietFor } from "./wait";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

/** A native drag of the row's open button onto the pane, as the browser dispatches it. */
function dragRowOnto(page: Page, row: string, pane: string): Promise<void> {
  return page.evaluate(({ row, pane }) => {
    const source = document.querySelector(`[data-agent-open="${row}"]`)!;
    const target = document.querySelector(`[data-pane-view="${pane}"]`)!;
    const data = new DataTransfer();
    source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: data }));
    const over = new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data });
    target.dispatchEvent(over);
    if (!over.defaultPrevented) throw new Error("the terminal ignored the agent drag");
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    source.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: data }));
  }, { row, pane });
}

test("an agent row dropped on another terminal types its reference there and nothing on its own pane", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  // Both fixture panes run the fake agent, whose input log is what its PTY received.
  const herdr = await startHerdr();
  const daemon = await startHided(herdr, "agent-reference-drop").catch((error) => { herdr.stop(); throw error; });
  try {
    const [target, agent] = herdr.panes;
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: daemon.origin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator(`nav[data-sidebar] [data-agent-open="${agent}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-pane-view="${target}"]`)).toBeVisible();

    // The pane menu's one copy item carries the name, the pane id and the task.
    await page.locator(`[data-pane-menu="${agent}"]`).click();
    await expect(page.locator('[data-menu-item="copy_reference"]')).toBeVisible();
    await screenshot(page, "agent-reference-pane-menu");
    await page.locator('[data-menu-item="copy_reference"]').click();
    const copied = () => page.evaluate(() => navigator.clipboard.readText());
    await expect.poll(copied).toMatch(new RegExp(`^@[^\\r\\n]+ \\(pane ${agent}( · [^\\r\\n]+)?\\)$`));
    const reference = await copied();

    const read = (log: string) => (fs.existsSync(log) ? fs.readFileSync(log, "utf8") : "");
    const [targetLog, agentLog] = herdr.inputLogs.map((log) => ({ log, before: read(log).length }));
    // A drag with the real pointer: the mouse events around it must not end it early.
    await page.locator(`nav[data-sidebar] [data-agent-open="${agent}"]`).dragTo(page.locator(`[data-pane-view="${target}"] [data-terminal-host]`));
    await expect.poll(() => read(targetLog!.log).slice(targetLog!.before)).toContain(`${reference} `);
    expect(read(targetLog!.log).slice(targetLog!.before)).not.toMatch(/[\r\n]/);
    await expect(page.locator(`[data-pane-view="${target}"]`)).toHaveAttribute("data-focused", "true");
    // The drag let go of the page: hover cards and hints open again.
    await expect(page.locator("html")).not.toHaveAttribute("data-view-drag");

    // Its own pane refuses the drop: the id would name the agent to itself.
    await dragRowOnto(page, agent, agent);
    await quietFor(page, 500, "a refused drop types nothing into the agent's own pane");
    expect(read(agentLog!.log).slice(agentLog!.before)).toBe("");
    await expect(page.locator(`[data-pane-view="${target}"]`)).toHaveAttribute("data-focused", "true");
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
