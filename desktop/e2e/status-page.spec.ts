// The host's status page, which a failed discovery leaves on screen. It is
// the only thing the operator sees when no shell loads, so a load of it that
// fails must not leave a blank window (docs/ARCHITECTURE.md, The desktop host).

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { detachedApp, hostLog, isolate, launch, screenshot, test, type Isolated } from "./fixture";

// Test-only candidate selection: a packaged executable carries its own status page inside its archive.
const packaged = process.env.HIDE_E2E_PACKAGED_APP;

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => {
  herdr = await startHerdr({ agents: false });
});

test.afterAll(() => {
  herdr?.stop();
});

test.beforeEach(() => {
  run = isolate(herdr, "status-page");
  // No hide CLI is found, so discovery fails with cli_missing at once.
  run.env.HIDE_CLI_PATH = path.join(run.root, "bin", "hide-not-installed");
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

test("a status page file that will not load still shows the failure, and Retry searches again", async () => {
  const appDir = detachedApp(run.root);
  fs.rmSync(path.join(appDir, "dist", "static"), { recursive: true });
  ({ app } = await launch(run.env, { appDir }));
  const page = await app.firstWindow();
  await expect(page.locator("#reason")).toHaveText("The hide command was not found.", { timeout: 20_000 });
  await expect(page.locator("#reason")).toHaveAttribute("data-reason", "cli_missing");
  await expect(page.locator("h1")).toHaveText("Can't connect to hided");
  await screenshot(page, "status-page-plain");
  const unavailable = hostLog(run.env).filter((line) => line.event === "status_page.unavailable");
  expect(unavailable).toMatchObject([{ file_exists: false, code: "ERR_FILE_NOT_FOUND" }]);
  const searches = () => hostLog(run.env).filter((line) => line.event === "discovery.start").map((line) => line.trigger);
  expect(searches()).toEqual(["launch"]);
  // The link is a fragment of the blank page, which the host reads as it reads the page's own Retry.
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(searches).toEqual(["launch", "retry"]);
  await expect(page.locator("#reason")).toHaveText("The hide command was not found.");
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(searches).toEqual(["launch", "retry", "retry"]);
});

test("the packaged app loads its own status page, which its archive must keep readable", async () => {
  test.skip(!packaged, "a packaged app is required");
  ({ app } = await launch(run.env, { executablePath: packaged }));
  const page = await app.firstWindow();
  // Copy diagnostics is drawn by the page file alone; the plain screen has none.
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("#reason")).toHaveText("The hide command was not found.");
  await screenshot(page, "status-page-packaged");
  const lines = hostLog(run.env);
  expect(lines.filter((line) => line.event === "window.load_failed" || line.event === "status_page.unavailable")).toEqual([]);
});
