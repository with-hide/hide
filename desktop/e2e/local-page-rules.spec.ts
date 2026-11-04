// A local HTML page (a `file:` page inside a checkout) is held to what a browser allows a local
// page: it loads its own folder's files, cannot read another file, and reaches no address of this
// machine or its network (docs/BROWSER_DISPLAYS.md, The file boundary). The app runs on the
// fused Electron the release is built with (`fused-electron.ts`), against a private hided and
// engine, never the operator's.
import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { agentPane } from "../../web/e2e/agent-pane";
import { enterWorkspace } from "../../web/e2e/wire";
import { fitWindow, hostLog, launchShell, startStack, test, type Stack } from "./fixture";

test.describe.configure({ timeout: 180_000 });

// A 1x1 red PNG.
const PIXEL = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==", "base64");

let stack: Stack;
let app: ElectronApplication | null = null;
// One stack per test: an agent pane is made once for an engine.
test.beforeEach(async () => {
  stack = await startStack("local-page-rules", { agents: false });
});
test.afterEach(async () => {
  await app?.close().catch(() => undefined);
  app = null;
  stack.stop();
});

/** The first non-loopback IPv4 address of this machine, where a runner has one. */
function lanAddress(): string | undefined {
  return Object.values(os.networkInterfaces()).flat().find((entry) => entry?.family === "IPv4" && !entry.internal)?.address;
}

/** A server that answers anyone and keeps what it was asked. */
async function listen(host: string): Promise<{ server: Server; origin: string; heard: string[] }> {
  const heard: string[] = [];
  const server = createServer((request, response) => {
    heard.push(`${request.method} ${request.url}`);
    response.writeHead(200, { "content-type": "text/plain", "access-control-allow-origin": "*" });
    response.end("served");
  });
  await new Promise<void>((resolve) => server.listen(0, host === "127.0.0.1" ? "127.0.0.1" : "0.0.0.0", resolve));
  return { server, origin: `http://${host}:${(server.address() as AddressInfo).port}`, heard };
}

