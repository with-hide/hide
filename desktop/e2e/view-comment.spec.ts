// The operator's Comment mode on a page (PRD view-opener-feedback B9-B11,
// B16, B21, B23): an agent opens a page from its own pane, the operator
// turns Comment on in the real desktop window, picks an element in the page
// with trusted input the main process sends, writes the note in the window
// over the page's still, where the page's scripts never see it (D-17), and
// Send gives that agent one letter with the page, the element and a
// screenshot it can read. The operator's app, Herdr server and browser are
// never used.
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import fs from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { agentPane, type HideAnswer } from "../../web/e2e/agent-pane";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { fitWindow, focusPage, isolate, launch, NEEDS_FOCUS, screenshot, test, type Isolated } from "./fixture";
import { evaluateIn, sendInput } from "./page-input";

test.describe.configure({ timeout: 180_000 });

const PAGE = `<!doctype html><meta charset="utf-8"><title>Sign in</title>
<h1 id="title" style="margin:40px">Welcome back</h1>
<button id="go" style="margin:40px;width:200px;height:48px" onclick="document.title='pressed'">Continue</button>
<script>window.keys = 0; addEventListener("keydown", () => { window.keys += 1; }, true);</script>`;

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;
let server: Server;
let origin: string;

const input = (url: string, events: Electron.MouseInputEvent[] | Electron.KeyboardInputEvent[]) => sendInput(app!, url, events);

/** Whether the window's shell, not a page, holds the keyboard. */
async function shellHasKeyboard(): Promise<boolean> {
  return app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.isFocused());
}

/** Whether the page shown at `url` holds the keyboard. */
async function pageHasKeyboard(url: string): Promise<boolean> {
  return app!.evaluate(({ BrowserWindow }, url) => {
    const contents = BrowserWindow.getAllWindows()[0]!.contentView.children
      .map((view) => (view as { webContents?: Electron.WebContents }).webContents)
      .find((page) => page && !page.isDestroyed() && page.getURL() === url);
    return contents?.isFocused() ?? false;
  }, url);
}

/** The press on `#go` a person's mouse makes, where the page draws it. */
async function pressGo(url: string): Promise<{ x: number; y: number; right: number; top: number }> {
  const box = await inPage<{ x: number; y: number; right: number; top: number }>(url, "(() => { const r = document.getElementById('go').getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2), right: Math.round(r.right), top: Math.round(r.top) }; })()");
  await input(url, [
    { type: "mouseMove", x: box.x, y: box.y },
    { type: "mouseDown", x: box.x, y: box.y, button: "left", clickCount: 1 },
    { type: "mouseUp", x: box.x, y: box.y, button: "left", clickCount: 1 },
  ]);
  return box;
}

/** Whether the page shows the overlay's blue pin centred at (`x`, `y`): the pins live in a closed shadow root the page cannot read. */
async function pinAt(url: string, x: number, y: number): Promise<boolean> {
  return app!.evaluate(async ({ BrowserWindow }, { url, x, y }) => {
    const contents = BrowserWindow.getAllWindows()[0]!.contentView.children
      .map((view) => (view as { webContents?: Electron.WebContents }).webContents)
      .find((page) => page && !page.isDestroyed() && page.getURL() === url);
    if (!contents) throw new Error(`no page shows ${url}`);
    // The pin is 20 points across with its number in the middle: most of its square is its blue.
    const image = await contents.capturePage({ x: x - 6, y: y - 6, width: 12, height: 12 });
    const bitmap = image.toBitmap();
    let blue = 0;
    for (let at = 0; at < bitmap.length; at += 4) {
      const [b, g, r] = [bitmap[at]!, bitmap[at + 1]!, bitmap[at + 2]!];
      if (b > 200 && r < 120 && g > 100 && g < 170) blue += 1;
    }
    return blue * 4 > bitmap.length / 4;
  }, { url, x, y });
}

const inPage = <T>(url: string, expression: string) => evaluateIn<T>(app!, url, expression);

