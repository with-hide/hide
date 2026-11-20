import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { quietFor } from "./wait";
import { countSent, enterWorkspace, screenshot, showTool } from "./wire";

// PRD lane-desk-pair-view D-50: the desk takes its place on the right and
// the tab's panes narrow beside it; a tool floats over the panes and moves
// none; a file opened from a floating tool floats with it, and settles when
// the tool closes. Each test starts one stack of two panes side by side.

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const railTool = (page: Page, tool: string) => page.locator(`[data-lane-tool="${tool}"]`);
const paneBox = async (page: Page, id: string) => (await page.locator(`[data-pane-view="${id}"]`).boundingBox())!;
const laneBox = async (page: Page) => (await page.locator("[data-lane-frame]").boundingBox())!;

type Stack = { page: Page; fixture: AgentFixture; daemon: Daemon; sent: Map<string, number> };

async function startStack(page: Page, name: string): Promise<Stack> {
  const fixture = await startFixture({ agents: false });
  const folder = path.join(fixture.root, "fixture");
  for (const file of ["a.md", "b.md"]) fs.writeFileSync(path.join(folder, file), `# ${file}\n`);
  const daemon = await startHided(fixture, name);
  const sent = countSent(page);
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  for (const pane of fixture.panes) await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible();
  await quietFor(page, 500, "the first fit has landed");
  return { page, fixture, daemon, sent };
}

const resizes = (stack: Stack) => stack.sent.get("terminal_resize") ?? 0;

/** Both panes' widths, left then right. */
const widths = async (stack: Stack) => Promise.all(stack.fixture.panes.map(async (pane) => Math.round((await paneBox(stack.page, pane)).width)));

test("the desk takes its place on the right, both panes narrowing beside it once, its divider lands once, and ⌘E gives the width back (D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const stack = await startStack(page, "desk-docks-stand");
  try {
    const [left, right] = stack.fixture.panes;
    const full = await widths(stack);
    const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
    const ran = await runInPane(stack.fixture, stack.daemon, left, "a.md", { env: { HIDE_STATE_DIR: stack.daemon.stateDir }, argv: [hide, "file", "open", "a.md"] });
    expect(ran.status, ran.stderr).toBe(0);
    await expect(railTool(page, "documents")).toBeEnabled();
    const before = resizes(stack);

    await railTool(page, "documents").click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    // Nothing covers a pane: the right pane ends where the desk's divider begins.
    const lane = await laneBox(page);
    const rightPane = await paneBox(page, right);
    expect(rightPane.x + rightPane.width).toBeLessThanOrEqual(lane.x);
    const narrowed = await widths(stack);
    expect(narrowed[0]).toBeLessThan(full[0]!);
    expect(narrowed[1]).toBeLessThan(full[1]!);
    // Each terminal follows its drawn width with one resize.
    await expect.poll(() => resizes(stack)).toBe(before + 2);
    // The desk's way into pair view is ◧, the pane head's mark.
    await expect(page.locator("[data-desk-pair]")).toBeVisible();
    await expect(page.locator("[data-desk-pin]")).toHaveCount(0);
    await screenshot(page, "desk-docks-standing");

    // The divider drags a guide and lands once, narrowing the panes once more.
    const grip = page.locator('[data-lane-frame="docked"] [data-lane-divider="lane"]');
    const gripBox = (await grip.boundingBox())!;
    const dragged = resizes(stack);
    await page.mouse.move(gripBox.x + gripBox.width / 2, gripBox.y + 200);
    await page.mouse.down();
    await page.mouse.move(gripBox.x + gripBox.width / 2 - 120, gripBox.y + 200, { steps: 6 });
    await expect(page.locator("[data-lane-guide]")).toHaveCount(1);
    expect(resizes(stack)).toBe(dragged);
    await page.mouse.up();
    await expect.poll(async () => Math.round((await laneBox(page)).width - lane.width)).toBeGreaterThanOrEqual(119);
    await expect.poll(() => resizes(stack)).toBe(dragged + 2);
    const wider = await widths(stack);
    expect(wider[1]).toBeLessThan(narrowed[1]!);

    // ⌘E closes the desk and the panes take the body back.
    await page.locator(`[data-pane-view="${left}"]`).click();
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect.poll(() => widths(stack)).toEqual(full);
  } finally {
    stack.daemon.stop();
    stack.fixture.stop();
  }
});

