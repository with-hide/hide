// B1-B7/B14: real cold restoration and one Hide-triggered terminal split.
import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startDraftTerminal } from "./group-start-helper";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { countSent, enterWorkspace, screenshot } from "./wire";

type Snapshot = {
  panes: { pane_id: string; tab_id: string; terminal_id: string; restore_error?: string }[];
  layouts: { tab_id: string; splits: { direction: string; ratio: number }[] }[];
};
const snapshot = (herdr: HerdrFixture) => (herdr.run(["api", "snapshot"]) as { result: { snapshot: Snapshot } }).result.snapshot;
test.describe.configure({ timeout: 120_000 });

test("Starting a terminal in a new draft and splitting it keeps one real Herdr tab and both terminal identities", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "coexistence-split");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page);
    const before = snapshot(herdr).panes.map((pane) => pane.pane_id).sort();
    await page.locator("[data-new-agent-tab]").first().click();
    await expect(page.locator("[data-agent-tab-start]")).toHaveAttribute("data-agent-tab-start", /^draft:/);
    expect(snapshot(herdr).panes.map((pane) => pane.pane_id).sort()).toEqual(before);
    await startDraftTerminal(page);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const tab = await page.locator("[data-canvas]").getAttribute("data-canvas");
    const first = await page.locator("[data-pane-view]").getAttribute("data-pane-view");
    await page.locator("[data-terminal-host]").click();
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.screenText())).toContain("fixture %");
    await page.keyboard.press(chord("split_right"));
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await expect.poll(() => sent.get("create_pane")).toBe(1);
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab!);
    const ids = await page.locator("[data-pane-view]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-pane-view")!).sort());
    expect(ids).toContain(first);
    await expect.poll(() => snapshot(herdr).panes.filter((pane) => pane.tab_id === tab).map((pane) => pane.pane_id).sort()).toEqual(ids);
    await expect.poll(() => snapshot(herdr).layouts.find((layout) => layout.tab_id === tab)?.splits).toEqual([expect.objectContaining({ direction: "right", ratio: 0.5 })]);
    const pane = ids.find((id) => id !== first)!;
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.screenText())).toContain("fixture %");
    await page.keyboard.type("printf 'COEXISTENCE_TERMINAL_OK\\n'");
    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.screenText())).toContain("COEXISTENCE_TERMINAL_OK");
    await expect.poll(() => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "visible"], { env: herdr.env, encoding: "utf8", timeout: 10_000 })).toContain("COEXISTENCE_TERMINAL_OK");
    await screenshot(page, "coexistence-new-tab-two-panes");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("a missing-folder cold restore shows failure without an agent and closes only after Herdr", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const folder = path.join(herdr.root, "fixture");
    await herdr.restart(() => fs.renameSync(folder, `${folder}-moved`));
    await expect.poll(() => snapshot(herdr).panes.filter((pane) => pane.restore_error).length).toBe(2);
    daemon = await startHided(herdr, "coexistence-restore");
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    const failed = page.locator("[data-pane-restore-failed]");
    await expect(failed).toHaveCount(2);
    await expect(failed.first()).toContainText(toPage(folder));
    await expect(page.locator("[data-agent-wake]")).toHaveCount(0);
    await screenshot(page, "coexistence-restoration-failed");
    const pane = await failed.first().getAttribute("data-pane-restore-failed");
    await failed.first().getByRole("button", { name: "Close pane", exact: true }).click();
    await expect.poll(() => snapshot(herdr).panes.some((item) => item.pane_id === pane)).toBe(false);
    await expect(failed).toHaveCount(1);
    await screenshot(page, "coexistence-restoration-peer-preserved");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("cold restoration failure retains a sleeping row and visiting it does not wake the agent", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "coexistence-sleep");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    const sleeper = herdr.panes[1];
    const row = page.locator(`nav[data-sidebar] [data-pane="${sleeper}"]`);
    await expect(row).toContainText("Agent two");
    await page.locator(`[data-pane-menu="${sleeper}"]`).click();
    await page.locator('[data-menu-item="sleep_agent"]').click();
    await expect(page.locator('[data-pane-sleep="sleeping"]')).toBeVisible();
    const other = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "other", "--no-focus"]) as { result: { tab: { tab_id: string } } };
    await page.locator(`[data-tab="${other.result.tab.tab_id}"]`).click();
    const folder = path.join(herdr.root, "fixture");
    daemon = await daemon.restart(async () => {
      await herdr.restart(() => fs.renameSync(folder, `${folder}-moved`));
    });
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    await page.locator(`[data-tab="${herdr.tab}"]`).click();
    const failed = page.locator(`[data-pane-restore-failed="${sleeper}"]`);
    await expect(failed).toBeVisible();
    await expect(row).toContainText("Agent two");
    await expect(row.locator('[data-agent-status="sleeping"]')).toBeVisible();
    await expect(page.locator("[data-agent-wake]")).toHaveCount(0);
    expect(sent.get("agent_wake") ?? 0).toBe(0);
    const agents = (herdr.run(["agent", "list"]) as { result: { agents: { pane_id: string }[] } }).result.agents;
    expect(agents.some((agent) => agent.pane_id === sleeper)).toBe(false);
    await failed.getByRole("button", { name: "Close pane", exact: true }).click();
    await expect.poll(() => snapshot(herdr).panes.some((pane) => pane.pane_id === sleeper)).toBe(false);
    await expect(row).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});