test("A local HTML page loads its own folder's files and nothing else", async () => {
  const { fixture, run } = stack;
  const launched = await launchShell(run.env);
  app = launched.app;
  await fitWindow(app, { width: 1400, height: 900 });
  await enterWorkspace(launched.page, "fixture");
  const agent = await agentPane(fixture, run.env.HIDE_STATE_DIR!, fixture.panes[0]);

  const checkout = path.join(fixture.root, "fixture");
  const outside = path.join(fixture.root, "outside");
  fs.mkdirSync(path.join(checkout, "assets"), { recursive: true });
  fs.mkdirSync(outside, { recursive: true });
  fs.writeFileSync(path.join(checkout, "assets", "pixel.png"), PIXEL);
  fs.writeFileSync(path.join(checkout, "assets", "own.js"), "window.ownScript = true;");
  fs.writeFileSync(path.join(checkout, "sibling.txt"), "beside the page");
  fs.writeFileSync(path.join(outside, "other.txt"), "another folder");
  fs.writeFileSync(path.join(outside, "pixel.png"), PIXEL);
  const linked = process.platform !== "win32";
  if (linked) fs.symlinkSync(path.join(outside, "pixel.png"), path.join(checkout, "assets", "linked.png"));
  const local = await listen("127.0.0.1");
  const lanIp = lanAddress();
  const lan = lanIp ? await listen(lanIp) : null;
  try {
    const page = path.join(checkout, "page.html");
    fs.writeFileSync(page, `<!doctype html><meta charset="utf-8"><title>Local page</title><h1>Local page</h1>
<img id="own" src="assets/pixel.png"><script src="assets/own.js"></script>`);
    const opened = await agent(["browser", "open", pathToFileURL(page).href, "--wait"]);
    expect(opened.status, opened.out).toBe(0);
    let url = "";
    await expect.poll(async () => (url = (await shownUrls()).find((shown) => shown.startsWith("file:") && shown.endsWith("/fixture/page.html")) ?? "")).not.toBe("");

    // Its own folder's image and script load.
    await expect.poll(() => inPage<boolean>(url, "document.getElementById('own').naturalWidth === 1 && window.ownScript === true")).toBe(true);

    const probe = `(async () => {
      const out = {};
      // A request that is cancelled may never answer at all, which reads as refused as well.
      const attempt = async (name, read) => { try { out[name] = await Promise.race([read(), new Promise((_, reject) => setTimeout(() => reject(new Error("silent")), 3000))]); } catch (error) { out[name] = "refused"; } };
      const image = (source) => new Promise((resolve, reject) => { const element = new Image(); element.onload = () => resolve(element); element.onerror = () => reject(new Error("image")); element.src = source; });
      const read = (element) => { const canvas = document.createElement("canvas"); canvas.width = canvas.height = 1; const context = canvas.getContext("2d"); context.drawImage(element, 0, 0); return Array.from(context.getImageData(0, 0, 1, 1).data); };
      await attempt("fetchSibling", async () => (await fetch("sibling.txt")).text());
      await attempt("fetchOther", async () => (await fetch(${JSON.stringify(pathToFileURL(path.join(outside, "other.txt")).href)})).text());
      await attempt("xhrSibling", () => new Promise((resolve, reject) => { const x = new XMLHttpRequest(); x.open("GET", "sibling.txt"); x.onload = () => resolve(x.responseText); x.onerror = () => reject(new Error("xhr")); x.send(); }));
      await attempt("imageOutside", () => image(${JSON.stringify(pathToFileURL(path.join(outside, "pixel.png")).href)}));
      await attempt("imageLinkedOut", () => image("assets/linked.png"));
      await attempt("frameText", () => new Promise((resolve, reject) => { const frame = document.body.appendChild(document.createElement("iframe")); frame.onload = () => resolve(frame.contentDocument.body.innerText); frame.onerror = () => reject(new Error("frame")); frame.src = "sibling.txt"; }));
      await attempt("canvasOwnImage", async () => read(await image("assets/pixel.png")));
      await attempt("fetchLoopback", async () => (await fetch(${JSON.stringify(`${local.origin}/fetch`)})).text());
      await attempt("imageLoopback", () => image(${JSON.stringify(`${local.origin}/image`)}));
      await attempt("socketLoopback", () => new Promise((resolve, reject) => { const socket = new WebSocket(${JSON.stringify(local.origin.replace("http:", "ws:") + "/socket")}); socket.onopen = () => resolve("open"); socket.onerror = () => reject(new Error("socket")); }));
      ${lan ? `await attempt("fetchLan", async () => (await fetch(${JSON.stringify(`${lan.origin}/fetch`)})).text());
      await attempt("imageLan", () => image(${JSON.stringify(`${lan.origin}/image`)}));` : ""}
      return out;
    })()`;
    const answers = await inPage<Record<string, unknown>>(url, probe);
    // No file is read: the reads of this folder's own file and of another folder's are refused alike,
    // an image of another folder or through a link out of the checkout does not load, and a frame of a file is not readable.
    for (const name of ["fetchSibling", "fetchOther", "xhrSibling", "imageOutside", "frameText"]) expect(answers[name], name).toBe("refused");
    if (linked) expect(answers.imageLinkedOut).toBe("refused");
    // Its own image draws, but its pixels are not readable back: a local page is its own origin, as in a browser.
    expect(answers.canvasOwnImage).toBe("refused");
    // Nothing on this machine or its network answers it, and nothing there was asked.
    for (const name of ["fetchLoopback", "imageLoopback", "socketLoopback", ...(lan ? ["fetchLan", "imageLan"] : [])]) expect(answers[name], name).toBe("refused");
    expect(local.heard).toEqual([]);
    expect(lan?.heard ?? []).toEqual([]);

    // Each kind of refusal is logged once for the page, however often it was tried, and says nothing of what was asked.
    const refusals = hostLog(run.env).filter((entry) => entry.event === "browser.request_refused" && entry.reason === "local_page_boundary");
    expect(refusals.map((entry) => entry.boundary).sort()).toEqual(["file", "network"]);
    expect(JSON.stringify(refusals)).not.toContain(local.origin);
  } finally {
    await new Promise<void>((resolve) => local.server.close(() => resolve()));
    if (lan) await new Promise<void>((resolve) => lan.server.close(() => resolve()));
  }

  async function shownUrls(): Promise<string[]> {
    return app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children
      .flatMap((view) => { const contents = (view as { webContents?: Electron.WebContents }).webContents; return contents && !contents.isDestroyed() ? [contents.getURL()] : []; }));
  }
  async function inPage<T>(shown: string, expression: string): Promise<T> {
    return app!.evaluate(({ BrowserWindow }, { shown, expression }) => {
      const contents = BrowserWindow.getAllWindows()[0]!.contentView.children
        .map((view) => (view as { webContents?: Electron.WebContents }).webContents)
        .find((candidate) => candidate && !candidate.isDestroyed() && candidate.getURL() === shown);
      if (!contents) throw new Error(`no page shows ${shown}`);
      return contents.executeJavaScript(expression) as Promise<T>;
    }, { shown, expression });
  }
});
