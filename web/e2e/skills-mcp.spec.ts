// Settings › 스킬·MCP on an isolated machine whose store the real kit keeps
// (PRD skill-mcp-manager; decisions of 2026-11-19): the list a row unfolds
// in, the table its switch turns to, the cell states and the update dot.

import { expect, test } from "@playwright/test";
import { withControl } from "./daemon-actions";
import { writeSshHost } from "./hided-fixture";
import { everyAgentOn, install, moveUpstream, openStore, seedStore, startStore, stopStore } from "./skills-mcp-fixture";
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

test("a skill is added from a git address and from a folder, picked and confirmed, and an MCP paste takes its key on the screen only", async ({ page }) => {
  const fixture = await startStore("store-add");
  try {
    await everyAgentOn(fixture.daemon);
    const machine = await openStore(page, fixture.daemon);
    await expect(machine.locator(`[data-store-empty="${fixture.daemon.node}"]`)).toContainText("Nothing on This Mac yet");

    // A source with several skills asks which; the ones left ticked are confirmed with their files and agents.
    await machine.locator("[data-store-empty-add]").click();
    await page.locator("[data-store-add-input]").fill(fixture.repos.archify);
    await page.locator("[data-store-add-next]").click();
    const pick = page.locator('[data-store-dialog="pick"]');
    await expect(pick).toContainText("3 skills found", { timeout: 60_000 });
    await pick.locator('[data-store-pick="archify-legacy"] button[role="checkbox"]').click();
    await screenshot(page, "store-add-pick");
    await pick.locator("[data-store-pick-next]").click();
    const confirm = page.locator('[data-store-dialog="install"]');
    await expect(confirm).toContainText("archify/scripts/render.py ▲ executable");
    await expect(confirm.locator("[data-store-agent-chip]")).toHaveCount(7);
    await expect(confirm).toContainText("Every profile included");
    await screenshot(page, "store-add-confirm");
    await confirm.locator("[data-store-install]").click();
    await expect(machine.locator('[data-store-row="skill:archify"]')).toBeVisible({ timeout: 60_000 });
    await expect(machine.locator('[data-store-row="skill:archify-export"]')).toBeVisible();
    await expect(machine.locator('[data-store-row="skill:archify-legacy"]')).toHaveCount(0);

    // A folder with one skill goes straight to its confirm.
    await machine.locator("[data-store-add]").click();
    await page.locator("[data-store-add-input]").fill(fixture.folder);
    await page.locator("[data-store-add-next]").click();
    await expect(page.locator('[data-store-dialog="install"]')).toContainText("Install 1 skill", { timeout: 60_000 });
    await page.locator("[data-store-install]").click();
    await expect(machine.locator('[data-store-row="skill:aside-browser"]')).toBeVisible({ timeout: 60_000 });

    // An MCP paste fills the fields; the key's value is typed here, hidden, and never shown again.
    await machine.locator("[data-store-add]").click();
    await page.locator("[data-store-add-input]").fill("npx -y @modelcontextprotocol/server-github");
    await page.locator("[data-store-add-next]").click();
    const fields = page.locator('[data-store-dialog="mcp"]');
    await expect(fields.locator("[data-store-mcp-line]")).toHaveValue("npx -y @modelcontextprotocol/server-github", { timeout: 60_000 });
    await fields.locator("[data-store-mcp-name]").fill("github");
    await fields.locator("[data-store-mcp-extra]").fill("GITHUB_TOKEN");
    await fields.getByRole("button", { name: "Add", exact: true }).click();
    await fields.locator('[data-store-key="GITHUB_TOKEN"]').fill("e2e-typed-value");
    await expect(fields.locator('[data-store-key="GITHUB_TOKEN"]')).toHaveAttribute("type", "password");
    await screenshot(page, "store-mcp-fields");
    await fields.locator("[data-store-mcp-next]").click();
    const mcp = page.locator('[data-store-dialog="mcp-confirm"]');
    await expect(mcp).toContainText("GITHUB_TOKEN ••••••");
    await screenshot(page, "store-mcp-confirm");
    await mcp.locator("[data-store-mcp-add]").click();
    await machine.locator('[data-store-row-toggle="mcp:github"]').click({ timeout: 60_000 });
    await expect(machine.locator('[data-store-keys="mcp:github"]')).toContainText("GITHUB_TOKEN ••••••");
    await expect(page.locator("body")).not.toContainText("e2e-typed-value");
    const snapshot = await withControl(fixture.daemon, (control) => control.until((rest) => JSON.stringify(rest), "snapshot"));
    expect(snapshot).not.toContain("e2e-typed-value");
  } finally {
    stopStore(fixture);
  }
});

