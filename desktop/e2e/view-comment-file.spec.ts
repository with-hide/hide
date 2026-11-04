// The operator's Comment mode on a local HTML page (a `file:` page inside a checkout). It draws the
// same overlay as on an http page, through a comment-only lease only hided holds, while an agent
// still cannot drive the page, and the comment world reads no more than the page itself could
// (docs/BROWSER_DISPLAYS.md, Comment mode and The file boundary). The operator's app, engine
// and browser are never used.
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import fs from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { agentPane, type HideAnswer } from "../../web/e2e/agent-pane";
import { enterWorkspace } from "../../web/e2e/wire";
import { fitWindow, launchShell, startStack, test, type Stack } from "./fixture";
import { captureNativeWindow } from "./native-window";
import { evaluateIn, pageUrls, sendInput } from "./page-input";

test.describe.configure({ timeout: 180_000 });

// The shape of the operator's own example page: a heading, a field, a button, and room below.
const PAGE = `<!doctype html><meta charset="utf-8"><title>Local fixture</title>
<style>body{margin:0;font:16px sans-serif} h1{margin:40px} #save{margin:0 40px;width:220px;height:48px} #room{height:320px}</style>
<h1 id="title">Local fixture</h1>
<input id="note" style="margin:0 40px 16px;display:block;width:300px">
<button id="save" type="button" onclick="document.title='saved'">Save fixture state</button>
<div id="room"></div>
<script>window.keys = 0; addEventListener("keydown", () => { window.keys += 1; }, true);</script>`;

let stack: Stack;
let app: ElectronApplication | null = null;

// One stack per test: an agent pane is made once for an engine.
test.beforeEach(async () => {
  stack = await startStack("view-comment-file", { agents: false });
});
test.afterEach(async () => {
  await app?.close().catch(() => undefined);
  app = null;
  stack.stop();
});

/** The address the window's native page shows for the file named `name` in the checkout. */
async function shownAs(name: string): Promise<string> {
  let found = "";
  await expect.poll(async () => (found = (await pageUrls(app!)).find((url) => url.startsWith("file:") && url.endsWith(`/fixture/${name}`)) ?? "")).not.toBe("");
  return found;
}

/** The centre of `selector` as the page lays it out, in its viewport's points. */
async function centreOf(url: string, selector: string): Promise<{ x: number; y: number }> {
  return evaluateIn(app!, url, `(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()`);
}

