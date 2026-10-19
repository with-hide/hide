// agy's own hook block on an isolated pinned Herdr and a hided that runs the install kit, with a stand-in `agy`
// (PRD antigravity-first-class B1, B2, B3, B9, D-09, D-12, D-18): the Agents switch writes Hide's `hide` block in
// `~/.gemini/config/hooks.json` beside the operator's block, the skill stub in `~/.gemini/config/skills`, and Hide
// AI's read-only folder with its trust, and takes out only Hide's; the operator's own skills under
// `~/.gemini/antigravity-cli/skills` are never touched. The INSTALLED tool command, read from the file the kit wrote
// and run the way agy runs it (agy's payload on stdin), refuses a direct `herdr agent start` in agy's shape and
// leaves every other call to agy's own question.
// The real agy is never run here: it signs in through the operator's browser whenever its home changes (D-18).

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

// The kit writes POSIX hook commands; Windows' `cmd /c` form is the Rust Windows lane's.
test.skip(process.platform === "win32", "the kit installs POSIX hooks");
test.describe.configure({ timeout: 150_000 });

const read = (file: string) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");
const json = <T = Record<string, unknown>>(file: string): T => JSON.parse(read(file)) as T;

/** What the operator keeps in agy's files: a hook block of their own, settings with a trusted folder, and a skill. */
const OPERATOR_HOOKS = { mine: { Stop: [{ type: "command", command: "/opt/mine.sh", timeout: 5 }] } };
const OPERATOR_SETTINGS = { theme: "dark", trustedWorkspaces: ["/opt/mine"] };
const OPERATOR_SKILL = "---\nname: mine\ndescription: the operator's own skill\n---\n";

type Stack = { herdr: HerdrFixture; daemon: Daemon; home: string };

const hooksFile = (home: string) => path.join(home, ".gemini", "config", "hooks.json");
const settingsFile = (home: string) => path.join(home, ".gemini", "antigravity-cli", "settings.json");
const stubFile = (home: string) => path.join(home, ".gemini", "config", "skills", "hide-browser", "SKILL.md");
const operatorSkills = (home: string) => path.join(home, ".gemini", "antigravity-cli", "skills");
const aiFolder = (home: string) => path.join(home, ".hide", "ai", "antigravity-read-only");

/**
 * A private HOME with agy's folders and the operator's files above, and a kit record; `chosen` is the operator's
 * recorded choice for agy, and without one agy is on by default (the operator's amendment to B1/D-14). `agy` is a
 * copy of the fixture's agent shim on the fixture PATH, which is the PATH the kit searches.
 */
async function start(label: string, options: { chosen?: boolean } = {}): Promise<Stack> {
  const herdr = await startHerdr();
  fs.copyFileSync(path.join(herdr.root, "bin", "claude"), path.join(herdr.root, "bin", "agy"));
  fs.chmodSync(path.join(herdr.root, "bin", "agy"), 0o755);
  const home = path.join(fs.mkdtempSync(path.join(herdr.root, "agy-")), "home");
  fs.mkdirSync(path.join(operatorSkills(home), "mine"), { recursive: true });
  fs.mkdirSync(path.dirname(hooksFile(home)), { recursive: true });
  fs.writeFileSync(hooksFile(home), JSON.stringify(OPERATOR_HOOKS));
  fs.writeFileSync(settingsFile(home), JSON.stringify(OPERATOR_SETTINGS));
  fs.writeFileSync(path.join(operatorSkills(home), "mine", "SKILL.md"), OPERATOR_SKILL);
  fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
  fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [], ...(options.chosen === undefined ? {} : { agents: { antigravity: options.chosen } }) }));
  try {
    return { herdr, daemon: await startHided(herdr, label, home, {}, true), home };
  } catch (error) {
    herdr.stop();
    throw error;
  }
}

const stop = ({ herdr, daemon }: Stack) => {
  daemon.stop();
  herdr.stop();
};

