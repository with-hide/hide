// Terminal links end to end (docs/UI_BEHAVIOR.md, Terminal links): text a
// shell printed into a pane is a link where it names a URL or a path that
// exists, whatever program printed it. A click opens it in the Workspace (a
// browser display, the file in View at its line, a folder in the Explorer),
// ⌘-click hands it to macOS, a file of another checkout opens in the Workspace
// in front without bringing that checkout forward, a path outside every
// checkout goes to macOS on a plain click too and a program there is only revealed, a path the terminal
// wrapped is one link, a program's own OSC 8 link opens without xterm's
// confirm dialog, a drag across a link selects rather than opens, and no link
// click reaches the program as a mouse click. Everything runs on a private Herdr server, hided and Electron
// profile (see `fixture.ts`); macOS's own handlers are replaced with
// recorders, so nothing opens outside the test.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { linkCandidates } from "../../web/src/terminalLinks";
import { toPage } from "../src/main/wirePath";
import { printFileCommand, startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { countSent, enterWorkspace, setLanePinned, showExplorer } from "../../web/e2e/wire";
import { fitWindow, hostLog, isolate, launch, screenshot, shellPage, test, type Isolated } from "./fixture";
import { measureFor, quietFor } from "../../web/e2e/wait";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

let herdr: HerdrFixture;
let run: Isolated;
/** A folder the test made outside every checkout, removed however the test ends. */
let scratch: string | null = null;
let app: ElectronApplication | null = null;
let server: http.Server;
let origin: string;

test.beforeAll(async () => {
  herdr = await startHerdr({ agents: false });
  server = http.createServer((request, response) => {
    const title = { "/a.html": "Page A", "/b.html": "Page B" }[request.url ?? ""];
    response.writeHead(title ? 200 : 404, { "content-type": "text/html; charset=utf-8" });
    response.end(title ? `<!doctype html><title>${title}</title><h1>${title}</h1>` : "not found");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

test.afterAll(async () => {
  herdr?.stop();
  await new Promise((resolve) => server?.close(resolve));
});

test.beforeEach(() => {
  run = isolate(herdr, "terminal-links");
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

type Probe = { paneId: () => string | null; paneText: (id: string) => string; paneGrid: (id: string) => { cols: number; rows: number } | null; diagnostics?: () => string[] };

/** Reloads the shell with the terminal probe, the only way to read what xterm draws through WebGL. */
async function withProbe(page: Page): Promise<void> {
  await page.evaluate(() => {
    const url = new URL(location.href);
    url.searchParams.set("probe", "1");
    location.href = url.href;
  });
  await expect
    .poll(() => page.evaluate(() => typeof (window as { __hideProbe?: unknown }).__hideProbe).catch(() => "reloading"), { timeout: 20_000 })
    .toBe("object");
}

/** Prints `lines` in the fixture pane through a program, using the pane's shell. */
async function print(page: Page, paneId: string, lines: string[], last: string): Promise<void> {
  // Leave the old row before replacing its text so xterm requests fresh links
  // when the next real hover enters it, including after an empty/error answer.
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  // The first column is covered by the pane resize grab, so use the middle.
  const away = await cellPoint(page, paneId, { row: grid.rows - 1, column: Math.floor(grid.cols / 2) });
  expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest(".xterm") != null, away)).toBe(true);
  await page.mouse.move(away.x, away.y);
  const file = path.join(herdr.root, `print-${Date.now()}.txt`);
  fs.writeFileSync(file, `${lines.join("\n")}\n`);
  const command = printFileCommand(herdr.shell, file);
  const sent = spawnSync(herdr.bin, ["pane", "run", herdr.panes[0], command], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status, sent.stderr).toBe(0);
  await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), paneId), { timeout: 20_000 }).toContain(last);
}

/** Where `text` is on the screen of `paneId`: its row and the column of its `offset`th character. */
async function cellOf(page: Page, paneId: string, text: string, offset = 0): Promise<{ row: number; column: number }> {
  const lines = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id).split("\n"), paneId);
  const row = lines.findIndex((line) => line.includes(text));
  expect(row, `${text} is not on the screen:\n${lines.join("\n")}`).toBeGreaterThanOrEqual(0);
  const prefix = lines[row]!.slice(0, lines[row]!.indexOf(text)) + text.slice(0, offset);
  const column = [...prefix].reduce((width, char) => width + (/\p{Script=Hangul}/u.test(char) ? 2 : 1), 0);
  return { row, column };
}

/** The window point at the middle of a cell of `paneId`'s screen. */
async function cellPoint(page: Page, paneId: string, cell: { row: number; column: number }): Promise<{ x: number; y: number }> {
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  const box = (await page.locator(`[data-terminal="${paneId}"] .xterm-screen`).boundingBox())!;
  return { x: box.x + ((cell.column + 0.5) * box.width) / grid.cols, y: box.y + ((cell.row + 0.5) * box.height) / grid.rows };
}

async function pointOf(page: Page, paneId: string, text: string, { offset = 0 } = {}): Promise<{ x: number; y: number }> {
  return cellPoint(page, paneId, await cellOf(page, paneId, text, offset));
}

/** Moves over a point and waits until xterm shows a link there. */
async function hoverLink(page: Page, paneId: string, point: { x: number; y: number }): Promise<void> {
  await page.mouse.move(point.x - 3, point.y);
  await page.mouse.move(point.x, point.y);
  await expect(page.locator(`[data-terminal="${paneId}"] .xterm-screen`)).toHaveClass(/xterm-cursor-pointer/, { timeout: 10_000 });
}

/** A click with ⌘ held; the mouse carries no modifiers of its own. */
async function metaClick(page: Page, point: { x: number; y: number }): Promise<void> {
  await page.keyboard.down("Meta");
  await page.mouse.click(point.x, point.y);
  await page.keyboard.up("Meta");
}

