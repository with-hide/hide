// This Mac's Home (PRD home-device-rail B16-B18, B21) against a private
// engine and hided: the sidebar has no Home row (sidebar-titlebar-refresh B11),
// so Home is reached from the start panel, whose start aimed at Home makes
// `~/hide` in the daemon's HOME with a link per registered project and hide's
// guide files, and runs the agent there; a `~/hide` that is not Hide's is left
// alone, and the reason shows inside the start panel.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import type { Rest } from "./hided-control.mjs";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 120_000 });

async function pinFixture(page: Page) {
  // The fixture is a plain folder, which the sidebar draws as one row.
  const row = page.locator("nav[data-sidebar]").getByRole("button", { name: /^fixture(,|$)/ });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Pin", exact: true }).click();
  await expect(page.locator('[data-section="pinned"]')).toBeVisible({ timeout: 20_000 });
}

/** ⌘K's Start an agent… with Home chosen as the target, the start panel left open. */
async function aimAtHome(page: Page, node: string) {
  await page.keyboard.press(chord("search"));
  await expect(page.locator('[data-palette="Search"] [data-palette-input]')).toBeFocused();
  await page.keyboard.type("Start an agent");
  await page.locator('[data-palette-row="command:start-agent"]').click();
  const panel = page.locator("[data-start-panel]");
  await panel.locator("[data-start-target]").click();
  await page.locator(`[data-start-target-option="home:${node}"]`).click();
  await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", `home:${node}`);
}

/**
 * Written only when the test fails, so the next failure says where the daemon
 * put Home: what it publishes for each workspace (is it Home, its checkouts'
 * tabs) and for the tab shown (its panes and the folder each is in), next to
 * what the page draws. Each read stands alone, so one that fails is
 * recorded instead of hiding the rest.
 */
async function recordFailure(page: Page, daemon: Daemon | null) {
  const read = async <T,>(what: () => Promise<T>): Promise<T | { error: string }> => {
    try {
      return await what();
    } catch (error) {
      return { error: String(error) };
    }
  };
  const published = daemon
    ? await read(() => withControl(daemon, (control) => control.until((rest) => {
      const navigator = rest.navigator as { workspaces?: { id: string; path: string; is_home?: boolean; checkouts: { path?: string; tabs?: { id: string }[] }[] }[] } | undefined;
      const shown = rest.tab as { id?: string; panes?: { id: string; cwd: string }[] } | null;
      return { workspaces: navigator?.workspaces?.map(({ id, path: folder, is_home, checkouts }) => ({ id, folder, is_home, checkouts: checkouts.map((checkout) => ({ path: checkout.path, tabs: checkout.tabs?.map((tab) => tab.id) })) })), shown: shown ? { id: shown.id, panes: shown.panes?.map(({ id, cwd }) => ({ id, cwd })) } : null };
    }, "the published workspaces", 5_000)))
    : null;
  const diagnostics = {
    daemonHome: daemon?.home ?? null,
    hide: {
      location: await page.locator('nav[aria-label="Location"]').textContent({ timeout: 1_000 }).catch((error: unknown) => `unreadable: ${String(error)}`),
      project: await page.locator("[data-go-overview]").getAttribute("data-go-overview", { timeout: 1_000 }).catch(() => null),
      kind: await page.locator("[data-workspace-location]").getAttribute("data-workspace-location", { timeout: 1_000 }).catch(() => null),
    },
    published,
  };
  const file = test.info().outputPath("home-diagnostics.json");
  fs.writeFileSync(file, JSON.stringify(diagnostics, null, 2));
  await test.info().attach("home-diagnostics", { path: file, contentType: "application/json" });
}

/** Whether the tab the daemon shows holds a pane in `folder` (a wire spelling) or below it. */
function paneAt(rest: Rest, folder: string): true | undefined {
  const shown = rest.tab as { panes?: { cwd?: string | null }[] } | null;
  return shown?.panes?.some((pane) => pane.cwd?.startsWith(folder)) ? true : undefined;
}

/** A start aimed at Home with a pinned project: `~/hide` is made and the agent runs in it. */
async function inOpenedHome(page: Page, check: (home: string, root: string) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startFixture();
  let daemon: Daemon | null = null;
  try {
    // The fixture's folders are in the daemon's home, as an operator's projects
    // are, so a pin registers them and a project outliving its last pane stays
    // listed without asking about a folder outside home (PRD wsl-device B15).
    daemon = await startHided(herdr, "home", herdr.root);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await pinFixture(page);
    const home = path.join(daemon.home, "hide");
    // B18: no Home until it is first used.
    expect(fs.existsSync(home)).toBe(false);

    await aimAtHome(page, daemon.node);
    await page.locator("[data-start-submit]").click();
    // B17: the agent opens in Home once Home's sync answered, so everything the
    // sync writes is on disk by the time the engine has a pane there. The core
    // spells a pane's folder as the wire does, so Home's folder is read the same way.
    await expect.poll(() => fs.existsSync(home), { timeout: 30_000 }).toBe(true);
    const folder = toPage(fs.realpathSync(home));
    await withControl(daemon, (control) => control.until((rest) => paneAt(rest, folder), `a pane in ${folder}`, 30_000));
    await check(home, herdr.root);
  } catch (error) {
    await recordFailure(page, daemon);
    throw error;
  } finally {
    try { daemon?.stop(); } finally { herdr.stop(); }
  }
}

test("a start aimed at Home makes ~/hide with its guide files and runs the agent there", async ({ page }) => {
  await inOpenedHome(page, async (home) => {
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    for (const file of [".hide-home.json", "AGENTS.md", "CLAUDE.md"]) expect(fs.existsSync(path.join(home, file))).toBe(true);

    // The Home is still no project (B16), and the sidebar draws no row for it (B11).
    await expect(page.locator("nav[data-sidebar]").getByRole("button", { name: /^(hide|Home)$/ })).toHaveCount(0);
    // The top row's path names Home's folder, never Home as a project.
    await expect(page.locator('nav[aria-label="Location"]')).toHaveText("~/hide");
    await screenshot(page, "home-tab-opened");
  });
});

// @platform: The link Home makes to a project: a symbolic link on macOS and Linux, a junction on Windows.
test("Home's ~/hide holds a link to each registered project", { tag: "@platform" }, async ({ page }) => {
  await inOpenedHome(page, async (home, root) => {
    const link = path.join(home, "fixture");
    expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
    expect(fs.realpathSync(link)).toBe(fs.realpathSync(path.join(root, "fixture")));
  });
});

test("a ~/hide that is not Hide's is left alone, and the reason shows in the start panel", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "home-conflict");
    const home = path.join(daemon.home, "hide");
    fs.mkdirSync(home);
    fs.writeFileSync(path.join(home, "mine.txt"), "the operator's own folder\n");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B21: a start aimed at Home is refused inside the panel, keeps the text, and leaves the folder untouched.
    await aimAtHome(page, daemon.node);
    const panel = page.locator("[data-start-panel]");
    await panel.locator("[data-start-text]").fill("홈에서 할 일");
    await panel.locator("[data-start-submit]").click();
    await expect(panel.locator("[data-start-failure]")).toContainText("~/hide", { timeout: 20_000 });
    await expect(panel.locator("[data-start-text]")).toHaveValue("홈에서 할 일");
    expect(fs.readdirSync(home)).toEqual(["mine.txt"]);
    await screenshot(page, "home-conflict-panel");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
