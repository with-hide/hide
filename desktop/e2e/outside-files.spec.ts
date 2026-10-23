// Outside files end to end (PRD outside-path-in-hide, docs/UI_BEHAVIOR.md,
// Terminal links): a plain click on a terminal path to a file outside every
// checkout opens it read-only in the front Workspace's View with an External
// badge, by a grant the desktop host holds for this window alone. An HTML
// page loads its own folder and nothing beside it, a reload of the window
// keeps every tab, and closing a tab ends its grant. Everything runs on a
// private Herdr server, hided and Electron profile (see `fixture.ts`); the
// operating system's handlers are replaced with recorders, so nothing opens
// outside the test.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { fitWindow, hostLog, isolate, launch, screenshot, shellPage, test, type Isolated } from "./fixture";
import { hoverLink, pointOf, printInPane, withProbe, type Probe } from "./terminal-probe";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;
/** The folder outside every checkout the files live in, removed however the test ends. */
let scratch: string | null = null;

// A one-pixel PNG.
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");

test.beforeAll(async () => {
  herdr = await startHerdr({ agents: false });
});

test.afterAll(() => herdr?.stop());

test.beforeEach(() => {
  run = isolate(herdr, "outside-files");
  scratch = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-outside-")));
  test.info().attach("scratch", { body: scratch });
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  scratch = null;
});

/** Launches the window on the fixture checkout with the probe, and returns its page, its pane and every frame it sent from the reload on. */
async function start(): Promise<{ page: Page; paneId: string; sent: string[] }> {
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  const sent: string[] = [];
  page.on("websocket", (socket) => socket.on("framesent", (frame) => sent.push(String(frame.payload))));
  await withProbe(page);
  // Zoomed out so the longest temporary path still fits a pane row.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.5));
  await enterWorkspace(page, "fixture");
  await app.evaluate(({ shell }) => {
    const opened: string[] = [];
    (globalThis as { opened?: string[] }).opened = opened;
    shell.openPath = async (target: string) => {
      opened.push(target);
      return "";
    };
    shell.showItemInFolder = (target: string) => {
      opened.push(target);
    };
  });
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  return { page, paneId, sent };
}

