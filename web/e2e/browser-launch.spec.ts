// Startup flags must survive the browser launch, including the defaults that
// prevent other upstream failures. This needs Chromium but no daemon or panes.
import { expect, test } from "@playwright/test";
import config from "../playwright.config";

// Command-line inspection requires this switch. Retain the suite's actual
// launch options so removing its Windows fix makes this regression fail.
test.use({ launchOptions: { ...config.use!.launchOptions, args: [...config.use!.launchOptions!.args!, "--enable-automation"] } });

test("test browser startup preserves defaults and the Windows loopback fix", { tag: "@platform" }, async ({ browser }) => {
  const session = await browser.newBrowserCDPSession();
  try {
    const { arguments: args } = await session.send("Browser.getBrowserCommandLine");
    const disabled = args.filter((arg) => arg.startsWith("--disable-features="));
    // CDP returns both the runner's switch and our additive switch; Chromium
    // applies the last one. The helper's own output is one switch (unit test).
    const effective = disabled.at(-1);
    expect(effective).toBeDefined();
    const features = effective!.slice("--disable-features=".length).split(",");
    expect(features).toEqual(expect.arrayContaining(["PaintHolding", "Translate"]));
    if (process.platform === "win32") expect(features).toContain("TcpPortRandomizationWin");
  } finally {
    await session.detach();
  }
});