test("an update shows what changes before it is taken, a delete can be undone, and a deleted name taken again cannot come back", async ({ page }) => {
  const fixture = await startStore("store-update");
  try {
    await everyAgentOn(fixture.daemon);
    await install(fixture.daemon, { kind: "git", url: fixture.repos.pdf }, ["pdf-tools"]);
    await install(fixture.daemon, { kind: "git", url: fixture.repos.archify }, ["archify"]);
    await moveUpstream(fixture);
    const machine = await openStore(page, fixture.daemon);

    // 업데이트 n walks the items a check found newer, each shown as its diff first (B23).
    await machine.locator("[data-store-updates]").click();
    const update = page.locator('[data-store-dialog="update"]');
    await expect(update.locator("[data-store-diff-files]")).toContainText("scripts/merge.py", { timeout: 60_000 });
    await expect(update).toContainText("▲ new executable");
    await expect(update).toContainText("Add merge command");
    await screenshot(page, "store-update-confirm");
    await update.locator("[data-store-update-confirm]").click();
    await expect(machine.locator('[data-store-update-dot="skill:pdf-tools"]')).toHaveCount(0, { timeout: 60_000 });
    await expect(machine.locator("[data-store-updates]")).toHaveCount(0);

    // A delete leaves the row's notice with Undo, which brings the row back whole.
    await machine.locator('[data-store-row-toggle="skill:archify"]').click();
    await machine.locator('[data-store-delete="skill:archify"]').click();
    await expect(machine.locator('[data-store-row="skill:archify"]')).toHaveCount(0, { timeout: 60_000 });
    const toast = machine.locator('[data-store-undo="skill:archify"]');
    await expect(toast).toContainText("Deleted archify");
    await screenshot(page, "store-undo");
    await toast.locator("[data-store-undo-restore]").click();
    await expect(machine.locator('[data-store-row="skill:archify"]')).toBeVisible({ timeout: 60_000 });

    // Deleted again and installed again under the same name, its undo entry cannot come back over it.
    await machine.locator('[data-store-row-toggle="skill:archify"]').click();
    await machine.locator('[data-store-delete="skill:archify"]').click();
    await expect(machine.locator('[data-store-row="skill:archify"]')).toHaveCount(0, { timeout: 60_000 });
    await install(fixture.daemon, { kind: "git", url: fixture.repos.archify }, ["archify"]);
    await machine.locator('[data-store-trash] summary').click();
    const entry = machine.locator("[data-store-trash-entry]").first();
    await entry.locator("[data-store-restore]").click();
    await expect(entry.locator("[data-store-trash-taken]")).toContainText("archify cannot come back while an item of that name is in the list", { timeout: 60_000 });
  } finally {
    stopStore(fixture);
  }
});

test("another machine is picked at the head, and one that is not connected says so with Reconnect and no controls", async ({ page }) => {
  const fixture = await startStore("store-devices", { ssh: true });
  try {
    writeSshHost(fixture.daemon, "studio");
    await withControl(fixture.daemon, async (control) => {
      control.send("register_device", { label: "studio", ssh_alias: "studio", host_consent: true });
      await control.until((rest) => JSON.stringify(rest.navigator).includes('"studio"'), "studio registered", 30_000);
    });
    const machine = await openStore(page, fixture.daemon);
    await machine.locator("[data-store-machines]").click();
    await page.locator('[data-store-machine-choice="studio"]').click();
    const studio = page.locator('[data-store-machine="studio"]');
    await expect(studio.locator('[data-store-away="studio"]')).toContainText("studio is not connected", { timeout: 60_000 });
    await expect(studio.locator("[data-store-add]")).toBeDisabled();
    await screenshot(page, "store-device-away");
  } finally {
    stopStore(fixture);
  }
});
