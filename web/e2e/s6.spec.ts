// The S6 flow on an isolated pinned Herdr and hided: a first run on Main, a Project's
// Overview and its Workspace (B1, B2, B4), the right lane from its rail, the
// path's menu and the chords, one tool at a time, pinned beside live agents
// with a terminal resized once per pin change (PRD right-lane), a delegated
// child reached from its parent's chip and left by its Return (B14-B16), and
// a restart that brings the Workspace back with a View tab whose file is gone
// marked unavailable (B19, B20).

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { declareParent, herdrHasFocus, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chooseLane, countSent, screenshot, showExplorer } from "./wire";
import { chord } from "./chords";
import { projectRow } from "./overview-entry";
import { toPage } from "../../desktop/src/main/wirePath";
import { quietFor, unchangedForFrames } from "./wait";

test.describe.configure({ timeout: 120_000 });

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

/**
 * Waits until `workspace-views.json` holds the one front View whose path
 * matches. A restart kills the daemon outright on Windows, so what was just
 * put in front has to be saved before it goes: the file that is about to go
 * away is put in front so its state shows after the restart.
 */
async function savedFront(daemon: Daemon, pattern: RegExp): Promise<void> {
  const savedFronts = (): string[] => {
    const fronts: string[] = [];
    const walk = (node: unknown): void => {
      if (!node || typeof node !== "object") return;
      if (Array.isArray(node)) return node.forEach(walk);
      const area = node as { active?: unknown; displays?: { id: string; path: string }[] };
      if (typeof area.active === "string" && Array.isArray(area.displays)) {
        const shown = area.displays.find((display) => display.id === area.active);
        if (shown) fronts.push(shown.path);
      }
      Object.values(node).forEach(walk);
    };
    const saved = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "workspace-views.json"), "utf8")) as {
      workspaces: { view_layout: unknown }[];
    };
    saved.workspaces.forEach((workspace) => walk(workspace.view_layout));
    return fronts;
  };
  await expect.poll(savedFronts).toEqual([expect.stringMatching(pattern)]);
}