/**
 * Pins the files tool so it docks, and waits for the pane's grid to settle.
 * A docked lane narrows Agent Views once (PRD right-lane B5) and stays
 * through the terminal clicks below; past the first open, which brings the
 * documents beside the Explorer, a link that opens a file or page moves
 * nothing and the printed rows stay where the probe reads them.
 */
async function dockFiles(page: Page, paneId: string): Promise<void> {
  const before = await paneCols(page, paneId);
  await showExplorer(page);
  await setLanePinned(page, true);
  await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked");
  await narrowed(page, paneId, before);
}

function paneCols(page: Page, paneId: string): Promise<number> {
  return page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)?.cols ?? 0, paneId);
}

/** Waits for the pane to narrow from `before`, then to hold its new width across two reads. */
async function narrowed(page: Page, paneId: string, before: number): Promise<void> {
  const cols = () => paneCols(page, paneId);
  await expect.poll(cols, { timeout: 10_000 }).toBeLessThan(before);
  let seen = -1;
  await expect
    .poll(async () => {
      const now = await cols();
      const settled = now === seen;
      seen = now;
      return settled;
    }, { intervals: [300], timeout: 10_000 })
    .toBe(true);
}

/** A View tab by what its label carries. */
function viewTab(page: Page, text: string) {
  return page.locator(`[data-view-tab-bar] [role="tab"][data-display][aria-label*="${text}"]`);
}

/** Replaces macOS's handlers in the main process with recorders. */
async function recordMacOS(): Promise<void> {
  await app!.evaluate(({ shell }) => {
    const opened: { how: string; target: string }[] = [];
    (globalThis as { opened?: unknown }).opened = opened;
    shell.openExternal = async (target: string) => {
      opened.push({ how: "external", target });
    };
    shell.openPath = async (target: string) => {
      opened.push({ how: "open", target });
      return "";
    };
    shell.showItemInFolder = (target: string) => {
      opened.push({ how: "reveal", target });
    };
  });
}

function opened(): Promise<{ how: string; target: string }[]> {
  return app!.evaluate(() => (globalThis as { opened?: { how: string; target: string }[] }).opened ?? []);
}

