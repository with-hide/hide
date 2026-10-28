// Hide AI's usage limit on an isolated pinned Herdr and hided (PRD hide-ai-usage-limit B1 to B5, B10):
// the daemon's PATH carries a `claude` that answers `/usage` with a week we choose and hands everything
// else to the fixture shim, so the weekly figure the core judges is scripted and the operator's own
// quota is never read. Grok is a stand-in with no usage figure, which is what an unreadable agent is.
// How each state is worded is the component tests'; this checks the figures reach the screen from the
// core, the limit is stored, and a change takes effect on the running daemon.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { aiSettingsFile, startHided, type Daemon } from "./hided-fixture";
import { fixtureExecutable, fixtureProgram, fixtureToolPath } from "./platform-fixture";
import { enterWorkspace, screenshot, showProjectMemory } from "./wire";

test.describe.configure({ timeout: 150_000 });

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** A reset the way the CLI prints it: `Oct 19 at 9am (UTC)`. */
function printedReset(at: Date): string {
  const hour = at.getUTCHours();
  return `${MONTHS[at.getUTCMonth()]} ${at.getUTCDate()} at ${hour % 12 === 0 ? 12 : hour % 12}${hour < 12 ? "am" : "pm"} (UTC)`;
}

/** A `claude` that prints the week in `frame.json` for `/usage` and runs the fixture shim for anything else. */
function usageShim(herdr: HerdrFixture) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-limit-"));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  const frame = path.join(root, "frame.json");
  const real = path.join(herdr.root, "bin", fixtureExecutable("claude"));
  const reset = printedReset(new Date(Math.floor(Date.now() / 3_600_000) * 3_600_000 + (4 * 24 + 3) * 3_600_000));
  const setWeek = (percent: number) =>
    fs.writeFileSync(
      frame,
      JSON.stringify({ type: "result", subtype: "success", is_error: false, result: ["Current session: 4% used", `Current week (all models): ${percent}% used · resets ${reset}`].join("\n") }),
    );
  fixtureProgram(
    bin,
    "claude",
    `const fs = require("fs");
const args = process.argv.slice(2);
if (args.includes("/usage")) { process.stdout.write(fs.readFileSync(${JSON.stringify(frame)})); process.exit(0); }
const run = require("child_process").spawnSync(${JSON.stringify(real)}, args, { stdio: "inherit" });
process.exit(run.status ?? 1);
`,
  );
  return {
    setWeek,
    path: `${fixtureToolPath(bin)}`,
    cleanup: () => fs.rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }),
  };
}

/** `grok` answers its model list and nothing else: signed in for Hide AI, with no weekly figure. */
function grokStandIn(herdr: HerdrFixture): string {
  const dir = path.join(herdr.root, "stand-ins");
  fixtureProgram(dir, "grok", `if (process.argv[2] === "models") { console.log("* grok-fixture (default)"); process.exit(0); }\nprocess.exit(1);\n`);
  return dir;
}

type Stored = { fallback?: { provider: string }[]; usage_limit?: number | "none" };
const stored = (daemon: Daemon): Stored => JSON.parse(fs.readFileSync(aiSettingsFile(daemon.home), "utf8")) as Stored;

async function start(label: string, percent: number) {
  const herdr = await startHerdr();
  const repoDir = path.join(herdr.root, "limit-repo");
  fs.mkdirSync(repoDir);
  const repo = fs.realpathSync(repoDir);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
  herdr.run(["workspace", "create", "--cwd", repo, "--label", "limit-repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
  const usage = usageShim(herdr);
  usage.setWeek(percent);
  const daemon = await startHided(herdr, label, undefined, { PATH: `${grokStandIn(herdr)}${path.delimiter}${usage.path}`, CODEX_HOME: undefined });
  const stop = () => {
    daemon.stop();
    herdr.stop();
    usage.cleanup();
  };
  return { daemon, usage, stop };
}

async function openHideAi(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1100 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "limit-repo");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="hideAi"]').click();
  return page.locator("[data-hide-ai-tab]");
}

/** A capture is taken once the theme and tab transitions have ended (a frame shows them half way). */
async function settle(page: Page) {
  await page.evaluate(() => Promise.all(document.getAnimations().map((animation) => animation.finished.catch(() => undefined))));
}

/** The theme choice lives in Settings > General; the page follows it, not the browser's colour scheme. */
async function chooseTheme(page: Page, theme: "light" | "dark") {
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", theme);
}

/** Settings > Hide AI as it stands, in light and dark, when a capture folder is set; it ends in dark with Hide AI open. */
async function captureSettings(page: Page, name: string) {
  for (const theme of ["light", "dark"] as const) {
    await chooseTheme(page, theme);
    await page.locator('[data-settings-tab="hideAi"]').click();
    await expect(page.locator("[data-hide-ai-tab]")).toBeVisible();
    await settle(page);
    await screenshot(page, `usage-limit-${name}-${theme}`);
  }
}

/** Closes Settings and turns Auto memory on for the front Project, leaving the Hide AI tool on its Memory tab. */
async function openMemory(page: Page) {
  await page.keyboard.press("Escape");
  await showProjectMemory(page);
  const memory = page.locator('[data-lane="memory"]');
  await memory.locator('[data-hide-ai-tool-tab="memory"]').click();
  await expect(memory.locator('[data-memory-switch="off"]')).toBeEnabled({ timeout: 20_000 });
  await memory.locator('[data-memory-switch="off"]').click();
  await expect(memory.locator('[data-memory="on"]')).toBeVisible({ timeout: 20_000 });
  return memory;
}