test("Main, Overview and a Workspace with its lane, tools and delegated child", async ({ page }) => {
  // Wide enough to dock the lane beside the agents.
  await page.setViewportSize({ width: 1920, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const [parent, child] = herdr.panes;
    // The Explorer names rows by the checkout's resolved path.
    const root = path.join(fs.realpathSync(herdr.root), "fixture");
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.md"), "# Notes\n\nkept across a restart\n");
    fs.writeFileSync(path.join(herdr.root, "fixture", "gone.txt"), "removed before the restart\n");
    daemon = await startHided(herdr, "s6");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await open(page, daemon);

    // A first run starts on the device Overview (D-11; PRD overview-device-tasks
    // D-02), whose Projects tab lists this machine's Project.
    const workspace = page.locator("[data-workspace-screen]");
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    await expect(workspace).toHaveCount(0);
    await expect(await projectRow(page, "fixture")).toBeVisible();
    await screenshot(page, "s6-main");

    // A project opens its local home with the retained settings, cleanup and Open controls.
    await (await projectRow(page, "fixture")).getByRole("button", { name: "fixture", exact: true }).click();
    const panel = page.locator("[data-overview-project-home]");
    await expect(panel).toBeVisible();
    await expect(panel.locator('[data-project-panel-now="2"]')).toBeVisible({ timeout: 20_000 });
    await screenshot(page, "s6-overview");
    await panel.locator("[data-project-panel-open]").click();
    await expect(workspace).toBeVisible();
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toBeVisible();
    // A new Workspace shows Agent Views alone with its lane closed (PRD
    // right-lane B1). The top row ends with Overview and Open server (its
    // server count, once known) as icons only; the tools are on the rail.
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-lane-frame]")).toHaveCount(0);
    const actions = page.locator("[data-workspace-toolbar] [data-workspace-actions] button");
    expect(await actions.evaluateAll((buttons) => buttons.map((button) => [button.getAttribute("aria-label"), button.textContent?.trim() ?? ""]))).toEqual([
      ["Overview", ""],
      [expect.stringMatching(/^Open server(, \d+ running)?$/), ""],
    ]);
    const body = page.locator("[data-lane-row]");
    const bodyBox = (await body.boundingBox())!;
    const toolbarBox = (await page.locator("[data-workspace-toolbar]").boundingBox())!;
    expect(toolbarBox.y + toolbarBox.height).toBeLessThanOrEqual(bodyBox.y + 1);

    // B1, B20: each rail icon names itself and its scope in its hint, and
    // reads pressed only while the lane shows its tool.
    const filesIcon = page.locator('[data-lane-tool="files"]');
    const prsIcon = page.locator('[data-lane-tool="prs"]');
    await expect(filesIcon).toHaveAttribute("aria-pressed", "false");
    await prsIcon.hover();
    await expect(page.locator('[data-slot="tooltip-content"]')).toContainText("PRs");
    await filesIcon.hover();
    await expect(page.locator('[data-slot="tooltip-content"]')).toContainText("Files");
    await page.mouse.move(bodyBox.x + 40, bodyBox.y + 40);

    // The PRs icon opens the lane on PRs over the agents, the path's menu
    // closes it, ⌘E brings the closed lane back on PRs, the tool it last
    // showed, and the rail swaps the one tool it holds (B2, B20, D-01).
    const lane = page.locator("[data-lane-frame]");
    const agentArea = page.locator("[data-agent-area]");
    await prsIcon.click();
    await expect(workspace).toHaveAttribute("data-lane-shows", "prs");
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect.poll(() => last.get("workspace_view")).toEqual({ tool: "prs" });
    await expect(prsIcon).toHaveAttribute("aria-pressed", "true");
    const laneIdeal = await page.evaluate(() => Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--size-lane-ideal")));
    expect((await lane.boundingBox())!.width).toBeCloseTo(laneIdeal, 0);
    await chooseLane(page);
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await expect.poll(() => last.get("workspace_view")).toEqual({ tool: null });
    await page.keyboard.press(chord("toggle_lane"));
    await expect(workspace).toHaveAttribute("data-lane-shows", "prs");
    await page.locator('[data-lane-tool="changes"]').click();
    await expect.poll(() => last.get("workspace_view")).toEqual({ tool: "changes" });
    await expect(page.locator('[data-lane="changes"]')).toBeVisible();
    await expect(page.locator("[data-explorer]")).toHaveCount(0);
    // The sidebar switch keeps no chord of its own: the chords above left it on Projects.
    await expect(page.locator("[data-sidebar]")).toHaveAttribute("data-sidebar", "projects");

    // Files from the tree open in the files tool; pinned, the lane docks
    // beside the agents, which resize once (B5, B16, B24).
    const resizes = () => sent.get("terminal_resize") ?? 0;
    const settled = () => unchangedForFrames(page, resizes);
    await showExplorer(page);
    await page.locator(`[data-explorer-row="${toPage(path.join(root, "notes.md"))}"]`).dblclick();
    await page.locator(`[data-explorer-row="${toPage(path.join(root, "gone.txt"))}"]`).dblclick();
    await expect(lane.locator('[data-tab-kind="file"]')).toHaveCount(2);
    await settled();
    const resizesBeforePin = resizes();
    await page.locator("[data-lane-pin]").click();
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect.poll(() => last.get("workspace_view")).toEqual({ pinned: true });
    await expect.poll(resizes, { timeout: 10_000 }).toBeGreaterThan(resizesBeforePin);
    await settled();
    const panesShown = await page.locator("[data-pane-view]").count();
    expect(resizes() - resizesBeforePin).toBe(panesShown);
    const agentsBox = (await agentArea.boundingBox())!;
    const laneBox = (await lane.boundingBox())!;
    expect(agentsBox.x + agentsBox.width).toBeLessThanOrEqual(laneBox.x);
    expect(laneBox.x + laneBox.width).toBeCloseTo(bodyBox.x + bodyBox.width, 0);

    // The agents beside the lane are live: a click focuses the pane, typing
    // reaches it, and ⌘F finds in that pane; in a View area ⌘F is the
    // document's (B23).
    const childHost = page.locator(`[data-pane-view="${child}"] [data-terminal-host]`);
    await childHost.click({ position: { x: 40, y: 60 } });
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });
    await expect.poll(() => last.get("focus_pane")).toMatchObject({ pane_id: child, origin: "operator" });
    await page.keyboard.type("typed beside the lane");
    await expect.poll(() => fs.readFileSync(herdr.inputLogs[1], "utf8"), { timeout: 10_000 }).toContain("typed beside the lane");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    const mouseReports = () => fs.readFileSync(herdr.inputLogs[1], "latin1").split("\x1b[<0;").length - 1;
    const reportsBefore = mouseReports();
    await childHost.click({ position: { x: 40, y: 60 } });
    await expect.poll(mouseReports, { timeout: 10_000 }).toBe(reportsBefore + 2);
    const heardBefore = fs.readFileSync(herdr.inputLogs[1], "latin1").length;
    await page.keyboard.press(chord("find_in_pane"));
    await expect.poll(() => JSON.stringify(fs.readFileSync(herdr.inputLogs[1], "latin1").slice(heardBefore)), { timeout: 10_000 }).toBe(JSON.stringify("\x0f/"));
    await expect.poll(() => last.get("pane_find_open")).toMatchObject({ pane_id: child });
    await expect(page.locator(".cm-search")).toHaveCount(0);
    await expect(page.locator("[data-find-bar]")).toHaveCount(0);
    await lane.locator("[data-editor-body] .cm-content").click();
    await page.keyboard.press(chord("find_in_pane"));
    await expect(page.locator(".cm-search")).toBeVisible();
    await page.keyboard.press("Escape");
    await screenshot(page, "s6-lane-docked");

    // ⌘E closes the lane without closing a view; the Files icon counts the
    // views it keeps, and the keyboard that was in the lane goes back to the
    // focused pane (B2, B7, B17). Opening it brings the same tabs, docked.
    await settled();
    const resizesBeforeClose = resizes();
    await lane.locator("[data-editor-body] .cm-content").click();
    await page.keyboard.press(chord("toggle_lane"));
    await expect(workspace).toHaveAttribute("data-lane-mode", "closed");
    await expect.poll(() => last.get("workspace_view")).toEqual({ tool: null });
    await expect(filesIcon).toHaveAttribute("aria-pressed", "false");
    await expect(filesIcon.locator("[data-lane-badge]")).toHaveText("2");
    await expect(filesIcon).toHaveAttribute("aria-description", "2 views open");
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.closest("[data-pane-view]")?.getAttribute("data-pane-view") ?? null))
      .not.toBeNull();
    await expect.poll(resizes, { timeout: 10_000 }).toBeGreaterThan(resizesBeforeClose);
    await settled();
    expect(resizes() - resizesBeforeClose).toBe(panesShown);
    await screenshot(page, "s6-lane-closed");
    await filesIcon.click();
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect(lane.locator('[data-tab-kind="file"]')).toHaveCount(2);
    await expect(filesIcon.locator("[data-lane-badge]")).toHaveCount(0);

    // An agent chosen from the sidebar changes no lane (B22, D-17).
    await page.locator(`[data-agent-open="${parent}"]`).first().click();
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect.poll(() => herdrHasFocus(herdr, parent), { timeout: 15_000 }).toBe(true);

    // B19: a window too narrow to dock opens the pinned lane over the agents
    // and stores nothing; widening docks it again.
    const sentBeforeNarrow = sent.get("workspace_view") ?? 0;
    await page.setViewportSize({ width: 1000, height: 1000 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "overlay");
    await expect(page.locator("[data-lane-pin]")).toHaveAttribute("data-lane-pin", "narrow");
    await expect(agentArea).toBeVisible();
    await screenshot(page, "s6-lane-narrow");
    expect(sent.get("workspace_view") ?? 0).toBe(sentBeforeNarrow);
    await page.setViewportSize({ width: 1920, height: 1000 });
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });

    // A declared child shows in its parent's header tree; the core
    // moves the delegated pane to its own tab, so the chip crosses tabs with
    // one tracked focus, and the child's Return comes back the same way
    // (B14-B16).
    declareParent(herdr, child, parent);
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveCount(0, { timeout: 20_000 });
    await page.locator(`[data-pane-view="${parent}"] [data-pane-tree="${parent}"]`).click();
    const chip = page.locator(`[data-agent-child="${child}"]`);
    await expect(chip).toBeVisible({ timeout: 20_000 });
    await expect(chip).toContainText("Agent two");
    await screenshot(page, "s6-child-chip");
    await chip.click();
    await expect.poll(() => last.get("focus_pane")?.pane_id).toBe(child);
    expect(last.get("focus_pane")?.request_id).toEqual(expect.any(String));
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });
    // Herdr's layout confirms the move, and the in-flight line goes away.
    await expect(page.locator("[data-relation-status]")).toHaveCount(0, { timeout: 15_000 });
    const back = page.locator(`[data-pane-view="${child}"] [data-pane-return="${parent}"]`);
    await expect(back).toBeVisible();
    await screenshot(page, "s6-child-return");
    await back.click();
    await expect.poll(() => last.get("focus_pane")?.pane_id).toBe(parent);
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true", { timeout: 15_000 });
    await expect(page.locator("[data-relation-status]")).toHaveCount(0, { timeout: 15_000 });

    // The pane menu leaves the pane's own children to its tree button, and
    // opening it moves no focus (B16, B18).
    const menuButton = page.locator(`[data-pane-menu="${parent}"]`);
    const focusCount = sent.get("focus_pane") ?? 0;
    await menuButton.click();
    const copyName = page.locator('[data-menu-group="copy"]');
    await expect(copyName).toBeVisible();
    await expect(page.locator(`[data-menu-item="open:${child}"]`)).toHaveCount(0);
    await screenshot(page, "s6-pane-menu");
    await page.mouse.move(0, 0);
    await page.keyboard.press("Escape");
    await expect(copyName).toHaveCount(0);
    expect(sent.get("focus_pane") ?? 0).toBe(focusCount);
    // The focus the closed menu hands back to its button does not bring the
    // button's hint up after it; the hint waits for the pointer to come back.
    await quietFor(page, 800, "the closed menu's button does not bring its hint up");
    await expect(page.locator('[data-slot="tooltip-content"]')).toHaveCount(0);

    // A restart brings the Workspace back with its lane docked at its width
    // and its View tabs; the file that went away stays as an unavailable tab
    // (S6 B19, B20; PRD right-lane B22).
    const laneWidth = (await lane.boundingBox())!.width;
    await lane.locator('[data-tab-kind="file"]', { hasText: "gone.txt" }).click();
    await savedFront(daemon, /\/gone\.txt$/);
    fs.rmSync(path.join(herdr.root, "fixture", "gone.txt"));
    daemon = await daemon.restart();
    // Reopening the app is a fresh page, not a reconnect of this one.
    await page.goto("about:blank");
    await open(page, daemon);
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked", { timeout: 20_000 });
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-shows", "files");
    await expect.poll(async () => (await lane.boundingBox())!.width).toBeCloseTo(laneWidth, 0);
    await expect(page.locator('[data-view-area] [data-tab-kind="file"]')).toHaveCount(2, { timeout: 15_000 });
    await expect(page.locator('[data-view-area] [data-unavailable="true"]')).toHaveCount(1);
    await expect(page.locator("[data-view-area] [data-close-unavailable]")).toBeVisible();
    await screenshot(page, "s6-restored-unavailable");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
