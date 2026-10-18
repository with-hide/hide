import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import fs from "node:fs";
import { dumpOnFailure, tabsBothSides } from "./failure-dump";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showExplorer } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 180_000 });

// The Agent tab strip shrinks in three stages (`tabStripFit`): every tab asks
// for 180 and all shrink alike while an equal share keeps a 104 title; then the
// selected tab keeps 104 and the others split the rest down to the 40 identity,
// compact while they hold 64 and marks below; then the selected tab gives up
// width, compact with its close while it holds 88, else 64 marks with its
// close, and the strip scrolls once even those overflow.
const PREFERRED = 180, TITLE_MIN = 104, ICON = 40, CONTROL = 24;
type Fit = "titled" | "compact" | "marks";

/** What the curve draws for `room` and `count`, written out independently of the shell's code. */
function expected(room: number, count: number): { selected: [number, Fit]; others: [number, Fit] } {
  const share = room / count;
  if (share >= TITLE_MIN) return { selected: [Math.min(PREFERRED, share), "titled"], others: [Math.min(PREFERRED, share), "titled"] };
  if (room >= TITLE_MIN + (count - 1) * ICON) {
    const others = (room - TITLE_MIN) / (count - 1);
    return { selected: [TITLE_MIN, "titled"], others: [Math.max(ICON, others), others >= ICON + CONTROL ? "compact" : "marks"] };
  }
  const selected = room - (count - 1) * ICON;
  return { selected: selected >= ICON + 2 * CONTROL ? [selected, "compact"] : [ICON + CONTROL, "marks"], others: [ICON, "marks"] };
}

type Strip = {
  room: number;
  scrolls: boolean;
  selectedInView: boolean;
  tabs: { selected: boolean; width: number; title: boolean; close: boolean }[];
};

async function strip(page: Page, column: "agent" | "view" = "agent"): Promise<Strip> {
  return page.locator(`[data-${column}-tab-bar] [role=tablist]`).evaluate((list) => {
    // A title can be inside a hidden density wrapper; measure rendered boxes,
    // which include ancestor visibility, rather than only its own display.
    const shown = (node: Element | null) => node !== null && node.getClientRects().length > 0;
    // The tabs' room is what the bar leaves beside its own controls: the zone
    // holding the strip and New tab, less New tab.
    const zone = list.parentElement!;
    const newTab = zone.querySelector<HTMLElement>("[data-new-agent-tab], [data-view-new-tab]")!;
    const bounds = list.getBoundingClientRect();
    const tabs = [...list.querySelectorAll<HTMLElement>("[role=tab]")];
    const selected = tabs.find((tab) => tab.getAttribute("aria-selected") === "true")?.getBoundingClientRect();
    return {
      room: Math.floor(zone.getBoundingClientRect().width - newTab.getBoundingClientRect().width),
      scrolls: list.scrollWidth > list.clientWidth,
      selectedInView: !!selected && selected.left >= bounds.left - 1 && selected.right <= bounds.right + 1,
      tabs: tabs.map((tab) => ({
        selected: tab.getAttribute("aria-selected") === "true",
        width: tab.getBoundingClientRect().width,
        title: shown(tab.querySelector("span.truncate")),
        close: shown(tab.querySelector("button")),
      })),
    };
  });
}

const settle = (page: Page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));

/** An Agent strip of `count` tabs: Herdr makes them, the page draws them. */
async function startStrip(page: Page, count: number) {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  const stop = () => { daemon?.stop(); herdr.stop(); };
  try {
    for (let made = 1; made < Math.min(count, 3); made += 1) herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--no-focus"]);
    daemon = await startHided(herdr, "tab-strip-fit");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tabs = page.locator("[data-agent-tab-bar] [role=tab]");
    await dumpOnFailure("tab-strip-fit tabs", () => tabsBothSides(herdr, page), () => expect(tabs).toHaveCount(Math.min(count, 3)));
    // The first screen is drawn before the rest are made, so a tab Herdr makes later is one the page had to take in.
    for (let made = 3; made < count; made += 1) herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--no-focus"]);
    await dumpOnFailure("tab-strip-fit tabs", () => tabsBothSides(herdr, page), () => expect(tabs).toHaveCount(count));
    return { tabs, stop };
  } catch (error) {
    stop();
    throw error;
  }
}

