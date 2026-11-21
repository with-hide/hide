// The real packaged status document while CLI discovery is still pending.
// The gate is in a private external CLI fixture, never a product delay.
import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startEngine, type EngineServer } from "../../web/e2e/engine-fixture";
import { fixtureProgram } from "../../web/e2e/platform-fixture";
import { bundledExecutable, linkBundle } from "../../web/e2e/bundled-app";
import { captureElectronWindow } from "./electron-window";
import { captureNativeWindow } from "./native-window";
import { fitWindow, hostLog, isolate, launch, launchNative, test, type Isolated } from "./fixture";

const packaged = process.env.HIDE_E2E_PACKAGED_APP;
// No test needs a pane: the engine is empty, one serves every test, and the one test that lets a daemon start only needs it to answer.
let engine: EngineServer;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => { engine = await startEngine(); });
test.afterAll(() => { engine?.stop(); });
test.beforeEach(() => { run = isolate(engine, "status-motion"); });
test.afterEach(async () => {
  if (test.info().status !== test.info().expectedStatus) console.log("[status-motion host]", JSON.stringify(hostLog(run.env)));
  await app?.close();
  app = null;
  run.cleanup();
});

function gate(outcome: "ready" | "failed") {
  const release = path.join(run.root, "release-connect");
  if (outcome === "ready") linkBundle();
  const cli = bundledExecutable("hide");
  run.env.HIDE_CLI_PATH = fixtureProgram(path.join(run.root, "bin"), "hide-gate", `
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const timer = setInterval(() => {
  if (!fs.existsSync(${JSON.stringify(release)})) return;
  clearInterval(timer);
  if (${JSON.stringify(outcome)} === 'failed') {
    console.log(JSON.stringify({ok:false, reason:'start_failed', detail:'fixture refusal'}));
  } else {
    const result = spawnSync(${JSON.stringify(cli)}, process.argv.slice(2), {env:process.env, encoding:'utf8'});
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    process.exitCode = result.status ?? 1;
  }
}, 20);
`);
  return () => fs.writeFileSync(release, "ready");
}

test("the blinking owl draws offline, holds its still for reduced motion, and yields to failure", async () => {
  const release = gate("failed");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  await page.waitForURL((url) => url.protocol === "file:" && url.pathname.endsWith("/status.html"), { waitUntil: "load" });
  const consoleErrors: string[] = [];
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
  await page.context().setOffline(true);
  await page.reload();
  const owl = page.locator("#owl");
  await expect(owl).toBeVisible();
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "playing");
  await expect(owl).toHaveAttribute("src", "owl-blink.svg");
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#brand strong")).toHaveText("hide");
  await captureElectronWindow(app, "startup-owl", { packaged: Boolean(packaged), offline: true });
  await captureNativeWindow(app, "startup-owl-native", { method: "screencapture exact candidate window without activation", packaged: Boolean(packaged) });
  expect(await fitWindow(app, { width: 720, height: 480 })).toEqual({ width: 720, height: 480 });
  const overflow = () => page.evaluate(() => ({
    width: innerWidth, html: document.documentElement.scrollWidth, body: document.body.scrollWidth,
  }));
  for (let phase = 0; phase < 5; phase++) {
    const movingOverflow = await overflow();
    expect(movingOverflow.html).toBe(movingOverflow.width);
    expect(movingOverflow.body).toBe(movingOverflow.width);
    await captureElectronWindow(app, `startup-owl-min-${phase}`, { packaged: Boolean(packaged), viewport: "720x480", phase, overflow: movingOverflow });
    await page.evaluate(() => new Promise<void>((resolve) => {
      let frames = 120;
      const advance = () => { if (--frames === 0) resolve(); else requestAnimationFrame(advance); };
      requestAnimationFrame(advance);
    }));
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "paused");
  expect(await page.locator("#loading-track span").evaluate((element) => getComputedStyle(element).animationName)).toBe("none");
  await expect(owl).toHaveAttribute("src", "owl-blink-still.svg");
  const restingOverflow = await overflow();
  expect(restingOverflow.html).toBe(restingOverflow.width);
  expect(restingOverflow.body).toBe(restingOverflow.width);
  await captureElectronWindow(app, "startup-reduced-motion-min", { packaged: Boolean(packaged), viewport: "720x480", reducedMotion: true, overflow: restingOverflow });
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(owl).toHaveAttribute("src", "owl-blink.svg");
  release();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.locator("#connecting")).toBeHidden();
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "paused");
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  expect(consoleErrors).toEqual([]);
  expect(hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable")).toEqual([]);
});

