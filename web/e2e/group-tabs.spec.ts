import { createLegacyDraft } from "./group-start-helper";
// Approved group-tabs B3/B5-B10/B12/B18/B21/B23: the topology the daemon
// publishes is the oracle for whether a blank tab starts anything; the tests
// that hold or lose an engine's answer to a start run on Herdr's socket.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { tabsOf, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });
test.use({ actionTimeout: 15_000 });

const stripTabs = (page: Page) => page.locator("[data-agent-tab-bar] [role=tab][data-tab]");
const selector = (page: Page) => page.locator("[data-agent-group-selector]");
const terminal = (page: Page) => page.locator("[data-group-start-terminal=true]");
const blank = (page: Page) => page.locator("[data-agent-tab-start]");

/** The tabs of the fixture's project as the daemon publishes them now: the engine's tabs, a draft being no tab of its own. */
async function tabsNow(daemon: Daemon, workspace: string): Promise<string[]> {
  return withControl(daemon, async (control) => (await control.until((rest) => tabsOf(rest, workspace), `the tabs of ${workspace}`)).filter((id) => !id.startsWith("draft:")));
}

async function open(page: Page, daemon: Daemon) {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await expect(selector(page)).toBeVisible();
}

async function createGroup(page: Page, name: string) {
  await selector(page).click();
  await page.getByRole("menuitem", { name: "New group…", exact: true }).click();
  await page.getByRole("textbox", { name: "Group name", exact: true }).fill(name);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(selector(page)).toContainText(name);
  const group = (await selector(page).getAttribute("data-agent-group-selector"))!;
  const row = page.locator(`[data-sidebar-group-select="${group}"]`);
  if (!(await row.isVisible())) await page.locator("[data-checkout-toggle][aria-expanded=false]").click();
  await expect(row).toBeVisible();
  return group;
}

async function chooseGroup(page: Page, name: string) {
  await selector(page).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
  await expect(selector(page)).toContainText(name);
}

test("an empty group and draft survive navigation and restart without creating a shell", async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "group-draft");
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page, daemon);
    const original = await tabsNow(daemon, fixture.workspace);
    await screenshot(page, "group-tabs-ungrouped-work");
    const group = await createGroup(page, "문서 검토");
    await expect(stripTabs(page)).toHaveCount(0);
    await expect(blank(page)).toHaveAttribute("data-agent-tab-start", "empty");
    await expect(terminal(page)).toBeEnabled();
    await createLegacyDraft(page, daemon!, path.join(fixture.root, "fixture"));
    await expect(stripTabs(page)).toHaveCount(1);
    const draft = await blank(page).getAttribute("data-agent-tab-start");
    expect(draft).toMatch(/^draft:/);
    await blank(page).getByRole("textbox").fill("아직 보내지 않은 작업");
    await chooseGroup(page, "Ungrouped");
    await expect(stripTabs(page)).toHaveCount(original.length);
    await chooseGroup(page, "문서 검토");
    await expect(blank(page).getByRole("textbox")).toHaveValue("아직 보내지 않은 작업");
    expect(await tabsNow(daemon, fixture.workspace)).toEqual(original);
    await screenshot(page, "group-tabs-blank-terminal-action");
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.copyFileSync(path.join(daemon.stateDir, "workspace-views.json"), path.join(evidence, "group-restart-before.json"));
    await page.goto("about:blank");
    daemon = await daemon.restart();
    if (evidence) fs.copyFileSync(path.join(daemon.stateDir, "workspace-views.json"), path.join(evidence, "group-restart-after-launch.json"));
    await open(page, daemon);
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(blank(page)).toHaveAttribute("data-agent-tab-start", draft!);
    expect(await tabsNow(daemon, fixture.workspace)).toEqual(original);
  } finally { try { daemon?.stop(); } finally { fixture.stop(); } }
});

test("Start agent opens a tab with its chosen model and prompt beside a terminal", async ({ page }) => {
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "group-add-agent");
    await open(page, daemon);
    const group = await createGroup(page, "Agent work");
    // UI_BEHAVIOR.md: New tab opens a terminal; Start agent opens its own tab.
    await page.locator("[data-new-agent-tab=true]").click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const tab = (await stripTabs(page).getAttribute("data-tab"))!;
    const originalPane = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    const originalTabs = await tabsNow(daemon, fixture.workspace);
    await expect(page.getByRole("button", { name: "Add agent", exact: true })).toHaveCount(0);
    await page.keyboard.press(chord("search"));
    await page.locator('[data-palette="Search"] [data-palette-input]').fill("Start an agent");
    await page.locator('[data-palette-row="command:start-agent"]').click();
    const panel = page.locator("[data-start-panel]");
    await expect(panel).toBeVisible();
    await panel.locator("[data-agent-kind]").click();
    await expect(page.locator('[data-agent-kind-option="claude"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-agent-kind-option="claude"]').click();
    const model = panel.locator("[data-agent-model-kind]");
    await expect(model).toBeEnabled();
    await model.click();
    await page.locator('[data-agent-model-option="opus"]').click();
    await panel.locator("[data-start-text]").fill("새 탭의 에이전트");
    await panel.locator("[data-start-submit]").click();
    await expect(panel).toHaveCount(0, { timeout: 30_000 });
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(stripTabs(page)).toHaveCount(2);
    await expect(page.locator(`[data-agent-tab-bar] [data-tab="${tab}"]`)).toBeVisible();
    expect(await tabsNow(daemon, fixture.workspace)).toHaveLength(originalTabs.length + 1);
    await page.locator(`[data-agent-tab-bar] [data-tab="${tab}"]`).click();
    await expect(page.locator(`[data-pane-view="${originalPane}"]`)).toBeVisible();
    await screenshot(page, "group-tabs-new-agent-tab");
  } finally { try { daemon?.stop(); } finally { fixture.stop(); } }
});
