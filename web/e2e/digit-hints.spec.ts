// The browser host has no numbered chords (PRD electron-digit-shortcuts-hints
// B3, D-09): holding ⌘ or ⌥ shows no keycap, ⌘2 and ⌥2 send nothing, and the
// ⌘/ sheet lists every family as not on this host.

import { expect, test } from "@playwright/test";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord, MOD_KEYS } from "./chords";
import { quietFor } from "./wait";

test.describe.configure({ timeout: 120_000 });

test("no keycap on a ⌘ or ⌥ hold, nothing on ⌘2 or ⌥2, and the sheet says so", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  const made = herdr.run([
    "tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "second", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
  ]) as { result: { tab: { tab_id: string } } };
  try {
    daemon = await startHided(herdr, "digit-hints");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const canvas = page.locator("[data-canvas]").first();
    await expect(page.locator(`[data-tab="${made.result.tab.tab_id}"]`)).toBeVisible();
    await expect(canvas).toHaveAttribute("data-canvas", herdr.tab);
    await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();

    // B3: a hold reveals nothing here, on either modifier.
    for (const key of MOD_KEYS) await page.keyboard.down(key);
    await quietFor(page, 500, "a hold reveals nothing");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    for (const key of [...MOD_KEYS].reverse()) await page.keyboard.up(key);
    await page.keyboard.down("Alt");
    await quietFor(page, 500, "a hold of Alt alone reveals nothing");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    await screenshot(page, "digit-hints-browser-alt-hold");
    await page.keyboard.up("Alt");

    // B3: ⌘2 is not a chord of this host, so nothing moves.
    const focused = sent.get("focus_tab") ?? 0;
    await page.keyboard.press(chord("select_device_2", "electron"));
    await page.keyboard.press("Alt+Digit2");
    await quietFor(page, 600, "a chord with a modifier left down reveals nothing");
    expect(sent.get("focus_tab") ?? 0).toBe(focused);
    expect(sent.get("focus_pane") ?? 0).toBe(0);
    expect(sent.get("focus_device") ?? 0).toBe(0);
    await expect(canvas).toHaveAttribute("data-canvas", herdr.tab);

    // B3, B4: the sheet names every family, without a chord.
    await page.keyboard.press(chord("shortcuts"));
    const sheet = page.locator("[data-shortcut-sheet]");
    await expect(sheet).toBeVisible();
    await expect(sheet.locator('[data-shortcut="select_device_1"] kbd')).toHaveText("-");
    await expect(sheet.locator('[data-shortcut="select_agent_1"]')).toContainText("Select agent 1-9");
    await expect(sheet.locator('[data-shortcut-absent="select_agent_1"]')).toHaveText("not on this host");
    await expect(sheet.locator('[data-shortcut="select_device_1"]')).toContainText("Select device 1-9");
    await expect(sheet.locator('[data-shortcut-absent="select_device_1"]')).toHaveText("not on this host");
    await expect(sheet.locator('[data-shortcut="select_device_2"]')).toHaveCount(0);
    await screenshot(page, "digit-hints-browser-sheet");
    await page.keyboard.press("Escape");
    await expect(sheet).toHaveCount(0);
  } finally {
    herdr.run(["tab", "close", made.result.tab.tab_id]);
    daemon?.stop();
    herdr.stop();
  }
});