test("links: a pane's URLs and paths open in the Workspace on a click and in macOS on ⌘-click, with no confirm dialog and no click for the program", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "src", "linked.ts"), Array.from({ length: 60 }, (_, index) => `// line ${index + 1}`).join("\n"));
  // Short, so its line never wraps; macOS keeps /tmp at /private/tmp.
  scratch = fs.mkdtempSync("/tmp/hide-links-");
  const outside = path.join(scratch, "o.txt");
  fs.writeFileSync(outside, "outside every checkout");
  const program = path.join(scratch, "tool");
  fs.writeFileSync(program, "#!/bin/sh\necho ran\n", { mode: 0o755 });
  test.info().attach("scratch", { body: scratch });

  ({ app } = await launch(run.env));
  // The window is the size a CI screen allows (macOS keeps it within the
  // screen), and the shell is zoomed out so its body is wide enough to dock
  // the lane beside the pane, with its tree once the first link opens, and
  // the pane still wide enough that no printed line wraps (PRD right-lane
  // B5, B10, B19).
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  // Counted from the reload on, so the socket it opens is heard.
  const sent = countSent(page);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
  await enterWorkspace(page, "fixture");
  const dialogs: string[] = [];
  page.on("dialog", (dialog) => {
    dialogs.push(dialog.message());
    void dialog.dismiss();
  });
  await recordMacOS();
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await dockFiles(page, paneId);
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);

  // A path longer than the pane is wide, so the terminal wraps it.
  const segments: string[] = [];
  while (["deep", ...segments].join("/").length < grid.cols + 8) segments.push(`segment-${segments.length}`);
  const wrapped = path.join("deep", ...segments, "wrapped.md");
  fs.mkdirSync(path.join(checkout, path.dirname(wrapped)), { recursive: true });
  fs.writeFileSync(path.join(checkout, wrapped), "# wrapped");

  await print(
    page,
    paneId,
    [
      `url ${origin}/a.html`,
      "file src/linked.ts:42 and missing/nothing.ts",
      `outside ${outside}`,
      `program ${program}`,
      `osc8 \u001b]8;;${origin}/b.html\u001b\\Page B link\u001b]8;;\u001b\\`,
      `osc8 \u001b]8;;file://${checkout}/src/linked.ts#L7\u001b\\Linked at 7\u001b]8;;\u001b\\`,
      "folder ./src",
      wrapped,
      "end-of-links",
    ],
    "end-of-links",
  );

  // A URL opens as a browser display in the Workspace, and the program hears
  // no click. It opens in the viewing column beside the Explorer, which stays
  // in its column, so the docked lane widens by it and the pane rewraps once
  // (PRD lane-two-columns B5, B7).
  const clicksBefore = sent.get("terminal_click") ?? 0;
  let point = await pointOf(page, paneId, `${origin}/a.html`, { offset: 4 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "Page A")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-lane="explorer"]')).toBeVisible();
  await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "documents");
  await narrowed(page, paneId, grid.cols);
  expect(sent.get("terminal_click") ?? 0).toBe(clicksBefore);
  expect(dialogs).toEqual([]);

  // A path that exists opens its file in View at the line; one that does not is no link.
  point = await pointOf(page, paneId, "src/linked.ts:42", { offset: 2 });
  await hoverLink(page, paneId, point);
  await screenshot(page, "terminal-link-hover");
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "linked.ts")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".cm-activeLine").first()).toHaveText("// line 42");
  await screenshot(page, "terminal-link-file-at-line");
  expect(sent.get("reveal_path")).toBe(1);
  point = await pointOf(page, paneId, "missing/nothing.ts", { offset: 2 });
  await page.mouse.move(point.x, point.y);
  await quietFor(page, 500, "no link underline appears for a missing path");
  await expect(page.locator(`[data-terminal="${paneId}"] .xterm-screen`)).not.toHaveClass(/xterm-cursor-pointer/);

  // ⌘-click hands the URL to the default browser and the file to macOS.
  point = await pointOf(page, paneId, `${origin}/a.html`, { offset: 4 });
  await hoverLink(page, paneId, point);
  await metaClick(page, point);
  point = await pointOf(page, paneId, "src/linked.ts:42", { offset: 2 });
  await hoverLink(page, paneId, point);
  await metaClick(page, point);
  // A path outside every checkout goes to macOS on a plain click, and a program there is only revealed.
  point = await pointOf(page, paneId, outside, { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  point = await pointOf(page, paneId, program, { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect.poll(opened).toEqual([
    { how: "external", target: `${origin}/a.html` },
    { how: "open", target: path.join(checkout, "src", "linked.ts") },
    { how: "open", target: fs.realpathSync(outside) },
    { how: "reveal", target: fs.realpathSync(program) },
  ]);

  // A program's own OSC 8 link opens without xterm's confirm dialog.
  point = await pointOf(page, paneId, "Page B link", { offset: 1 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "Page B")).toBeVisible({ timeout: 20_000 });
  expect(dialogs).toEqual([]);
  // A program's file link opens the file at the line its address names.
  point = await pointOf(page, paneId, "Linked at 7", { offset: 1 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(page.locator(".cm-activeLine").first()).toHaveText("// line 7", { timeout: 20_000 });
  expect(sent.get("reveal_path")).toBe(2);

  // A folder is revealed and selected in the file tree (PRD right-lane B16).
  point = await pointOf(page, paneId, "./src", { offset: 1 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(page.locator(`[data-explorer-row="${path.join(checkout, "src")}"][data-selected="true"]`)).toBeVisible({ timeout: 20_000 });
  expect(sent.get("reveal_path")).toBe(3);

  // A path the terminal wrapped is one link from its second row.
  const head = await cellOf(page, paneId, "deep/segment-0");
  point = await cellPoint(page, paneId, { row: head.row + 1, column: 1 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "wrapped.md")).toBeVisible({ timeout: 20_000 });
  expect(sent.get("terminal_click") ?? 0).toBe(clicksBefore);
  await screenshot(page, "terminal-link-wrapped");
  expect(sent.get("reveal_path")).toBe(4);

  // A drag across a link selects its text and opens nothing.
  const from = await pointOf(page, paneId, "src/linked.ts:42", { offset: 1 });
  const to = await pointOf(page, paneId, "src/linked.ts:42", { offset: 12 });
  await hoverLink(page, paneId, from);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up();
  await quietFor(page, 500, "the drag selection holds");
  expect(sent.get("reveal_path")).toBe(4);
  expect(await opened()).toHaveLength(4);
  expect(sent.get("terminal_click") ?? 0).toBe(clicksBefore);

  // A click on plain text is still the program's, so the counts above heard the wire.
  point = await pointOf(page, paneId, "end-of-links", { offset: 1 });
  await page.mouse.move(point.x, point.y);
  await page.mouse.click(point.x, point.y);
  await expect.poll(() => sent.get("terminal_click") ?? 0).toBe(clicksBefore + 1);
  expect(hostLog(run.env).filter((line) => line.event.startsWith("probe_paths.refused"))).toEqual([]);
});

// Its own spec because the links journey is at its size ceiling: the keyboard
// goes with the file a link opens, so ⌘W closes that file and never the pane
// that printed the link; a folder a link reveals opens nothing to type in,
// so the keyboard stays in the pane.
test("a file a terminal link opens takes the keyboard, so Command W closes the file and the pane stays", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "src", "focus.ts"), "// the keyboard follows the open");
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
  await enterWorkspace(page, "fixture");
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await dockFiles(page, paneId);
  await print(page, paneId, ["folder ./src", "file src/focus.ts", "FOCUS-END"], "FOCUS-END");

  const folder = await pointOf(page, paneId, "./src", { offset: 1 });
  await hoverLink(page, paneId, folder);
  await page.mouse.click(folder.x, folder.y);
  await expect(page.locator(`[data-explorer-row="${path.join(checkout, "src")}"][data-selected="true"]`)).toBeVisible({ timeout: 20_000 });
  // Two frames, so a keyboard move the reveal set off would have run before the pane is read.
  await page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
  await expect(page.locator(`[data-pane-view="${paneId}"] .xterm-helper-textarea`)).toBeFocused();

  const point = await pointOf(page, paneId, "src/focus.ts", { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  const editor = page.locator("[data-editor-body] .cm-content");
  await expect(editor).toContainText("the keyboard follows the open", { timeout: 20_000 });
  await expect(editor).toBeFocused();

  await page.keyboard.press("Meta+KeyW");
  await expect(viewTab(page, "focus.ts")).toHaveCount(0);
  await expect(page.locator(`[data-pane-view="${paneId}"]`)).toBeVisible();
});

/** Records each page the shell asks the desktop host to give the keyboard to, by display id. */
async function recordPageFocus(): Promise<() => Promise<string[]>> {
  await app!.evaluate(({ ipcMain }) => {
    const asked: string[] = [];
    (globalThis as { pageFocus?: string[] }).pageFocus = asked;
    ipcMain.on("hide:browser-command", (_event, target: unknown, name: unknown) => {
      if (name === "focus") asked.push(String((target as { id?: unknown } | null)?.id));
    });
  });
  return () => app!.evaluate(() => (globalThis as { pageFocus?: string[] }).pageFocus ?? []);
}

// The keyboard goes with a page a link opens, a URL's or an HTML file's, as it
// does with a file: the shell asks the desktop host to focus the page the core
// answered with. Whether macOS then hands that page the keys is the host's,
// which needs the window in front (close-policy.spec.ts, @needs-focus).
test("a page a terminal link opens, a URL's or an HTML file's, is the one the keyboard is handed to", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "src", "page.html"), "<!doctype html><title>Linked HTML</title><h1>Linked HTML</h1>\n");
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
  await enterWorkspace(page, "fixture");
  const focused = await recordPageFocus();
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await dockFiles(page, paneId);
  await print(page, paneId, [`url ${origin}/a.html`, "html src/page.html", "PAGES-END"], "PAGES-END");

  for (const [text, title] of [[`${origin}/a.html`, "Page A"], ["src/page.html", "Linked HTML"]] as const) {
    const point = await pointOf(page, paneId, text, { offset: 2 });
    await hoverLink(page, paneId, point);
    await page.mouse.click(point.x, point.y);
    await expect(viewTab(page, title)).toBeVisible({ timeout: 20_000 });
    const display = await viewTab(page, title).evaluate((tab) => (tab as HTMLElement).dataset.display ?? "");
    await expect.poll(focused, { message: `the keyboard is handed to ${title}` }).toContain(display);
  }
});

// Its own spec because the links journey is at its size ceiling: a file of
// another checkout is one contract, the routing that keeps the screen where it is.
test("an image of another checkout opens in the Workspace in front and its checkout stays where it is", async () => {
  // A second checkout, a Herdr workspace of its own beside the fixture's,
  // holding a screenshot an agent there saved.
  const elsewhere = path.join(fs.realpathSync(herdr.root), "elsewhere");
  fs.mkdirSync(elsewhere, { recursive: true });
  const pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
  fs.writeFileSync(path.join(elsewhere, "shot.png"), Buffer.from(pixel, "base64"));
  const second = herdr.run(["workspace", "create", "--cwd", elsewhere, "--label", "elsewhere", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { workspace: { workspace_id: string } };
  };
  try {
    ({ app } = await launch(run.env));
    await fitWindow(app, { width: 1024, height: 681 });
    const page = await shellPage(app);
    const sent = countSent(page);
    await withProbe(page);
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
    await enterWorkspace(page, "fixture");
    const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
    await dockFiles(page, paneId);
    // A program's own link, so the long temporary path never wraps.
    await print(page, paneId, [`osc8 \u001b]8;;file://${elsewhere}/shot.png\u001b\\Elsewhere shot\u001b]8;;\u001b\\`, "ELSEWHERE-END"], "ELSEWHERE-END");

    const point = await pointOf(page, paneId, "Elsewhere shot", { offset: 1 });
    await hoverLink(page, paneId, point);
    await page.mouse.click(point.x, point.y);
    await expect(viewTab(page, "shot.png")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('[data-viewer="image"] img')).toBeVisible();
    // The keyboard goes with the image into the Workspace in front.
    await expect(viewTab(page, "shot.png")).toBeFocused();
    // The pane that printed the link is still the one on screen.
    await expect(page.locator(`[data-pane-view="${paneId}"]`)).toBeVisible();
    expect(sent.get("file_open")).toBe(1);
    expect(sent.get("reveal_path") ?? 0).toBe(0);
    await screenshot(page, "terminal-link-other-checkout");
  } finally {
    herdr.run(["workspace", "close", second.result.workspace.workspace_id]);
  }
});

// @platform: File URLs, physical paths and the file-manager reveal label follow the host OS.
test("terminal HTML opens a rendered page and right-click offers actions for the exact link or path", { tag: "@platform" }, async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  const html = path.join(checkout, "src", "report.HTML");
  fs.writeFileSync(html, "<!doctype html><title>Terminal HTML</title>\n<h1>Rendered terminal report</h1>\n");
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  const sent = countSent(page);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.5));
  await enterWorkspace(page, "fixture");
  await recordMacOS();
  // Clipboard writes are recorded in this private renderer; QA leaves the
  // operator's clipboard and OS applications alone.
  await page.evaluate(() => Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
    writeText: async (text: string) => { (window as unknown as { copied: string }).copied = text; },
  } }));
  const copied = () => page.evaluate(() => (window as unknown as { copied?: string }).copied);
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await dockFiles(page, paneId);
  // The page this HTML link opens widens the lane until Agent Views keep
  // their 480px (UI_BEHAVIOR.md, lane widths), about 31 columns on macOS
  // here, so the address goes on a line of its own that still fits.
  await print(page, paneId, [
    "html src/report.HTML:2",
    `${origin}/a.html`,
    `osc8 \u001b]8;;${pathToFileURL(html).href}#L2\u001b\\HTML source label\u001b]8;;\u001b\\`,
    "folder ./src",
    "plain MENU-END",
  ], "MENU-END");
  const clicks = sent.get("terminal_click") ?? 0;
  let point = await pointOf(page, paneId, "src/report.HTML:2", { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "Terminal HTML")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".cm-content")).toHaveCount(0);
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children
    .filter((child) => "webContents" in child)
    .some((child) => (child as Electron.WebContentsView).webContents.getTitle() === "Terminal HTML"))).toBe(true);
  await screenshot(page, "terminal-html-rendered");
  if (process.platform === "darwin") await nativeCapture("terminal-html-rendered-native");

  await page.mouse.click(point.x, point.y, { button: "right" });
  await expect(page.getByRole("menuitem", { name: "Open in browser", exact: true })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Open source", exact: true })).toBeVisible();
  await screenshot(page, "terminal-html-menu");
  await page.getByRole("menuitem", { name: "Open source", exact: true }).click();
  await expect(page.locator('[data-view-tab-bar] [role="tab"][data-tab-kind="file"][aria-label*="report.HTML"]')).toBeVisible();
  await expect(page.locator(".cm-activeLine").first()).toHaveText("<h1>Rendered terminal report</h1>");
  await screenshot(page, "terminal-html-source");
  if (process.platform === "darwin") await nativeCapture("terminal-html-source-native");

  point = await pointOf(page, paneId, "HTML source label", { offset: 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y, { button: "right" });
  await page.getByRole("menuitem", { name: "Copy path", exact: true }).click();
  await expect.poll(copied).toBe(toPage(fs.realpathSync(html)));
  point = await pointOf(page, paneId, "src/report.HTML:2", { offset: 2 });
  await page.mouse.click(point.x, point.y, { button: "right" });
  const reveal = process.platform === "darwin" ? "Reveal in Finder" : process.platform === "win32" ? "Reveal in File Explorer" : "Open Containing Folder";
  await page.getByRole("menuitem", { name: reveal, exact: true }).click();
  expect(await opened()).toEqual([{ how: "reveal", target: fs.realpathSync(html) }]);

  point = await pointOf(page, paneId, `${origin}/a.html`, { offset: 3 });
  await page.mouse.click(point.x, point.y, { button: "right" });
  await screenshot(page, "terminal-url-menu");
  await expect(page.getByRole("menuitem", { name: "Open in Hide", exact: true })).toBeVisible();
  await page.getByRole("menuitem", { name: "Copy link", exact: true }).click();
  await expect.poll(copied).toBe(`${origin}/a.html`);
  point = await pointOf(page, paneId, `${origin}/a.html`, { offset: 3 });
  await page.mouse.click(point.x, point.y, { button: "right" });
  await page.getByRole("menuitem", { name: "Open in default browser", exact: true }).click();
  expect(await opened()).toContainEqual({ how: "external", target: `${origin}/a.html` });

  point = await pointOf(page, paneId, "./src", { offset: 1 });
  await page.mouse.click(point.x, point.y, { button: "right" });
  await expect(page.getByRole("menuitem", { name: "Copy path", exact: true })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Open source", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  point = await pointOf(page, paneId, "MENU-END", { offset: 2 });
  await page.mouse.move(point.x, point.y);
  await page.mouse.click(point.x, point.y, { button: "right" });
  await expect(page.getByRole("menuitem", { name: "Copy link", exact: true })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Copy path", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  expect(sent.get("terminal_click") ?? 0).toBe(clicks);
  expect(sent.get("reveal_path")).toBe(1);
});

/** A current native frame of this exact candidate, without activating any app. */
async function nativeCapture(name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  const pid = app!.process().pid!;
  const script = 'ObjC.import("CoreGraphics"); JSON.stringify(ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements, 0))).filter(w => w.kCGWindowLayer === 0 && w.kCGWindowOwnerPID === ' + pid + ').map(w => ({id:w.kCGWindowNumber,pid:w.kCGWindowOwnerPID})))';
  const listed = spawnSync("osascript", ["-l", "JavaScript", "-e", script], { encoding: "utf8", timeout: 10_000 });
  expect(listed.status, listed.stderr).toBe(0);
  const windows = JSON.parse(listed.stdout) as { id: number; pid: number }[];
  expect(windows).toHaveLength(1);
  fs.writeFileSync(path.join(dir, name + ".json"), JSON.stringify({ candidate: "worktree desktop/dist/main.js", ...windows[0], daemonPid: run.daemonPid(), socket: herdr.socket }, null, 2));
  const captured = spawnSync("screencapture", ["-x", "-o", "-l", String(windows[0]!.id), path.join(dir, name + ".png")], { encoding: "utf8", timeout: 10_000 });
  expect(captured.status, captured.stderr).toBe(0);
}

test("Korean prose links only the real path and opens that file", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "docs"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "docs/README.md"), "# issue 271 exact file");
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
  await enterWorkspace(page, "fixture");
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await dockFiles(page, paneId);
  await print(page, paneId, ["보드 보기 (docs/README.md)에 C안을 추가했습니다.", "KOREAN-END"], "KOREAN-END");
  const lines = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id).split("\n"), paneId);
  const row = lines.findIndex((line) => line.startsWith("보드 보기"));
  expect(row).toBeGreaterThanOrEqual(0);
  // The fixture prefix occupies ten cells, then the opening bracket one.
  const point = await cellPoint(page, paneId, { row, column: 12 });
  await page.mouse.move(point.x, point.y);
  await nativeCapture("korean-prose-hover");
  await hoverLink(page, paneId, point);
  await nativeCapture("korean-prose-hover");
  for (const column of [10, 25, 26, 27]) {
    const outside = await cellPoint(page, paneId, { row, column });
    await page.mouse.move(outside.x, outside.y);
    await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
  }
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(viewTab(page, "README.md")).toBeVisible();
  await expect(page.locator(".cm-content")).toContainText("issue 271 exact file");
  await nativeCapture("korean-prose-open");

  for (const [written, selected] of [
    ["docs/원문.md)에", "docs/원문.md)에"],
    ["docs/괄호.md)", "docs/괄호.md)"],
    ["docs/위치.md:12:4", "docs/위치.md:12:4"],
  ]) {
    fs.writeFileSync(path.join(checkout, selected!), "# selected " + selected);
    fs.writeFileSync(path.join(checkout, selected!.split(/[):]/u)[0]!), "# shorter alternative");
    await print(page, paneId, ["확인 " + written, "COLLISION-" + selected], "COLLISION-" + selected);
    const lastCell = await cellOf(page, paneId, written!, written!.length - 1);
    const lastPoint = await cellPoint(page, paneId, lastCell);
    await hoverLink(page, paneId, lastPoint);
    await page.mouse.click(lastPoint.x, lastPoint.y);
    await expect(page.locator(".cm-content")).toContainText("selected " + selected);
  }
  await nativeCapture("korean-original-file");

  // Independent fixture names keep TTL hits from a prior collision from
  // deciding this one. Every shorter stage really exists alongside the winner.
  const collisions = [];
  for (let caseIndex = 0; caseIndex < 12; caseIndex += 1) {
    const stage = caseIndex % 6;
    const punctuation = caseIndex < 6 ? "" : ".";
    const base = "docs/composite-" + caseIndex + ".ts";
    const written = "(" + base + ":12:5)에" + punctuation;
    const spellings = [written, "(" + base + ":12:5)", base + ":12:5)에", base + ":12:5)", base + ":12:5", base];
    const selected = spellings[stage]!;
    for (const file of spellings.slice(stage)) {
      fs.mkdirSync(path.join(checkout, path.dirname(file)), { recursive: true });
      fs.writeFileSync(path.join(checkout, file), Array.from({ length: 20 }, (_, line) => "// selected " + file + " line " + (line + 1)).join("\n"));
    }
    await print(page, paneId, ["합성 " + written, "COMPOSITE-END-" + caseIndex], "COMPOSITE-END-" + caseIndex);
    const first = await cellOf(page, paneId, written);
    const text = stage === 5 ? spellings[4]! : selected;
    const start = first.column + (text.startsWith("(") ? 0 : 1);
    const end = start + [...text].reduce((cells, char) => cells + (/\p{Script=Hangul}/u.test(char) ? 2 : 1), 0);
    for (const column of [start, end - 1]) {
      const inside = await cellPoint(page, paneId, { row: first.row, column });
      await hoverLink(page, paneId, inside);
    }
    for (const column of [start - 1, end]) {
      const outside = await cellPoint(page, paneId, { row: first.row, column });
      await page.mouse.move(outside.x, outside.y);
      await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
    }
    const inside = await cellPoint(page, paneId, { row: first.row, column: start });
    await hoverLink(page, paneId, inside);
    await nativeCapture("korean-composite-stage-" + caseIndex + "-hover");
    await page.mouse.click(inside.x, inside.y);
    await expect(page.locator(".cm-content")).toContainText("selected " + selected + " line ");
    if (stage === 5) await expect(page.locator(".cm-activeLine").first()).toHaveText("// selected " + selected + " line 12");
    collisions.push({ stage, punctuation, written, selected, range: { row: first.row, start, end }, line: stage === 5 ? 12 : null, column: stage === 5 ? 5 : null });
    await nativeCapture("korean-composite-stage-" + caseIndex + "-open");
  }
  if (process.env.HIDE_E2E_SCREENSHOT_DIR) fs.writeFileSync(path.join(process.env.HIDE_E2E_SCREENSHOT_DIR, "composite-click-observations.json"), JSON.stringify(collisions, null, 2));

  // A real IPC request still goes through the native host. Inject only an
  // unexpected filesystem failure; it must not establish a shorter link.
  fs.writeFileSync(path.join(checkout, "docs/fault.md"), "# native probe retry selected");
  await app.evaluate((_electron, failingPath) => {
    const fsp = process.getBuiltinModule("fs/promises") as typeof import("node:fs/promises");
    const realpath = fsp.realpath;
    (globalThis as { restoreLinkProbe?: () => void }).restoreLinkProbe = () => { fsp.realpath = realpath; };
    fsp.realpath = (async (...args: unknown[]) => {
      if (String(args[0]) === failingPath) throw Object.assign(new Error("private fixture " + failingPath), { code: "EIO" });
      return Reflect.apply(realpath, fsp, args);
    }) as typeof fsp.realpath;
  }, path.join(checkout, "docs/fault.md)에"));
  try {
    await print(page, paneId, ["오류 docs/fault.md)에", "FAULT-END"], "FAULT-END");
    const fault = await pointOf(page, paneId, "docs/fault.md", { offset: 2 });
    await page.mouse.move(fault.x, fault.y);
    await expect.poll(() => page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.diagnostics?.().findLast((line) => line.includes("native path probe failed: EIO")) ?? null)).not.toBeNull();
    await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
    const diagnostic = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.diagnostics?.().findLast((line) => line.includes("native path probe failed: EIO")));
    expect(diagnostic).not.toContain(checkout);
  } finally {
    await app.evaluate(() => { (globalThis as { restoreLinkProbe?: () => void }).restoreLinkProbe!(); });
  }
  await print(page, paneId, ["재시도 docs/fault.md)에", "FAULT-RETRY-END"], "FAULT-RETRY-END");
  const retry = await pointOf(page, paneId, "docs/fault.md", { offset: 2 });
  await hoverLink(page, paneId, retry);
  await page.mouse.click(retry.x, retry.y);
  await expect(page.locator(".cm-content")).toContainText("native probe retry selected");
  await nativeCapture("korean-probe-retry-open");

  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  const segments: string[] = [];
  while (["docs", ...segments].join("/").length < grid.cols + 10) segments.push("segment" + segments.length);
  const wrapped = ["docs", ...segments, "한글.md"].join("/");
  fs.mkdirSync(path.join(checkout, path.dirname(wrapped)), { recursive: true });
  fs.writeFileSync(path.join(checkout, wrapped), "# wrapped Korean selected");
  await print(page, paneId, ["확인 (" + wrapped + ")에서도", "WRAPPED-KOREAN-END"], "WRAPPED-KOREAN-END");
  const endCell = await cellOf(page, paneId, "한글.md");
  // Probe both halves of each Hangul glyph and the file extension.
  for (const extra of [0, 1, 2, 3, 4, 5, 6]) {
    const inside = await cellPoint(page, paneId, { row: endCell.row, column: endCell.column + extra });
    await hoverLink(page, paneId, inside);
  }
  const endPoint = await cellPoint(page, paneId, { row: endCell.row, column: endCell.column + 6 });
  await hoverLink(page, paneId, endPoint);
  await nativeCapture("korean-wrapped-hover");
  for (const extra of [7, 8, 9, 10, 11, 12]) {
    const outside = await cellPoint(page, paneId, { row: endCell.row, column: endCell.column + extra });
    await page.mouse.move(outside.x, outside.y);
    await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
  }
  await hoverLink(page, paneId, endPoint);
  await page.mouse.click(endPoint.x, endPoint.y);
  await expect(page.locator(".cm-content")).toContainText("wrapped Korean selected");
  await nativeCapture("korean-wrapped-open");
});


