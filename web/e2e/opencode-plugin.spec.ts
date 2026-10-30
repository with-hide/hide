// Hide's OpenCode plugin on the pinned Herdr and a hided that runs the install kit (PRD opencode-plugin
// B1, B3, B4, B7, B9, D-13): the kit writes the plugin into the fixture HOME's OpenCode folder, and two
// OpenCode stand-ins (`opencode-host.ts`) load it from there in two panes. A letter from one reaches the
// other's next prompt and is confirmed only once that prompt is stored, a shell call that starts an agent
// through Herdr is refused with the spawn guard's reason, and child sessions move the pane's subagent counts.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { bundledExecutable } from "./bundled-app";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { installOpenCodeHost, openCodeHost, type Host } from "./opencode-host";
import { snapshotCounts } from "./subagent-counts";
import { enterWorkspace } from "./wire";

// The plugin starts its helper by path with no shell, which the kit writes only on macOS and Linux.
test.skip(process.platform === "win32", "the OpenCode plugin is not written on Windows");
test.describe.configure({ timeout: 120_000 });

type Stack = { herdr: HerdrFixture; daemon: Daemon; home: string; sender: Host; recipient: Host };

/**
 * A private HOME that has OpenCode (its program and its configuration folder) and a kit record with OpenCode
 * on and the default agents off, a daemon that runs the kit from the bundle, and a stand-in in each pane once
 * the kit has written the plugin.
 */
async function start(label: string): Promise<Stack> {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const home = path.join(herdr.root, "home");
    const git = (...args: string[]) => execFileSync("git", ["-C", path.join(herdr.root, "fixture"), ...args], { env: { ...process.env, GIT_AUTHOR_NAME: "fixture", GIT_AUTHOR_EMAIL: "fixture@example.invalid", GIT_COMMITTER_NAME: "fixture", GIT_COMMITTER_EMAIL: "fixture@example.invalid" } });
    git("init", "-q", "-b", "main");
    git("commit", "-q", "--allow-empty", "-m", "fixture");
    fs.mkdirSync(path.join(home, ".config", "opencode"), { recursive: true });
    fs.mkdirSync(path.join(home, ".local", "bin"), { recursive: true });
    fs.writeFileSync(path.join(home, ".local", "bin", "opencode"), "#!/bin/sh\n", { mode: 0o755 });
    fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
    fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [], agents: { "claude-code": false, codex: false, opencode: true } }));
    daemon = await startHided(herdr, label, home, {}, true);
    const plugin = path.join(home, ".config", "opencode", "plugins", "hide.js");
    await expect.poll(() => fs.existsSync(plugin), { message: "the kit wrote Hide's OpenCode plugin", timeout: 60_000 }).toBe(true);
    // B1: the plugin names this build's helper, the bundle's.
    expect(fs.readFileSync(plugin, "utf8")).toContain(JSON.stringify(bundledExecutable("hide-agent-hooks")));
    const dir = path.join(herdr.root, "opencode");
    installOpenCodeHost(path.join(herdr.root, "bin"), { herdr: herdr.bin, stateDir: daemon.stateDir, plugin, dir });
    for (const pane of herdr.panes) execFileSync(herdr.bin, ["pane", "run", pane, "opencode"], { env: herdr.env, timeout: 10_000 });
    const [sender, recipient] = await Promise.all(herdr.panes.map((pane) => openCodeHost(dir, pane)));
    return { herdr, daemon, home, sender: sender!, recipient: recipient! };
  } catch (error) {
    daemon?.stop();
    herdr.stop();
    throw error;
  }
}

function stop(stack: Stack): void {
  stack.daemon.stop();
  stack.herdr.stop();
}

/** `hide <args>` run inside `host`'s pane, as its agent would run it, parsed. */
async function hide(host: Host, ...args: string[]): Promise<{ ok?: boolean; result?: Record<string, unknown> }> {
  const ran = await host.send({ op: "shell", argv: [bundledExecutable("hide"), ...args] });
  expect(ran.stdout, JSON.stringify(ran)).not.toBe("");
  return JSON.parse(String(ran.stdout)) as { ok?: boolean; result?: Record<string, unknown> };
}

