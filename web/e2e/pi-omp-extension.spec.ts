// Hide's extension for Pi and omp on the pinned Herdr and a hided that runs the install kit (PRD pi-omp-extension
// B1, B3, B5, B7, B9, B16, D-11): the kit writes the extension into the fixture HOME's Pi and omp folders, and a Pi
// stand-in and an omp stand-in (`pi-omp-host.ts`) load it from there in the two panes. A letter from one reaches the
// other's next prompt as a hidden message and is confirmed only once the host wrote that prompt and its reply, a shell
// call that starts an agent through Herdr is refused with the spawn guard's reason, and omp's subagents move the pane's
// counts without taking the pane's letters.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { bundledExecutable } from "./bundled-app";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { installPiOmpHosts, piOmpHost, type Host } from "./pi-omp-host";
import { snapshotCounts } from "./subagent-counts";
import { enterWorkspace } from "./wire";

// The extension starts its helper by path with no shell, which the kit writes only on macOS and Linux.
test.skip(process.platform === "win32", "the Pi and omp extension is not written on Windows");
test.describe.configure({ timeout: 120_000 });

type Stack = { herdr: HerdrFixture; daemon: Daemon; home: string; pi: Host; omp: Host };

/**
 * A private HOME that has Pi and omp (their programs and agent folders) and a kit record with both on and the
 * default agents off, a daemon that runs the kit from the bundle, and Pi in the first pane and omp in the second
 * once the kit has written the extension.
 */
async function start(label: string): Promise<Stack> {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const home = path.join(herdr.root, "home");
    const git = (...args: string[]) => execFileSync("git", ["-C", path.join(herdr.root, "fixture"), ...args], { env: { ...process.env, GIT_AUTHOR_NAME: "fixture", GIT_AUTHOR_EMAIL: "fixture@example.invalid", GIT_COMMITTER_NAME: "fixture", GIT_COMMITTER_EMAIL: "fixture@example.invalid" } });
    git("init", "-q", "-b", "main");
    git("-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "fixture");
    fs.mkdirSync(path.join(home, ".local", "bin"), { recursive: true });
    for (const agent of ["pi", "omp"]) {
      fs.mkdirSync(path.join(home, `.${agent}`, "agent"), { recursive: true });
      fs.writeFileSync(path.join(home, ".local", "bin", agent), "#!/bin/sh\n", { mode: 0o755 });
    }
    fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
    fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [], agents: { "claude-code": false, codex: false, pi: true, omp: true } }));
    daemon = await startHided(herdr, label, home, {}, true);
    const extension = { pi: path.join(home, ".pi", "agent", "extensions", "hide.ts"), omp: path.join(home, ".omp", "agent", "extensions", "hide.ts") };
    for (const file of Object.values(extension)) {
      await expect.poll(() => fs.existsSync(file), { message: `the kit wrote ${file}`, timeout: 60_000 }).toBe(true);
      // B1: the extension names this build's helper, the bundle's.
      expect(fs.readFileSync(file, "utf8")).toContain(JSON.stringify(bundledExecutable("hide-agent-hooks")));
    }
    const dir = path.join(herdr.root, "pi-omp");
    installPiOmpHosts(path.join(herdr.root, "bin"), { herdr: herdr.bin, stateDir: daemon.stateDir, extension, dir });
    execFileSync(herdr.bin, ["pane", "run", herdr.panes[0], "pi"], { env: herdr.env, timeout: 10_000 });
    execFileSync(herdr.bin, ["pane", "run", herdr.panes[1], "omp"], { env: herdr.env, timeout: 10_000 });
    const [pi, omp] = await Promise.all(herdr.panes.map((pane) => piOmpHost(dir, pane)));
    // The core learns each pane's session from its own Herdr subscription, which can trail `herdr agent list`; a
    // mailbox command answers only once it has, so the panes' agents are heard before the spec writes to them.
    for (const host of [pi!, omp!]) {
      await expect.poll(async () => (await hide(host, "inbox")).ok, { message: `hided heard the ${host.agent} pane's session`, timeout: 20_000 }).toBe(true);
    }
    return { herdr, daemon, home, pi: pi!, omp: omp! };
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

type Hidden = { customType: string; content: string; display: boolean; details: { hide: string } };

