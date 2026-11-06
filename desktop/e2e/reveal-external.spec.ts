// `reveal_external` end to end (issue 324, docs/UI_BEHAVIOR.md): the
// Explorer's file and folder rows, a History row, a View tab and a sidebar row
// each offer the OS file manager's reveal under the host's OS label, and
// choosing it hands that file or folder to `shell.showItemInFolder`. A deleted
// file's History row lists it disabled with the reason. The main process
// refuses a path that is not absolute or no longer exists and logs the
// outcome without the path. Everything runs on a private engine, hided
// and Electron profile (see `fixture.ts`); macOS's handler is replaced with a
// recorder, so Finder never comes forward.

import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "../../web/e2e/engine-fixture";
import { enterWorkspace, showExplorer, showTool } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { revealMenuLabel } from "../../web/e2e/platform-fixture";
import { afterCleanup } from "../../web/e2e/worker-owned";
import { connectHome, fitWindow, hostLog, isolate, launchShell, screenshot, test, type Isolated } from "./fixture";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

let fixture: AgentFixture;
let run: Isolated;
let checkout: string;
let app: ElectronApplication | null = null;

/** The checkout's files and its Git history, with a changed and a deleted file, as the History list shows them. */
function seedCheckout(folder: string): void {
  fs.mkdirSync(path.join(folder, "src"), { recursive: true });
  fs.writeFileSync(path.join(folder, "src", "a.txt"), "a\n");
  fs.writeFileSync(path.join(folder, "notes.md"), "# notes\n");
  fs.writeFileSync(path.join(folder, "gone.txt"), "gone\n");
  const git = (...args: string[]) => execFileSync("git", args, { cwd: folder });
  git("init", "-q", "-b", "main");
  git("add", "-A");
  git("-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "base");
  fs.appendFileSync(path.join(folder, "src", "a.txt"), "changed\n");
  fs.rmSync(path.join(folder, "gone.txt"));
}

// The checkout is a repository before the home's daemon registers it, so this composes `startStack`'s
// steps: the fixture, the checkout's files, the home, its daemon, then the fixture's panes.
test.beforeEach(async () => {
  fixture = await startFixture({ agents: false });
  let made: Isolated | undefined;
  try {
    checkout = path.join(fs.realpathSync(fixture.root), "fixture");
    seedCheckout(checkout);
    made = isolate(fixture, "reveal-external");
    await fixture.opened!(await connectHome(made));
    run = made;
  } catch (error) {
    throw afterCleanup(error, () => {
      try { made?.cleanup(); } finally { fixture.stop(); }
    });
  }
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  try { run.cleanup(); } finally { fixture.stop(); }
});

/** Replaces the OS file manager's handler in the main process with a recorder. */
async function recordReveals(): Promise<void> {
  await app!.evaluate(({ shell }) => {
    const revealed: string[] = [];
    (globalThis as { revealed?: unknown }).revealed = revealed;
    shell.showItemInFolder = (target: string) => {
      revealed.push(target);
    };
  });
}

function revealed(): Promise<string[]> {
  return app!.evaluate(() => (globalThis as { revealed?: string[] }).revealed ?? []);
}

/** Each item of an open menu as drawn: a separator, or its label with "(disabled)" and its reason. */
async function menuLines(menu: Locator): Promise<string[]> {
  return menu.evaluate((element) =>
    Array.from(element.children).map((child) => {
      if (child.getAttribute("role") === "separator") return "─";
      const label = child.querySelector("[data-menu-label]")?.textContent ?? "";
      const reason = child.querySelector("[data-menu-reason]")?.textContent;
      return [label, child.hasAttribute("data-disabled") ? "(disabled)" : null, reason].filter(Boolean).join(" ");
    }),
  );
}

async function openMenu(page: Page, target: Locator, name: string): Promise<Locator> {
  await target.click({ button: "right" });
  const menu = page.getByRole("menu", { name });
  await expect(menu).toBeVisible();
  return menu;
}

// @platform: The reveal is labelled and handed over by the system's file manager: Finder, File Explorer or the Linux file manager.
test("reveal: Explorer, History, a View tab and a sidebar row hand the item to the OS file manager under the OS's label", { tag: "@platform" }, async () => {
  const label = revealMenuLabel;

  const launched = await launchShell(run.env);
  app = launched.app;
  await fitWindow(app, { width: 1024, height: 681 });
  const page = launched.page;
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.7));
  await enterWorkspace(page, "fixture");
  await recordReveals();

  // Explorer: a file row's opens, the reveal, Cut, Copy and Duplicate, the path copies, Rename, then Move to Trash.
  await showExplorer(page);
  const notes = path.join(checkout, "notes.md");
  let menu = await openMenu(page, page.locator(`[data-explorer-row="${toPage(notes)}"]`), "notes.md actions");
  expect(await menuLines(menu)).toEqual(["Open to the side", "─", label, "─", "Cut", "Copy", "Duplicate", "─", "Copy Path", "Copy Relative Path", "─", "Rename", "─", "Move to Trash"]);
  await screenshot(page, "reveal-explorer-file-menu");
  await menu.locator('[data-menu-item="reveal_external"]').click();
  await expect.poll(revealed).toEqual([notes]);

  // A folder row: its creations, the reveal, Cut, Copy and Duplicate, the path copies, Rename, Move to Trash.
  const src = path.join(checkout, "src");
  menu = await openMenu(page, page.locator(`[data-explorer-row="${toPage(src)}"]`), "src actions");
  expect(await menuLines(menu)).toEqual(["New File", "New Folder", "─", label, "─", "Cut", "Copy", "Duplicate", "─", "Copy Path", "Copy Relative Path", "─", "Rename", "─", "Move to Trash"]);
  await menu.locator('[data-menu-item="reveal_external"]').click();
  await expect.poll(revealed).toEqual([notes, src]);

  // A View tab: Copy path, Select in File Tree, the reveal, Close view.
  await page.locator(`[data-explorer-row="${toPage(notes)}"]`).dblclick();
  const tab = page.locator('[data-view-tab-bar] [role="tab"][data-display][aria-label*="/notes.md"]');
  await expect(tab).toBeVisible({ timeout: 20_000 });
  await tab.click({ button: "right" });
  menu = page.locator('[role="menu"]').filter({ has: page.locator('[data-menu-item="close_view"]') });
  await expect(menu).toBeVisible();
  expect((await menuLines(menu)).slice(-6)).toEqual(["─", "Copy path", "Select in File Tree", label, "─", "Close view"]);
  expect(await menu.locator("[data-menu-item]").evaluateAll((items) => items.map((item) => item.getAttribute("data-menu-item")).slice(-4))).toEqual([
    "copy_path",
    "select_in_tree",
    "reveal_external",
    "close_view",
  ]);
  await screenshot(page, "reveal-view-tab-menu");
  await menu.locator('[data-menu-item="reveal_external"]').click();
  await expect.poll(revealed).toEqual([notes, src, notes]);

  // History: Open to the side, then the reveal; a deleted file has nothing to show.
  await showTool(page, "changes");
  const changed = page.locator('[data-history-group="working"][data-history-path="src/a.txt"]');
  await expect(changed).toBeVisible({ timeout: 30_000 });
  menu = await openMenu(page, changed, "a.txt actions");
  expect((await menuLines(menu)).map((line) => line.replace(/ \(disabled\).*/, ""))).toEqual(["Open to the side", "─", label]);
  await menu.locator('[data-menu-item="reveal_external"]').click();
  await expect.poll(revealed).toEqual([notes, src, notes, path.join(checkout, "src", "a.txt")]);
  menu = await openMenu(page, page.locator('[data-history-group="working"][data-history-path="gone.txt"]'), "gone.txt actions");
  await expect(menu.locator('[data-menu-item="reveal_external"]')).toBeDisabled();
  await expect(menu.locator('[data-menu-item="reveal_external"]')).toContainText("The file was deleted.");
  await screenshot(page, "reveal-history-deleted-menu");
  await page.keyboard.press("Escape");

  // The sidebar's project row: the same item, which reveals the project's folder.
  const projectRow = page.locator("[data-project-menu]").filter({ has: page.getByRole("button", { name: /^fixture(,|$)/ }) }).first();
  await expect(projectRow).toBeVisible({ timeout: 20_000 });
  await projectRow.click({ button: "right" });
  menu = page.locator('[role="menu"]').filter({ has: page.locator('[data-menu-item="reveal_external"]') });
  await expect(menu.locator('[data-menu-item="reveal_external"]')).toContainText(label);
  await menu.locator('[data-menu-item="reveal_external"]').click();
  await expect.poll(revealed).toEqual([notes, src, notes, path.join(checkout, "src", "a.txt"), checkout]);

  // The host refuses what it cannot show and logs the outcome, never the path.
  await page.evaluate(([missing]) => {
    window.hideHost!.revealPath("notes.md");
    window.hideHost!.revealPath(missing!);
  }, [toPage(path.join(checkout, "missing.md"))]);
  await expect
    .poll(() => hostLog(run.env).filter((line) => line.event === "reveal.refused").map((line) => line.reason))
    .toEqual(["path", "missing"]);
  expect(await revealed()).toHaveLength(5);
  const lines = hostLog(run.env).filter((line) => line.event.startsWith("reveal."));
  expect(lines.filter((line) => line.event === "reveal.shown").map((line) => line.kind)).toEqual(["file", "directory", "file", "file", "directory"]);
  // The log is JSON, which writes a backslash twice, so the folder is searched for as the log would spell it.
  for (const spelling of [checkout, toPage(checkout)]) expect(JSON.stringify(lines)).not.toContain(JSON.stringify(spelling).slice(1, -1));
});
