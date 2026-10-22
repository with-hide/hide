// Recent navigation on an isolated pinned Herdr and hided
// (docs/UI_BEHAVIOR.md, Recent navigation): explicitly bound global Recent Panels
// walks one order over Herdr tabs and View displays across checkouts and
// commits one event on releasing ⌥; Recent Projects (⌥Tab) brings the
// previous project back on the surface it was last used on. Outside a View
// area ⌥` walks the Agent panes the keyboard has been in, across projects;
// a focused View keeps its local tabs.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { fixtureExecutable } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, showExplorer, showProjectRow } from "./wire";
import { chord, held, label } from "./chords";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

test("Recent Panels crosses checkouts onto a display and a tab; Recent Projects restores the last surface", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "plan.txt"), "plan line\n");
    fs.mkdirSync(path.join(herdr.root, "beta"), { recursive: true });
    const beta = herdr.run([
      "workspace", "create", "--cwd", path.join(herdr.root, "beta"), "--label", "beta", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
    ]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } };
    const betaTab = beta.result.tab.tab_id;
    // One agent per tab, so each row is named and marked by its agent: a
    // Codex in beta (the fixture's shim under that name) and a Claude alone
    // in a second fixture tab. The fixture's first tab holds two agents and
    // keeps its Herdr label and the neutral mark.
    fs.copyFileSync(path.join(herdr.root, "bin", fixtureExecutable("claude")), path.join(herdr.root, "bin", fixtureExecutable("codex")));
    const solo = herdr.run([
      "tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "solo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
    ]) as { result: { root_pane: { pane_id: string } } };
    for (const [name, kind, pane] of [["three", "codex", beta.result.root_pane.pane_id], ["four", "claude", solo.result.root_pane.pane_id]] as const) {
      await expect.poll(() => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "recent", "--lines", "5"], { env: herdr.env, encoding: "utf8" }), { timeout: 20_000 }).toContain("fixture %");
      herdr.run(["agent", "start", name, "--kind", kind, "--pane", pane]);
    }
    daemon = await startHided(herdr, "recent");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const root = path.join(fs.realpathSync(herdr.root), "fixture");
    const canvas = page.locator("[data-canvas]").first();
    const cycleRow = page.locator("[data-cycle] [aria-selected=true]");

    // Global Recent Panels is separate and unbound by default.
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="shortcuts"]').click();
    await page.locator('[data-shortcut-record="recent_panel"]').click();
    await page.keyboard.press("Alt+KeyG");
    await expect(page.locator('[data-shortcut-effective="recent_panel"]')).toHaveText(label({ code: "KeyG", alt: true }));
    await page.keyboard.press("Escape");

    // plan.txt pinned in the fixture Workspace's View area, the keyboard in it.
    await showExplorer(page);
    await page.locator(`[data-explorer-row="${path.join(root, "plan.txt")}"]`).dblclick();
    const editor = page.locator("[data-view-area-id] [data-editor-body] .cm-content").first();
    await expect(editor).toContainText("plan line");
    await editor.click();
    const displayLocator = page.locator('[data-view-tab-bar] [role="tab"][data-display][aria-selected="true"]').first();
    await expect(displayLocator).toHaveAttribute("data-display", /./);
    const display = await displayLocator.getAttribute("data-display");

    // Then beta's terminal, from the sidebar.
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "beta" }).locator("[data-checkout]").first())).click();
    await expect(canvas).toHaveAttribute("data-canvas", betaTab);

    // ⌥` once, held: the previous surface is the other checkout's display.
    const focusCheckouts = sent.get("focus_checkout") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("KeyG");
    await expect(cycleRow).toHaveAttribute("data-cycle-row", display!);
    await expect(cycleRow).toHaveAttribute("data-cycle-kind", "file");
    await expect(page.locator("[data-cycle=panels]")).toContainText("Recent Panels");
    await expect(page.locator("[data-cycle=panels] [role=option]").first()).toContainText("beta · Terminal");
    await screenshot(page, "recent-panels");
    expect(sent.get("focus_checkout") ?? 0).toBe(focusCheckouts);

    // Releasing ⌥ is one focus_checkout naming the display; the keyboard lands in it.
    await page.keyboard.up("Alt");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    await expect.poll(() => sent.get("focus_checkout") ?? 0).toBe(focusCheckouts + 1);
    expect(last.get("focus_checkout")).toMatchObject({ display_id: display });
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest("[data-view-area]") !== null)).toBe(true);
    await expect(page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]').first()).toHaveAttribute("data-display", display!);

    // ⌥` again goes straight back to beta's tab: one focus_tab across checkouts.
    const focusTabs = sent.get("focus_tab") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("KeyG");
    await expect(cycleRow).toHaveAttribute("data-cycle-row", betaTab);
    await page.keyboard.up("Alt");
    await expect(canvas).toHaveAttribute("data-canvas", betaTab);
    await expect.poll(() => sent.get("focus_tab") ?? 0).toBe(focusTabs + 1);

    // ⌥Tab (Ctrl+Shift+` off macOS): Recent Projects puts fixture first, on the display it was left on.
    const beforeProjects = sent.get("focus_checkout") ?? 0;
    const projects = held("recent_project");
    for (const key of projects.modifiers) await page.keyboard.down(key);
    await page.keyboard.press(projects.key);
    await expect(page.locator("[data-cycle=projects]")).toContainText("Recent Projects");
    await expect(cycleRow).toContainText("plan.txt");
    await screenshot(page, "recent-projects");
    for (const key of projects.modifiers.toReversed()) await page.keyboard.up(key);
    await expect.poll(() => sent.get("focus_checkout") ?? 0).toBe(beforeProjects + 1);
    expect(last.get("focus_checkout")).toMatchObject({ display_id: display });
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest("[data-view-area]") !== null)).toBe(true);

    // Escape while held keeps the original selection and commits nothing.
    // Meanwhile every row wears its marks: an agent's status mark alone (its
    // product name is in the row's label), the neutral mark for a tab with no
    // single agent, a file's own mark.
    const quiet = [sent.get("focus_checkout") ?? 0, sent.get("focus_tab") ?? 0];
    await page.keyboard.down("Alt");
    await page.keyboard.press("KeyG");
    await expect(page.locator("[data-cycle=panels]")).toBeVisible();
    const marks = (row: string) => page.locator(`[data-cycle=panels] [data-cycle-row="${row}"] [data-cycle-marks]`);
    await expect(marks(betaTab)).toHaveAttribute("data-cycle-marks", "status");
    await expect(marks(betaTab).locator("[data-cycle-status]")).toHaveCount(1);
    await expect(marks(betaTab).locator("[data-agent-mark]")).toHaveCount(0);
    await expect(page.locator(`[data-cycle=panels] [data-cycle-row="${betaTab}"]`)).toHaveAttribute("aria-label", /Codex agent/);
    await expect(page.locator('[data-cycle=panels] [data-agent-mark]:not([data-agent-mark="neutral"])')).toHaveCount(0);
    await expect(marks(herdr.tab)).toHaveAttribute("data-cycle-marks", "herdr");
    await expect(marks(herdr.tab).locator('[data-agent-mark="neutral"]')).toHaveCount(1);
    await expect(marks(herdr.tab).locator("[data-cycle-status]")).toHaveCount(0);
    await expect(marks(display!).locator('[data-view-mark="file"]')).toHaveCount(1);
    await screenshot(page, "recent-panels-marks");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    await page.keyboard.up("Alt");
    expect([sent.get("focus_checkout") ?? 0, sent.get("focus_tab") ?? 0]).toEqual(quiet);

    // The Agent area's ⌥` crosses projects: the keyboard in beta's Codex,
    // then in the pane the fixture opens on, and the first candidate is
    // beta's pane, which one focus_pane brings back with its project.
    const betaPane = beta.result.root_pane.pane_id;
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "beta" }).locator("[data-checkout]").first())).click();
    await expect(canvas).toHaveAttribute("data-canvas", betaTab);
    await page.locator(`[data-pane-view="${betaPane}"]`).click();
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "fixture" }).locator("[data-checkout]").first())).click();
    await expect(canvas).not.toHaveAttribute("data-canvas", betaTab);
    const fixturePane = (await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"))!;
    // The fixture's View panel stays open over the agents' right side.
    await page.locator(`[data-pane-view="${fixturePane}"]`).click({ position: { x: 30, y: 60 } });
    const paneFocuses = sent.get("focus_pane") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    const agents = page.locator("[data-cycle=agents]");
    await expect(agents).toContainText("Recent Agent panes");
    await expect(agents.locator("[role=option]").first()).toHaveAttribute("data-cycle-row", fixturePane);
    await expect(cycleRow).toHaveAttribute("data-cycle-row", betaPane);
    await expect(cycleRow).toContainText("beta · Terminal");
    await expect(cycleRow).toHaveAttribute("aria-label", /Codex agent/);
    // Only panes: no View display, Overview or project rows.
    await expect(agents.locator('[role=option]:not([data-cycle-kind="herdr"])')).toHaveCount(0);
    await screenshot(page, "recent-agent-panes");
    expect(sent.get("focus_pane") ?? 0).toBe(paneFocuses);
    await page.keyboard.up("Alt");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBe(paneFocuses + 1);
    expect(last.get("focus_pane")).toMatchObject({ pane_id: betaPane });
    await expect(canvas).toHaveAttribute("data-canvas", betaTab);

    // A sidebar control owns the keyboard: recent agents still open, starting
    // at the most recent pane, without marking an underlying pane as visited.
    const sidebarOverview = page.locator('[data-sidebar-overview]');
    await sidebarOverview.focus();
    await expect(sidebarOverview).toBeFocused();
    let focuses = sent.get("focus_pane") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    await expect(cycleRow).toHaveAttribute("data-cycle-row", betaPane);
    await page.keyboard.press("Backquote");
    await expect(cycleRow).toHaveAttribute("data-cycle-row", fixturePane);
    expect(sent.get("focus_pane") ?? 0).toBe(focuses);
    await page.keyboard.up("Alt");
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBe(focuses + 1);
    await expect(canvas).not.toHaveAttribute("data-canvas", betaTab);

    // The Explorer also opens the agent list; Escape cancels without a move.
    await showExplorer(page);
    const file = page.locator(`[data-explorer-row="${path.join(root, "plan.txt")}"]`);
    const explorer = page.locator('[data-lane="explorer"] [role="tree"]');
    await explorer.focus();
    await expect(explorer).toBeFocused();
    focuses = sent.get("focus_pane") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    await expect(agents).toBeVisible();
    await expect(cycleRow).toHaveAttribute("data-cycle-row", fixturePane);
    await screenshot(page, "recent-agents-from-explorer");
    await page.keyboard.press("Escape");
    await page.keyboard.up("Alt");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    expect(sent.get("focus_pane") ?? 0).toBe(focuses);

    // A focused file keeps its View scope. One file is a no-op even when
    // recent agents exist; another file enables only that area's View cycle.
    await file.dblclick();
    await editor.click();
    await page.keyboard.press("Alt+Backquote");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    expect(sent.get("focus_pane") ?? 0).toBe(focuses);
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.txt"), "notes line\n");
    await page.locator(`[data-explorer-row="${path.join(root, "notes.txt")}"]`).dblclick();
    await expect(editor).toContainText("notes line");
    await editor.click();
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    await expect(page.locator("[data-cycle=area]")).toContainText("Recent View tabs");
    await expect(page.locator("[data-cycle=agents]")).toHaveCount(0);
    await page.keyboard.up("Alt");
    await expect(editor).toContainText("plan line");
    expect(sent.get("focus_pane") ?? 0).toBe(focuses);

    // Overview has no drawn Agent area; reverse cycling still selects an
    // agent and returns to its Workspace with one event on release.
    await page.keyboard.press(chord("overview"));
    await expect(page.locator("[data-overview-modal]")).toBeVisible();
    await page.keyboard.down("Alt");
    await page.keyboard.press("Shift+Backquote");
    await expect(agents).toBeVisible();
    const chosenPane = await cycleRow.getAttribute("data-cycle-row");
    expect(sent.get("focus_pane") ?? 0).toBe(focuses);
    await page.keyboard.up("Alt");
    await expect.poll(() => sent.get("focus_pane") ?? 0).toBe(focuses + 1);
    expect(last.get("focus_pane")).toMatchObject({ pane_id: chosenPane });
    await expect(page.locator(`[data-pane-view="${chosenPane}"] .xterm-helper-textarea`)).toBeFocused();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
