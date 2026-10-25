// The sidebar's width (PRD sidebar-typography B10-B12) on an isolated pinned
// Herdr and hided: the right edge drags between 220 and 440 and stops there,
// the nav alone follows the pointer while the center and its terminal stay
// put, one ui_state_update lands the width on release, a reload keeps it, and
// a double-click returns it to 292. Captures land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { countSent, rest, screenshot, showProjectRow } from "./wire";
import { quietFor } from "./wait";

test.describe.configure({ timeout: 120_000 });

async function prompt(herdr: HerdrFixture, pane: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    if (read.status === 0 && read.stdout.includes("fixture %")) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`no prompt in pane ${pane}`);
}

/** `--size-rail`: the always-shown device rail, which the nav and its box carry beside the content column the width names. */
const RAIL = 48;

const width = async (element: Locator) => Math.round((await element.boundingBox())!.width);

/** Presses on the edge's centre, moves by `dx` in steps, and returns the release. */
async function dragEdge(page: Page, edge: Locator, dx: number): Promise<() => Promise<void>> {
  const box = (await edge.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y, { steps: 12 });
  return () => page.mouse.up();
}

test("the sidebar's edge drags between its bounds, lands once, survives a reload and resets on a double-click", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  const daemon = await startHided(herdr, "sidebar-width");
  try {
    const folder = path.join(herdr.root, "notes");
    fs.mkdirSync(folder);
    const created = herdr.run(["workspace", "create", "--cwd", folder, "--label", "notes", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
      result: { root_pane: { pane_id: string } };
    };
    await prompt(herdr, created.result.root_pane.pane_id);

    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const nav = page.locator("nav[data-sidebar]");
    const box = page.locator("[data-sidebar-box]");
    const edge = page.locator("[data-sidebar-edge]");
    const main = page.locator("main");
    // Open the folder's Workspace so a terminal sits beside the sidebar.
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: /^notes/ }).locator("[data-checkout]"))).click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(".xterm").first()).toBeVisible({ timeout: 20_000 });
    await rest(page);
    expect(await width(nav)).toBe(292 + RAIL);

    // B10: the edge shows the line under the pointer.
    const line = edge.locator("span");
    await expect(line).toHaveCSS("opacity", "0");
    await edge.hover();
    await expect(line).toHaveCSS("opacity", "1");
    await expect(edge).toHaveCSS("cursor", "col-resize");

    // B10, B12: dragged far past the maximum, the nav follows and stops at
    // 440 while the center and its terminal have not moved.
    const mainBefore = await main.boundingBox();
    const before = new Map(sent);
    const release = await dragEdge(page, edge, 400);
    expect(await width(nav)).toBe(440 + RAIL);
    expect(await width(box)).toBe(292 + RAIL);
    expect(await main.boundingBox()).toEqual(mainBefore);
    expect((sent.get("ui_state_update") ?? 0) - (before.get("ui_state_update") ?? 0)).toBe(0);
    expect((sent.get("terminal_resize") ?? 0) - (before.get("terminal_resize") ?? 0)).toBe(0);
    await screenshot(page, "sidebar-width-dragging");
    await release();
    // One event lands it, and the center takes the width once the core carries it.
    await expect.poll(() => (sent.get("ui_state_update") ?? 0) - (before.get("ui_state_update") ?? 0)).toBe(1);
    expect(last.get("ui_state_update")?.sidebar_width).toBe(440);
    await expect.poll(() => width(box)).toBe(440 + RAIL);
    expect(await width(nav)).toBe(440 + RAIL);
    await rest(page);
    await screenshot(page, "sidebar-width-440");

    // Dragged far past the minimum, it stops at 220.
    const shrink = await dragEdge(page, edge, -600);
    expect(await width(nav)).toBe(220 + RAIL);
    await shrink();
    await expect.poll(() => width(box)).toBe(220 + RAIL);
    expect(last.get("ui_state_update")?.sidebar_width).toBe(220);
    await rest(page);
    await screenshot(page, "sidebar-width-220");

    // B11: a reload keeps the width.
    await page.reload();
    await expect(nav).toBeVisible({ timeout: 20_000 });
    await expect.poll(() => width(nav)).toBe(220 + RAIL);

    // A press that does not move sends nothing; a double-click returns to 292 and keeps it.
    const quiet = new Map(sent);
    await edge.click();
    await quietFor(page, 300, "a click on the edge sends nothing");
    expect((sent.get("ui_state_update") ?? 0) - (quiet.get("ui_state_update") ?? 0)).toBe(0);
    await edge.dblclick();
    await expect.poll(() => width(box)).toBe(292 + RAIL);
    expect(last.get("ui_state_update")?.sidebar_width).toBe(292);
    await page.reload();
    await expect(nav).toBeVisible({ timeout: 20_000 });
    await expect.poll(() => width(nav)).toBe(292 + RAIL);
  } finally {
    await daemon.stop();
    await herdr.stop();
  }
});
