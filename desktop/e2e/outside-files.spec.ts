// Outside files end to end (PRD outside-path-in-hide, docs/UI_BEHAVIOR.md,
// Terminal links): a plain click on a terminal path to a file outside every
// checkout, or an agent's `hide file open` or `hide browser open` of one,
// opens it read-only in the View with an External badge, by a grant the
// desktop host holds for this window alone. An HTML
// page loads its own folder and nothing beside it, a reload of the window
// keeps every tab, and closing a tab ends its grant. Everything runs on a
// private Herdr server, hided and Electron profile (see `fixture.ts`); the
// operating system's handlers are replaced with recorders, so nothing opens
// outside the test.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { agentPane } from "../../web/e2e/agent-pane";
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
  await click(page, paneId, path.join("site", "index.html"));
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

/** A one-page PDF with a correct cross-reference table, the smallest a PDF reader opens. */
function minimalPdf(): Buffer {
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>"];
  let out = "%PDF-1.4\n";
  const offsets = objects.map((object, index) => {
    const at = out.length;
    out += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return at;
  });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((at) => `${String(at).padStart(10, "0")} 00000 n \n`).join("")}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, "latin1");
}

test("an outside PDF and video open in their viewers, a file deleted before the click opens unavailable, and a second click goes to its line", async () => {
  const pdf = path.join(scratch!, "report.pdf");
  fs.writeFileSync(pdf, minimalPdf());
  const video = path.join(scratch!, "clip.mp4");
  fs.writeFileSync(video, new Uint8Array([0, 0, 0, 24, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d]));
  const deleted = path.join(scratch!, "deleted.txt");
  fs.writeFileSync(deleted, "here at the hover");
  const notes = path.join(scratch!, "lines.txt");
  fs.writeFileSync(notes, ["one", "two", "three", "four"].join("\n"));
  const { page, paneId } = await start();
  await printInPane(herdr, page, paneId, [`pdf ${pdf}`, `video ${video}`, `gone ${deleted}`, `at ${notes}:1`, `again ${notes}:4`, "END-KINDS"], "END-KINDS");

  // A PDF and a video open in the viewers a checkout file uses (B23).
  await click(page, paneId, pdf);
  await expect(outsideTab(page, "report.pdf")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator('[data-viewer="pdf"] [data-pdf-page="1"]')).toBeAttached({ timeout: 20_000 });
  await click(page, paneId, video);
  await expect(outsideTab(page, "clip.mp4")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator('[data-viewer="video"], [data-viewer-state="video-codec"]')).toBeVisible();

  // A file deleted between the hover and the click opens a tab that says it cannot be opened (B16).
  const point = await pointOf(page, paneId, deleted, { offset: 2 });
  await hoverLink(page, paneId, point);
  fs.rmSync(deleted);
  await page.mouse.click(point.x, point.y);
  await expect(outsideTab(page, "deleted.txt")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator('[data-area-empty="view-unavailable"]')).toBeVisible();
  await expect.poll(() => hostLog(run.env).some((line) => line.event === "external.read_failed" && line.reason === "ENOENT")).toBe(true);

  // A second click on an open file brings its one tab forward, read again, at the new line (B2, B10).
  await click(page, paneId, `${notes}:1`);
  await expect(page.locator("[data-external-text] .cm-activeLine")).toHaveText("one", { timeout: 20_000 });
  await click(page, paneId, `${notes}:4`);
  await expect(page.locator("[data-external-text] .cm-activeLine")).toHaveText("four", { timeout: 20_000 });
  await expect(outsideTab(page, "lines.txt")).toHaveCount(1);
  expect(await opened()).toEqual([]);
});

/** The last JSON line `hide` printed. */
function answer(out: string): Record<string, unknown> {
  return JSON.parse(out.trim().split("\n").at(-1) || "{}") as Record<string, unknown>;
}

// @platform: the agent's path goes from the CLI through hided to the desktop
// host in the wire spelling, and the host resolves it on its own system
// (`/tmp` is a link on macOS; a drive letter on Windows).
test("an agent's hide file open and hide browser open of outside files open as its External tabs and wait for the operator", { tag: "@platform" }, async () => {
  const shot = path.join(scratch!, "agent-shot.png");
  fs.writeFileSync(shot, PNG);
  const report = path.join(scratch!, "report.html");
  fs.writeFileSync(report, "<!doctype html><title>Agent report</title><h1>Report</h1>");
  fs.mkdirSync(path.join(scratch!, "folder"));
  const { page, sent } = await start();
  const [pane, other] = herdr.panes;
  const agent = await agentPane(herdr, run.env.HIDE_STATE_DIR!, pane);
  const hide = async (args: string[]) => answer((await agent(args)).out);
  const row = page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();
  // The operator's keyboard is in the other pane, so the agent's opens wait for them.
  await page.locator(`[data-pane-view="${other}"] .xterm-screen`).click();
  await expect(page.locator(`[data-pane-view="${other}"]`)).toHaveAttribute("data-focused", "true");

  // An image opened behind the operator waits on the agent's row in Needs You as To look at.
  expect(await hide(["file", "open", shot])).toMatchObject({ ok: true });
  await expect(row.locator("[data-agent-status-mark]")).toHaveAttribute("data-agent-status-mark", "look");
  await expect(row).toContainText("agent-shot.png");
  // `hide view list` names who opened it and nothing of the file.
  const listed = (await hide(["view", "list"])).result as { views: { kind: string; target: string; unseen: boolean; openers: { here: boolean }[] }[] };
  expect(listed.views.filter((view) => view.kind === "external")).toEqual([
    expect.objectContaining({ target: "", unseen: true, openers: [expect.objectContaining({ here: true })] }),
  ]);

  // Pressing the row brings the image forward in its viewer as an External tab, which sees it.
  await row.locator(`[data-agent-open="${pane}"]`).click();
  await expect(outsideTab(page, "agent-shot.png")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect(page.locator('[data-viewer="image"]')).toBeVisible();
  await expect(row.locator("[data-agent-status-mark]")).not.toHaveAttribute("data-agent-status-mark", "look");

  // With the operator in the agent's pane, an HTML page opens in front as a page of its own folder.
  expect(await hide(["browser", "open", report])).toMatchObject({ ok: true });
  await expect(outsideTab(page, "report.html")).toHaveAttribute("data-external", "held", { timeout: 20_000 });
  await expect.poll(() => pageEval<string>("Agent report", "document.querySelector('h1').textContent"), { timeout: 20_000 }).toBe("Report");
  await screenshot(page, "agent-outside-external");

  // What Hide cannot show is refused with a reason, and nothing went to the system.
  expect(await hide(["file", "open", path.join(scratch!, "missing.png")])).toMatchObject({ ok: false, reason: "file_missing" });
  expect(await hide(["file", "open", path.join(scratch!, "folder")])).toMatchObject({ ok: false, reason: "unsupported_kind" });
  expect(await hide(["browser", "open", shot])).toMatchObject({ ok: false, reason: "not_html" });
  expect(await opened()).toEqual([]);

  // No path or file name reached the shell's frames or the host's log (D-05, B22).
  for (const frame of sent) expect(frame).not.toContain(scratch!);
  const grants = hostLog(run.env).filter((line) => line.event.startsWith("external."));
  expect(grants.length).toBeGreaterThan(0);
  for (const line of grants) expect(JSON.stringify(line)).not.toMatch(/agent-shot|report\.html|hide-outside-/);
});