/** agy's hook piece as the page's snapshot frames last said it. */
function watchHook(page: Page) {
  let piece: { state: string } | undefined;
  type Row = { agents?: { id: string; skill?: unknown; hook?: { state: string } | null }[] };
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) return value.forEach(visit);
    if (!value || typeof value !== "object") return;
    const row = value as Row;
    if (Array.isArray(row.agents) && row.agents.every((agent) => typeof agent?.skill === "object")) {
      const agy = row.agents.find((agent) => agent.id === "antigravity");
      if (agy?.hook) piece = agy.hook;
    }
    Object.values(row).forEach(visit);
  };
  page.on("websocket", (socket) =>
    socket.on("framereceived", ({ payload }) => {
      if (typeof payload !== "string") return;
      const frame = JSON.parse(payload) as { type?: string; payload?: unknown };
      if (frame.type === "snapshot" || frame.type === "delta") visit(frame.payload);
    }),
  );
  return () => piece?.state;
}

type Block = { hide: Record<string, ({ command: string } | { matcher: string; hooks: { command: string }[] })[]> };

/** Hide's installed tool command, exactly as agy's file holds it. */
const toolCommand = (home: string): string => {
  const group = json<Block>(hooksFile(home)).hide.PreToolUse![0] as { matcher: string; hooks: { command: string }[] };
  return group.hooks[0]!.command;
};

async function openAgents(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  return page.locator(`[data-agents-machine-list="${daemon.node}"]`);
}

test("the agy switch writes Hide's block, skill and Hide AI folder beside the operator's files and takes out only Hide's", async ({ page }) => {
  const stack = await start("agy-switch", { chosen: false });
  const { daemon, home } = stack;
  const hook = watchHook(page);
  try {
    const list = await openAgents(page, daemon);
    const switchOf = (state: "on" | "off") => list.locator(`[data-agent-switch="${daemon.node}:antigravity:${state}"]`);
    await expect(switchOf("off")).toBeVisible({ timeout: 60_000 });
    // B1: agy takes letters and is refused a launch, so its row wears no Basic chip.
    await expect(list.locator('[data-agent-partial="antigravity"]')).toHaveCount(0);

    // B3 and onboarding B23: Hide's block joins the operator's, including passive tool results.
    await switchOf("off").click();
    await expect(switchOf("on")).toBeVisible();
    await expect.poll(() => read(hooksFile(home)), { timeout: 60_000 }).toContain("--runtime antigravity");
    const block = json<Block & typeof OPERATOR_HOOKS>(hooksFile(home));
    expect(block.mine).toEqual(OPERATOR_HOOKS.mine);
    expect(Object.keys(block.hide).sort()).toEqual(["PostToolUse", "PreInvocation", "PreToolUse", "SessionStart", "Stop"]);
    const post = block.hide.PostToolUse![0] as { matcher: string; hooks: { command: string }[] };
    expect(post.matcher).toBe("run_command");
    expect(post.hooks[0]!.command).toContain("--event PostToolUse");
    expect((block.hide.PreToolUse![0] as { matcher: string }).matcher).toBe("run_command|view_file|write_to_file|replace_file_content|multi_replace_file_content");
    // D-12: the stub goes where agy's model reads skills, never into the operator's antigravity-cli folder.
    await expect.poll(() => read(stubFile(home)), { timeout: 60_000 }).toContain("hide browser help");
    expect(fs.readdirSync(operatorSkills(home))).toEqual(["mine"]);
    expect(read(path.join(operatorSkills(home), "mine", "SKILL.md"))).toBe(OPERATOR_SKILL);
    // D-09: Hide AI's folder refuses every tool and is trusted beside the operator's own entry.
    await expect.poll(() => fs.existsSync(path.join(aiFolder(home), ".agents", "hooks.json")), { timeout: 60_000 }).toBe(true);
    await expect.poll(() => json<typeof OPERATOR_SETTINGS>(settingsFile(home)).trustedWorkspaces, { timeout: 60_000 }).toEqual(["/opt/mine", fs.realpathSync(aiFolder(home))]);
    expect(json<typeof OPERATOR_SETTINGS>(settingsFile(home)).theme).toBe("dark");
    await expect.poll(hook, { timeout: 60_000 }).toBe("installed");
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
    await screenshot(page, "agy-hooks-on");

    // B3: off takes out Hide's block, stub, folder and trust, and the operator's files read as they did.
    await switchOf("on").click();
    await expect(switchOf("off")).toBeVisible();
    await expect.poll(() => json(hooksFile(home)), { timeout: 60_000 }).toEqual(OPERATOR_HOOKS);
    await expect.poll(() => fs.existsSync(stubFile(home)), { timeout: 60_000 }).toBe(false);
    await expect.poll(() => fs.existsSync(aiFolder(home)), { timeout: 60_000 }).toBe(false);
    await expect.poll(() => json(settingsFile(home)), { timeout: 60_000 }).toEqual(OPERATOR_SETTINGS);
    expect(fs.readdirSync(operatorSkills(home))).toEqual(["mine"]);
  } finally {
    stop(stack);
  }
});