test("a letter rides each agent's next prompt as a hidden message and is confirmed only once the host wrote it", async ({ page }) => {
  const stack = await start("pi-omp-letter");
  try {
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    // B5: omp names its session by a file path longer than 250 bytes.
    expect(Buffer.byteLength(stack.omp.session)).toBeGreaterThan(250);

    for (const [sender, recipient, pane] of [[stack.pi, stack.omp, stack.herdr.panes[1]], [stack.omp, stack.pi, stack.herdr.panes[0]]] as const) {
      const body = `E2E-LETTER-TO-${recipient.agent.toUpperCase()}`;
      const sent = await hide(sender, "request", "send", pane, "--intent", `e2e-${recipient.agent}-letter`, "--body", body);
      expect(sent.ok, JSON.stringify(sent)).toBe(true);
      const id = String(sent.result?.id);

      // B3: the next prompt carries the letter in Hide's hidden message, which the host keeps off the screen.
      const prompted = (await recipient.send({ op: "prompt", text: "Fix the failing parser test" })).message as Hidden | null;
      expect(prompted, recipient.agent).not.toBeNull();
      expect(prompted!.customType).toBe("hide");
      expect(prompted!.display).toBe(false);
      expect(prompted!.content).toContain(`Hide letter ${id}`);
      expect(prompted!.content).toContain(body);

      // Until the host wrote the prompt and its reply, the letter is not confirmed.
      const shown = async () => (await hide(sender, "request", "show", id)).result ?? {};
      expect((await shown()).hook_confirmed).toBe(false);
      await recipient.send({ op: "written" });
      await expect.poll(async () => (await shown()).hook_confirmed, { message: `${recipient.agent} confirmed the letter`, timeout: 20_000 }).toBe(true);

      // A confirmed letter does not ride the prompt after it.
      const next = (await recipient.send({ op: "prompt", text: "And the next one" })).message as Hidden | null;
      expect(next?.content ?? "").not.toContain(body);
      await recipient.send({ op: "written" });
    }
  } finally {
    stop(stack);
  }
});

test("a shell call that starts an agent through Herdr is refused with the spawn guard's reason, and others run", async () => {
  const stack = await start("pi-omp-guard");
  try {
    for (const host of [stack.pi, stack.omp]) {
      // B7: the model reads the reason as the tool's result.
      const refused = await host.send({ op: "tool", tool: "bash", input: { command: "herdr agent start helper --kind claude" } });
      expect(String(refused.refused), host.agent).toContain("hide agent spawn --parent here");
      expect(String(refused.refused)).toContain("--kind claude");
      expect(await host.send({ op: "tool", tool: "bash", input: { command: "ls -la" } })).toEqual({ ran: true });
      expect(await host.send({ op: "tool", tool: "read", input: { path: "/etc/hosts" } })).toEqual({ ran: true });
    }
    const log = fs.readFileSync(path.join(stack.home, ".hide", "agent-hooks", "spawn-guard.log"), "utf8");
    expect(log).toContain('"runtime":"pi"');
    expect(log).toContain('"runtime":"omp"');
  } finally {
    stop(stack);
  }
});

test("omp's subagents move the pane's counts in the snapshot, and their prompts take no letters", async ({ page }) => {
  const stack = await start("pi-omp-counts");
  try {
    const pane = stack.herdr.panes[1];
    // The hook reports to the core, so the counts are what the core puts in the pane's row.
    const counts = snapshotCounts(page, pane);
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The extension reports zero when the session starts, so Hide hears the pane before any subagent.
    await expect.poll(counts, { message: "the extension's first report", timeout: 20_000 }).toEqual({ working: 0, done: 0 });

    const sent = await hide(stack.pi, "request", "send", pane, "--intent", "e2e-omp-sub-letter", "--body", "E2E-LETTER-FOR-THE-PANE");
    expect(sent.ok, JSON.stringify(sent)).toBe(true);

    // B9: the parent dispatches a task; the subagent counts as working from dispatch until its turn ends.
    await stack.omp.send({ op: "spawn", id: "MassMoth" });
    await expect.poll(counts, { message: "one working subagent", timeout: 20_000 }).toEqual({ working: 1, done: 0 });
    await stack.omp.send({ op: "sub", id: "MassMoth", step: "start" });
    const subPrompt = await stack.omp.send({ op: "sub", id: "MassMoth", step: "prompt", text: "Complete assignment thoroughly:\n\nSay ok." });
    expect(subPrompt.message, "a subagent's prompt carries no letters or guidance").toBeNull();
    await stack.omp.send({ op: "sub", id: "MassMoth", step: "end" });
    await expect.poll(counts, { message: "the subagent finished", timeout: 20_000 }).toEqual({ working: 0, done: 1 });

    // The letter waited for the pane's own next prompt.
    const prompted = (await stack.omp.send({ op: "prompt", text: "What did it find?" })).message as Hidden | null;
    expect(prompted?.content ?? "").toContain("E2E-LETTER-FOR-THE-PANE");
  } finally {
    stop(stack);
  }
});