test("actual connect readiness leaves the owl immediately without waiting for its loop", async () => {
  const release = gate("ready");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  await expect(page.locator("#owl")).toBeVisible();
  release();
  await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 60_000 });
  await expect(page.locator("#connecting")).toHaveCount(0);
  expect(hostLog(run.env).filter((line) => line.event === "discovery.attached")).toHaveLength(1);
});

test("a missing moving owl falls back to its still and preserves the status and recovery actions", async () => {
  const release = gate("failed");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  await page.waitForURL((url) => url.protocol === "file:" && url.pathname.endsWith("/status.html"), { waitUntil: "load" });
  await page.route("**/owl-blink.svg", (route) => route.abort());
  await page.reload();
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#owl")).toHaveAttribute("src", "owl-blink-still.svg");
  await expect(page.locator("#owl")).toBeVisible();
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "paused");
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable").length).toBe(1);
  release();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "discovery.start").length).toBe(2);
});

test("an unavailable still leaves readable status and recovery", async () => {
  const release = gate("failed");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  await page.waitForURL((url) => url.protocol === "file:" && url.pathname.endsWith("/status.html"), { waitUntil: "load" });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.route("**/owl-blink-still.svg", (route) => route.abort());
  await page.reload();
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#owl")).toBeHidden();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable").length).toBe(1);
  release();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
});

test("the native dev host pauses startup and first-run owls without renderer focus emulation", async () => {
  const release = gate("ready");
  fs.rmSync(path.join(run.env.HOME!, ".hide/kit/installed.json"));
  // The packaged executable ignores Node -r observers. This same-fuse dev host
  // uses the identical built status files; packaged loading is tested above.
  const close = await launchNative(run.env, path.join(__dirname, "status-motion-probe.cjs"));
  const dir = run.env.HIDE_DESKTOP_USER_DATA_DIR!;
  fs.mkdirSync(dir, { recursive: true });
  let id = 0;
  const observations: unknown[] = [];
  async function command(action = "read") {
    const current = ++id;
    const file = path.join(dir, "owl-command.json");
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ id: current, action }));
    fs.renameSync(`${file}.tmp`, file);
    let reply: Record<string, unknown> = {};
    await expect.poll(() => {
      const answer = path.join(dir, "owl-answer.json");
      if (fs.existsSync(answer)) reply = JSON.parse(fs.readFileSync(answer, "utf8")) as Record<string, unknown>;
      return reply.id;
    }).toBe(current);
    expect(reply.error).toBeUndefined();
    observations.push(reply);
    return reply;
  }
  try {
    await command("show");
    await expect.poll(command).toMatchObject({ reducedMotion: false, visible: true, hidden: false, motion: "playing" });
    await command("hide");
    await expect.poll(command).toMatchObject({ visible: false, hidden: true, motion: "paused" });
    await command("show");
    await expect.poll(command).toMatchObject({ visible: true, hidden: false, motion: "playing" });
    release();
    await expect.poll(command, { timeout: 60_000 }).toMatchObject({ shell: true, pose: "agents", owls: 1, motion: "playing" });
    await command("hide");
    await expect.poll(command).toMatchObject({ hidden: true, motion: "paused" });
    await command("show");
    await expect.poll(command).toMatchObject({ hidden: false, motion: "playing" });
  } finally {
    try {
      const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (evidence) fs.writeFileSync(path.join(evidence, "native-owl-visibility.json"), JSON.stringify({ surface: "isolated native dev host with shipped fuses and built status bytes", packaged: false, observations }, null, 2));
    } finally { await close(); }
  }
});
