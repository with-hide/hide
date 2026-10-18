// Fork agent and Fork agent to the side from the pane menu, on an isolated
// pinned Herdr and hided (PRD fork-operator-root): each starts the pane's
// conversation again as a new session, the operator's own root, which takes
// the keyboard and wears the fork mark naming its source. What is typed
// between the press and the agent's start reaches the fork's agent, not the
// shell that starts it. The disabled items of an agent with no conversation
// yet are the pane menu unit test's and the core's.

import { spawnSync } from "node:child_process";
import { expect, test, type Page } from "@playwright/test";
import { claudeProjects, herdrHasFocus, setFixtureSession, startHerdr, writeFixtureTranscript, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, screenshot } from "./wire";

test.describe.configure({ timeout: 150_000 });

const SESSION = "11111111-2222-3333-4444-555555555555";
const FORKED_SESSION = "66666666-7777-8888-9999-000000000000";
const LAUNCH = `claude --resume ${SESSION} --fork-session`;

type Listed = { result: { agents: { pane_id: string }[] } };
type ProcessInfo = { result: { process_info: { foreground_processes: { argv?: string[] | null }[] } } };

/** The source pane runs a Claude conversation Herdr reported; hided serves the page. */
async function forkStack(page: Page, herdr: HerdrFixture, name: string): Promise<{ daemon: Daemon; forker: string; last: Map<string, Record<string, unknown>> }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const [, forker] = herdr.panes;
  writeFixtureTranscript(claudeProjects(herdr), SESSION, { task: "Agent two" });
  setFixtureSession(herdr, forker, SESSION);
  const daemon = await startHided(herdr, name);
  const last = new Map<string, Record<string, unknown>>();
  countSent(page, last);
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await page.locator("[data-checkout]").first().click();
  await expect(page.locator(`[data-pane-view="${forker}"]`)).toBeVisible({ timeout: 20_000 });
  return { daemon, forker, last };
}

/**
 * Presses `item` in the source's pane menu and types a marker as soon as the
 * fork's terminal has the keyboard. Returns the fork's pane once its agent runs
 * the clean launch, how long that took, and whether the marker was typed before.
 */
async function fork(page: Page, herdr: HerdrFixture, forker: string, item: "fork_agent" | "fork_agent_side", marker: string): Promise<{ forked: string; startMs: number }> {
  const agents = () => (herdr.run(["agent", "list"]) as Listed).result.agents.map((agent) => agent.pane_id);
  const before = agents();
  await page.locator(`[data-pane-menu="${forker}"]`).click();
  await expect(page.locator('[data-menu-group="fork"]')).toBeEnabled({ timeout: 20_000 });
  await page.locator('[data-menu-group="fork"]').hover();
  await expect(page.locator('[data-menu-item="fork_agent"]')).toBeEnabled();
  await expect(page.locator('[data-menu-item="fork_agent_side"]')).toBeEnabled();
  // The source is the rightmost pane: the submenu flips left at the window
  // edge. Moving directly into it must not dismiss it or lose the target.
  await expect(page.locator('[data-menu-submenu="fork"]')).toHaveAttribute("data-side", "left");
  await page.locator(`[data-menu-item="${item}"]`).hover();
  await expect(page.locator('[data-menu-submenu="fork"]')).toBeVisible();
  if (item === "fork_agent") await screenshot(page, "agent-fork-menu");
  const pressed = Date.now();
  await page.locator(`[data-menu-item="${item}"]`).click();
  const terminalWithFocus = () => page.evaluate(() => document.activeElement?.closest<HTMLElement>("[data-terminal-host]")?.dataset.terminalHost ?? "");
  let focused = "";
  await expect.poll(async () => (focused = await terminalWithFocus()), { timeout: 30_000 }).not.toMatch(new RegExp(`^(${forker})?$`));
  const focusMs = Date.now() - pressed;
  await page.keyboard.type(marker);
  const launched = () => (herdr.run(["pane", "process-info", "--pane", focused]) as ProcessInfo).result.process_info.foreground_processes.map((process) => process.argv?.join(" "));
  const typedBeforeStart = !launched().includes(LAUNCH);
  let forked = "";
  await expect.poll(() => (forked = agents().find((pane) => !before.includes(pane)) ?? ""), { timeout: 60_000 }).toBe(focused);
  await expect.poll(launched, { timeout: 30_000 }).toContain(LAUNCH);
  const startMs = Date.now() - pressed;
  test.info().annotations.push({ type: `${item} timing`, description: `keyboard on fork ${focusMs} ms, agent started ${startMs} ms, typed before start ${typedBeforeStart}` });
  console.log(`${item}: keyboard on fork ${focusMs} ms, agent started ${startMs} ms after the press; marker typed before start: ${typedBeforeStart}`);
  // The official Claude integration reports the fork's own conversation id.
  setFixtureSession(herdr, forked, FORKED_SESSION);
  return { forked, startMs };
}

