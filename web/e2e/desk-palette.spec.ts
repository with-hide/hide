import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { startHided } from "./hided-fixture";
import { chord } from "./chords";
import { countSent, enterWorkspace, screenshot } from "./wire";

// PRD desk-2-ui B37, B38, B39: ⌘P and ⌘K while Desk is in front, on an isolated
// engine and hided. With no session open beside Desk, ⌘P searches every
// project at once, each result led by its project, and a pick is opened as the
// preview of the tab panel without leaving Desk; ⌘K finds the same rows as
// elsewhere. Every other screen keeps its palettes (`Palette.desk.test.tsx`,
// `SearchPalette.desk.test.tsx` hold the rules; this holds the wire and the
// screen). Captured for the design comparison.

test.describe.configure({ timeout: 120_000 }); // wait-ok: a daemon, a page and the engine finish their real startup before Desk shows the files

/** Settings › Labs › Desk's switch turned on, as an operator does. */
async function switchDeskOn(page: Page) {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="labs"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== "true") await use.click();
  await expect(use).toHaveAttribute("data-desk-use", "true");
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
}

test("⌘P on Desk with no session searches every project, and a pick opens as the tab panel's preview while Desk stays (B37, B38)", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture({ agents: false });
  const daemon = await startHided(engine, "desk-palette-files");
  try {
    const folder = path.join(engine.root, "fixture");
    fs.mkdirSync(path.join(folder, "web", "src"), { recursive: true });
    fs.writeFileSync(path.join(folder, "web", "src", "token.ts"), "export const token = 1;\n");
    fs.writeFileSync(path.join(folder, "token-notes.md"), "# token\n");
    const sent = countSent(page);
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await switchDeskOn(page);
    await page.locator("[data-desk-sidebar-row]").click();
    await expect(page.locator("[data-desk-screen]")).toBeVisible();

    await page.keyboard.press(chord("open_file"));
    const palette = page.locator('[data-palette="Open file"]');
    await expect(palette).toBeVisible();
    await expect(palette.locator("[data-palette-input]")).toHaveAttribute("placeholder", "Search files in every project");
    await palette.locator("[data-palette-input]").fill("token");
    const rows = palette.locator("[data-palette-row]");
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) await expect(row).toContainText("fixture ›");
    await screenshot(page, "palette-p-all-projects");

    const before = sent.get("desk_files") ?? 0;
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
    await expect.poll(() => sent.get("desk_files") ?? 0).toBe(before + 1);
    // Desk stays the screen.
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(page.locator("[data-desk-tab-panel]")).toBeVisible();
    await screenshot(page, "palette-p-all-opened");
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("⌘K on Desk lists the sessions, and Escape hands the screen back to Desk (B39)", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  const daemon = await startHided(engine, "desk-palette-search");
  try {
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await switchDeskOn(page);
    await page.locator("[data-desk-sidebar-row]").click();
    await expect(page.locator("[data-desk-screen]")).toBeVisible();

    await page.keyboard.press(chord("search"));
    const palette = page.locator('[data-palette="Search"]');
    await expect(palette).toBeVisible();
    await palette.locator("[data-palette-input]").fill("Agent");
    await expect(palette.locator('[data-palette-group="agents"] [data-palette-row]').first()).toBeVisible();
    await screenshot(page, "palette-k-desk");
    await page.keyboard.press("Escape");
    await expect(palette).toHaveCount(0);
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("⌘P with a session beside Desk searches its checkout and opens the file in the tab panel beside the session; ⌘↵ opens it to the side (B37)", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const engine = await startFixture();
  const daemon = await startHided(engine, "desk-palette-session");
  try {
    const folder = path.join(engine.root, "fixture");
    fs.mkdirSync(path.join(folder, "web", "src"), { recursive: true });
    fs.writeFileSync(path.join(folder, "web", "src", "login-retry.ts"), "export const retry = 3;\n");
    fs.writeFileSync(path.join(folder, "web", "src", "login-retry.test.ts"), "export {};\n");
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await switchDeskOn(page);
    await page.locator("[data-desk-sidebar-row]").click();
    await page.locator(`[data-agent-open="${engine.panes[0]}"]`).click();
    await expect(page.locator("[data-desk-room]")).toHaveAttribute("data-desk-room", "session");

    await page.keyboard.press(chord("open_file"));
    const palette = page.locator('[data-palette="Open file"]');
    await expect(palette.locator("[data-palette-input]")).toHaveAttribute("placeholder", "Search files by name");
    await palette.locator("[data-palette-input]").fill("login-retry");
    await expect(palette.locator("[data-palette-row]")).toHaveCount(2);
    await screenshot(page, "palette-p-session");
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
    const panel = page.locator("[data-desk-tab-panel]");
    await expect(panel).toBeVisible();
    await expect(page.locator("[data-desk-tab]")).toHaveCount(1);
    await screenshot(page, "palette-p-session-opened");

    await page.keyboard.press(chord("open_file"));
    await palette.locator("[data-palette-input]").fill("login-retry.test");
    await expect(palette.locator("[data-palette-row]")).toHaveCount(1);
    await page.keyboard.press(`${process.platform === "darwin" ? "Meta" : "Control"}+Enter`);
    await expect(palette).toHaveCount(0);
    await expect(page.locator("[data-desk-tab]")).toHaveCount(2);
    await screenshot(page, "palette-p-session-beside");
    // Desk stays the screen.
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
  } finally {
    daemon.stop();
    engine.stop();
  }
});
