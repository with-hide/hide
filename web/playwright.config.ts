import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  // Builds the fixtures' C programs once, before any test (e2e/shims/build.ts).
  globalSetup: "./e2e/global-setup.ts",
  timeout: 30_000,
  // Each spec starts its own isolated Herdr server, hided and Chromium; running
  // them in parallel makes them contend for the runner's cores, and a timing
  // assertion that fails under load reads as a product failure. One worker
  // keeps a runner deterministic; CI deals the tests out across runners
  // (scripts/e2e-shard.py), which is where the suite's parallelism lives.
  workers: 1,
  // Every test is its own unit: each starts and stops its own stack, so none
  // depends on the one before it. Saying so lets the CI shards deal out tests
  // instead of whole files, which is what keeps them even (s3 alone is a
  // third of the suite).
  fullyParallel: true,
  // CI retries a failed test once. A test that passes the retry is flaky, not
  // failing: the run passes and `scripts/ci-flaky-report.py` files it as an
  // issue with a deadline; a test that fails twice fails the lane. The retry
  // classifies and reports, it never replaces a fix (docs/TESTING.md, "Flaky
  // tests"). Locally nothing retries, so a flaky test fails where it is
  // written.
  retries: process.env.CI ? 1 : 0,
  // The list reporter names every test with its duration, so a CI log says
  // where the minutes went; the dot reporter CI would default to does not. CI
  // also writes the JSON report the flaky-test step reads, and on Windows logs
  // what holds TCP connections when a test attempt fails.
  reporter: process.env.CI
    ? [["list"], ["json", { outputFile: "e2e-report.json" }], ...(process.platform === "win32" ? [["./e2e/windows-sockets-reporter.ts"] as [string]] : [])]
    : "list",
  use: {
    baseURL: process.env.HIDE_E2E_ORIGIN ?? "http://127.0.0.1:4173",
    headless: true,
    // Headless Chromium composites in software, so every frame of xterm's WebGL
    // canvas is read back synchronously on the page's main thread (ReadPixels
    // in the layer commit), 250 to 500 ms per frame in a local trace. That
    // stalls input and WebSocket frames for every spec, and a timing assertion
    // or a product deadline reads the stall as a failure. No spec tests the
    // terminal renderer, so the shell runs the DOM renderer it falls back to
    // when WebGL is missing. A spec that does need WebGL sets its own
    // `launchOptions`.
    launchOptions: { args: ["--disable-webgl"] },
    // Wide enough for a pinned lane to dock beside Agent Views (PRD
    // right-lane B19); a test about a narrower body sets its own.
    viewport: { width: 1920, height: 1080 },
  },
  webServer: undefined,
});