test("Comment on a local HTML page: point at an element and drag an area, and the agent gets both while it still cannot drive the page", async () => {
  const { fixture, run } = stack;
  const launched = await launchShell(run.env);
  app = launched.app;
  const shell: Page = launched.page;
  await fitWindow(app, { width: 1400, height: 900 });
  await enterWorkspace(shell, "fixture");
  const agent = await agentPane(fixture, run.env.HIDE_STATE_DIR!, fixture.panes[0]);
  const ok = (answer: HideAnswer) => {
    expect(answer.status, answer.out).toBe(0);
    return answer.out;
  };
  const file = path.join(fixture.root, "fixture", "index.html");
  fs.writeFileSync(file, PAGE);
  const opened = JSON.parse(ok(await agent(["browser", "open", pathToFileURL(file).href, "--wait"])).trim().split("\n").at(-1)!) as { result: { view_id: string } };
  const display = opened.result.view_id;
  const url = await shownAs("index.html");

  // The page takes Comment: the overlay is drawn in it, and it says so.
  const comment = shell.locator("[data-browser-comment]");
  await comment.click();
  await expect(comment).toHaveAttribute("data-browser-comment", "on");
  await shell.mouse.move(0, 0);
  await expect(shell.getByRole("tooltip")).toHaveCount(0);
  await expect.poll(() => evaluateIn<boolean>(app!, url, "document.querySelector('hide-comment-overlay') !== null")).toBe(true);
  // The lease is hided's alone: an agent's page command on a file page keeps its answer.
  const refused = await agent(["browser", "snapshot", display]);
  expect(refused.status).not.toBe(0);
  expect(refused.out).toContain("display_unsupported");

  // A click on an element: the note opens over the page's still with the element's selector, and Add keeps it.
  const save = await centreOf(url, "#save");
  await sendInput(app, url, [{ type: "mouseMove", x: save.x, y: save.y }]);
  await captureNativeWindow(app, "file-page-comment-1-hover", { claim: "the overlay outlines the element under the pointer on a local HTML page" });
  await sendInput(app, url, [
    { type: "mouseDown", x: save.x, y: save.y, button: "left", clickCount: 1 },
    { type: "mouseUp", x: save.x, y: save.y, button: "left", clickCount: 1 },
  ]);
  const note = shell.locator('[data-page-note="pick"]');
  await expect(note.locator("[data-view-note-form]")).toContainText("#save");
  await expect(shell.locator(`[data-browser-still="${display}"]`)).toBeVisible();
  await captureNativeWindow(app, "file-page-comment-2-element-note", { claim: "a click on an element opens the note with its selector" });
  await shell.keyboard.type("Make this button one size smaller.");
  await shell.keyboard.press("ControlOrMeta+Enter");
  await expect(note).toHaveCount(0);
  // The page never saw the press nor a key of the note.
  expect(await evaluateIn<string>(app, url, "document.title")).toBe("Local fixture");
  expect(await evaluateIn<number>(app, url, "window.keys")).toBe(0);

  // A drag over empty room: an area, not an element.
  const room = await centreOf(url, "#room");
  await sendInput(app, url, [
    { type: "mouseMove", x: room.x - 120, y: room.y - 60 },
    { type: "mouseDown", x: room.x - 120, y: room.y - 60, button: "left", clickCount: 1 },
    { type: "mouseMove", x: room.x, y: room.y, modifiers: ["leftbuttondown"] },
    { type: "mouseMove", x: room.x + 120, y: room.y + 60, modifiers: ["leftbuttondown"] },
    { type: "mouseUp", x: room.x + 120, y: room.y + 60, button: "left", clickCount: 1 },
  ]);
  await expect(note.locator("[data-view-note-form]")).toBeVisible();
  await captureNativeWindow(app, "file-page-comment-3-area-note", { claim: "a drag over empty room opens the note on an area" });
  await shell.keyboard.type("Leave this space empty.");
  await shell.keyboard.press("ControlOrMeta+Enter");
  await expect(note).toHaveCount(0);
  const front = shell.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
  await expect(front.locator("[data-view-comments]")).toHaveText("2");

  // Esc ends Comment; Send gives the agent the page, the element and the area, each with a screenshot.
  await sendInput(app, url, [{ type: "keyDown", keyCode: "Escape" }]);
  await expect(comment).toHaveAttribute("data-browser-comment", "off");
  const row = shell.locator("[data-view-feedback-row]");
  await row.locator('[data-view-feedback-send-main="default"]').click();
  await expect(row).toHaveAttribute("data-view-feedback-state", "sent");
  const inbox = ok(await agent(["inbox"]));
  expect(inbox).toContain("index.html");
  expect(inbox).toContain("#save");
  expect(inbox).toContain("Make this button one size smaller.");
  expect(inbox).toContain("Leave this space empty.");
  const shots = [...inbox.matchAll(/Screenshot: (\S+\.png)/g)].map((match) => match[1]!);
  expect(shots).toHaveLength(2);
  for (const shot of shots) expect(fs.readFileSync(shot).subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
});

test("The comment world of a local HTML page reads exactly what the page itself reads", async () => {
  const { fixture, run } = stack;
  const launched = await launchShell(run.env);
  app = launched.app;
  await fitWindow(app, { width: 1400, height: 900 });
  await enterWorkspace(launched.page, "fixture");
  const agent = await agentPane(fixture, run.env.HIDE_STATE_DIR!, fixture.panes[0]);
  // Secrets in the checkout beside the page and outside every checkout, and a server that must hear nothing.
  const checkout = path.join(fixture.root, "fixture");
  const outside = path.join(fixture.root, "outside");
  fs.mkdirSync(outside, { recursive: true });
  const secret = "TOPSECRET-" + path.basename(run.root);
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==", "base64");
  fs.writeFileSync(path.join(checkout, "sibling.txt"), secret);
  fs.writeFileSync(path.join(outside, "secret.txt"), secret);
  fs.writeFileSync(path.join(outside, "pixel.png"), png);
  const heard: string[] = [];
  const server: Server = createServer((request, response) => { heard.push(request.url ?? ""); response.end(secret); });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const page = path.join(checkout, "probe.html");
    fs.writeFileSync(page, "<!doctype html><meta charset=\"utf-8\"><title>Probe</title><h1>Probe</h1>");
    const opened = await agent(["browser", "open", pathToFileURL(page).href, "--wait"]);
    expect(opened.status, opened.out).toBe(0);
    const url = await shownAs("probe.html");
    const probe = `(async () => {
      const out = {};
      const attempt = async (name, read) => { try { out[name] = await read(); } catch (error) { out[name] = "blocked: " + error.name; } };
      const loaded = (element, source) => new Promise((resolve, reject) => { element.onload = () => resolve(element); element.onerror = () => reject(new Error("load")); element.src = source; });
      await attempt("fetchBeside", async () => (await fetch(${JSON.stringify(pathToFileURL(path.join(checkout, "sibling.txt")).href)})).text());
      await attempt("fetchOutside", async () => (await fetch(${JSON.stringify(pathToFileURL(path.join(outside, "secret.txt")).href)})).text());
      await attempt("xhrOutside", () => new Promise((resolve, reject) => { const x = new XMLHttpRequest(); x.open("GET", ${JSON.stringify(pathToFileURL(path.join(outside, "secret.txt")).href)}); x.onload = () => resolve(x.responseText); x.onerror = () => reject(new Error("xhr")); x.send(); }));
      await attempt("pixels", async () => { const image = await loaded(new Image(), ${JSON.stringify(pathToFileURL(path.join(outside, "pixel.png")).href)}); const canvas = document.createElement("canvas"); canvas.width = canvas.height = 1; const context = canvas.getContext("2d"); context.drawImage(image, 0, 0); return Array.from(context.getImageData(0, 0, 1, 1).data); });
      await attempt("frameText", async () => { const frame = await loaded(document.body.appendChild(document.createElement("iframe")), ${JSON.stringify(pathToFileURL(path.join(checkout, "sibling.txt")).href)}); return frame.contentDocument.body.innerText; });
      await attempt("network", async () => (await fetch(${JSON.stringify(`http://127.0.0.1:${(server.address() as AddressInfo).port}/leak`)})).text());
      return out;
    })()`;
    // The page's own world, an isolated world of the same document, which is where the comment session
    // evaluates, and the page as its own script host runs it.
    const answers = await app.evaluate(async ({ BrowserWindow }, { url, probe }) => {
      const contents = BrowserWindow.getAllWindows()[0]!.contentView.children
        .map((view) => (view as { webContents?: Electron.WebContents }).webContents)
        .find((candidate) => candidate && !candidate.isDestroyed() && candidate.getURL() === url)!;
      const script = await contents.executeJavaScript(probe) as Record<string, unknown>;
      const cdp = contents.debugger;
      cdp.attach("1.3");
      try {
        const { frameTree } = await cdp.sendCommand("Page.getFrameTree") as { frameTree: { frame: { id: string } } };
        const { executionContextId } = await cdp.sendCommand("Page.createIsolatedWorld", { frameId: frameTree.frame.id, worldName: "proof" }) as { executionContextId: number };
        const evaluate = async (contextId?: number) => ((await cdp.sendCommand("Runtime.evaluate", { expression: probe, awaitPromise: true, returnByValue: true, ...(contextId === undefined ? {} : { contextId }) })) as { result: { value: Record<string, unknown> } }).result.value;
        return { script, page: await evaluate(), comment: await evaluate(executionContextId) };
      } finally { cdp.detach(); }
    }, { url, probe });
    console.log(`file-page probe: ${JSON.stringify(answers)}`);
    // The isolated world is the page's document under the page's origin, session and request rules: it reads
    // what the page's own script reads, and nothing more, whatever the page can reach.
    expect(answers.comment).toEqual(answers.script);
    expect(answers.page).toEqual(answers.script);
    // Each read that went out is one the page's own script made too: the server heard the network attempt once per world.
    expect(heard.length).toBe(Object.values(answers).filter((world) => !String(world.network).startsWith("blocked")).length);
  } finally { await new Promise<void>((resolve) => server.close(() => resolve())); }
});
