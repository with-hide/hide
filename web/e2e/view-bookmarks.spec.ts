import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, setLanePinned, showExplorer } from "./wire";

// CLI background Views: the checkout owns one list, and each Agent tab only
// leaves its selection alone. An agent in any
// pane but the one the operator is in, beside it in the same tab or in another
// tab, adds its View to the list and moves nothing.

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

/** Runs `hide <args>` in the shell of `pane`, as an agent in that tab would. */
async function hideFrom(herdr: HerdrFixture, daemon: Daemon, pane: string, args: string[], sequence: number): Promise<void> {
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  const ran = await runInPane(herdr, pane, `cli-${sequence}`, { env: { HIDE_STATE_DIR: daemon.stateDir }, argv: [hide, ...args] });
  expect(ran.status, ran.stderr).toBe(0);
}

const agentTab = (page: Page, id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
const viewTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

/**
 * The Agent tab each saved Workspace has in front. A restart kills the
 * daemon outright on Windows, so the state it restores has to be on disk
 * before it goes, as `savedFront` in `s6.spec.ts` waits for.
 */
function savedAgentTabs(daemon: Daemon): string[] {
  type Node = { area?: { id: string; active?: string }; split?: { first: Node; second: Node } };
  const areaIn = (node: Node, id: string): { id: string; active?: string } | undefined =>
    node.area ? (node.area.id === id ? node.area : undefined) : node.split && (areaIn(node.split.first, id) ?? areaIn(node.split.second, id));
  const tabs: string[] = [];
  const walk = (node: unknown): void => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) return node.forEach(walk);
    const layout = (node as { agent_layout?: { root: Node; active_area: string } }).agent_layout;
    const active = layout && areaIn(layout.root, layout.active_area)?.active;
    if (active) tabs.push(active);
    Object.values(node).forEach(walk);
  };
  walk(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "workspace-views.json"), "utf8")));
  return tabs;
}

test("Agent switching preserves the selected View and background opens stay behind it", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(herdr.root, "fixture");
    for (const name of ["a.md", "b.md", "c.md", "d.md"]) fs.writeFileSync(path.join(fixture, name), `${name}\n`);
    const created = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", fixture, "--label", "second", "--no-focus"]) as {
      result: { tab: { tab_id: string }; root_pane: { pane_id: string } };
    };
    const [first, second] = [herdr.tab, created.result.tab.tab_id];
    const [firstPane, secondPane] = [herdr.panes[0], created.result.root_pane.pane_id];
    daemon = await startHided(herdr, "view-bookmarks");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(agentTab(page, second)).toBeVisible();
    // The lane pinned open on the Explorer, so the agents' opens behind the
    // operator are seen through the tab clicks below (PRD right-lane B5).
    await showExplorer(page);
    await setLanePinned(page, true);
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked");

    // Tab 1 is in front and opens a.md.
    await hideFrom(herdr, daemon, firstPane, ["file", "open", "a.md"], 1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // Tab 2 is shown for the first time and the
    // checkout's list stays as it is (PRD lane-files-checkout-views B1, B3);
    // its own agent opens b.md, which comes to the front.
    await agentTab(page, second).click();
    await expect(agentTab(page, second)).toHaveAttribute("aria-selected", "true");
    await expect(viewTabs(page)).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await hideFrom(herdr, daemon, secondPane, ["file", "open", "b.md"], 2);
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(viewTabs(page)).toHaveCount(2);

    // B1, B2: back and forth, one tab click each, and the page sends no
    // View action of its own after the switch.
    const viewActions = sent.get("view_layout") ?? 0;
    await agentTab(page, first).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await agentTab(page, second).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(viewTabs(page)).toHaveCount(2);
    expect(sent.get("view_layout") ?? 0).toBe(viewActions);
    await screenshot(page, "view-bookmarks-tab-2-front");
    await agentTab(page, first).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await screenshot(page, "view-bookmarks-tab-1-front");

    // B8: the operator's own tab split in two. The agent in the pane beside
    // the operator's opens c.md: it joins the strip behind a.md and moves
    // nothing, though both panes are on screen in the tab in front.
    const split = herdr.run(["pane", "split", firstPane, "--direction", "right", "--cwd", fixture, "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as { result: { pane: { pane_id: string } } };
    await hideFrom(herdr, daemon, split.result.pane.pane_id, ["file", "open", "c.md"], 3);
    await expect(viewTabs(page)).toHaveCount(3);
    await expect(viewTabs(page).filter({ hasText: "c.md" })).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(agentTab(page, first)).toHaveAttribute("aria-selected", "true");
    await screenshot(page, "view-bookmarks-sibling-open-behind");

    // B7: while the operator is on tab 2, tab 1's agent opens d.md. It joins
    // the strip behind the operator's b.md and moves nothing, and tab 1 keeps
    // what the operator had in front there.
    await agentTab(page, second).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await hideFrom(herdr, daemon, firstPane, ["file", "open", "d.md"], 4);
    await expect(viewTabs(page)).toHaveCount(4);
    await expect(viewTabs(page).filter({ hasText: "d.md" })).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(agentTab(page, second)).toHaveAttribute("aria-selected", "true");
    await screenshot(page, "view-bookmarks-agent-open-behind");
    await agentTab(page, first).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(viewTabs(page)).toHaveCount(4);
    await agentTab(page, second).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);

    // The list and its selection survive a daemon restart.
    await expect.poll(() => fs.readFileSync(path.join(daemon!.stateDir, "workspace-views.json"), "utf8")).toContain("view_layout");
    await expect.poll(() => savedAgentTabs(daemon!)).toContain(second);
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await expect(agentTab(page, second)).toHaveAttribute("aria-selected", "true");
    await agentTab(page, first).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await agentTab(page, second).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
