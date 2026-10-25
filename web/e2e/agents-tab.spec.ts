// Settings > Agents on an isolated pinned Herdr and a hided that runs the install kit (PRD settings-cleanup
// B8 to B20): the seven supported agents of this Mac, the ones whose program the private HOME holds under
// Installed and the rest folded under Not installed, a switch that writes and removes Hide's own entries,
// the Partial popover from its chip by keyboard, and a hook the operator removed shown on its row with
// Reinstall.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";

// The kit writes POSIX hook files; the guidance hooks are not written on Windows.
test.skip(process.platform === "win32", "the kit installs POSIX hooks");
test.describe.configure({ timeout: 120_000 });

const read = (file: string) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");

/**
 * A private HOME with five active agents installed and a retired Cursor program that must produce no row
 * (programs in `~/.local/bin`, which the kit searches; Pi and agy have none), and a kit record, so Claude Code
 * and Codex are on and the rest wait. The kit also searches the
 * usual install folders such as `/opt/homebrew/bin`, so the agents left out are ones a developer Mac is least
 * likely to have there.
 */
async function start(label: string) {
  const herdr = await startHerdr({ readiness: "registered" });
  const home = path.join(fs.mkdtempSync(path.join(herdr.root, "ag-")), "home");
  for (const folder of [".claude", ".codex", ".grok/hooks", ".cursor"]) fs.mkdirSync(path.join(home, folder), { recursive: true });
  fs.mkdirSync(path.join(home, ".local", "bin"), { recursive: true });
  for (const program of ["claude", "codex", "grok", "opencode", "omp", "cursor-agent"]) fs.writeFileSync(path.join(home, ".local", "bin", program), "#!/bin/sh\n", { mode: 0o755 });
  fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
  fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [] }));
  const daemon = await startHided(herdr, label, home, {}, true);
  return { herdr, daemon, home };
}

async function openAgents(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  return page.locator(`[data-agents-machine-list="${daemon.node}"]`);
}