test("the Explorer floats over the panes; a file opened from it floats with it, and settles beside the panes when Escape closes the Explorer (D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const stack = await startStack(page, "desk-docks-float");
  try {
    const [left, right] = stack.fixture.panes;
    const full = await widths(stack);
    const before = resizes(stack);

    await showTool(page, "explorer");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    // The floating Explorer moves no pane.
    expect(await widths(stack)).toEqual(full);

    await page.locator('[data-explorer-row$="/a.md"]').click();
    await expect(page.locator("[data-editor-body] .cm-content")).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "explorer");
    // The desk floats on the Explorer's left, the two one layer over the panes.
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    const viewColumn = (await page.locator('[data-lane-column="view"]').boundingBox())!;
    const toolColumn = (await page.locator('[data-lane-column="tool"]').boundingBox())!;
    expect(viewColumn.x + viewColumn.width).toBeLessThanOrEqual(toolColumn.x);
    expect(await widths(stack)).toEqual(full);
    await quietFor(page, 300, "nothing resized while the desk floats");
    expect(resizes(stack)).toBe(before);
    await screenshot(page, "desk-docks-floating-with-explorer");

    // Escape closes the Explorer; the desk settles and the panes narrow once.
    await page.locator('[data-lane="explorer"] [data-explorer-tree]').focus();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    const lane = await laneBox(page);
    const rightPane = await paneBox(page, right);
    expect(rightPane.x + rightPane.width).toBeLessThanOrEqual(lane.x);
    await expect.poll(() => resizes(stack)).toBe(before + 2);
    await screenshot(page, "desk-docks-settled");

    // The Explorer opened beside the standing desk floats at the right edge
    // with the desk on its left, one layer, and moves no pane; a press on a
    // pane closes it and the desk takes its place again.
    const standing = await widths(stack);
    await showTool(page, "explorer");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    const desk = (await page.locator('[data-lane-column="view"]').boundingBox())!;
    const tree = (await page.locator('[data-lane-column="tool"]').boundingBox())!;
    expect(desk.x + desk.width).toBeLessThanOrEqual(tree.x);
    expect(await widths(stack)).toEqual(standing);
    await screenshot(page, "desk-docks-explorer-beside-standing-desk");
    await page.locator(`[data-pane-view="${left}"]`).click();
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    expect(await widths(stack)).toEqual(standing);
    expect(resizes(stack)).toBe(before + 2);
  } finally {
    stack.daemon.stop();
    stack.fixture.stop();
  }
});

test("a press outside the floating desk and Explorer lands on what was pressed before the desk takes its place (D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const stack = await startStack(page, "desk-docks-press");
  try {
    const [left] = stack.fixture.panes;
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/a.md"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");

    // The left pane's zoom button is outside the floating layer; while the
    // press is held the desk still floats, so the button stays under it.
    const zoom = page.locator(`[data-pane-view="${left}"] [data-pane-zoom="false"]`);
    const box = (await zoom.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await quietFor(page, 300, "a desk that stood on the press would have been drawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await page.mouse.up();
    // The press zoomed the pane, then the Explorer closed and the desk stood.
    await expect(page.locator('[data-canvas][data-zoomed="true"]')).toHaveCount(1);
    await expect(screen(page)).toHaveAttribute("data-lane-shows", "none");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
  } finally {
    stack.daemon.stop();
    stack.fixture.stop();
  }
});

test("coming back to a Workspace whose desk stands fits each terminal to its narrowed width only, never to the whole body first (D-50) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const stack = await startStack(page, "desk-docks-return");
  try {
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/a.md"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await page.locator('[data-lane="explorer"] [data-explorer-tree]').focus();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await quietFor(page, 500, "the panes narrowed beside the desk");
    const standing = await widths(stack);

    await page.locator("[data-sidebar-missions]").click();
    await expect(page.locator("[data-mission-screen-view]")).toBeVisible();
    await expect(screen(page)).toHaveCount(0);
    const before = resizes(stack);
    await page.locator("nav[data-sidebar] [data-project]").first().locator("[data-project-row], button[data-checkout]").first().click({ position: { x: 12, y: 8 } });
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect.poll(() => widths(stack)).toEqual(standing);
    await quietFor(page, 500, "every terminal's fit has landed");
    // Each terminal attaches again with one fit at its narrowed width; a first grid at the whole body would add one each.
    expect(resizes(stack)).toBe(before + stack.fixture.panes.length);
  } finally {
    stack.daemon.stop();
    stack.fixture.stop();
  }
});
