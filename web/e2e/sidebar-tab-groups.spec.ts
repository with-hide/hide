import { test, expect, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { agentIn, areaShowing, closePane, startAgentInCheckout, tabOfPane, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { dumpOnFailure } from "./failure-dump";
import { setAgentSession } from "./fixture-agents";
import type { HidedControl } from "./hided-control.mjs";
import { startHided } from "./hided-fixture";
import { chooseTheme, enterWorkspace, rest, screenshot, sendEvent } from "./wire";

const tabButton = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab="${tab}"]`);
const guide = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab-guide="${tab}"]`);
const segment = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab-group="${tab}"]`);

/** Names `tab` as the operator does, from its tab in the strip. */
async function renameTab(page: Page, tab: string, name: string) {
  await page.locator(`[data-tab="${tab}"]`).first().click({ button: "right" });
  await page.locator('[data-menu-item="rename_tab"]').click();
  const input = page.getByRole("textbox", { name: "Tab name", exact: true });
  await input.fill(name);
  await input.press("Enter");
}

/** The checkout's folder, in the spelling the core gives its panes. */
const folderOf = (engine: AgentFixture) => toPage(fs.realpathSync(path.join(engine.root, "fixture")));

/** The core's task slot, its last refusal and its newest diagnostics, as a start leaves them. */
function coreStart(control: HidedControl) {
  return control.until((snapshot) => {
    const status = snapshot.status as { last_error?: unknown; diagnostics?: { kind: string; message: string }[] } | undefined;
    return { task: snapshot.task_operation ?? null, refused: status?.last_error ?? null, diagnostics: (status?.diagnostics ?? []).slice(-12).map((entry) => `${entry.kind}: ${entry.message}`) };
  }, "the core's start state", 3_000);
}

/**
 * Starts a Claude agent as the start panel does: in a tab of its own, or,
 * given `beside`, in a new pane beside the agents of that tab. Returns where
 * it runs once the daemon lists it.
 */
async function startAgent(engine: AgentFixture, daemon: Awaited<ReturnType<typeof startHided>>, beside?: string): Promise<{ tab: string; pane: string }> {
  const folder = folderOf(engine);
  const started = await withControl(daemon, async (control) => {
    const area = beside ? await control.until((rest) => areaShowing(rest, beside), `the area showing ${beside}`) : undefined;
    const what = beside ? `a new agent beside ${beside}` : "a new agent in a tab of its own";
    // The test's own timeout is 30 s, so the start gives up before it and a
    // failed one says what the core held (with-hide/hide-private#518).
    const pane = await dumpOnFailure(`start of ${what}`, () => coreStart(control), () => startAgentInCheckout(control, folder, beside ? { target: { ...area, tab_id: beside, add_pane: true } } : {}, what, 15_000));
    return control.until((rest) => {
      const tab = tabOfPane(rest, pane);
      return tab && agentIn(rest, pane) ? { tab, pane } : undefined;
    }, what, 30_000);
  });
  // The stand-in reports its session, as Claude Code does, and its start settles, so its row leaves Starting.
  await setAgentSession(engine, daemon, started.pane, `sidebar-${started.pane.replaceAll(":", "-")}`);
  return started;
}

const position = async (row: ReturnType<Page["locator"]>) => (await row.boundingBox())!;

test("the only tab of a checkout draws its segment with no guide and no line, named or not, and a second agent joins the segment", { tag: "@platform" }, async ({ page }) => {
  // The platform's engine owns pane/process existence.
  const engine = await startFixture();
  const daemon = await startHided(engine, "sidebar-tab-segments");
  try {
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const agents = segment(page, engine.tab).locator("[data-agent-open]");
    const noLine = async () => {
      await expect(page.locator("[data-project-list] [data-sidebar-tab-line]")).toHaveCount(0);
      await expect(tabButton(page, engine.tab)).toHaveCount(0);
      await expect(page.locator("[data-sidebar-tab-name]")).toHaveCount(0);
    };
    await expect(guide(page, engine.tab)).toHaveCount(0);
    await expect(agents).toHaveCount(2);
    await noLine();
    await screenshot(page, "sidebar-tab-unnamed");

    // A name the operator gives it is told by the tab strip; the sidebar still draws no line for a tab with agents.
    await renameTab(page, engine.tab, "아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인");
    await expect(page.locator(`[data-agent-tab-bar] [data-tab="${engine.tab}"]`)).toContainText("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인");
    await expect(agents).toHaveCount(2);
    await noLine();
    await screenshot(page, "sidebar-tab-named");

    await withControl(daemon, (control) => closePane(control, engine.panes[1]));
    await expect(agents).toHaveCount(1);
    await expect(guide(page, engine.tab)).toHaveCount(0);
    await noLine();

    // A second agent joins the same segment, still with no guide: one tab has no other segment to tell it from.
    await startAgent(engine, daemon, engine.tab);
    await expect(agents).toHaveCount(2);
    await expect(guide(page, engine.tab)).toHaveCount(0);
    await noLine();
  } finally { daemon.stop(); engine.stop(); }
});

test("2-1-2 tab segments align every tab's guide with its rows, and no tab with agents draws a line", async ({ page }) => {
  const engine = await startFixture();
  let daemon: Awaited<ReturnType<typeof startHided>> | undefined;
  try {
    daemon = await startHided(engine, "sidebar-tab-boundary");
    const alone = await startAgent(engine, daemon);
    const pair = await startAgent(engine, daemon);
    await startAgent(engine, daemon, pair.tab);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tabs = [engine.tab, alone.tab, pair.tab];
    for (const tab of tabs) await expect(guide(page, tab)).toHaveCount(1);
    await expect(page.locator("[data-project-list] [data-sidebar-tab-line]")).toHaveCount(0);
    const singleAgent = segment(page, alone.tab).locator("[data-agent-open]");
    await expect(singleAgent).toBeVisible();
    await expect(segment(page, engine.tab).locator("[data-agent-open]")).toHaveCount(2);
    await expect(segment(page, pair.tab).locator("[data-agent-open]")).toHaveCount(2);
    await page.evaluate(() => document.fonts.ready);

    // This checkout holds no named group, so it draws no collection header at
    // all (group-tabs D-14): its tabs and agents stand on the checkout's own
    // columns, and each tab's guide hangs under the checkout's own glyph. The
    // fixture's checkout is a plain folder, so its row is the project's.
    const owner = page.locator("[data-project-list] [data-project-menu]").first();
    const ownerRow = await position(page.locator("[data-project-list] [data-project-menu] > div").first());
    await expect(page.locator("[data-project-list] [data-sidebar-named-group]")).toHaveCount(0);
    await expect(page.locator("[data-project-list] [data-sidebar-group-select]")).toHaveCount(0);
    const kind = await position(owner.locator("svg").first());
    const marks = await Promise.all((await page.locator("[data-project-list] li[data-pane][data-depth='0'] [data-agent-status-mark]").all()).map(position));
    expect(marks).toHaveLength(5);
    expect(new Set(marks.map((mark) => mark.x)).size).toBe(1);
    expect(marks[0]!.x).toBeGreaterThanOrEqual(kind.x + kind.width);
    // The title follows the status mark alone, one mark and one gap after it (agent-mark-quiet B1): no provider logo stands between them.
    const titles = await Promise.all((await page.locator("[data-project-list] li[data-pane][data-depth='0'] [data-agent-title]").all()).map(position));
    expect(titles.map((title) => title.x - marks[0]!.x)).toEqual(titles.map(() => 16));

    // Every tab draws a 1px guide under the checkout's own kind glyph, which bends into its last row toward the marks.
    const center = kind.x + kind.width / 2;
    for (const tab of tabs) {
      const line = await position(guide(page, tab));
      expect(Math.abs(line.x + 0.5 - center)).toBeLessThanOrEqual(0.5);
      expect(line.x + line.width).toBeLessThanOrEqual(marks[0]!.x);
    }
    const first = await position(segment(page, engine.tab));
    const middle = await position(segment(page, alone.tab));
    const last = await position(segment(page, pair.tab));
    // The first segment hangs 2px under its checkout, and segments are
    // contiguous: a tab's segment is its rows alone, beside its guide.
    expect(first.y - ownerRow.y - ownerRow.height).toBe(2);
    expect(middle.y - first.y - first.height).toBe(0);
    expect(last.y - middle.y - middle.height).toBe(0);
    expect((await position(segment(page, alone.tab).locator("li[data-pane]"))).height).toBe(30);
    expect(middle.height).toBe(30);
    await rest(page);
    await screenshot(page, "sidebar-tab-segments");

    // Arrow keys visit every agent row across the tabs; Home and End reach the ends.
    const rowsOf = (tab: string) => segment(page, tab).locator("[data-agent-open]");
    await rowsOf(engine.tab).first().focus();
    await page.keyboard.press("ArrowDown");
    await expect(rowsOf(engine.tab).nth(1)).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(singleAgent).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(rowsOf(pair.tab).first()).toBeFocused();
    await page.keyboard.press("Home");
    await expect(rowsOf(engine.tab).first()).toBeFocused();
    await page.keyboard.press("End");
    await expect(rowsOf(pair.tab).last()).toBeFocused();
    await singleAgent.click();
    await expect(page.locator(`[data-tab="${alone.tab}"]`).first()).toHaveAttribute("aria-selected", "true");
  } finally { daemon?.stop(); engine.stop(); }
});

test("a named tab with no agent keeps its line and hint inside the sidebar at 240 and 292 in both themes", async ({ page }) => {
  const engine = await startFixture();
  const daemon = await startHided(engine, "sidebar-tab-narrow");
  const name = "아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인 · Sidebar tab segments and long names";
  try {
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await renameTab(page, engine.tab, name);
    // Closing its agents leaves a shell in the named tab, the one tab that still says its name here.
    for (const pane of engine.panes) await withControl(daemon, (control) => closePane(control, pane));
    await expect(tabButton(page, engine.tab)).toHaveAccessibleName(name);
    await expect(segment(page, engine.tab).locator("[data-agent-open]")).toHaveCount(0);
    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      for (const width of [240, 292]) {
        await sendEvent(page, daemon, "ui_state_update", { sidebar_width: width, expanded_paths: [], selected_path: null });
        await expect.poll(async () => Math.round((await page.locator("nav[data-sidebar]").boundingBox())!.width)).toBe(width + 48);
        expect(await segment(page, engine.tab).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        const label = page.locator("[data-sidebar-tab-name] > span");
        await page.evaluate(() => document.fonts.ready);
        expect(await label.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
        await tabButton(page, engine.tab).hover();
        await expect(page.getByRole("tooltip")).toHaveText(name);
        await screenshot(page, `sidebar-tab-tooltip-${theme}-${width}`);
        await rest(page);
        await screenshot(page, `sidebar-tab-${theme}-${width}`);
      }
    }
  } finally { daemon.stop(); engine.stop(); }
});