/** Sets the window width, waits for the frames that draw it, and checks the strip against the curve. */
async function check(page: Page, width: number) {
  await page.setViewportSize({ width, height: 900 });
  await settle(page);
  const drawn = await strip(page);
  const want = expected(drawn.room, drawn.tabs.length);
  const selected = drawn.tabs.find((tab) => tab.selected)!;
  const unselected = drawn.tabs.filter((tab) => !tab.selected);
  expect(selected.width, `selected at room ${drawn.room}`).toBeCloseTo(want.selected[0], 0);
  expect(selected.title).toBe(want.selected[1] !== "marks");
  expect(selected.close).toBe(true);
  for (const tab of unselected) {
    expect(tab.width, `unselected at room ${drawn.room}`).toBeCloseTo(want.others[0], 0);
    expect(tab.title).toBe(want.others[1] !== "marks");
    // An unselected titled tab keeps its close control's place; a smaller one gives it up.
    expect(tab.close).toBe(want.others[1] === "titled");
  }
  const total = want.selected[0] + unselected.length * want.others[0];
  expect(drawn.scrolls).toBe(total > drawn.room + 0.01);
  expect(drawn.selectedInView).toBe(true);
  const stage = drawn.scrolls ? "scroll"
    : want.others[1] === "titled" ? "titled"
    : want.selected[1] === "titled" ? `others-${want.others[1]}`
    : `selected-${want.selected[1]}`;
  return { room: drawn.room, stage, widths: drawn.tabs.map((tab) => Math.round(tab.width * 10) / 10) };
}

test("Agent tabs take the preferred width, whatever their titles, while the strip has room", async ({ page }) => {
  const { stop } = await startStrip(page, 3);
  try {
    const rest = await strip(page);
    expect(rest.tabs.map((tab) => Math.round(tab.width))).toEqual([PREFERRED, PREFERRED, PREFERRED]);
    expect(rest.tabs.every((tab) => tab.title)).toBe(true);
    await screenshot(page, "tab-strip-fit-preferred");
  } finally { stop(); }
});

test("Agent tabs shrink in stages, the selected one keeping its title longest, and marks come only when nothing else fits", async ({ page }) => {
  const { stop } = await startStrip(page, 16);
  try {
    // Narrowing and then widening through the same widths draws the same strip both ways.
    const widths: number[] = [];
    for (let width = 2800; width >= 700; width -= 10) widths.push(width);
    const narrowing = new Map<number, Awaited<ReturnType<typeof check>>>();
    const seen = new Set<string>();
    for (const width of widths) {
      const drawn = await check(page, width);
      narrowing.set(width, drawn);
      if (!seen.has(drawn.stage)) await screenshot(page, `tab-strip-fit-${drawn.stage}`);
      seen.add(drawn.stage);
    }
    expect([...seen].sort()).toEqual(["others-compact", "others-marks", "scroll", "selected-compact", "selected-marks", "titled"]);
    for (const width of [...widths].reverse()) expect(await check(page, width)).toEqual(narrowing.get(width));
  } finally { stop(); }
});

test("Selecting an Agent tab moves the title width to it, and the selected tab stays in sight once the strip scrolls", async ({ page }) => {
  const { tabs, stop } = await startStrip(page, 16);
  try {
    // Selecting another tab moves the title width to it in the same frame.
    // The tool rail takes its width from the strip's room (PRD right-lane B1).
    expect((await check(page, 1500 + 48)).stage).toBe("others-compact");
    const before = await strip(page);
    expect(before.tabs.filter((tab) => !tab.selected).every((tab) => tab.width < TITLE_MIN)).toBe(true);
    const target = before.tabs.findIndex((tab) => !tab.selected);
    await tabs.nth(target).click();
    await expect(tabs.nth(target)).toHaveAttribute("aria-selected", "true");
    const after = await strip(page);
    expect(after.tabs[target]!.width).toBeCloseTo(TITLE_MIN, 0);
    expect(after.tabs.filter((tab) => !tab.selected).every((tab) => Math.abs(tab.width - before.tabs[target]!.width) < 1)).toBe(true);

    // Once even marks overflow, the selected tab stays in sight: after it is
    // chosen at the strip's far end, and after the strip narrows further.
    expect((await check(page, 800)).stage).toBe("scroll");
    await tabs.last().click();
    await expect(tabs.last()).toHaveAttribute("aria-selected", "true");
    await settle(page);
    expect((await strip(page)).selectedInView).toBe(true);
    await page.setViewportSize({ width: 700, height: 900 });
    await settle(page);
    const narrowest = await strip(page);
    expect(narrowest.scrolls).toBe(true);
    expect(narrowest.selectedInView).toBe(true);
  } finally { stop(); }
});

