// Startup flags must survive the browser launch, including the defaults that
// prevent other upstream failures. This needs Chromium but no daemon or panes.
import { expect, test } from "@playwright/test";

test("test browser startup preserves defaults and the Windows loopback fix", { tag: "@platform" }, async ({ browser }) => {
  const session = await browser.newBrowserCDPSession();
  try {
    const { arguments: args } = await session.send("Browser.getBrowserCommandLine");
    const disabled = args.filter((arg) => arg.startsWith("--disable-features="));
    expect(disabled).toHaveLength(1);
    const features = disabled[0]!.slice("--disable-features=".length).split(",");
    expect(features).toEqual(expect.arrayContaining(["PaintHolding", "Translate"]));
    if (process.platform === "win32") expect(features).toContain("TcpPortRandomizationWin");
  } finally {
    await session.detach();
  }
});
