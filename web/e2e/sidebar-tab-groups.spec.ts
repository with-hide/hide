import { test, expect, type Page } from "@playwright/test";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { chooseTheme, countSent, enterWorkspace, rest, screenshot, sendEvent } from "./wire";

const tabButton = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab="${tab}"]`);
const guide = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab-guide="${tab}"]`);
const segment = (page: Page, tab: string) => page.locator(`[data-project-list] [data-sidebar-tab-group="${tab}"]`);
const position = async (row: ReturnType<Page["locator"]>) => (await row.boundingBox())!;

test("a tab with two agents opens with a line that opens it, takes its name once renamed, and keeps it at one agent while named", { tag: "@platform" }, async ({ page }) => {
  // The platform's Herdr owns pane/process existence and the tab names.
  const herdr = await startHerdr();
  const daemon = await startHided(herdr, "sidebar-tab-segments");
  const sent = countSent(page);
  try {
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(tabButton(page, herdr.tab)).toHaveAccessibleName("Tab 1, 2 agents");
    await expect(segment(page, herdr.tab).locator("[data-agent-open]")).toHaveCount(2);
    // Unnamed: the line reads the core's stable number.
    await expect(page.locator("[data-sidebar-tab-name]")).toHaveText("Tab 1");
    await screenshot(page, "sidebar-tab-unnamed");

    const tabs = page.locator(`[data-tab="${herdr.tab}"]`).first();
    await tabs.click({ button: "right" });
    await page.locator('[data-menu-item="rename_tab"]').click();
    const input = page.getByRole("textbox", { name: "Tab name", exact: true });
    await input.fill("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인");
    await input.press("Enter");
    await expect(tabButton(page, herdr.tab)).toHaveAccessibleName("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인, 2 agents");
    await expect(page.locator("[data-sidebar-tab-name]")).toHaveText("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인");
    await screenshot(page, "sidebar-tab-named");

    // The line opens the real tab; it folds nothing and persists nothing.
    const before = sent.get("focus_pane") ?? 0;
    await tabButton(page, herdr.tab).click();
    await expect(tabs).toHaveAttribute("aria-selected", "true");
    expect(sent.get("focus_pane") ?? 0).toBe(before);
    await expect(segment(page, herdr.tab).locator("[data-agent-open]")).toHaveCount(2);

    // One agent left: the named tab keeps its guide and line, now counting one agent.
    herdr.run(["pane", "close", herdr.panes[1]]);
    await expect(segment(page, herdr.tab).locator("[data-agent-open]")).toHaveCount(1);
    await expect(tabButton(page, herdr.tab)).toHaveAccessibleName("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인, 1 agent");
    await expect(page.locator("[data-sidebar-tab-name]")).toHaveText("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인");
    await expect(guide(page, herdr.tab)).toHaveCount(1);
    await screenshot(page, "sidebar-tab-named-single");

    // A second agent joins the same line under the retained name.
    const split = herdr.run(["pane", "split", herdr.panes[0], "--direction", "right", "--cwd", path.join(herdr.root, "fixture"), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { pane: { pane_id: string } } };
    herdr.run(["agent", "start", "returned", "--kind", "claude", "--pane", split.result.pane.pane_id]);
    await expect(tabButton(page, herdr.tab)).toHaveAccessibleName("아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인, 2 agents");
    expect(sent.get("sidebar_tab_expanded") ?? 0).toBe(0);
  } finally { daemon.stop(); herdr.stop(); }
});

test("2-1-2 tab segments align every tab's guide and line, and an unnamed single agent's tab keeps its guide alone", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Awaited<ReturnType<typeof startHided>> | undefined;
  try {
    herdr.run(["tab", "rename", herdr.tab, "작업 묶음 A"]);
    // An unnamed single-agent tab draws its guide like any other tab, but no tab line: `Tab N` says nothing its one row does not.
    const created = [null, "작업 묶음 B"].map((name) => herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--env", `PATH=${herdr.fixturePath}`, ...(name ? ["--label", name] : []), "--no-focus"]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } });
    const single = created[0]!.result;
    const paired = created[1]!.result;
    herdr.run(["agent", "start", "single", "--kind", "claude", "--pane", single.root_pane.pane_id]);
    herdr.run(["agent", "start", "paired", "--kind", "claude", "--pane", paired.root_pane.pane_id]);
    const split = herdr.run(["pane", "split", paired.root_pane.pane_id, "--direction", "right", "--cwd", path.join(herdr.root, "fixture"), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { pane: { pane_id: string } } };
    herdr.run(["agent", "start", "paired-two", "--kind", "claude", "--pane", split.result.pane.pane_id]);
    daemon = await startHided(herdr, "sidebar-tab-boundary");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(tabButton(page, herdr.tab)).toBeVisible();
    await expect(tabButton(page, single.tab.tab_id)).toHaveCount(0);
    const singleAgent = segment(page, single.tab.tab_id).locator("[data-agent-open]");
    await expect(singleAgent).toHaveCount(1);
    await expect(singleAgent).toBeVisible();
    await expect(tabButton(page, paired.tab.tab_id)).toHaveAccessibleName("작업 묶음 B, 2 agents");
    await page.evaluate(() => document.fonts.ready);

    // This checkout holds no named group, so it draws no collection header at
    // all (group-tabs D-14): its tabs and agents stand on the checkout's own
    // columns, and each titled tab's guide hangs under the checkout's own glyph. The
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

    // Every tab draws a 1px guide under the checkout's own kind glyph, the unnamed single agent's too.
    const center = kind.x + kind.width / 2;
    for (const tab of [herdr.tab, single.tab.tab_id, paired.tab.tab_id]) {
      const line = await position(guide(page, tab));
      expect(line.width).toBe(1);
      expect(Math.abs(line.x + line.width / 2 - center)).toBeLessThanOrEqual(0.5);
    }
    // A tab line stands inside its guide like a row: the guide starts beside it, its glyph is centered on the status-mark column and its
    // name starts on the title column, so nothing of the tab stands on the checkout glyph's column.
    const mark = marks[0]!;
    for (const tab of [herdr.tab, paired.tab.tab_id]) {
      const head = await position(segment(page, tab).locator("[data-sidebar-tab-line] svg"));
      expect(Math.abs(head.x + head.width / 2 - (mark.x + mark.width / 2))).toBeLessThanOrEqual(0.5);
      expect((await position(segment(page, tab).locator("[data-sidebar-tab-name]"))).x).toBe(titles[0]!.x);
      const line = await position(segment(page, tab).locator("[data-sidebar-tab-line]"));
      expect((await position(guide(page, tab))).y).toBeLessThan(line.y + line.height / 2);
    }
    const first = await position(segment(page, herdr.tab));
    const middle = await position(segment(page, single.tab.tab_id));
    const last = await position(segment(page, paired.tab.tab_id));
    // The first segment hangs 2px under its checkout, and segments are
    // contiguous: the single agent's segment is its row alone, beside its guide.
    expect(first.y - ownerRow.y - ownerRow.height).toBe(2);
    expect(middle.y - first.y - first.height).toBe(0);
    expect(last.y - middle.y - middle.height).toBe(0);
    expect((await position(segment(page, single.tab.tab_id).locator("li[data-pane]"))).height).toBe(28);
    expect(middle.height).toBe(28);
    await rest(page);
    await screenshot(page, "sidebar-tab-segments");

    // Arrow keys visit every tab line and row; the single agent's tab has no line to stop on.
    // Home and End reach the ends.
    await tabButton(page, herdr.tab).focus();
    await tabButton(page, herdr.tab).press("ArrowDown");
    await expect(segment(page, herdr.tab).locator("[data-agent-open]").first()).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(segment(page, herdr.tab).locator("[data-agent-open]").nth(1)).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(singleAgent).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(tabButton(page, paired.tab.tab_id)).toBeFocused();
    await page.keyboard.press("Home");
    await expect(tabButton(page, herdr.tab)).toBeFocused();
    await page.keyboard.press("End");
    await expect(segment(page, paired.tab.tab_id).locator("[data-agent-open]").last()).toBeFocused();
    await singleAgent.click();
    await expect(page.locator(`[data-tab="${single.tab.tab_id}"]`).first()).toHaveAttribute("aria-selected", "true");
    await tabButton(page, paired.tab.tab_id).focus();
    await tabButton(page, paired.tab.tab_id).press("Enter");
    await expect(page.locator(`[data-tab="${paired.tab.tab_id}"]`).first()).toHaveAttribute("aria-selected", "true");
    await tabButton(page, herdr.tab).focus();
    await tabButton(page, herdr.tab).press("Space");
    await expect(page.locator(`[data-tab="${herdr.tab}"]`).first()).toHaveAttribute("aria-selected", "true");

    // Focus brings the line's hint, the name and the agent count.
    await tabButton(page, herdr.tab).focus();
    await expect(page.getByRole("tooltip")).toHaveText("작업 묶음 A, 2 agents");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("long manual tab names keep their hint and line inside the sidebar at 240 and 292 in both themes", async ({ page }) => {
  const herdr = await startHerdr();
  const daemon = await startHided(herdr, "sidebar-tab-narrow");
  const name = "아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인 · Sidebar tab segments and long names";
  try {
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-tab="${herdr.tab}"]`).first().click({ button: "right" });
    await page.locator('[data-menu-item="rename_tab"]').click();
    const input = page.getByRole("textbox", { name: "Tab name", exact: true });
    await input.fill(name);
    await input.press("Enter");
    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      for (const width of [240, 292]) {
        await sendEvent(page, daemon, "ui_state_update", { sidebar_width: width, expanded_paths: [], selected_path: null });
        await expect.poll(async () => Math.round((await page.locator("nav[data-sidebar]").boundingBox())!.width)).toBe(width + 48);
        await expect(tabButton(page, herdr.tab)).toHaveAccessibleName(`${name}, 2 agents`);
        expect(await segment(page, herdr.tab).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        const label = page.locator("[data-sidebar-tab-name] > span");
        await page.evaluate(() => document.fonts.ready);
        expect(await label.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
        await tabButton(page, herdr.tab).hover();
        await expect(page.getByRole("tooltip")).toHaveText(`${name}, 2 agents`);
        await screenshot(page, `sidebar-tab-tooltip-${theme}-${width}`);
        await rest(page);
        await screenshot(page, `sidebar-tab-${theme}-${width}`);
      }
    }
  } finally { daemon.stop(); herdr.stop(); }
});
