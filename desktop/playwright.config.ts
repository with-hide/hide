import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  // The web fixtures' C programs, built once before any test.
  globalSetup: "./e2e/global-setup.ts",
  timeout: 90_000,
  // Each spec starts its own isolated Herdr server, hided and Electron app;
  // one worker keeps them from contending for the machine.
  workers: 1,
  // CI retries a failed test once and reports one that passes the retry as
  // flaky; see web/playwright.config.ts.
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["json", { outputFile: "e2e-report.json" }]] : "list",
  // A `@needs-focus` test brings its app to the front and then quits it. On a
  // machine with no other app in front, a CI runner, macOS then activates the
  // next app that opens, and every background test after it would fail the
  // focus guard. The background tests run first, and the focus tests after.
  // A CI shard runs with `--no-deps`, since a dependency runs whole in every
  // shard that holds a dependent test, and keeps that order through this
  // list's order and the one worker (.github/workflows/desktop-e2e.yml).
  projects: [
    { name: "background", grepInvert: /@needs-focus/ },
    { name: "needs-focus", grep: /@needs-focus/, dependencies: ["background"] },
  ],
});
