// The real packaged status document while CLI discovery is still pending.
// The gate is in a private external CLI fixture, never a product delay.
import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { fixtureProgram } from "../../web/e2e/platform-fixture";
import { bundledExecutable, linkBundle } from "../../web/e2e/bundled-app";
import { captureElectronWindow } from "./electron-window";
import { captureNativeWindow } from "./native-window";
import { hostLog, isolate, launch, launchNative, test, type Isolated } from "./fixture";

const packaged = process.env.HIDE_E2E_PACKAGED_APP;
let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => { herdr = await startHerdr({ agents: false }); });
test.afterAll(() => { herdr?.stop(); });
test.beforeEach(() => { run = isolate(herdr, "status-motion"); });
test.afterEach(async () => {
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

test("the local flight draws offline, freezes live for reduced motion, and yields to failure", async () => {
  const release = gate("failed");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  const consoleErrors: string[] = [];
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
  await page.context().setOffline(true);
  await page.reload();
  await expect(page.locator("#owl-flight svg")).toBeVisible();
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "playing");
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#brand strong")).toHaveText("hide");
  await captureElectronWindow(app, "startup-flight", { packaged: Boolean(packaged), offline: true });
  await captureNativeWindow(app, "startup-flight-native", { method: "screencapture exact candidate window without activation", packaged: Boolean(packaged), privateSocket: herdr.socket });
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.setSize(720, 480));
  for (let phase = 0; phase < 5; phase++) {
    await captureElectronWindow(app, `startup-flight-min-${phase}`, { packaged: Boolean(packaged), viewport: "720x480", phase });
    await page.evaluate(() => new Promise<void>((resolve) => {
      let frames = 120;
      const advance = () => { if (--frames === 0) resolve(); else requestAnimationFrame(advance); };
      requestAnimationFrame(advance);
    }));
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "paused");
  expect(await page.locator("#loading-track span").evaluate((element) => getComputedStyle(element).animationName)).toBe("none");
  const frame = () => page.locator("#owl-flight").innerHTML();
  const frozen = await frame();
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  expect(await frame()).toBe(frozen);
  await captureElectronWindow(app, "startup-reduced-motion-min", { packaged: Boolean(packaged), viewport: "720x480", reducedMotion: true });
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect.poll(frame).not.toBe(frozen);
  release();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.locator("#connecting")).toBeHidden();
  await expect(page.locator("body")).toHaveAttribute("data-owl-motion", "paused");
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  expect(consoleErrors).toEqual([]);
  expect(hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable")).toEqual([]);
});

test("actual connect readiness leaves the flight immediately without waiting for its loop", async () => {
  const release = gate("ready");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  await expect(page.locator("#owl-flight svg")).toBeVisible();
  release();
  await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 60_000 });
  await expect(page.locator("#connecting")).toHaveCount(0);
  expect(hostLog(run.env).filter((line) => line.event === "discovery.attached")).toHaveLength(1);
});

test("an illustration renderer failure preserves the status and recovery actions", async () => {
  const release = gate("failed");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  // Fail only the SVG drawing boundary, after the script itself has loaded.
  await page.addInitScript(`const create = document.createElementNS.bind(document);
    document.createElementNS = (...args) => {
      if (args[1] === 'g') throw new Error('fixture illustration unavailable');
      return create(...args);
    };`);
  await page.reload();
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#owl-still")).toBeVisible();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable").length).toBe(1);
  release();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => hostLog(run.env).filter((line) => line.event === "discovery.start").length).toBe(2);
});

for (const boundary of ["canvas", "clipPath", "still"] as const) {
  test(`an unavailable ${boundary} leaves readable status and recovery`, async () => {
    const release = gate("failed");
    ({ app } = await launch(run.env, { executablePath: packaged }));
    const page = await app.firstWindow();
    if (boundary === "still") {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.route("**/owl-roam-still.svg", (route) => route.abort());
    } else {
      await page.addInitScript((kind) => {
        if (kind === "canvas") HTMLCanvasElement.prototype.getContext = (() => null) as never;
        else {
          const create = document.createElementNS.bind(document);
          document.createElementNS = ((...args: Parameters<typeof create>) => {
            if (args[1] === "clipPath") throw new Error("fixture SVG config unavailable");
            return create(...args);
          }) as typeof document.createElementNS;
        }
      }, boundary);
    }
    await page.reload();
    await expect(page.getByRole("status")).toHaveText("Starting Hide…");
    await expect.poll(() => hostLog(run.env).filter((line) => line.event === "brand_owl.unavailable").length).toBe(1);
    release();
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  });
}

test("real native visibility pauses startup and first-run owls without renderer focus emulation", async () => {
  const release = gate("ready");
  fs.rmSync(path.join(run.env.HOME!, ".hide/kit/installed.json"));
  const close = launchNative(run.env, path.join(__dirname, "status-motion-probe.cjs"), packaged);
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
    await expect.poll(command).toMatchObject({ visible: true, hidden: false, motion: "playing" });
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
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.writeFileSync(path.join(evidence, "native-owl-visibility.json"), JSON.stringify(observations, null, 2));
    await close();
  }
});
