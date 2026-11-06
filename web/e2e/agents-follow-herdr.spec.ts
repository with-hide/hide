// A Mac an earlier build put Gemini CLI on loses only Hide's Gemini pieces on the first run and
// lists no Gemini row (PRD agent-list-follows-herdr, B31 retirement). Every step runs against the core's own
// engine and a hided that runs the install kit.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startEngine } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";

// The kit's agent pieces are POSIX files and hooks; the guidance hooks are not written on Windows.
test.skip(process.platform === "win32", "the kit installs POSIX hooks");
test.describe.configure({ timeout: 120_000 });

const read = (file: string) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");

/** A private HOME with `programs` in `~/.local/bin` (which the kit searches), `folders` made, and `files` written. */
async function start(label: string, programs: string[], folders: string[], files: Record<string, string>) {
  const herdr = await startEngine();
  const home = path.join(fs.mkdtempSync(path.join(herdr.root, "fh-")), "home");
  for (const folder of [".local/bin", ...folders]) fs.mkdirSync(path.join(home, folder), { recursive: true });
  for (const program of programs) fs.writeFileSync(path.join(home, ".local", "bin", program), "#!/bin/sh\n", { mode: 0o755 });
  for (const [file, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(home, file)), { recursive: true });
    fs.writeFileSync(path.join(home, file), text);
  }
  const daemon = await startHided(herdr, label, home, {}, true);
  return { herdr, daemon, home };
}

async function openAgents(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  return page.locator(`[data-agents-machine-list="${daemon.node}"]`);
}

const RECORD = ".hide/kit/installed.json";
// The stub an earlier build wrote there, which the kit now takes out.
const OLD_SHARED_SKILL = ".agents/skills/hide-browser/SKILL.md";

/** What an earlier build left on a Mac it put Gemini CLI on, in the shapes that build wrote. */
const GEMINI_SETTINGS = JSON.stringify({
  security: { auth: { selectedType: "oauth-personal" } },
  hooks: {
    SessionStart: [
      { matcher: "startup", hooks: [{ type: "command", command: "/opt/mine.sh" }] },
      {
        matcher: "*",
        hooks: [{ name: "hide-guidance", type: "command", command: "if [ -x '/kit/hide-agent-hooks' ]; then exec '/kit/hide-agent-hooks' hook --runtime gemini-cli --event SessionStart --source hide-guidance@1; fi", timeout: 8000 }],
      },
    ],
  },
});
const OLD_STUB = `---\nname: hide-browser\ndescription: Read and drive a browser display inside Hide. Use when a task involves a web page, a local app in a browser tab, or checking what a page shows.\n---\n\n<!-- hide-skill@1: written by Hide; remove it from Settings, Agents -->\n\nRun \`hide browser help\` and follow what it prints. It explains how to open a page, read it by refs, check a change with \`--diff\`, and what to do when a command fails.\n`;

test("a Mac an earlier build put Gemini CLI on loses only Hide's Gemini pieces and lists no Gemini row", async ({ page }) => {
  const sessions = ".gemini/tmp/project/chats/session-1.json";
  const { herdr, daemon, home } = await start("follow-gemini", ["claude", "gemini"], [".claude"], {
    ".gemini/settings.json": GEMINI_SETTINGS,
    ".gemini/oauth_creds.json": '{"token":"synthetic"}',
    [sessions]: '{"messages":[]}',
    [OLD_SHARED_SKILL]: OLD_STUB,
    [RECORD]: JSON.stringify({ format: 1, installed: ["hook:gemini-cli", "skill:agents"], agents: { "gemini-cli": true } }),
  });
  try {
    // B2: the first run takes Hide's hook group out of settings.json; the operator's hook and settings stay.
    const settings = path.join(home, ".gemini", "settings.json");
    await expect.poll(() => read(settings), { timeout: 60_000 }).not.toContain("hide-guidance");
    const left = JSON.parse(read(settings));
    expect(left.hooks.SessionStart).toEqual([{ matcher: "startup", hooks: [{ type: "command", command: "/opt/mine.sh" }] }]);
    expect(left.security.auth.selectedType).toBe("oauth-personal");
    expect(read(path.join(home, ".gemini", "oauth_creds.json"))).toBe('{"token":"synthetic"}');
    expect(read(path.join(home, sessions))).toBe('{"messages":[]}');
    // B3: no agent that is on reads the shared folder, so Hide's stub goes; the record no longer names Gemini.
    await expect.poll(() => fs.existsSync(path.join(home, OLD_SHARED_SKILL)), { timeout: 60_000 }).toBe(false);
    await expect.poll(() => read(path.join(home, RECORD)), { timeout: 60_000 }).not.toContain("gemini");

    // B1, B8, B31: seven active agents, with neither retired Gemini CLI nor Cursor in the list.
    const list = await openAgents(page, daemon);
    await expect(list.locator("[data-agent-row]")).toHaveCount(7, { timeout: 60_000 });
    await expect(list.locator('[data-agent-row*="cursor"]')).toHaveCount(0);
    await expect(list.locator('[data-agent-row*="gemini"]')).toHaveCount(0);
    await expect(list).not.toContainText("Gemini");
    await expect(page.locator("body")).not.toContainText("from the screen");
    await expect(list.locator("[data-agents-check]")).toHaveAttribute("data-agents-check", "idle");
    await screenshot(page, "agents-after-gemini-retired");
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