test.beforeAll(async () => {
  herdr = await startHerdr({ agents: false });
  server = createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end(PAGE);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
test.afterAll(async () => {
  herdr?.stop();
  await new Promise<void>((resolve) => server?.close(() => resolve()));
});
test.beforeEach(() => { run = isolate(herdr, "view-comment"); });
test.afterEach(async () => {
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

test("Comment on a page: the operator picks an element in the page and its agent gets the page, the element and a screenshot", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  const shell: Page = launched.page;
  await fitWindow(app, { width: 1400, height: 900 });
  await enterWorkspace(shell, "fixture");

  const pane = herdr.panes[0]!;
  const agent = await agentPane(herdr, run.env.HIDE_STATE_DIR!, pane);
  const ok = (answer: HideAnswer) => {
    expect(answer.status, answer.out).toBe(0);
    return answer.out;
  };
  const url = `${origin}/login`;
  const opened = JSON.parse(ok(await agent(["browser", "open", url, "--wait"])).trim().split("\n").at(-1)!) as { result: { view_id: string } };
  const display = opened.result.view_id;
  const front = shell.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

  // The selected page's toolbar keeps the agent's identity outside its tab.
  await expect(shell.locator("[data-browser-toolbar] [data-view-opener]")).toHaveAttribute("data-view-opener", pane);

  // B9: Comment draws hided's overlay in the page, and the page's own clicks stop.
  const comment = shell.locator('[data-browser-comment]');
  await comment.click();
  await expect(comment).toHaveAttribute("data-browser-comment", "on");
  // The operator's pointer leaves for the page; a tooltip left open over it would hold the page still and keep its keys.
  await shell.mouse.move(0, 0);
  await expect(shell.getByRole("tooltip")).toHaveCount(0);
  await expect.poll(() => inPage<boolean>(url, "document.querySelector('hide-comment-overlay') !== null")).toBe(true);
  // While the operator comments, the agent's page commands wait their turn (BROWSER_DISPLAYS.md, Comment mode).
  const busy = await agent(["browser", "snapshot", display]);
  expect(busy.status).not.toBe(0);
  expect(busy.out).toContain("display_busy");

  // B10: a press on an element opens the note with its selector in the window, over the page's
  // still; the page never sees the press, nor a key of the note (D-17).
  const box = await pressGo(url);
  const note = shell.locator('[data-page-note="pick"]');
  await expect(note.locator("[data-view-note-form]")).toContainText("#go");
  const field = note.locator("[data-view-note-field]");
  await expect(field).toBeFocused();
  await expect(shell.locator(`[data-browser-still="${display}"]`)).toBeVisible();
  // The picked place stays outlined over the still while its note is open.
  await screenshot(shell, "comment-page-area-note-open");
  await expect(note.locator("[data-view-area-outline]")).toHaveCount(1);
  await shell.keyboard.type("Make the button one size smaller.");
  await shell.keyboard.press("ControlOrMeta+Enter");
  await expect(note).toHaveCount(0);
  expect(await inPage<string>(url, "document.title")).toBe("Sign in");
  expect(await inPage<number>(url, "window.keys")).toBe(0);

  // B11, B14: the comment counts on the tab and the line under the View.
  await expect(front.locator("[data-view-comments]")).toHaveText("1");
  const row = shell.locator("[data-view-feedback-row]");
  await expect(row).toHaveAttribute("data-view-feedback-state", "unsent");
  await expect(row.locator("[data-view-feedback-summary]")).toHaveText(/1 comment · 1 view/);

  // B12: the comment's pin, at the element's corner, opens that comment again in the window, where it is edited.
  await expect.poll(() => pinAt(url, box.right, box.top)).toBe(true);
  await input(url, [
    { type: "mouseMove", x: box.right, y: box.top },
    { type: "mouseDown", x: box.right, y: box.top, button: "left", clickCount: 1 },
    { type: "mouseUp", x: box.right, y: box.top, button: "left", clickCount: 1 },
  ]);
  const card = shell.locator('[data-page-note="open"]');
  await expect(card.locator("[data-view-comment]")).toContainText("Make the button one size smaller.");
  await card.locator("[data-view-comment-edit]").click();
  // Editing keeps the comment's place outlined too.
  await expect(card.locator("[data-view-area-outline]")).toHaveCount(1);
  await card.locator("[data-view-note-field]").fill("Make the button one size smaller. Keep the label.");
  await shell.keyboard.press("ControlOrMeta+Enter");
  await expect(card).toHaveCount(0);
  expect(await inPage<number>(url, "window.keys")).toBe(0);

  // A place picked at an address the page then leaves goes with it, and so does its note:
  // a route within the same document first, then another document.
  await pressGo(url);
  await expect(note).toHaveCount(1);
  const routed = `${url}?step=1`;
  await inPage(url, `history.pushState({}, "", ${JSON.stringify(routed)}); true`);
  await expect(note).toHaveCount(0);
  await pressGo(routed);
  await expect(note).toHaveCount(1);
  const next = `${url}?step=2`;
  await inPage(routed, `location.assign(${JSON.stringify(next)}); true`);
  await expect(note).toHaveCount(0);
  await expect.poll(() => inPage<boolean>(next, "document.querySelector('hide-comment-overlay') !== null")).toBe(true);

  // Esc ends Comment mode: the overlay goes and the page takes clicks again; the edit left one comment.
  await input(next, [{ type: "keyDown", keyCode: "Escape" }]);
  await expect(comment).toHaveAttribute("data-browser-comment", "off");
  await expect.poll(() => inPage<boolean>(next, "document.querySelector('hide-comment-overlay') === null")).toBe(true);
  await expect(row.locator("[data-view-feedback-summary]")).toHaveText(/1 comment · 1 view/);

  // B16, B21, B23: Send gives the agent the page, the View, the element and a screenshot on its machine.
  await row.locator('[data-view-feedback-send-main="default"]').click();
  await expect(row).toHaveAttribute("data-view-feedback-state", "sent");
  await expect(front.locator("[data-view-comments]")).toHaveCount(0);
  const inbox = ok(await agent(["inbox"]));
  expect(inbox).toContain(url);
  expect(inbox).toContain(`hide browser snapshot ${display}`);
  expect(inbox).toContain("#go");
  expect(inbox).toContain("Make the button one size smaller.");
  expect(inbox).toContain("Keep the label.");
  const shot = /Screenshot: (\S+\.png)/.exec(inbox)?.[1];
  expect(shot, inbox).toBeTruthy();
  const bytes = fs.readFileSync(shot!);
  expect(bytes.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
  // D-16: the attachment is the owner's alone.
  if (process.platform !== "win32") expect(fs.statSync(shot!).mode & 0o077).toBe(0);
});

test(`A note written over a page takes the keyboard from the page and gives it back ${NEEDS_FOCUS}`, async () => {
  const launched = await launch(run.env);
  app = launched.app;
  const shell: Page = launched.page;
  await fitWindow(app, { width: 1400, height: 900 });
  await enterWorkspace(shell, "fixture");
  const agent = await agentPane(herdr, run.env.HIDE_STATE_DIR!, herdr.panes[0]!);
  const url = `${origin}/focus`;
  const opened = await agent(["browser", "open", url, "--wait"]);
  expect(opened.status, opened.out).toBe(0);
  await focusPage(app, { url });

  const comment = shell.locator("[data-browser-comment]");
  await comment.click();
  await expect(comment).toHaveAttribute("data-browser-comment", "on");
  await expect.poll(() => pageHasKeyboard(url)).toBe(true);

  // The page hides behind its still while the note is open, and a hidden page takes no keys: the note does.
  // Comment shows on only once hided's overlay is in the page, so the press is a pick, never the page's.
  await pressGo(url);
  expect(await inPage<string>(url, "document.title")).toBe("Sign in");
  await expect(shell.locator('[data-page-note="pick"] [data-view-note-field]')).toBeFocused();
  await expect.poll(shellHasKeyboard).toBe(true);

  // Esc closes the note, and the page, live again, takes the keyboard back so its next Esc ends Comment.
  await shell.keyboard.press("Escape");
  await expect(shell.locator("[data-page-note]")).toHaveCount(0);
  await expect.poll(() => pageHasKeyboard(url)).toBe(true);
  expect(await inPage<number>(url, "window.keys")).toBe(0);
});
