// Settings › 스킬·MCP on an isolated machine whose store the real kit keeps
// (PRD skill-mcp-manager; decisions of 2026-11-19): the list a row unfolds
// in, the table its switch turns to, the cell states and the update dot.

import { expect, test } from "@playwright/test";
import { openStore, seedStore, startStore, stopStore } from "./skills-mcp-fixture";
import { chooseTheme, screenshot } from "./wire";

test.skip(process.platform === "win32", "the fixture's sources are POSIX git repositories over file://");
test.describe.configure({ timeout: 240_000 });

test("the list shows each row's summary and unfolds in place, and the table merges the rows every agent has", async ({ page }) => {
  const fixture = await startStore("store-views");
  try {
    await seedStore(fixture);
    await page.setViewportSize({ width: 1200, height: 1000 });
    await page.goto(`${fixture.daemon.origin}/#token=${fixture.daemon.token}`);
    await chooseTheme(page, "dark");
    const machine = await openStore(page, fixture.daemon);
    await expect(machine).toHaveAttribute("data-store-view", "list");

    // A row every shown agent has says so; one off everywhere says Off; one that differs names only where.
    await expect(machine.locator('[data-store-row="skill:archify"]')).toHaveAttribute("data-store-summary", "all");
    await expect(machine.locator('[data-store-row="skill:pdf-tools"]')).toHaveAttribute("data-store-summary", "off");
    await expect(machine.locator('[data-store-summary-part="antigravity:off"]')).toHaveText("Antigravity off");
    // A Grok file Hide cannot read fails Grok's cell of every server, not the others'.
    await expect(machine.locator('[data-store-row="mcp:github"] [data-store-summary-part="grok:failed"]')).toHaveText("Grok failed");
    await expect(machine.locator('[data-store-row="mcp:github"] [data-store-summary-part]')).toHaveCount(1);
    await expect(machine.locator('[data-store-version="skill:archify"]')).toHaveText("v0.4.1");
    await expect(machine.locator('[data-store-update-dot="skill:pdf-tools"]')).toBeVisible();
    await expect(machine.locator("[data-store-updates]")).toHaveText(/1 update/);
    await expect(machine.locator('[data-store-unmanaged="skill:ast-grep"]')).toBeVisible();

    // A row unfolds in place: its source and files, the per-folder switches, a failure with Retry, a hidden key.
    await machine.locator('[data-store-row-toggle="skill:aside-browser"]').click();
    const detail = machine.locator('[data-store-detail="skill:aside-browser"]');
    await expect(detail).toContainText("follows the folder");
    await expect(detail).toContainText("▲ 1 executable");
    await expect(detail.locator('[data-store-switch="skill:aside-browser:antigravity"]')).toHaveAttribute("aria-checked", "false");
    await expect(detail.locator('[data-store-switch="skill:aside-browser:claude"]')).toHaveAttribute("aria-checked", "true");
    await machine.locator('[data-store-row-toggle="mcp:github"]').click();
    const github = machine.locator('[data-store-detail="mcp:github"]');
    await expect(github.locator('[data-store-retry="mcp:github:grok"]')).toBeVisible();
    await expect(github.locator('[data-store-keys="mcp:github"]')).toContainText("GITHUB_TOKEN ••••••");
    await expect(page.locator("body")).not.toContainText("e2e-fixture-value");
    await screenshot(page, "store-list-dark");

    // The table: rows every agent has, or none, read as one dim line; a partial row shows its cells.
    await machine.locator('[data-store-view-choice="table"]').click();
    await expect(machine).toHaveAttribute("data-store-view", "table");
    await expect(machine.locator('[data-store-merged="skill:archify"]')).toHaveText("All agents");
    await expect(machine.locator('[data-store-merged="skill:pdf-tools"]')).toHaveText("Off");
    await expect(machine.locator('[data-store-cell="mcp:linear:codex"]')).toHaveAttribute("data-store-cell-state", "unsupported");
    await expect(machine.locator('[data-store-cell="mcp:github:grok"]')).toHaveAttribute("data-store-cell-state", "failed");
    await screenshot(page, "store-table-dark");
    // The choice is this viewer's, kept across a reload.
    await page.reload();
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="skills"]').click();
    await expect(page.locator(`[data-store-machine="${fixture.daemon.node}"]`)).toHaveAttribute("data-store-view", "table");

    await page.keyboard.press("Escape");
    await chooseTheme(page, "light");
    const light = await openStore(page, fixture.daemon);
    await screenshot(page, "store-table-light");
    await light.locator('[data-store-view-choice="list"]').click();
    await light.locator('[data-store-row-toggle="skill:aside-browser"]').click();
    await light.locator('[data-store-row-toggle="mcp:github"]').click();
    await screenshot(page, "store-list-light");
  } finally {
    stopStore(fixture);
  }
});
