// Test-browser startup only. Randomized TCP port allocation can refuse a
// Windows loopback connection with ERR_NO_BUFFER_SPACE while ports remain
// available (crbug.com/572515601, microsoft/playwright#43605).
const { createRequire } = require("node:module");

/** Preserve the existing feature list, including defaults installed by Electron's test loader. */
function disabledTestFeatures(current, platform = process.platform) {
  if (platform !== "win32") return current;
  return [...new Set([...current.split(",").filter(Boolean), "TcpPortRandomizationWin"])].join(",");
}

/** Chromium keeps only the last --disable-features switch: merge, never replace its defaults. */
function testBrowserArgs(args, platform = process.platform) {
  if (platform !== "win32") return args;
  // The locked package bundles its defaults. Its exported server entrypoint
  // can compute them without launching a browser; no list is copied here.
  // Fail test startup if this upstream boundary changes on a future upgrade.
  const fromTest = createRequire(require.resolve("@playwright/test"));
  const fromPlaywright = createRequire(fromTest.resolve("playwright"));
  const { server } = fromPlaywright("playwright-core/lib/coreBundle");
  const browser = server.createPlaywright({ sdkLanguage: "javascript", isInternalPlaywright: true }).chromium;
  const prefix = "--disable-features=";
  const defaults = browser._innerDefaultArgs({}).filter((arg) => arg.startsWith(prefix));
  if (defaults.length !== 1) throw new Error("expected one Playwright disabled-feature list");
  const features = [...defaults, ...args.filter((arg) => arg.startsWith(prefix))].map((arg) => arg.slice(prefix.length)).join(",");
  return [...args.filter((arg) => !arg.startsWith(prefix)), `${prefix}${disabledTestFeatures(features, platform)}`];
}

module.exports = { testBrowserArgs, disabledTestFeatures };
