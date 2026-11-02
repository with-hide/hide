import { expect, test } from "@playwright/test";
import { type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";
import { openServerButton, startServer } from "./server-fixture";

test.describe.configure({ timeout: 180_000 });

test("running Workspace ports: the globe's count, one listener opens at once, two open the keyboard picker", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  const servers: ChildProcess[] = [];
  try {
    daemon = await startHided(fixture, "server-ports");
    const root = fs.realpathSync(path.join(fixture.root, "fixture"));
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const globe = openServerButton(page);
    await expect(globe).toHaveCount(1);
    await globe.click();
    await expect(page.getByText("No running servers in this Workspace.")).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press("Escape");
    // The globe's name says how many listeners the core knows, so each click
    // waits for that count: one listener opens its page at once, two open the
    // picker.
    const first = await startServer(root); servers.push(first.child);
    await expect(globe).toHaveAccessibleName("Open server, 1 running", { timeout: 20_000 });
    await globe.click();
    await expect(page.getByRole("tab", { name: new RegExp(String(first.port)) })).toBeVisible();
    await expect(page.locator("[data-browser-address]")).toHaveText(`127.0.0.1:${first.port}`);
    const second = await startServer(root); servers.push(second.child);
    await expect(globe).toHaveAccessibleName("Open server, 2 running", { timeout: 20_000 });
    await globe.click();
    await expect(page.locator("[data-server-port]")).toHaveCount(2);
    // The picker lists listeners by port, and the system picks each port, so
    // the second server is last only when its port is the higher one.
    await page.keyboard.press(second.port > first.port ? "End" : "Home");
    await expect(page.locator(`[data-server-port="${second.port}"]`)).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.locator("[data-browser-address]")).toHaveText(`127.0.0.1:${second.port}`);
    expect(errors).toEqual([]);
  } finally {
    for (const child of servers) child.kill();
    daemon?.stop(); fixture.stop();
  }
});