/** The agent Hide AI asks now, as the tool's ⚙ names it; the sheet closes again. */
async function expectAsking(page: Page, memory: Locator, label: string) {
  await memory.locator("[data-hide-ai-settings-open]").click();
  await expect(page.locator("[data-hide-ai-provider-chip]")).toContainText(label);
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-hide-ai-settings]")).toHaveCount(0);
}

/** The Memory tab in light and dark: the theme is picked in Settings, which then closes. */
async function captureMemory(page: Page, name: string) {
  for (const theme of ["light", "dark"] as const) {
    await page.locator("[data-open-settings]").click();
    await chooseTheme(page, theme);
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-lane="memory"]')).toBeVisible();
    await settle(page);
    await screenshot(page, `usage-limit-${name}-${theme}`);
  }
}

test("a week under the limit shows its percent, and a lower limit moves Hide AI at once", async ({ page }) => {
  const { daemon, stop } = await start("limit-runs", 86);
  try {
    const tab = await openHideAi(page, daemon);
    // B1: 90 until chosen; the figure the core read is on the Runs on row (B3).
    await expect(tab.locator("[data-ai-usage-limit]")).toHaveAttribute("data-ai-usage-limit", "90");
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("Signed in · 86% this week", { timeout: 30_000 });
    await expect(tab.locator("[data-ai-using]")).toHaveCount(0);
    await captureSettings(page, "runs-settings");
    // B4: the Hide AI tool's ⚙ names the agent Hide AI asks now.
    const runsMemory = await openMemory(page);
    await expectAsking(page, runsMemory, "Claude Code");
    await captureMemory(page, "runs-memory");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="hideAi"]').click();

    // B2: 80 is stored and takes effect on the running daemon: 86 is past it.
    await tab.locator("[data-ai-usage-limit]").click();
    await page.getByRole("option", { name: "80%" }).click();
    await expect.poll(() => stored(daemon).usage_limit).toBe(80);
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("Past your limit · 86% this week");
    await expect(tab.locator("[data-ai-paused]")).toContainText("Claude Code is past your 80% limit");

    // B7: No limit never holds an agent back; the figure stays, the limit words go.
    await tab.locator("[data-ai-usage-limit]").click();
    await page.getByRole("option", { name: "No limit" }).click();
    await expect.poll(() => stored(daemon).usage_limit).toBe("none");
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("Signed in · 86% this week");
    await expect(tab.locator("[data-ai-paused]")).toHaveCount(0);
  } finally {
    stop();
  }
});

test("Runs on past the limit hands Hide AI to the next listed agent, which shows no figure", async ({ page }) => {
  const { daemon, stop } = await start("limit-fallback", 93);
  try {
    const tab = await openHideAi(page, daemon);
    await tab.locator("[data-ai-add-agent]").click();
    await page.locator("[data-ai-add-item='grok']").click();
    await expect.poll(() => stored(daemon).fallback?.map((entry) => entry.provider)).toEqual(["grok"]);
    // B4: Claude is past 90, Grok answers; B10: Grok has no figure, so it shows none and is never held back.
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("Past your limit · 93% this week", { timeout: 30_000 });
    await expect(tab.locator("[data-ai-using='grok']")).toContainText("Using Grok · Claude Code is past your 90% limit");
    await expect(tab.locator("[data-ai-fallback='grok'] [data-ai-state]")).not.toContainText("this week");
    await expect(tab.locator("[data-ai-paused]")).toHaveCount(0);
    await captureSettings(page, "fallback-settings");
    // B4: Memory asks Grok while Claude Code is past the limit, and says nothing is wrong.
    const memory = await openMemory(page);
    await expectAsking(page, memory, "Grok");
    await expect(memory.locator("[data-memory-ai='not_ready']")).toHaveCount(0);
    await captureMemory(page, "fallback-memory");

    // B11: the same rows in Korean at a narrow width wrap and never push the dialog sideways.
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    await expect(page.locator("html")).toHaveAttribute("lang", "ko");
    await page.locator('[data-settings-tab="hideAi"]').click();
    await page.setViewportSize({ width: 560, height: 1100 });
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("한도 초과 · 이번 주 93%");
    await expect(tab.locator("[data-ai-using='grok']")).toContainText("Claude Code");
    await settle(page);
    expect(await tab.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await screenshot(page, "usage-limit-fallback-settings-ko-narrow");
  } finally {
    stop();
  }
});

test("every agent past the limit pauses Hide AI, and Memory shows its dimmed state", async ({ page }) => {
  const { daemon, stop } = await start("limit-paused", 93);
  try {
    const tab = await openHideAi(page, daemon);
    // B5: the only listed agent is past the limit; nothing else on the tab changes.
    await expect(tab.locator("[data-ai-state='claude:ready']")).toContainText("Past your limit · 93% this week", { timeout: 30_000 });
    await expect(tab.locator("[data-ai-paused]")).toContainText("Claude Code is past your 90% limit");
    await expect(tab.locator("[data-ai-paused]")).toContainText("Hide AI is paused");
    await captureSettings(page, "paused-settings");

    const memory = await openMemory(page);
    await expect(memory.locator("[data-memory-ai='not_ready']")).toContainText("every agent is past your usage limit this week", { timeout: 20_000 });
    await captureMemory(page, "paused-memory");
  } finally {
    stop();
  }
});