test("lists the installed agents with a status and a switch, and folds the others under Not installed with an install link", async ({ page }) => {
  const { herdr, daemon } = await start("agents-list");
  try {
    const sent = countSent(page);
    const list = await openAgents(page, daemon);
    // B8, B9: the programs the home holds are Installed, in the fixed order; the rest are folded.
    await expect(list.locator("[data-agent-row]")).toHaveCount(7, { timeout: 60_000 });
    await expect(list).toContainText("Installed 5");
    await expect(list).toContainText("Not installed 2");
    const rows = await list.locator("[data-agent-row]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-agent-row")));
    expect(rows).toEqual([`${daemon.node}:claude-code:on`, `${daemon.node}:codex:on`, `${daemon.node}:grok:off`, `${daemon.node}:opencode:off`, `${daemon.node}:omp:off`, "pi:not-installed", "antigravity:not-installed"]);
    // B31: even an installed retired program has neither a row nor a switch.
    await expect(list.locator('[data-agent-row*="cursor"], [data-agent-switch*="cursor"]')).toHaveCount(0);
    // One machine: no switch at the top, and no 'Skill and session hook' line on any row (B11, B12).
    await expect(page.locator("[data-agents-machines]")).toHaveCount(0);
    await expect(list).not.toContainText("Skill and session hook");

    // B16: an agent that is on and has no session is Ready; one that is off has no status.
    await expect(list.locator(`[data-agent-status="${daemon.node}:codex:ready"]`)).toHaveText(/Ready/);
    await expect(list.locator(`[data-agent-row="${daemon.node}:grok:off"] [data-agent-status]`)).toHaveCount(0);

    // B16: the fixture's two Claude Code panes are running sessions, and the row says how many and nothing more.
    await expect(list.locator(`[data-agent-status="${daemon.node}:claude-code:sessions"]`)).toHaveText(/2 sessions/);
    await expect(list.locator(`[data-agent-row="${daemon.node}:claude-code:on"]`)).not.toContainText("connected");

    // B8: Not installed is closed until opened and each row's Install opens the vendor's guide.
    const fold = list.locator("[data-agents-not-installed]");
    await expect(fold).not.toHaveAttribute("open", "");
    await fold.locator("summary").click();
    await expect(fold.locator("[data-agent-install]")).toHaveCount(2);
    await expect(fold.locator('[data-agent-install="pi"]')).toHaveAttribute("href", "https://pi.dev/");
    await screenshot(page, "agents-list");

    // B10: Check again asks the kit to read the machine again (one `kit_check` frame on top of the one the open
    // tab sent) and the list stays as it was; the in-progress and failed states come from the kit snapshot's
    // `checking` and `check_failed`, which the runtime and component tests own.
    const opened = sent.get("kit_check") ?? 0;
    await list.locator("[data-agents-check]").click();
    await expect.poll(() => sent.get("kit_check") ?? 0).toBe(opened + 1);
    await expect(list.locator("[data-agents-check]")).toHaveAttribute("data-agents-check", "idle");
    await expect(list.locator("[data-agent-row]")).toHaveCount(7);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("the Partial chip opens by keyboard with a mark and a word per feature, and Escape returns to the chip", async ({ page }) => {
  const { herdr, daemon } = await start("agents-partial");
  try {
    const list = await openAgents(page, daemon);
    const chip = list.locator('[data-agent-partial="grok"]');
    await expect(chip).toBeVisible({ timeout: 60_000 });
    // B18: Partial is on every agent Hide does only some things for, on or off; a Full agent has none, and omp is Full
    // through Hide's extension (PRD pi-omp-extension).
    for (const id of ["claude-code", "omp"]) await expect(list.locator(`[data-agent-partial="${id}"]`)).toHaveCount(0);
    await chip.focus();
    await page.keyboard.press("Enter");
    const popover = page.locator('[data-agent-partial-popover="grok"]');
    await expect(popover).toBeVisible();
    await expect(popover.locator("[data-agent-feature]").first()).toBeVisible();
    // A mark and a word for each feature; the table is the kit's, so a '–' is something Hide does not do.
    await expect(popover.locator("[data-agent-feature$=':no']").first()).toContainText("–");
    await expect(popover.locator("[data-agent-feature$=':yes']").first()).toContainText("✓");
    // Every supported agent has Herdr's integration: the popover lists it and says nothing of a screen-only status.
    await expect(popover.locator('[data-agent-feature="herdr_integration:yes"]')).toBeVisible();
    await expect(popover).not.toContainText("from the screen");
    await screenshot(page, "agents-partial-popover");
    await page.keyboard.press("Escape");
    await expect(popover).toHaveCount(0);
    await expect(chip).toBeFocused();
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("a switch installs and takes out Hide's own entries, and a hook the operator removed shows on its row until Reinstall", async ({ page }) => {
  const { herdr, daemon, home } = await start("agents-switch");
  try {
    const list = await openAgents(page, daemon);
    const grok = path.join(home, ".grok", "hooks", "hide.json");
    const claude = path.join(home, ".claude", "settings.json");
    await expect(list.locator(`[data-agent-switch="${daemon.node}:grok:off"]`)).toBeVisible({ timeout: 60_000 });
    // B13: turning an agent on writes its hook on this machine.
    await list.locator(`[data-agent-switch="${daemon.node}:grok:off"]`).click();
    await expect(list.locator(`[data-agent-switch="${daemon.node}:grok:on"]`)).toBeVisible();
    await expect.poll(() => read(grok), { timeout: 60_000 }).toContain("hide-guidance@");
    // B14: turning it off takes out what Hide wrote.
    await list.locator(`[data-agent-switch="${daemon.node}:grok:on"]`).click();
    await expect(list.locator(`[data-agent-switch="${daemon.node}:grok:off"]`)).toBeVisible();
    await expect.poll(() => fs.existsSync(grok), { timeout: 60_000 }).toBe(false);

    // B20: a hook the operator took out is not put back; its row says so with Reinstall, on that row only.
    await expect.poll(() => read(claude), { timeout: 60_000 }).toContain("hide-subagents@");
    fs.writeFileSync(claude, "{}\n");
    await list.locator("[data-agents-check]").click();
    const problem = list.locator(`[data-agent-problem="${daemon.node}:claude-code"]`);
    await expect(problem).toContainText("Hook: Removed", { timeout: 60_000 });
    await expect(list.locator("[data-agent-problem]")).toHaveCount(1);
    await screenshot(page, "agents-problem");
    expect(read(claude)).not.toContain("hide-subagents@");
    await problem.locator("[data-hook-reinstall]").click();
    await expect.poll(() => read(claude), { timeout: 60_000 }).toContain("hide-subagents@");
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("reads without sideways scrolling in Korean and English, light and dark, on a narrow window", async ({ page }) => {
  // B65: the sheet shrinks with its window; the rows' status, the Partial chip and the Not installed list wrap, not clip.
  const { herdr, daemon } = await start("agents-narrow");
  try {
    const list = await openAgents(page, daemon);
    await expect(list.locator("[data-agent-row]")).toHaveCount(7, { timeout: 60_000 });
    // The smallest window (PRD settings-left-nav B13): the list keeps its width beside the page.
    await page.setViewportSize({ width: 720, height: 1000 });
    await list.locator("[data-agents-not-installed] summary").click();
    const panel = page.locator('[data-settings="true"] [data-settings-page]');
    const overflow = () => panel.evaluate((node) => node.scrollWidth - node.clientWidth);
    for (const [language, theme] of [
      ["en", "light"],
      ["en", "dark"],
      ["ko", "light"],
      ["ko", "dark"],
    ] as const) {
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", theme);
      await page.locator('[data-settings-tab="agents"]').click();
      await expect(list).toBeVisible();
      // The list fades the left page's item out; the picture waits for the fade so it shows the resting sheet.
      await expect(page.locator('[data-settings-tab="general"]')).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      await screenshot(page, `agents-${language}-${theme}-narrow`);
    }
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