test("File View tabs use the same shrinking strip and transfer the longest title on selection", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const names = ["한글 노트.md", "検証結果.md", "项目计划.md", ...Array.from({ length: 9 }, (_, i) => `notes-${i + 1}.md`)];
    const root = fs.realpathSync(path.join(herdr.root, "fixture"));
    for (const name of names) fs.writeFileSync(path.join(root, name), `# ${name}\n`);
    daemon = await startHided(herdr, "file-tab-strip");
    await page.setViewportSize({ width: 2400, height: 1000 });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    const tabs = page.locator("[data-view-tab-bar] [role=tab]");
    for (const name of names) await page.locator(`[data-explorer-row="${toPage(root)}/${name}"]`).dblclick();
    await expect(tabs).toHaveCount(names.length);
    // Pinned, the lane docks with a divider that widens it (PRD right-lane B5, B6).
    await page.locator("[data-lane-pin]").click();
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked");
    const handle = page.locator('[data-lane-divider="lane"]');
    const divider = await handle.boundingBox();
    const body = await page.locator("[data-lane-row]").boundingBox();
    if (!divider || !body) throw new Error("the docked lane has no resize geometry");
    const sidebarWidth = (await page.locator("[data-sidebar-box]").boundingBox())!.width;
    await page.mouse.move(divider.x + divider.width / 2, divider.y + 150);
    expect(await handle.evaluate((node, point) => node.contains(document.elementFromPoint(point.x, point.y)), {
      x: divider.x + divider.width / 2, y: divider.y + 150,
    })).toBe(true);
    await page.mouse.down();
    await page.mouse.move(body.x + 10, divider.y + 150, { steps: 4 });
    await expect(page.locator("[data-lane-guide]")).toHaveCount(1);
    await page.mouse.up();
    await expect(page.locator("[data-lane-guide]")).toHaveCount(0);
    const stages = new Set<Fit>();
    // The stored lane width clamps as the window narrows. An overlay's
    // outside-left divider can be behind the sidebar's resize hit area;
    // dragging raw coordinates there would resize an unrelated surface.
    for (const width of [2400, 2200, 2000, 1900, 1800, 1700, 1600, 1500, 1400, 1300, 1200, 1100]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
      await expect(page.locator("[data-view-tab-bar]")).toBeVisible();
      expect((await page.locator("[data-sidebar-box]").boundingBox())!.width).toBe(sidebarWidth);
      // A divider release commits once through the core; observe that final
      // geometry rather than sampling the optimistic drag's guide frame.
      await expect.poll(async () => {
        const drawn = await strip(page, "view");
        const want = expected(drawn.room, names.length);
        return drawn.tabs.every((tab) => Math.abs(tab.width - (tab.selected ? want.selected[0] : want.others[0])) < 0.5);
      }).toBe(true);
      const drawn = await strip(page, "view");
      const want = expected(drawn.room, names.length);
      for (const tab of drawn.tabs) {
        const fit = tab.selected ? want.selected : want.others;
        expect(tab.width, `File tab at room ${drawn.room}`).toBeCloseTo(fit[0], 0);
        expect(tab.title).toBe(fit[1] !== "marks");
        expect(tab.close).toBe(tab.selected || fit[1] === "titled");
        stages.add(fit[1]);
      }
      expect(drawn.scrolls).toBe(want.selected[0] + (names.length - 1) * want.others[0] > drawn.room + 0.01);
      expect(drawn.selectedInView).toBe(true);
      await screenshot(page, `file-tab-strip-${width}`);
      if (want.selected[1] === "titled" && want.others[1] !== "titled") {
        await tabs.first().click();
        await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
        const moved = await strip(page, "view");
        expect(moved.tabs[0]!.width).toBeCloseTo(TITLE_MIN, 0);
        expect(moved.tabs.at(-1)!.width).toBeCloseTo(want.others[0], 0);
        await tabs.last().click();
        await expect(tabs.last()).toHaveAttribute("aria-selected", "true");
      }
    }
    expect([...stages].sort()).toEqual(["compact", "marks", "titled"]);
    await tabs.first().click();
    await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
    expect((await strip(page, "view")).selectedInView).toBe(true);
    await expect(tabs.first()).toHaveAccessibleName(/한글 노트.md/);
    await tabs.first().locator("button").click();
    await expect(tabs).toHaveCount(names.length - 1);
  } finally { daemon?.stop(); herdr.stop(); }
});
