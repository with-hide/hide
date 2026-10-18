// The first-run agent choice against a daemon that runs the install kit: a Mac with no kit record
// is asked once and nothing is written to any agent before the answer; a Mac with a record never is.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { aiSettingsFile, startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";

// The guidance hooks Apply writes are not written on Windows (their documentation names no shell there).
test.skip(process.platform === "win32", "the first-run choice installs POSIX hooks");

const modal = (page: Page) => page.locator("[data-first-run]");
const tile = (page: Page, id: string, state: string) => page.locator(`[data-onboarding-tile="${id}:${state}"]`);
const read = (file: string) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");

/**
 * A private HOME with two agents installed (a program in `~/.local/bin`, which the kit searches, and the
 * folder each agent makes), a kit record only when `record`, and a daemon that runs the kit.
 */
async function start(record: boolean, options: { seedHideAi?: boolean } = {}) {
  const herdr = await startHerdr();
  const home = path.join(fs.mkdtempSync(path.join(herdr.root, "ob-")), "home");
  for (const folder of [".claude", ".cursor"]) fs.mkdirSync(path.join(home, folder), { recursive: true });
  fs.mkdirSync(path.join(home, ".local", "bin"), { recursive: true });
  for (const program of ["claude", "cursor-agent"]) fs.writeFileSync(path.join(home, ".local", "bin", program), "#!/bin/sh\n", { mode: 0o755 });
  if (record) {
    fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
    fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [] }));
  }
  const daemon = await startHided(herdr, "agent-onboarding", home, {}, true, options);
  return { daemon, home };
}

async function open(page: Page, daemon: Daemon) {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

test("a Mac with no kit record is asked once, the installed agents are on, and Apply writes only what is left on", async ({ page }) => {
  const { daemon, home } = await start(false);
  try {
    await open(page, daemon);
    await expect(modal(page)).toBeVisible({ timeout: 60_000 });
    await expect(modal(page)).toHaveAttribute("data-first-run", "agents");
    await expect(modal(page).locator('[data-brand-owl="agents"]')).toBeVisible();
    await expect(page.locator("[data-first-run-progress]")).toHaveCount(0);
    await expect(tile(page, "claude-code", "on")).toBeVisible();
    await expect(tile(page, "cursor", "on")).toBeVisible();
    // Nothing was written to any agent while the question was open.
    expect(read(path.join(home, ".claude", "settings.json"))).not.toContain("hide-subagents@");
    expect(fs.existsSync(path.join(home, ".cursor", "hooks.json"))).toBe(false);

    // B46: under Apply, the agent Hide AI will use (the fixture's signed-in Claude Code, first in the fixed order).
    await expect(page.locator("[data-onboarding-hide-ai]")).toHaveText("Hide AI uses Claude Code");
    await screenshot(page, "hide-ai-first-run");

    await tile(page, "claude-code", "on").click();
    await expect(tile(page, "claude-code", "off")).toHaveAttribute("aria-checked", "false");
    // Turned off, nothing chosen is signed in (Hide AI cannot use Cursor yet), so the line goes (B46).
    await expect(page.locator("[data-onboarding-hide-ai]")).toHaveCount(0);
    await page.locator("[data-onboarding-apply]").click();
    await expect(modal(page)).toHaveCount(0);
    await expect(page.locator('[data-first-run="notifications"], [data-first-run="project"]')).toHaveCount(0);
    await expect.poll(() => read(path.join(home, ".cursor", "hooks.json")), { timeout: 60_000 }).toContain("hide-guidance@");
    expect(read(path.join(home, ".claude", "settings.json"))).not.toContain("hide-subagents@");

    // Decided: a reload does not ask again.
    await page.reload();
    await expect(page.locator("[data-open-settings], [data-sidebar-title-name]").first()).toBeVisible();
    await expect(modal(page)).toHaveCount(0);
  } finally {
    daemon.stop();
  }
});

// D-18, B45, B46: with no Hide AI choice stored at all (the fixture's seed left out), the first-run
// rule, not a seeded file, picks the first agent that is on and signed in.
test("with no Hide AI choice stored, the first-run rule names the agent Hide AI will use", async ({ page }) => {
  const { daemon, home } = await start(false, { seedHideAi: false });
  try {
    expect(fs.existsSync(aiSettingsFile(home))).toBe(false);
    await open(page, daemon);
    await expect(modal(page)).toBeVisible({ timeout: 60_000 });
    await expect(page.locator("[data-onboarding-hide-ai]")).toHaveText("Hide AI uses Claude Code");
    // Asking the question chose nothing: the choice is written by Apply, not by looking.
    expect(fs.existsSync(aiSettingsFile(home))).toBe(false);
  } finally {
    daemon.stop();
  }
});

test("Apply is the only way out: Escape and a click outside do nothing, and the question outlasts a reload", async ({ page }) => {
  const { daemon, home } = await start(false);
  try {
    await open(page, daemon);
    await expect(modal(page)).toBeVisible({ timeout: 60_000 });
    await expect(page.locator("[data-onboarding-later]")).toHaveCount(0);
    await page.mouse.click(5, 5);
    await expect(modal(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(modal(page)).toBeVisible();
    // Nothing is installed, and the kit record still waits: a new window asks again.
    await page.reload();
    await expect(modal(page)).toBeVisible({ timeout: 60_000 });
    expect(read(path.join(home, ".claude", "settings.json"))).not.toContain("hide-subagents@");
    expect(fs.existsSync(path.join(home, ".cursor", "hooks.json"))).toBe(false);

    await page.locator("[data-onboarding-apply]").click();
    await expect(modal(page)).toHaveCount(0);
    await expect.poll(() => read(path.join(home, ".claude", "settings.json")), { timeout: 60_000 }).toContain("hide-subagents@");
  } finally {
    daemon.stop();
  }
});

test("a Mac that already has a kit record is never asked and keeps getting its default agents", async ({ page }) => {
  const { daemon, home } = await start(true);
  try {
    await open(page, daemon);
    await expect.poll(() => read(path.join(home, ".claude", "settings.json")), { timeout: 60_000 }).toContain("hide-subagents@");
    await expect(page.locator("[data-open-settings], [data-sidebar-title-name]").first()).toBeVisible();
    await expect(modal(page)).toHaveCount(0);
    // Not turned on by a pass: an agent beyond the defaults waits for the operator.
    expect(fs.existsSync(path.join(home, ".cursor", "hooks.json"))).toBe(false);
  } finally {
    daemon.stop();
  }
});
