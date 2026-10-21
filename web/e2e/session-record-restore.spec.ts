// The record restores the exact retained native conversation after its pane closes.
import { expect, test } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { labelAgent, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 120_000 });

function resumedPane(herdr: HerdrFixture, session: string): string | null {
  const listed = herdr.run(["pane", "list"]) as { result: { panes: { pane_id: string }[] } };
  for (const { pane_id } of listed.result.panes) {
    const read = spawnSync(herdr.bin, ["pane", "process-info", "--pane", pane_id], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    if (read.status !== 0) continue;
    const info = JSON.parse(read.stdout) as { result: { process_info: { foreground_processes: { argv?: string[] }[] } } };
    if (info.result.process_info.foreground_processes.some(process => process.argv?.join(" ") === `claude --resume ${session}`)) return pane_id;
  }
  return null;
}

test("a closed pane's record restores the same native session in a fresh pane", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    const source = herdr.panes[0];
    const session = labelAgent(herdr, source, { task: "Restore this conversation", end: "done", cwd: `${herdr.root}/fixture` });
    daemon = await startHided(herdr, "record-restore");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-view="${source}"] [data-pane-record]`).click();
    const record = page.locator('[data-session-record]');
    await expect(record).toContainText("Restore this conversation 진행해줘", { timeout: 30_000 });
    await expect(record.locator('[data-record-now-button="pane"]')).toBeEnabled();
    herdr.run(["pane", "close", source]);
    const restore = record.getByRole("button", { name: "Restore session", exact: true });
    await expect(restore).toBeEnabled({ timeout: 30_000 });
    await restore.click();
    let pane: string | null = null;
    await expect.poll(() => (pane = resumedPane(herdr, session)), { timeout: 45_000 }).not.toBeNull();
    expect(pane).not.toBe(source);
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible({ timeout: 30_000 });
    // The actual fixture process was launched with --resume, but sends no
    // hook identity. This is a locator test, not a real-agent observation.
    const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"]`);
    await expect(row.locator("[data-agent-title]")).toHaveText("Restore this conversation", { timeout: 30_000 });
    await expect(row.locator('[data-record-connection="unconfirmed"]')).toBeVisible();
    await expect(row.locator('[data-agent-status-mark="turn"]')).toHaveCount(0);
  } finally {
    await page.goto("about:blank");
    await daemon?.stop();
    herdr.stop();
  }
});
