// PRD herdr-takeover: a Herdr on another protocol holds all of Hide. The operator's server is a stand-in on the
// daemon's socket that answers `ping` as an older (0.8.2, protocol 20) or a newer (protocol 23) release and refuses
// the subscription as such a release does, so the core's own read, its look and the screen are the product's. No
// protocol-23 Herdr exists yet, which is why a stand-in plays both. The daemon runs from a bundle with the pinned
// Herdr, the one a packaged app has, so its look offers the move.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { chord } from "./chords";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { localEndpoint } from "./platform-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

/** Requests that would make a workspace, tab or pane on the operator's server. */
const MAKES = ["workspace.create", "tab.create", "pane.split", "layout.apply", "agent.start"];

/** A server on `socket` that pings as `version` on `protocol` and refuses everything else, recording each method. */
async function standIn(herdr: HerdrFixture, version: string, protocol: number) {
  const socket = path.join(herdr.root, `stand-in-${protocol}.sock`);
  const methods: string[] = [];
  const server = net.createServer((client) => {
    client.on("error", () => client.destroy());
    let buffer = "";
    client.on("data", (chunk) => {
      buffer += chunk.toString();
      let end: number;
      while ((end = buffer.indexOf("\n")) >= 0) {
        const request = JSON.parse(buffer.slice(0, end)) as { id: unknown; method: string };
        buffer = buffer.slice(end + 1);
        methods.push(request.method);
        const answer = request.method === "ping"
          ? { id: request.id, result: { type: "pong", version, protocol, capabilities: { live_handoff: true } } }
          // Herdr 0.8 refuses 0.9's topics before any snapshot could name its protocol.
          : { id: request.id, error: { code: "invalid_request", message: `unknown method or topic for protocol ${protocol}` } };
        client.write(`${JSON.stringify(answer)}\n`);
      }
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(localEndpoint(socket), resolve);
  });
  // The core checks a marker file before it connects to a Windows pipe.
  if (process.platform === "win32") fs.writeFileSync(socket, `${process.pid}:${Date.now()}`);
  return { socket, methods, stop: () => new Promise<void>((resolve) => server.close(() => resolve())) };
}

test("an older Herdr holds Hide on its move: no key opens a workspace, and a failed move keeps the hold", async ({ page }) => {
  const herdr = await startHerdr();
  const old = await standIn(herdr, "0.8.2", 20);
  const daemon = await startHided(herdr, "gate-older", undefined, { HERDR_SOCKET_PATH: old.socket }, true, { bundledHerdr: true });
  try {
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const gate = page.locator("[data-herdr-gate]");
    // Herdr hands a running server over on Unix only; Windows restarts it (D-13).
    await expect(gate).toHaveAttribute("data-herdr-gate", process.platform === "win32" ? "restart" : "move", { timeout: 60_000 });
    await expect(page.locator("[data-herdr-takeover-later]")).toHaveCount(0);
    await screenshot(page, "herdr-gate-older");

    // The shell is not behind the screen: a new tab's chord, Escape and a click outside reach nothing.
    await page.keyboard.press(chord("new_tab"));
    await page.keyboard.press("Escape");
    await page.mouse.click(5, 300);
    await expect(gate).toBeVisible();
    await expect(page.locator("[data-title-row], [data-sidebar-switch]")).toHaveCount(0);
    expect(old.methods.filter((method) => MAKES.includes(method))).toEqual([]);
    await expect(page.locator("body")).not.toContainText("before workspace creation");

    if (process.platform !== "win32") {
      // The stand-in refuses the handoff: the failure is named, Try again stays, and nothing lets Hide past it.
      await page.locator("[data-herdr-takeover-move]").click();
      await expect(page.locator("[data-herdr-takeover-failure]")).toBeVisible({ timeout: 30_000 });
      await expect(page.locator("[data-herdr-takeover-move]")).toContainText("Try again");
      await expect(page.locator("[data-herdr-takeover-later]")).toHaveCount(0);
      await expect(gate).toBeVisible();
    }
  } finally {
    daemon.stop();
    await old.stop();
  }
});

test("a newer Herdr holds Hide with the wait for a Hide update and the two versions read", async ({ page }) => {
  const herdr = await startHerdr();
  const newer = await standIn(herdr, "0.10.0", 23);
  const daemon = await startHided(herdr, "gate-newer", undefined, { HERDR_SOCKET_PATH: newer.socket }, true, { bundledHerdr: true });
  try {
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const gate = page.locator("[data-herdr-gate]");
    await expect(gate).toHaveAttribute("data-herdr-gate", "update_hide", { timeout: 60_000 });
    await expect(gate).toContainText("Herdr is newer than Hide");
    await expect(page.locator("[data-herdr-gate-versions]")).toHaveAttribute("data-herdr-gate-versions", /^0\.10\.0>\d/);
    await screenshot(page, "herdr-gate-newer");
    await page.keyboard.press(chord("new_tab"));
    await expect(gate).toBeVisible();
    expect(newer.methods.filter((method) => MAKES.includes(method))).toEqual([]);
  } finally {
    daemon.stop();
    await newer.stop();
  }
});