/** The shared fixture has two splits; give an unwrapped dense row one whole pane. */
async function zoomDensePane(page: Page, paneId: string): Promise<void> {
  await page.locator(`[data-terminal="${paneId}"]`).click({ button: "right" });
  await page.locator('[data-menu-item="toggle_zoom"]').click();
}

test("prose written straight against a bracket links only the path or address inside it", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "docs"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "docs/README.md"), "# glued prose");
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.4));
  await enterWorkspace(page, "fixture");
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  // Agents write Korean with no space before a bracket: only the path or the
  // address is the link, never the word before it or the marks around it.
  for (const [written, link] of [["캡처는(docs/README.md).", "docs/README.md"], [`열어뒀어요(${origin}/a.html). 이전에는`, `${origin}/a.html`], ["「docs/README.md」를", "docs/README.md"]] as const) {
    await print(page, paneId, [written, "GLUED-END-" + written.length], "GLUED-END-" + written.length);
    const first = await cellOf(page, paneId, link);
    for (const column of [first.column, first.column + link.length - 1]) await hoverLink(page, paneId, await cellPoint(page, paneId, { row: first.row, column }));
    for (const column of [first.column - 1, first.column - 2, first.column + link.length]) {
      const outside = await cellPoint(page, paneId, { row: first.row, column });
      await page.mouse.move(outside.x, outside.y);
      await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
    }
  }
  await nativeCapture("glued-prose-hover");
  const glued = await cellOf(page, paneId, "docs/README.md");
  const point = await cellPoint(page, paneId, { row: glued.row, column: glued.column + 2 });
  await hoverLink(page, paneId, point);
  await page.mouse.click(point.x, point.y);
  await expect(page.locator(".cm-content")).toContainText("glued prose");
});

