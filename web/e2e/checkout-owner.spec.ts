import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { tabsOf, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { waitForTerminal } from "./group-start-helper";
import type { Rest } from "./hided-control.mjs";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, openOtherCheckouts, screenshot } from "./wire";
import { chord } from "./chords";

// PRD checkout-workspace-binding B1, B2, B13: a tab Hide creates goes to the
// checkout's owner workspace, never to the workspace its other tabs sit
// in. Two quick terminal requests each
// reuse one owner. These sequential starts do not exercise concurrent owner creation.
// The tests say what the operator sees of the new tabs.

/** The tabs `dir`'s checkout holds in the navigator, whichever workspace row lists it. */
function checkoutTabs(rest: Rest, dir: string): string[] | undefined {
  type Listed = { path?: string; agent_groups?: { groups: { tabs: string[] }[] } };
  const workspaces = ((rest.navigator as { workspaces?: { checkouts: Listed[] }[] } | undefined)?.workspaces ?? []);
  const wanted = toPage(fs.realpathSync(dir));
  const checkout = workspaces.flatMap((workspace) => workspace.checkouts).find((listed) => listed.path === wanted);
  return checkout?.agent_groups?.groups.flatMap((group) => group.tabs);
}

/** Two quick new-tab chords from the checkout's terminal; the strip draws both new tabs and the later one is in front. */
async function twoQuickTabs(page: Page, daemon: Daemon, tabsOfCheckout: (rest: Rest) => string[] | undefined): Promise<string[]> {
  const before = await withControl(daemon, (control) => control.until(tabsOfCheckout, "the checkout's tabs"));
  await page.locator("[data-terminal-host]").first().click();
  await page.keyboard.press(chord("new_tab"));
  await page.keyboard.press(chord("new_tab"));
  const made = await withControl(daemon, (control) => control.until((rest) => {
    const now = tabsOfCheckout(rest) ?? [];
    return now.length === before.length + 2 ? now.filter((tab) => !before.includes(tab)) : undefined;
  }, "two new tabs in the checkout", 30_000));
  await expect(page.locator('[data-agent-tab-bar] [role="tab"]')).toHaveCount(before.length + 2);
  await waitForTerminal(page, made.at(-1)!);
  return made;
}

test("Two quick new tabs in a plain folder each open a terminal in the checkout", async ({ page }) => {
  test.setTimeout(120_000);
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "checkout-owner-quick");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator('[data-agent-tab-bar] [role="tab"]')).toHaveCount(1);
    await twoQuickTabs(page, daemon, (rest) => tabsOf(rest, fixture.workspace));
    // B6: no workspace label names a tab.
    await expect(page.locator('[data-agent-tab-bar] [role="tab"]').filter({ hasText: "home-graph" })).toHaveCount(0);
    await screenshot(page, "checkout-owner-quick");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

// @platform: A linked worktree's path: macOS reaches /tmp through a symlink and ignores case.
test("A new tab in a linked worktree opens in that worktree's checkout and the next tab joins it", { tag: "@platform" }, async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1400, height: 900 });
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(fixture.root, "fixture");
    const linked = path.join(fixture.root, "fixture-feature");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args]);
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "Initial files");
    git("worktree", "add", "-q", "-b", "feature", linked);

    daemon = await startHided(fixture, "checkout-owner-linked-tabs");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^fixture/ }) });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="feature"]') });
    // Session UI B9: a clean worktree with no agent starts in the project's Checkouts fold, with every other checkout.
    await openOtherCheckouts(project);
    await expect(feature).toBeVisible({ timeout: 30_000 });
    const newTabHere = async () => {
      await feature.locator("[data-checkout-menu]").click({ button: "right" });
      const menu = page.getByRole("menu", { name: "feature actions" });
      await menu.locator('[data-menu-item="new_tab_here"]').click();
    };
    const tabsHere = (rest: Rest) => checkoutTabs(rest, linked);

    await newTabHere();
    const first = await withControl(daemon, (control) => control.until((rest) => tabsHere(rest)?.[0], "the worktree's first tab", 30_000));
    await waitForTerminal(page, first);
    // Opening the terminal puts the checkout in front, still in its fold.
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");

    await newTabHere();
    const second = await withControl(daemon, (control) => control.until((rest) => (tabsHere(rest)?.length === 2 ? tabsHere(rest)![1] : undefined), "the worktree's second tab", 30_000));
    expect(await withControl(daemon, (control) => control.until(tabsHere, "the worktree's tabs"))).toEqual([first, second]);
    await waitForTerminal(page, second);
    await screenshot(page, "checkout-owner-linked-tabs");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