/** What the fork's agent echoed: the keys typed before it started reach it. */
function screenOf(herdr: HerdrFixture, pane: string): string {
  const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "recent-unwrapped", "--lines", "40", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  return read.status === 0 ? read.stdout : "";
}

test("Fork agent puts the operator's own fork in a new tab right of the source and moves the keyboard there", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    // A tab after the source's, so the fork's tab has to land between them.
    const after = (herdr.run(["tab", "create", "--workspace", herdr.workspace, "--label", "after", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { tab: { tab_id: string } } }).result.tab.tab_id;
    const stack = await forkStack(page, herdr, "agent-fork-tab");
    daemon = stack.daemon;
    const { forker, last } = stack;
    const { forked } = await fork(page, herdr, forker, "fork_agent", "typed-before-start");
    expect(last.get("fork_pane")).toMatchObject({ pane_id: forker, placement: "tab" });
    await expect(page.locator(`[data-pane-view="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => herdrHasFocus(herdr, forked), { timeout: 30_000 }).toBe(true);
    const strip = page.locator("[data-agent-tab-bar] [data-tab]");
    await expect.poll(async () => (await strip.evaluateAll((tabs) => tabs.map((tab) => (tab as HTMLElement).dataset.tab))).join(" ")).toMatch(new RegExp(`^${herdr.tab} \\S+ ${after}$`));
    // Its keys reached the fork's agent after the clean launch, not the shell.
    await expect.poll(() => screenOf(herdr, forked), { timeout: 10_000 }).toContain("typed-before-start");

    // The fork is the operator's own root: a row of its own with the mark,
    // the source unmarked and with no child under it.
    const row = page.locator(`nav[data-sidebar] [data-pane="${forked}"]`);
    await expect(row).toBeVisible({ timeout: 30_000 });
    await expect(row).not.toHaveAttribute("data-delegated", "true");
    await expect(row.locator(`[data-agent-fork="${forked}"]`)).toBeVisible();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${forker}"] [data-agent-fork]`)).toHaveCount(0);
    await expect(page.locator(`[data-tree-chevron="${forker}"]`)).toHaveCount(0);
    const mark = page.locator(`[data-pane-view="${forked}"] [data-pane-fork]`);
    await expect(mark).toHaveAttribute("data-pane-fork", forker, { timeout: 30_000 });
    await expect(mark).toHaveAccessibleName("Forked from Agent two");
    await expect(page.locator(`[data-pane-view="${forker}"] [data-pane-fork]`)).toHaveCount(0);
    await screenshot(page, "agent-fork-tab");

    // The header mark opens the source.
    await mark.click();
    await expect.poll(() => herdrHasFocus(herdr, forker), { timeout: 30_000 }).toBe(true);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("Fork agent to the side splits the source's tab and moves the keyboard to the fork", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const stack = await forkStack(page, herdr, "agent-fork-side");
    daemon = stack.daemon;
    const { forker, last } = stack;
    const { forked } = await fork(page, herdr, forker, "fork_agent_side", "typed-beside");
    expect(last.get("fork_pane")).toMatchObject({ pane_id: forker, placement: "side" });
    await expect.poll(() => herdrHasFocus(herdr, forked), { timeout: 30_000 }).toBe(true);
    // The fork shares the source's tab, to its right.
    await expect(page.locator(`[data-pane-view="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    const source = await page.locator(`[data-pane-view="${forker}"]`).boundingBox();
    const beside = await page.locator(`[data-pane-view="${forked}"]`).boundingBox();
    expect(beside!.x).toBeGreaterThan(source!.x);
    await expect.poll(() => screenOf(herdr, forked), { timeout: 10_000 }).toContain("typed-beside");
    await expect(page.locator(`nav[data-sidebar] [data-pane="${forked}"] [data-agent-fork="${forked}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-pane-view="${forked}"] [data-pane-fork]`)).toHaveAccessibleName("Forked from Agent two");
    await screenshot(page, "agent-fork-side");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