test("dense terminal path hover measures cold and warm native work", async () => {
  const checkout = path.join(fs.realpathSync(herdr.root), "fixture");
  fs.mkdirSync(path.join(checkout, "dense"), { recursive: true });
  // Half the row carries punctuation, prose and a location around its path;
  // the first plain path stays hoverable on the original baseline.
  const tokens = Array.from({ length: 12 }, (_, index) => {
    const base = "dense/f" + String(index).padStart(2, "0") + ".md";
    return index % 2 ? "(" + base + ":12:5)에" : base;
  });
  for (let index = 0; index < tokens.length; index += 1) {
    fs.writeFileSync(path.join(checkout, "dense/f" + String(index).padStart(2, "0") + ".md"), "# dense file " + index);
  }
  ({ app } = await launch(run.env));
  await fitWindow(app, { width: 1024, height: 681 });
  const page = await shellPage(app);
  await withProbe(page);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.3));
  await enterWorkspace(page, "fixture");
  const paneId = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneId()!);
  await zoomDensePane(page, paneId);
  await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)?.cols ?? 0, paneId)).toBeGreaterThan(tokens.join(" ").length + 12);
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  await print(page, paneId, [tokens.join(" "), "DENSE-END"], "DENSE-END");
  const sourceHashes = Object.fromEntries([
    ["terminalLinks.ts", "../../web/src/terminalLinks.ts"],
    ["terminalLinkProvider.ts", "../../web/src/terminalLinkProvider.ts"],
    ["localPath.ts", "../src/main/localPath.ts"],
  ].map(([name, relative]) => [name!, createHash("sha256").update(fs.readFileSync(path.resolve(__dirname, relative!))).digest("hex")]));
  const cells = [...tokens.join(" ")].flatMap((char) => /\p{Script=Hangul}/u.test(char) ? [char, null] : [char]);
  while (cells.length < grid.cols) cells.push(" ");
  // Logical candidates use the actual parser source on this same unwrapped
  // fixture row. cwd and checkout root coincide, so physical lookup bases dedup.
  const logicalUniqueLookups = new Set(linkCandidates([cells], 0).flat().flatMap((candidate) =>
    candidate.target.kind === "path" ? [path.resolve(checkout, candidate.target.path)] : [],
  )).size;
  const pids = [app.process().pid!, run.daemonPid()!];
  const hostLoad = () => {
    const result = spawnSync("ps", ["-o", "pid=,%cpu=,rss=", "-p", pids.join(",")], { encoding: "utf8" });
    expect(result.status, result.stderr).toBe(0);
    const processes = result.stdout.trim().split("\n").map((line) => {
      const [pid, cpuPercent, rssKiB] = line.trim().split(/\s+/u).map(Number);
      return { pid, cpuPercent, rssKiB };
    });
    expect(processes).toHaveLength(2);
    return { at: new Date().toISOString(), machineLoadAverage: os.loadavg(), freeMemoryBytes: os.freemem(), processes };
  };
  // Observe the real IPC handler and native filesystem calls, preserving
  // every argument, answer and refusal. These taps live only in this app.
  await app.evaluate(({ ipcMain }) => {
    type Counts = { paths: string[]; batchSizes: number[]; realpath: number; stat: number };
    const counts: Counts = { paths: [], batchSizes: [], realpath: 0, stat: 0 };
    (globalThis as { linkCounts?: Counts }).linkCounts = counts;
    const handlers = (ipcMain as unknown as { _invokeHandlers: Map<string, (...args: unknown[]) => unknown> })._invokeHandlers;
    const original = handlers.get("hide:probe-paths")!;
    handlers.set("hide:probe-paths", (event, paths) => {
      const request = paths as string[];
      counts.paths.push(...request);
      counts.batchSizes.push(request.length);
      return original(event, paths);
    });
    const fsp = process.getBuiltinModule("fs/promises") as typeof import("node:fs/promises");
    const realpath = fsp.realpath;
    const stat = fsp.stat;
    fsp.realpath = ((...args: unknown[]) => { counts.realpath += 1; return Reflect.apply(realpath, fsp, args); }) as typeof fsp.realpath;
    fsp.stat = ((...args: unknown[]) => { counts.stat += 1; return Reflect.apply(stat, fsp, args); }) as typeof fsp.stat;
  });
  const samples = [];
  for (let trial = 0; trial < 5; trial += 1) {
    if (trial > 0) {
      await page.mouse.move(0, 0);
      await page.reload();
      await expect.poll(() => page.evaluate(() => typeof (window as { __hideProbe?: unknown }).__hideProbe)).toBe("object");
      await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id).includes("DENSE-END"), paneId)).toBe(true);
    }
    const point = await pointOf(page, paneId, tokens[0]!, { offset: 2 });
    const away = await pointOf(page, paneId, "DENSE-END", { offset: 2 });
    for (const phase of ["cold", "warm"]) {
      await page.mouse.move(away.x, away.y);
      await expect(page.locator('[data-terminal="' + paneId + '"] .xterm-screen')).not.toHaveClass(/xterm-cursor-pointer/);
      await measureFor(page, 400, "deliberate idle sampling interval, outside the hover latency boundary"); // Deliberate idle sampling interval, outside the hover latency boundary.
      const idleHostLoad = hostLoad();
      await app.evaluate(() => {
        const counts = (globalThis as { linkCounts?: { paths: string[]; batchSizes: number[]; realpath: number; stat: number } }).linkCounts!;
        counts.paths = []; counts.batchSizes = []; counts.realpath = 0; counts.stat = 0;
      });
      const start = performance.now();
      try { await hoverLink(page, paneId, point); }
      catch (error) {
        console.log("dense hover failed", { trial, phase, point, grid }, await app.evaluate(() => (globalThis as { linkCounts?: unknown }).linkCounts));
        await nativeCapture("dense-failed");
        throw error;
      }
      const latencyMs = performance.now() - start;
      const counts = await app.evaluate(() => {
        const counts = (globalThis as { linkCounts?: { paths: string[]; batchSizes: number[]; realpath: number; stat: number } }).linkCounts!;
        return { hostUniqueLookups: new Set(counts.paths).size, batchSizes: counts.batchSizes, nativeRealpath: counts.realpath, nativeStat: counts.stat };
      });
      const drivenHostLoad = hostLoad();
      const providerDiagnostic = await page.evaluate(() => (window as unknown as { __hideProbe: Probe }).__hideProbe.diagnostics?.().filter((line) => line.startsWith("terminal link: path_probe ")).at(-1) ?? null);
      const fields = providerDiagnostic ? Object.fromEntries([...providerDiagnostic.matchAll(/(\w+)=(\d+)/gu)].map((match) => [match[1]!, Number(match[2])])) : null;
      if (fields) {
        expect(fields.unique).toBe(logicalUniqueLookups);
        expect(fields.cache_misses).toBe(counts.hostUniqueLookups);
        if (phase === "warm") expect(fields.cache_hits).toBe(logicalUniqueLookups);
      }
      expect(counts.hostUniqueLookups).toBeLessThanOrEqual(512);
      expect(counts.batchSizes.every((size) => size <= 64)).toBe(true);
      if (phase === "warm") expect(counts.hostUniqueLookups + counts.nativeRealpath + counts.nativeStat).toBe(0);
      samples.push({ trial, phase, cacheState: phase === "cold" ? "new renderer, empty provider cache" : "same renderer, preceding hover within 10s TTL", logicalUniqueLookups: fields?.unique ?? logicalUniqueLookups, cacheMisses: fields?.cache_misses ?? counts.hostUniqueLookups, providerDiagnostic, latencyMs, ...counts, idleHostLoad, drivenHostLoad });
      // The baseline and candidate take the same extra pointer movement,
      // outside the measured hover. Only the candidate links this suffix token.
      const suffix = await pointOf(page, paneId, tokens[1]!, { offset: 2 });
      await page.mouse.move(suffix.x, suffix.y);
      const screen = page.locator('[data-terminal="' + paneId + '"] .xterm-screen');
      if (fields) await expect(screen).toHaveClass(/xterm-cursor-pointer/);
      else await expect(screen).not.toHaveClass(/xterm-cursor-pointer/);
    }
  }
  await nativeCapture("dense-hover");
  const evidence = JSON.stringify({ sourceHashes, workload: tokens, grid, logicalBoundary: "provider diagnostic when available, cross-checked against actual parser on identical unwrapped fixture cells with cwd=root; original baseline lacks count diagnostics and uses its parser plus real IPC misses", hostBoundary: "unique paths received by real native IPC handler, equal to admitted cache misses in this below-budget workload", loadBoundary: "idle after 400ms without input; driven immediately after one hover; ps CPU is the platform recent average, RSS in KiB; machine load is 1/5/15 minute average", latencyBoundary: "driver mouse movement to observed xterm pointer class, includes IPC and polling", samples }, null, 2);
  await test.info().attach("dense-hover", { body: evidence, contentType: "application/json" });
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (dir) fs.writeFileSync(path.join(dir, "dense-hover-metrics.json"), evidence);
});
