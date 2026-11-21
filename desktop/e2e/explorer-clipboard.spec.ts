// The Explorer's clipboard in the desktop host (docs/UI_BEHAVIOR.md, Explorer):
// the native Edit menu owns ⌘C and ⌘V as roles, so the tree has to answer the
// clipboard events those roles raise as well as its own keydown. Both routes
// copy a file into its folder under a `copy` name, and a file dropped from
// the file manager lands in a folder. Everything runs on a private engine,
// hided and Electron profile (see `fixture.ts`).

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { enterWorkspace, showExplorer } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { fitWindow, hostLog, launchShell, screenshot, startStack, test, type Stack } from "./fixture";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

let stack: Stack;
let app: ElectronApplication | null = null;

test.beforeEach(async () => {
  stack = await startStack("explorer-clipboard", { agents: false });
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(stack.run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  stack.stop();
});

// @platform: ⌘C and ⌘V reach the page through the system's own Edit menu roles.
test("clipboard: the native copy and paste roles copy a file under a copy name, and a dropped file lands in a folder", { tag: "@platform" }, async () => {
  const checkout = path.join(fs.realpathSync(stack.fixture.root), "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "notes.md"), "# notes\n");
  fs.writeFileSync(path.join(checkout, "src", "a.txt"), "a\n");

  const launched = await launchShell(stack.run.env);
  app = launched.app;
  const page = launched.page;
  await fitWindow(app, { width: 1024, height: 681 });
  await enterWorkspace(page, "fixture");
  await showExplorer(page);
  const row = (name: string) => page.locator(`[data-explorer-row="${toPage(path.join(checkout, name))}"]`);

  // The native roles: what the Edit menu's Copy and Paste do to the focused page.
  await row("notes.md").click();
  await expect(page.locator('[data-tab-kind="file"]').filter({ hasText: "notes.md" })).toBeVisible();
  await page.locator("[data-explorer-tree]").focus();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.copy());
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.paste());
  await expect(row("notes copy.md")).toBeVisible({ timeout: 20_000 });
  expect(fs.readFileSync(path.join(checkout, "notes copy.md"), "utf8")).toBe("# notes\n");
  await screenshot(page, "explorer-clipboard-pasted");

  // A real key press, through the window's own handler.
  await expect(row("notes copy.md")).toHaveAttribute("data-selected", "true");
  await page.keyboard.press("ControlOrMeta+v");
  await expect(row("notes copy 2.md")).toBeVisible({ timeout: 20_000 });

  // A file dropped from the file manager into the folder under the pointer.
  await page.evaluate((target) => {
    const data = new DataTransfer();
    data.items.add(new File(["from the file manager"], "dropped.txt"));
    const element = document.querySelector(target)!;
    element.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
    element.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
  }, `[data-explorer-row="${toPage(path.join(checkout, "src"))}"]`);
  await expect.poll(() => fs.existsSync(path.join(checkout, "src", "dropped.txt")), { timeout: 20_000 }).toBe(true);
  expect(fs.readFileSync(path.join(checkout, "src", "dropped.txt"), "utf8")).toBe("from the file manager");
  await screenshot(page, "explorer-clipboard-dropped");
});
