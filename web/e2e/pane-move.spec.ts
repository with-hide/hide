// Pane-header move B1-B10/B18/B21: production UI and real isolated PTYs.
import { expect, test as base } from "@playwright/test";
// Throwaway probe (with-hide/hide-private#476, #187, #140, #534, #518): this spec's flaky test only, registered 20 times.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const test = Object.assign((title: string, ...rest: any[]) => {
  if (title !== "a tab drop and the keyboard move menu keep the same terminal in a new tab") return;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  for (let run = 1; run <= 20; run++) (base as any)(`${title} #${run}`, ...rest);
}, { describe: base.describe, use: base.use, step: base.step, setTimeout: base.setTimeout });
import { beginDrag, expectDirection, paneView, selectMarker, startMove, tabNow, tabTitle } from "./pane-move-fixture";
import { sessionIn } from "./fixture-agents";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 90_000 });

// Process ownership and PTY identity must hold through each system's engine.
test("header edges insert in all four directions while retaining terminal, process, session and xterm", { tag: "@platform" }, async ({ page }) => {
  const flow = await startMove(page);
  try {
    const [source, target] = flow.panes;
    const session = await sessionIn(flow.daemon, source), tab = await tabNow(flow, source);
    const textarea = await paneView(page, source).locator(".xterm-helper-textarea").elementHandle();
    // The terminal keeps what it showed: a line typed before the moves is still on its screen after them.
    await paneView(page, source).locator(".xterm-helper-textarea").focus(); await page.keyboard.type("MOVE_KEPT\n");
    await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_KEPT");
    for (const [index, edge] of (["left", "right", "up", "down"] as const).entries()) await test.step(edge, async () => {
      await beginDrag(page, source, paneView(page, target), edge);
      await expect(page.locator('[data-pane-move-target="pane"]')).toBeVisible(); await page.mouse.up();
      await flow.completed(index + 1);
      await expect(page.locator('[data-pane-move-status]')).toHaveCount(0);
      await expectDirection(page, source, target, edge);
    });
    expect(await tabNow(flow, source)).toBe(tab);
    expect(await page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_KEPT");
    expect(await sessionIn(flow.daemon, source)).toBe(session);
    expect(await textarea!.evaluate((node, source) => node === document.querySelector(`[data-pane-view="${source}"] .xterm-helper-textarea`), source)).toBe(true);
    await screenshot(page, "pane-move-four-directions");
  } finally { flow.stop(); }
});

test("hover and Escape preserve tab selection, keyboard, selection and terminal grid", async ({ page }) => {
  const flow = await startMove(page);
  try {
    const source = flow.panes[0];
    await paneView(page, source).locator(".xterm-helper-textarea").focus(); await page.keyboard.type("MOVE_SELECTION\n");
    await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_SELECTION");
    await selectMarker(page, source, "MOVE_SELECTION");
    const keyboard = await page.evaluateHandle(() => document.activeElement);
    const grid = await page.evaluate(id => window.__hideProbe!.paneGrid(id), source);
    const focus = flow.sent.get("focus_pane") ?? 0, selection = flow.sent.get("agent_layout.focus") ?? 0;
    await beginDrag(page, source, tabTitle(page, flow.target.tab));
    await expect(page.locator('[data-pane-move-preview-tab]')).toBeVisible();
    // The dragged pane's name rides under the pointer while the drag runs.
    await expect(page.locator("[data-pane-move-chip]")).not.toBeEmpty();
    await expect(tabTitle(page, flow.fixture.tab)).toHaveAttribute("aria-selected", "true");
    expect(await keyboard.evaluate(node => node === document.activeElement)).toBe(true);
    expect(await page.evaluate(id => ({ grid: window.__hideProbe!.paneGrid(id), selected: window.__hideProbe!.paneSelection(id) }), source)).toEqual({ grid, selected: "MOVE_SELECTION" });
    await screenshot(page, "pane-move-hover-preview"); await page.keyboard.press("Escape"); await page.mouse.up();
    await expect(page.locator('[data-pane-move-preview-tab]')).toHaveCount(0);
    await expect(page.locator("[data-pane-move-chip]")).toHaveCount(0);
    expect(flow.sent.get("pane_move") ?? 0).toBe(0); expect(flow.sent.get("focus_pane") ?? 0).toBe(focus); expect(flow.sent.get("agent_layout.focus") ?? 0).toBe(selection);
    await expect(paneView(page, source)).toBeVisible();
  } finally { flow.stop(); }
});

test("a tab drop and the keyboard move menu keep the same terminal in a new tab", async ({ page }) => {
  const flow = await startMove(page);
  try {
    const source = flow.panes[0];
    const session = await sessionIn(flow.daemon, source);
    await paneView(page, source).locator(".xterm-helper-textarea").focus(); await page.keyboard.type("MOVE_KEPT\n");
    await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_KEPT");
    await beginDrag(page, source, tabTitle(page, flow.target.tab)); await page.mouse.up(); await flow.completed();
    await expect.poll(() => tabNow(flow, source)).toBe(flow.target.tab);
    await expect(paneView(page, source)).toBeVisible();
    await expectDirection(page, source, flow.target.pane, "right");
    await paneView(page, source).locator(`[data-pane-menu="${source}"]`).focus(); await page.keyboard.press("Enter");
    await page.locator('[data-menu-item="pane_move"]').focus(); await page.keyboard.press("ArrowRight");
    await page.locator('[data-menu-item="pane_move_area:a1"]').focus(); await page.keyboard.press("ArrowRight");
    const choice = page.getByRole("menuitem", { name: "Move to new tab", exact: true }); await choice.focus(); await page.keyboard.press("Enter"); await flow.completed(2);
    await expect.poll(async () => { const tab = await tabNow(flow, source); return tab !== undefined && tab !== flow.target.tab; }).toBe(true);
    // The pane's view in the new tab is a new one: it takes the screen back from its terminal when it attaches, after the move completes.
    await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_KEPT");
    expect(await sessionIn(flow.daemon, source)).toBe(session);
    expect(flow.sent.get("pane_move")).toBe(2);
    await screenshot(page, "pane-move-menu-new-tab");
  } finally { flow.stop(); }
});