test("a letter rides the recipient's next prompt and is confirmed only once OpenCode stored it", async ({ page }) => {
  const stack = await start("opencode-letter");
  try {
    // The plugin registered the hooks it works through in a managed pane.
    expect(stack.recipient.hooks).toEqual(expect.arrayContaining(["chat.message", "event", "tool.execute.before"]));
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");

    const sent = await hide(stack.sender, "request", "send", stack.herdr.panes[1], "--intent", "e2e-opencode-letter", "--body", "E2E-LETTER-BODY");
    expect(sent.ok, JSON.stringify(sent)).toBe(true);
    const id = String(sent.result?.id);

    // B3: the next prompt carries the letter as Hide's synthetic part, after the operator's text.
    const prompted = await stack.recipient.send({ op: "prompt", text: "Fix the failing parser test" });
    const parts = prompted.parts as { text: string; synthetic?: boolean }[];
    expect(parts).toHaveLength(2);
    expect(parts[0]!.text).toBe("Fix the failing parser test");
    expect(parts[1]!.synthetic).toBe(true);
    expect(parts[1]!.text).toContain(`Hide letter ${id}`);
    expect(parts[1]!.text).toContain("E2E-LETTER-BODY");

    // B4: until OpenCode reports the part stored, the letter is not confirmed.
    const shown = async () => (await hide(stack.sender, "request", "show", id)).result ?? {};
    expect((await shown()).hook_confirmed).toBe(false);
    await stack.recipient.send({ op: "stored", part: parts[1]! });
    await expect.poll(async () => (await shown()).hook_confirmed, { message: "the stored prompt confirmed the letter", timeout: 20_000 }).toBe(true);

    // A confirmed letter does not ride the prompt after it.
    const next = await stack.recipient.send({ op: "prompt", text: "And the next one" });
    expect(JSON.stringify(next.parts)).not.toContain("E2E-LETTER-BODY");
  } finally {
    stop(stack);
  }
});

test("a shell call that starts an agent through Herdr is refused with the spawn guard's reason, and others run", async () => {
  const stack = await start("opencode-guard");
  try {
    // B7: the model reads the reason as the tool's error.
    const refused = await stack.recipient.send({ op: "tool", tool: "bash", args: { command: "herdr agent start --kind claude --name helper" } });
    expect(String(refused.refused)).toContain("hide agent spawn --parent here");
    expect(String(refused.refused)).toContain("--kind claude");
    const log = fs.readFileSync(path.join(stack.home, ".hide", "agent-hooks", "spawn-guard.log"), "utf8");
    expect(log).toContain('"runtime":"opencode"');

    expect(await stack.recipient.send({ op: "tool", tool: "bash", args: { command: "ls -la" } })).toEqual({ ran: true });
    expect(await stack.recipient.send({ op: "tool", tool: "read", args: { filePath: "/etc/hosts" } })).toEqual({ ran: true });
  } finally {
    stop(stack);
  }
});

// A background child that outlives the root's turn is the plugin test's (`hide-agent-hooks/tests/opencode`):
// across processes nothing marks the moment the root's idle sweep is done.
test("child sessions of the prompted root session move the pane's subagent counts in the snapshot", async ({ page }) => {
  const stack = await start("opencode-counts");
  try {
    const pane = stack.herdr.panes[1];
    // The hook reports to the core, so the counts are what the core puts in the pane's row.
    const counts = snapshotCounts(page, pane);
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The plugin reports zero at load, so Hide hears the pane before any child.
    await expect.poll(counts, { message: "the plugin's first report", timeout: 20_000 }).toEqual({ working: 0, done: 0 });

    // B9: the operator's prompt starts a turn whose task tool runs a child session.
    await stack.recipient.send({ op: "prompt", text: "Explore the parser with a subagent" });
    await stack.recipient.send({ op: "created", id: "ses_child", parent: stack.recipient.session });
    await stack.recipient.send({ op: "status", id: "ses_child", status: "busy" });
    await expect.poll(counts, { message: "one working child", timeout: 20_000 }).toEqual({ working: 1, done: 0 });

    await stack.recipient.send({ op: "status", id: "ses_child", status: "idle" });
    await expect.poll(counts, { message: "the child finished", timeout: 20_000 }).toEqual({ working: 0, done: 1 });
  } finally {
    stop(stack);
  }
});