async function click(page: Page, paneId: string, text: string): Promise<void> {
  const point = await pointOf(page, paneId, text, { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
}

function outsideTab(page: Page, name: string) {
  return page.locator(`[data-view-tab-bar] [role="tab"][data-tab-kind="external"][aria-label*="${name}"]`);
}

/** The text of the outside HTML page's own WebContents, by the title it set. */
function pageEval<T>(title: string, script: string): Promise<T | null> {
  return app!.evaluate(({ BrowserWindow }, { title, script }) => {
    const view = BrowserWindow.getAllWindows()[0]!.contentView.children
      .find((child) => "webContents" in child && (child as Electron.WebContentsView).webContents.getTitle() === title) as Electron.WebContentsView | undefined;
    return view ? view.webContents.executeJavaScript(script) : null;
  }, { title, script });
}

function opened(): Promise<string[]> {
  return app!.evaluate(() => (globalThis as { opened?: string[] }).opened ?? []);
}

test("an outside image, text file and HTML page open read-only in the View with the External badge", { tag: "@platform" }, async () => {
  const shot = path.join(scratch!, "shot.png");
  fs.writeFileSync(shot, PNG);
  const notes = path.join(scratch!, "notes.txt");
  fs.writeFileSync(notes, ["first", "second", "third line", "fourth"].join("\n"));
  const site = path.join(scratch!, "site");
  fs.mkdirSync(site);
  fs.writeFileSync(path.join(site, "style.css"), "body { background: rgb(1, 2, 3); }");
  fs.writeFileSync(path.join(scratch!, "secret.css"), "body { color: rgb(9, 9, 9); }");
  fs.writeFileSync(path.join(site, "index.html"), '<!doctype html><title>Outside page</title><link rel="stylesheet" href="style.css"><link rel="stylesheet" href="../secret.css"><h1>outside</h1>');
  const { page, paneId, sent } = await start();
  await printInPane(herdr, page, paneId, [`img ${shot}`, `txt ${notes}:3`, `html ${path.join(site, "index.html")}`, "END-OUTSIDE"], "END-OUTSIDE");

  // An image opens in the image viewer, its tab marked External, and the badge's hint is the file's path (B1, B11).
  await click(page, paneId, shot);
  await expect(outsideTab(page, "shot.png")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator('[data-view-tab-bar] [data-external-badge="held"]')).toHaveText("External");
  await expect(page.locator('[data-viewer="image"]')).toBeVisible();
  await page.locator('[data-external-badge="held"]').hover();
  await expect(page.getByRole("tooltip").first()).toContainText(toPage(shot));
  await screenshot(page, "outside-image-badge");

  // A text file opens read-only at the link's line, and typing changes nothing (B2).
  await click(page, paneId, notes);
  await expect(outsideTab(page, "notes.txt")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator("[data-external-text] .cm-activeLine")).toHaveText("third line");
  await page.locator("[data-external-text] .cm-content").click();
  await page.keyboard.type("typed");
  await expect(page.locator("[data-external-text] .cm-content")).not.toContainText("typed");

  // An HTML page loads its folder's stylesheet and not the one beside the folder (B3).
  await click(page, paneId, "site/index.html");
  await expect(outsideTab(page, "index.html")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect.poll(() => pageEval<string>("Outside page", "getComputedStyle(document.body).backgroundColor"), { timeout: 20_000 }).toBe("rgb(1, 2, 3)");
  expect(await pageEval<string>("Outside page", "getComputedStyle(document.body).color")).not.toBe("rgb(9, 9, 9)");
  await expect.poll(() => hostLog(run.env).some((line) => line.event === "browser.request_refused" && line.reason === "local_page_boundary")).toBe(true);
  // A navigation out of the folder is refused and logged (B5).
  await pageEval("Outside page", "location.href = new URL('../secret.css', location.href).href; true");
  await expect.poll(() => hostLog(run.env).some((line) => line.event === "browser.navigation_refused" && line.reason === "external_folder_boundary")).toBe(true);

  // Nothing went to the system, and no path or file name reached hided or the host's log (D-05, B22).
  expect(await opened()).toEqual([]);
  expect(sent.filter((frame) => frame.includes("external_open"))).toHaveLength(3);
  for (const frame of sent) expect(frame).not.toContain(scratch!);
  const grants = hostLog(run.env).filter((line) => line.event.startsWith("external."));
  expect(grants.length).toBeGreaterThan(0);
  for (const line of grants) expect(JSON.stringify(line)).not.toMatch(/shot\.png|notes\.txt|index\.html|hide-outside-/);
});

test("a reload keeps outside tabs readable, and closing one ends its grant", async () => {
  const notes = path.join(scratch!, "kept.txt");
  fs.writeFileSync(notes, "kept across a reload");
  const gone = path.join(scratch!, "gone.txt");
  fs.writeFileSync(gone, "closed and let go");
  const { page, paneId } = await start();
  await printInPane(herdr, page, paneId, [`keep ${notes}`, `close ${gone}`, "END-RELOAD"], "END-RELOAD");
  await click(page, paneId, notes);
  await expect(outsideTab(page, "kept.txt")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await click(page, paneId, gone);
  await expect(outsideTab(page, "gone.txt")).toHaveAttribute("data-external", "held", { timeout: 20_000 });

  // A reload of the window lets go of nothing: both tabs still read their files (B15).
  await withProbe(page);
  await expect(outsideTab(page, "kept.txt")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await outsideTab(page, "kept.txt").click();
  await expect(page.locator("[data-external-text] .cm-content")).toHaveText("kept across a reload");
  expect(hostLog(run.env).filter((line) => line.event === "external.released")).toEqual([]);

  // Closing a tab ends its grant and only its grant (B14).
  await outsideTab(page, "gone.txt").click();
  await outsideTab(page, "gone.txt").getByRole("button").click();
  await expect(outsideTab(page, "gone.txt")).toHaveCount(0);
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "external.released")).toHaveLength(1);
  await expect(outsideTab(page, "kept.txt")).toHaveAttribute("data-external", "held");
});