test("the installed agy tool command refuses a direct agent start in agy's shape and leaves every other call to agy", async ({ page }) => {
  // No recorded choice: agy is on by default, and its block goes in on the first pass.
  const stack = await start("agy-guard");
  const { herdr, home } = stack;
  try {
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect.poll(() => read(hooksFile(home)), { timeout: 60_000 }).toContain("--runtime antigravity");
    const pane = await shellPane(herdr);
    const checkout = path.join(herdr.root, "fixture");
    const call = (name: string, args: Record<string, unknown>) => ({ conversationId: "ade08778-fcdd-4e96-88ba-fd1c4e594fa2", toolCall: { name, args } });

    const refused = runHook(stack, pane, toolCommand(home), call("run_command", { CommandLine: `herdr agent start worker --kind claude --pane ${pane}`, Cwd: checkout }));
    expect(refused.status, refused.stderr).toBe(0);
    const answer = JSON.parse(refused.stdout) as { decision: string; reason: string };
    expect(answer.decision).toBe("deny");
    expect(answer.reason).toContain("hide agent spawn");
    expect(refusals(home)).toEqual(["antigravity"]);

    // Every other path leaves the call to agy's own permission question: ask, never allow.
    for (const payload of [call("run_command", { CommandLine: "cargo test", Cwd: checkout }), call("view_file", { AbsolutePath: path.join(checkout, "README.md") })]) {
      const run = runHook(stack, pane, toolCommand(home), payload);
      expect(run.status, run.stderr).toBe(0);
      expect(JSON.parse(run.stdout)).toEqual({ decision: "ask" });
    }
    expect(refusals(home)).toEqual(["antigravity"]);
  } finally {
    stop(stack);
  }
});

/** A shell pane of its own in the fixture's checkout, once its prompt is drawn. */
async function shellPane(herdr: HerdrFixture): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", path.join(herdr.root, "fixture"), "--label", "agy", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  const screen = () => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "recent", "--lines", "10"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  await expect.poll(screen, { message: `a prompt in pane ${pane}`, timeout: 20_000 }).toContain("fixture %");
  return pane;
}

/** Runs `command` as agy does (a shell, the payload on stdin) from the fixture's checkout, in the pane's environment. */
function runHook(stack: Stack, pane: string, command: string, payload: unknown): { stdout: string; stderr: string; status: number | null } {
  const { herdr, daemon } = stack;
  const run = spawnSync("/bin/sh", ["-c", command], {
    cwd: path.join(herdr.root, "fixture"),
    input: JSON.stringify(payload),
    env: { PATH: herdr.fixturePath, HOME: daemon.home, HIDE_STATE_DIR: daemon.stateDir, HERDR_SOCKET_PATH: herdr.socket, HERDR_PANE_ID: pane },
    encoding: "utf8",
    timeout: 20_000,
  });
  return { stdout: run.stdout, stderr: run.stderr, status: run.status };
}

/** The runtimes of the guard's refusals so far, in order: its own log, one `launch.refused` line each. */
const refusals = (home: string): string[] =>
  read(path.join(home, ".hide", "agent-hooks", "spawn-guard.log"))
    .split("\n")
    .filter((line) => line.includes('"launch.refused"'))
    .map((line) => (JSON.parse(line) as { runtime: string }).runtime);
