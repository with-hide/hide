// Pane-header move B1-B10/B18/B21: production UI and real isolated PTYs.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import { beginDrag, expectDirection, paneFacts, paneView, processFacts, selectMarker, startMove, tabTitle } from "./pane-move-fixture";
import { sessionOf } from "./herdr-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 90_000 });

// Process ownership and PTY identity must hold through each system's Herdr.
test("header edges insert in all four directions while retaining terminal, process, session and xterm", { tag: "@platform" }, async ({ page }) => {
  const flow = await startMove(page);
  try {
    const [source, target] = flow.herdr.panes;
    const before = paneFacts(flow.herdr).find(pane => pane.pane_id === source)!;
    const process = processFacts(flow.herdr, source), session = sessionOf(flow.herdr, source);
    const textarea = await paneView(page, source).locator(".xterm-helper-textarea").elementHandle();
    for (const [index, edge] of (["left", "right", "up", "down"] as const).entries()) await test.step(edge, async () => {
      await beginDrag(page, source, paneView(page, target), edge);
      await expect(page.locator('[data-pane-move-target="pane"]')).toBeVisible(); await page.mouse.up();
      await expect.poll(() => flow.sent.get("pane_move")).toBe(index + 1);
      await expect(page.locator('[data-pane-move-status]')).toHaveCount(0);
      await expectDirection(page, source, target, edge);
    });
    expect(paneFacts(flow.herdr).find(pane => pane.pane_id === source)?.terminal_id).toBe(before.terminal_id);
    expect(sessionOf(flow.herdr, source)).toBe(session);
    expect(processFacts(flow.herdr, source)).toEqual(process);
    expect(await textarea!.evaluate((node, source) => node === document.querySelector(`[data-pane-view="${source}"] .xterm-helper-textarea`), source)).toBe(true);
    await screenshot(page, "pane-move-four-directions");
  } finally { flow.stop(); }
});

test("hover and Escape preserve tab selection, keyboard, selection and terminal grid", async ({ page }) => {
  const flow = await startMove(page);
  try {
    const source = flow.herdr.panes[0];
    await paneView(page, source).locator(".xterm-helper-textarea").focus(); await page.keyboard.type("MOVE_SELECTION\n");
    await expect.poll(() => page.evaluate(id => window.__hideProbe!.paneText(id), source)).toContain("MOVE_SELECTION");
    await selectMarker(page, source, "MOVE_SELECTION");
    const keyboard = await page.evaluateHandle(() => document.activeElement);
    const grid = await page.evaluate(id => window.__hideProbe!.paneGrid(id), source);
    const focus = flow.sent.get("focus_pane") ?? 0, selection = flow.sent.get("agent_layout.focus") ?? 0;
    await beginDrag(page, source, tabTitle(page, flow.target.tab.tab_id));
    await expect(page.locator('[data-pane-move-preview-tab]')).toBeVisible();
    await expect(tabTitle(page, flow.herdr.tab)).toHaveAttribute("aria-selected", "true");
    expect(await keyboard.evaluate(node => node === document.activeElement)).toBe(true);
    expect(await page.evaluate(id => ({ grid: window.__hideProbe!.paneGrid(id), selected: window.__hideProbe!.paneSelection(id) }), source)).toEqual({ grid, selected: "MOVE_SELECTION" });
    await screenshot(page, "pane-move-hover-preview"); await page.keyboard.press("Escape"); await page.mouse.up();
    await expect(page.locator('[data-pane-move-preview-tab]')).toHaveCount(0);
    expect(flow.sent.get("pane_move") ?? 0).toBe(0); expect(flow.sent.get("focus_pane") ?? 0).toBe(focus); expect(flow.sent.get("agent_layout.focus") ?? 0).toBe(selection);
    await expect(paneView(page, source)).toBeVisible();
  } finally { flow.stop(); }
});

test("a tab drop and the keyboard move menu keep the same terminal in a new tab", async ({ page }) => {
  const flow = await startMove(page);
  try {
    const source = flow.herdr.panes[0], before = paneFacts(flow.herdr).find(pane => pane.pane_id === source)!;
    const process = processFacts(flow.herdr, source), session = sessionOf(flow.herdr, source);
    await beginDrag(page, source, tabTitle(page, flow.target.tab.tab_id)); await page.mouse.up();
    await expect.poll(() => paneFacts(flow.herdr).find(pane => pane.terminal_id === before.terminal_id)?.tab_id).toBe(flow.target.tab.tab_id);
    await expect(paneView(page, source)).toBeVisible();
    await expectDirection(page, source, flow.target.root_pane.pane_id, "right");
    await paneView(page, source).locator(`[data-pane-menu="${source}"]`).focus(); await page.keyboard.press("Enter");
    await page.locator('[data-menu-item="pane_move"]').focus(); await page.keyboard.press("ArrowRight");
    await page.locator('[data-menu-item="pane_move_area:a1"]').focus(); await page.keyboard.press("ArrowRight");
    const choice = page.getByRole("menuitem", { name: "Move to new tab", exact: true }); await choice.focus(); await page.keyboard.press("Enter");
    await expect.poll(() => paneFacts(flow.herdr).find(pane => pane.terminal_id === before.terminal_id)?.tab_id).not.toBe(flow.target.tab.tab_id);
    expect(paneFacts(flow.herdr).find(pane => pane.terminal_id === before.terminal_id)).toMatchObject({ pane_id: source, terminal_id: before.terminal_id });
    expect(processFacts(flow.herdr, source)).toEqual(process);
    expect(sessionOf(flow.herdr, source)).toBe(session);
    expect(flow.sent.get("pane_move")).toBe(2);
    await screenshot(page, "pane-move-menu-new-tab");
  } finally { flow.stop(); }
});

// Amendment 1: Herdr changes the public pane id across native workspaces.
test("a cross-workspace tab drop keeps the terminal and xterm under the new public pane id", { tag: "@platform" }, async ({ page }) => {
  const flow = await startMove(page, true);
  try {
    const source = flow.herdr.panes[0], before = paneFacts(flow.herdr).find(pane => pane.pane_id === source)!;
    const process = processFacts(flow.herdr, source), session = sessionOf(flow.herdr, source);
    const textarea = await paneView(page, source).locator(".xterm-helper-textarea").elementHandle();
    await beginDrag(page, source, tabTitle(page, flow.target.tab.tab_id)); await page.mouse.up();
    await expect.poll(() => paneFacts(flow.herdr).find(pane => pane.terminal_id === before.terminal_id)?.tab_id).toBe(flow.target.tab.tab_id);
    const after = paneFacts(flow.herdr).find(pane => pane.terminal_id === before.terminal_id)!;
    expect(after.pane_id).not.toBe(source); expect(sessionOf(flow.herdr, after.pane_id)).toBe(session);
    expect(processFacts(flow.herdr, after.pane_id)).toEqual(process);
    await expect(paneView(page, after.pane_id)).toHaveAttribute("data-transport", "controlling");
    expect(await textarea!.evaluate((node, id) => node === document.querySelector(`[data-pane-view="${id}"] .xterm-helper-textarea`), after.pane_id)).toBe(true);
    await paneView(page, after.pane_id).locator(".xterm-helper-textarea").focus(); await page.keyboard.type("AFTER_MOVE");
    await expect.poll(() => fs.readFileSync(flow.herdr.inputLogs[0], "utf8")).toContain("AFTER_MOVE");
    await screenshot(page, "pane-move-cross-workspace");
  } finally { flow.stop(); }
});
