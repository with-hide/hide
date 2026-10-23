// Active Grok hooks and retired Cursor compatibility on an isolated pinned Herdr and private install kit.
// Grok's switch writes and removes only Hide's file, and its installed commands refuse a direct agent start
// and report subagents. Cursor's installed program produces no switch, user settings stay byte-for-byte,
// and synthetic stale Cursor callbacks are silent (onboarding B31/D-09). Under Grok, only its own hook counts
// and refuses, even alongside Claude Code compatibility and a stale Cursor callback.
// The web shell draws no subagent count (docs/UI_BEHAVIOR.md, Subagent count), so the count is read where the core
// publishes it: the pane's `children.subagents` in the snapshot frames the page receives. Hide's hook commands run from a process that
// is no descendant of the pane's shell, so hided binds them to the registered checkout holding their working
// folder (`hided/src/pane_auth.rs`, `attest_checkout`), which is the fixture's own checkout.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

// The kit writes POSIX hook files and the agents' hook commands are shell commands.
test.skip(process.platform === "win32", "the kit installs POSIX hooks");
test.describe.configure({ timeout: 150_000 });

const read = (file: string) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");
const json = <T = Record<string, unknown>>(file: string): T => JSON.parse(read(file)) as T;
const sample = (name: string) => JSON.parse(fs.readFileSync(path.resolve("..", "hide-agent-hooks", "tests", "fixtures", name), "utf8")) as Record<string, unknown>;

/** What another tool left beside Hide's: Herdr's and Orca's Grok files, odd spacing kept, and the operator's Cursor hook with Orca's. */
const HERDR_FILE = '{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/opt/herdr/hook stop"}]}]}}';
const ORCA_FILE = '{\n\t"hooks": {\n\t\t"SessionStart": [ {"hooks": [{"type": "command", "command": "/opt/orca/hook", "timeout": 3}]} ]\n\t}\n}';
const CURSOR_FILE = { version: 1, hooks: { stop: [{ command: "/opt/mine.sh", timeout: 5 }], sessionStart: [{ command: "/opt/orca/cursor-hook", timeout: 3 }] } };
const CURSOR_BYTES = `${JSON.stringify(CURSOR_FILE, null, "\t")}\n`;

type Stack = { herdr: HerdrFixture; daemon: Daemon; home: string };

/**
 * A private HOME with the agents' folders (`.claude` only for `claude`), the third-party files above and a kit
 * record; `on` seeds Grok on and a retired Cursor preference that must be removed. The programs `grok` and
 * `cursor-agent` are copies of the fixture's agent shim on the fixture PATH, which is the PATH the kit searches
 * and the PATH of every pane, so Herdr classifies a pane that runs one as that agent.
 */
async function start(label: string, options: { on?: boolean; claude?: boolean } = {}): Promise<Stack> {
  const herdr = await startHerdr();
  for (const program of ["grok", "cursor-agent"]) {
    fs.copyFileSync(path.join(herdr.root, "bin", "claude"), path.join(herdr.root, "bin", program));
    fs.chmodSync(path.join(herdr.root, "bin", program), 0o755);
  }
  const home = path.join(fs.mkdtempSync(path.join(herdr.root, "gc-")), "home");
  for (const folder of [".grok/hooks", ".cursor", ...(options.claude ? [".claude"] : [])]) fs.mkdirSync(path.join(home, folder), { recursive: true });
  fs.writeFileSync(path.join(home, ".grok", "hooks", "herdr.json"), HERDR_FILE);
  fs.writeFileSync(path.join(home, ".grok", "hooks", "orca.json"), ORCA_FILE);
  fs.writeFileSync(path.join(home, ".cursor", "hooks.json"), CURSOR_BYTES);
  fs.mkdirSync(path.join(home, ".hide", "kit"), { recursive: true });
  fs.writeFileSync(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [], ...(options.on ? { agents: { grok: true, cursor: true } } : {}) }));
  try {
    return { herdr, daemon: await startHided(herdr, label, home, {}, true), home };
  } catch (error) {
    herdr.stop();
    throw error;
  }
}

type Piece = { state: string; why?: { cause: string; value?: string } | null };
type Subagents = { working: number | null; done: number | null };

/**
 * What the page's snapshot frames say, latest first: each agent's hook piece in the machine's kit, and each
 * pane's `children` (whether Hide hears its hook, and the subagent counts, which no screen draws).
 */
function watchWire(page: Page) {
  const hooks = new Map<string, Piece>();
  const panes = new Map<string, { instrumented: boolean; subagents: Subagents }>();
  type Row = { agents?: { id: string; skill?: unknown; hook?: Piece | null }[]; pane_id?: string; id?: string; children?: { instrumented: boolean; subagents?: Subagents } };
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) return value.forEach(visit);
    if (!value || typeof value !== "object") return;
    const row = value as Row;
    // A machine's kit lists its agents with a skill piece each; a pane row carries `children`.
    if (Array.isArray(row.agents) && row.agents.every((agent) => typeof agent?.skill === "object")) {
      for (const agent of row.agents) if (agent.hook) hooks.set(agent.id, agent.hook);
    }
    const id = row.pane_id ?? row.id;
    if (typeof id === "string" && row.children?.subagents) panes.set(id, { instrumented: row.children.instrumented, subagents: row.children.subagents });
    Object.values(row).forEach(visit);
  };
  page.on("websocket", (socket) =>
    socket.on("framereceived", ({ payload }) => {
      if (typeof payload !== "string") return;
      const frame = JSON.parse(payload) as { type?: string; payload?: unknown };
      if (frame.type === "snapshot" || frame.type === "delta") visit(frame.payload);
    }),
  );
  return {
    hook: (agent: string) => hooks.get(agent),
    pane: (pane: string) => panes.get(pane),
    counts: (pane: string) => [panes.get(pane)?.subagents.working ?? null, panes.get(pane)?.subagents.done ?? null],
  };
}

const stop = ({ herdr, daemon }: Stack) => {
  daemon.stop();
  herdr.stop();
};

const grokFile = (home: string) => path.join(home, ".grok", "hooks", "hide.json");
const cursorFile = (home: string) => path.join(home, ".cursor", "hooks.json");
const capturedState = (daemon: Daemon) => `--state-dir-b64 ${Buffer.from(daemon.stateDir).toString("base64url")}`;

/** Grok's and Claude Code's files group a command in `hooks`. */
type Grouped = { hooks: Record<string, { matcher?: string; hooks: { command: string }[] }[]> };

/** Hide's installed command for `event`, exactly as the agent's own file holds it. */
const installed = {
  grok: (home: string, event: string): string => json<Grouped>(grokFile(home)).hooks[event]![0]!.hooks[0]!.command,
  claude: (home: string, event: string): string => json<Grouped>(path.join(home, ".claude", "settings.json")).hooks[event]![0]!.hooks[0]!.command,
};

/** A synthetic stale entry using the actual helper path, captured state and shell quoting the kit installed for Grok.
 * It is invoked explicitly for retirement proof, never written into Cursor's user settings or called installed support.
 */
const legacyCursor = (home: string, event: string) => installed.grok(home, "PreToolUse")
  .replace("--runtime grok", "--runtime cursor")
  .replace("--event PreToolUse", `--event ${event}`);

/** The active hook files the first pass writes; the retired preference disappears without changing user bytes. */
async function installedEverywhere(home: string, claude: boolean): Promise<void> {
  const files = [grokFile(home), ...(claude ? [path.join(home, ".claude", "settings.json")] : [])];
  await expect.poll(() => files.every((file) => read(file).includes("hide-")), { timeout: 60_000 }).toBe(true);
  await expect.poll(() => json<{ agents?: Record<string, boolean> }>(path.join(home, ".hide", "kit", "installed.json")).agents?.cursor).toBeUndefined();
  expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
}

async function openAgents(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="agents"]').click();
  return page.locator(`[data-agents-machine-list="${daemon.node}"]`);
}

test("Grok's switch installs only its owned file while retired Cursor has no switch and keeps its user bytes", async ({ page }) => {
  const stack = await start("grok-cursor-switch");
  const { daemon, home } = stack;
  const wire = watchWire(page);
  try {
    const list = await openAgents(page, daemon);
    const switchOf = (id: string, state: "on" | "off") => list.locator(`[data-agent-switch="${daemon.node}:${id}:${state}"]`);
    await expect(switchOf("grok", "off")).toBeVisible({ timeout: 60_000 });
    await expect(list.locator('[data-agent-row*="cursor"], [data-agent-switch*="cursor"]')).toHaveCount(0);
    // The active switch routes installed hooks back to the daemon that wrote them.
    await switchOf("grok", "off").click();
    await expect(switchOf("grok", "on")).toBeVisible();
    await expect.poll(() => read(grokFile(home)), { timeout: 60_000 }).toContain(capturedState(daemon));
    expect(Object.keys(json<Grouped>(grokFile(home)).hooks).sort()).toEqual(["PostToolUse", "PostToolUseFailure", "PreToolUse", "SessionStart", "Stop", "SubagentStart", "SubagentStop", "UserPromptSubmit"]);
    expect(json<Grouped>(grokFile(home)).hooks.UserPromptSubmit![0]!.matcher).toBeUndefined();
    expect(installed.grok(home, "UserPromptSubmit")).toContain("--event UserPromptSubmit");
    // B6 (letters-to-busy-agents): the tool hook carries no matcher, so every tool call is a gap a due letter goes at.
    expect(json<Grouped>(grokFile(home)).hooks.PreToolUse![0]!.matcher).toBeUndefined();
    // Third-party Grok files and the complete retired Cursor file retain their exact original bytes.
    expect(read(path.join(home, ".grok", "hooks", "herdr.json"))).toBe(HERDR_FILE);
    expect(read(path.join(home, ".grok", "hooks", "orca.json"))).toBe(ORCA_FILE);
    expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
    // Only the active hook piece is installed; the popover names its current features.
    await expect.poll(() => wire.hook("grok")?.state, { timeout: 60_000 }).toBe("installed");
    expect(wire.hook("cursor")).toBeUndefined();
    await expect(list.locator("[data-agents-check]")).toHaveAttribute("data-agents-check", "idle");
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
    await list.locator('[data-agent-partial="grok"]').click();
    const popover = page.locator('[data-agent-partial-popover="grok"]');
    for (const feature of ["spawn_guard:yes", "letters:no", "bell:yes", "memory:no"]) await expect(popover.locator(`[data-agent-feature="${feature}"]`)).toBeVisible();
    await screenshot(page, "grok-cursor-hooks-on");
    await page.keyboard.press("Escape");

    // B8: off takes out only Hide's Grok file.
    await switchOf("grok", "on").click();
    await expect(switchOf("grok", "off")).toBeVisible();
    await expect.poll(() => fs.existsSync(grokFile(home)), { timeout: 60_000 }).toBe(false);
    expect(fs.readdirSync(path.join(home, ".grok", "hooks")).sort()).toEqual(["herdr.json", "orca.json"]);
    expect(read(path.join(home, ".grok", "hooks", "herdr.json"))).toBe(HERDR_FILE);
    expect(read(path.join(home, ".grok", "hooks", "orca.json"))).toBe(ORCA_FILE);
    expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
  } finally {
    stop(stack);
  }
});

test("a Grok file the operator removed or edited shows on its row until Reinstall, and an agent whose folder is missing gets none and its row says why", async ({ page }) => {
  const stack = await start("grok-cursor-removed", { on: true });
  const { daemon, home } = stack;
  const wire = watchWire(page);
  try {
    const list = await openAgents(page, daemon);
    await expect.poll(() => read(grokFile(home)), { timeout: 60_000 }).toContain(capturedState(daemon));
    const original = json<Grouped>(grokFile(home));
    // B8: the hook the operator took out is not put back; its row says so with Reinstall, on that row only.
    fs.rmSync(grokFile(home));
    await list.locator("[data-agents-check]").click();
    const problem = list.locator(`[data-agent-problem="${daemon.node}:grok"]`);
    await expect(problem).toContainText("Hook: Removed", { timeout: 60_000 });
    await expect(list.locator("[data-agent-problem]")).toHaveCount(1);
    expect(fs.existsSync(grokFile(home))).toBe(false);
    await problem.locator("[data-hook-reinstall]").click();
    await expect.poll(() => (fs.existsSync(grokFile(home)) ? json<Grouped>(grokFile(home)) : null), { timeout: 60_000 }).toEqual(original);
    await expect.poll(() => wire.hook("grok")?.state, { timeout: 60_000 }).toBe("installed");
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
    expect(read(path.join(home, ".grok", "hooks", "herdr.json"))).toBe(HERDR_FILE);

    // B8: an entry the operator edited stays theirs; the row says so with Reinstall, which puts Hide's back.
    const edited = read(grokFile(home)).replace("--event Stop", "--event Stop --verbose");
    fs.writeFileSync(grokFile(home), edited);
    await list.locator("[data-agents-check]").click();
    await expect(problem).toContainText("edited after Hide wrote them", { timeout: 60_000 });
    await expect(list.locator("[data-agent-problem]")).toHaveCount(1);
    expect(read(grokFile(home))).toBe(edited);
    await problem.locator("[data-hook-reinstall]").click();
    // Reinstall restores every event and command, including the prompt observer, regardless of formatting.
    await expect.poll(() => json<Grouped>(grokFile(home)), { timeout: 60_000 }).toEqual(original);
    await expect.poll(() => wire.hook("grok")?.state, { timeout: 60_000 }).toBe("installed");
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
    expect(read(path.join(home, ".grok", "hooks", "herdr.json"))).toBe(HERDR_FILE);
    expect(read(path.join(home, ".grok", "hooks", "orca.json"))).toBe(ORCA_FILE);

    // B8: an agent that is on but has not made its folder gets nothing made, and its row says why in a quiet
    // line without Reinstall (the kit piece's cause, worded with its folder).
    fs.rmSync(path.join(home, ".grok"), { recursive: true });
    await list.locator("[data-agents-check]").click();
    const waiting = list.locator(`[data-agent-hook-wait="${daemon.node}:grok"]`);
    await expect(waiting).toContainText("once the agent makes this folder", { timeout: 60_000 });
    await expect(waiting).toContainText(".grok");
    expect(wire.hook("grok")).toMatchObject({ state: "absent", why: { cause: "not_created_yet" } });
    expect(fs.existsSync(path.join(home, ".grok"))).toBe(false);
    await expect(list.locator('[data-agent-row*="cursor"]')).toHaveCount(0);
    expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
    await expect(list.locator("[data-agent-problem]")).toHaveCount(0);
    await screenshot(page, "grok-cursor-hooks-no-folder");
  } finally {
    stop(stack);
  }
});

/**
 * A pane of its own in the fixture's checkout that runs the stand-in `kind`, which Herdr classifies by the
 * program in the pane's foreground. Returns the pane once the sidebar lists its row.
 */
async function agentPane(page: Page, herdr: HerdrFixture, kind: "grok" | "cursor"): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", path.join(herdr.root, "fixture"), "--label", kind, "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  const screen = () => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "recent", "--lines", "10"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  await expect.poll(screen, { message: `a prompt in pane ${pane}`, timeout: 20_000 }).toContain("fixture %");
  herdr.run(["agent", "start", kind, "--kind", kind, "--pane", pane]);
  await expect(page.locator(`nav[data-sidebar] [data-pane="${pane}"]`)).toBeVisible({ timeout: 30_000 });
  return pane;
}

/** Runs `command` as the agent does (a shell, the payload on stdin) from the fixture's checkout, in the pane's environment. */
function runHook(stack: Stack, pane: string, command: string, payload: unknown, env: Record<string, string>): { stdout: string; stderr: string; status: number | null } {
  const { herdr, daemon } = stack;
  const run = spawnSync("/bin/sh", ["-c", command], {
    cwd: path.join(herdr.root, "fixture"),
    input: typeof payload === "string" ? payload : JSON.stringify(payload),
    env: { PATH: herdr.fixturePath, HOME: daemon.home, HIDE_STATE_DIR: daemon.stateDir, HERDR_SOCKET_PATH: herdr.socket, HERDR_PANE_ID: pane, ...env },
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

const GROK_ENV = (event: string) => ({ GROK_HOOK_EVENT: event, GROK_SESSION_ID: "grok-session" });
const CURSOR_ENV = { CURSOR_VERSION: "2026.10.01" };

test("the installed Grok guard refuses direct starts while retired Cursor callbacks stay silent", async ({ page }) => {
  const stack = await start("grok-cursor-guard", { on: true });
  const { herdr, home } = stack;
  try {
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    await installedEverywhere(home, false);
    const grok = await agentPane(page, herdr, "grok");
    const cursor = await agentPane(page, herdr, "cursor");

    // B2, Grok: Claude Code's envelope with the same reason; the guard's log holds the one refusal.
    const grokCall = (command: string) => ({ ...sample("grok-pre-tool-use.json"), cwd: path.join(herdr.root, "fixture"), toolInput: { command } });
    const start = `herdr agent start worker --kind claude --pane ${grok}`;
    const refused = runHook(stack, grok, installed.grok(home, "PreToolUse"), grokCall(start), GROK_ENV("pre_tool_use"));
    expect(refused.status).toBe(0);
    const answer = JSON.parse(refused.stdout) as { hookSpecificOutput: { hookEventName: string; permissionDecision: string; permissionDecisionReason: string } };
    expect(answer.hookSpecificOutput).toMatchObject({ hookEventName: "PreToolUse", permissionDecision: "deny" });
    expect(answer.hookSpecificOutput.permissionDecisionReason).toContain("hide agent spawn");
    expect(refusals(home)).toEqual(["grok"]);
    expect(runHook(stack, grok, installed.grok(home, "PreToolUse"), grokCall("cargo test"), GROK_ENV("pre_tool_use")).stdout).toBe("");

    // B31: an explicitly invoked stale Cursor entry neither refuses nor emits an allow response.
    const cursorCall = (command: string) => ({ ...sample("cursor-pre-tool-use.json"), cwd: path.join(herdr.root, "fixture"), tool_input: { command, working_directory: path.join(herdr.root, "fixture") } });
    for (const command of [`herdr agent start worker --kind claude --pane ${cursor}`, "cargo test"]) {
      expect(runHook(stack, cursor, legacyCursor(home, "PreToolUse"), cursorCall(command), CURSOR_ENV)).toEqual({ status: 0, stdout: "", stderr: "" });
    }
    expect(refusals(home)).toEqual(["grok"]);
    expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
  } finally {
    stop(stack);
  }
});

test("installed Grok events update counts while a historical Cursor pane stays uninstrumented", async ({ page }) => {
  const stack = await start("grok-cursor-counts", { on: true });
  const { herdr, home } = stack;
  const wire = watchWire(page);
  try {
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    await installedEverywhere(home, false);
    await expect.poll(() => wire.hook("grok")?.state, { timeout: 60_000 }).toBe("installed");
    expect(wire.hook("cursor")).toBeUndefined();
    const grok = await agentPane(page, herdr, "grok");
    const cursor = await agentPane(page, herdr, "cursor");
    const kinds = (herdr.run(["agent", "list"]) as { result: { agents: { pane_id: string; agent: string }[] } }).result.agents;
    expect([grok, cursor].map((pane) => kinds.find((agent) => agent.pane_id === pane)?.agent)).toEqual(["grok", "cursor"]);
    const send = (pane: string, event: string, nativeEvent: string, payload: unknown) => {
      const run = runHook(stack, pane, installed.grok(home, event), payload, GROK_ENV(nativeEvent));
      expect(run.status, run.stderr).toBe(0);
      return run.stdout;
    };

    // B3, Grok: two subagents work; the turn ends with one still running in the background, which stays counted;
    // its own SubagentStop moves it to done.
    send(grok, "SessionStart", "session_start", { sessionId: "grok-session" });
    await expect.poll(() => wire.pane(grok)?.instrumented, { timeout: 20_000 }).toBe(true);
    for (const type of ["explore", "general"]) send(grok, "SubagentStart", "subagent_start", { sessionId: "grok-session", subagentType: type });
    await expect.poll(() => wire.counts(grok), { timeout: 20_000 }).toEqual([2, 0]);
    const background = { sessionId: "grok-session", reason: "end_turn", backgroundTasks: [{ id: "a", type: "subagent", status: "running", agentType: "general" }, { id: "b", type: "shell", status: "running", command: "npm run dev" }] };
    send(grok, "Stop", "stop", background);
    await expect.poll(() => wire.counts(grok), { timeout: 20_000 }).toEqual([1, 0]);
    send(grok, "Stop", "stop", { sessionId: "child", subagentType: "general", backgroundTasks: [] });
    send(grok, "SubagentStop", "subagent_stop", { sessionId: "child", subagentType: "general" });
    await expect.poll(() => wire.counts(grok), { timeout: 20_000 }).toEqual([0, 1]);

    // B31: historical Cursor classification remains readable, but stale session/subagent callbacks do no work.
    for (const event of ["SessionStart", "SubagentStart", "SubagentStop"]) {
      expect(runHook(stack, cursor, legacyCursor(home, event), { conversation_id: "c1", hook_event_name: event, subagent_type: "explore" }, CURSOR_ENV)).toEqual({ status: 0, stdout: "", stderr: "" });
    }
    expect(read(cursorFile(home))).toBe(CURSOR_BYTES);
    await expect.poll(() => wire.pane(cursor)?.instrumented, { timeout: 20_000 }).toBe(false);
    await expect.poll(() => wire.pane(cursor)?.subagents, { timeout: 20_000 }).toMatchObject({ working: null, done: null });
    // The other pane's count was never touched by these.
    expect(wire.counts(grok)).toEqual([0, 1]);
  } finally {
    stop(stack);
  }
});

test("inside Grok, only its own hook counts and refuses beside Claude compatibility and stale Cursor callbacks", async ({ page }) => {
  const stack = await start("grok-cursor-once", { on: true, claude: true });
  const { herdr, home } = stack;
  const wire = watchWire(page);
  try {
    await page.goto(`${stack.daemon.origin}/#token=${stack.daemon.token}`);
    await enterWorkspace(page, "fixture");
    await installedEverywhere(home, true);
    const grok = await agentPane(page, herdr, "grok");
    const cwd = path.join(herdr.root, "fixture");
    const start = `herdr agent start worker --kind claude --pane ${grok}`;
    // Each hook gets its own agent's documented payload for the one event, in the environment Grok gives all three.
    const payloads = (event: string, grokFields: Record<string, unknown>, grokTool?: string) => ({
      claude: { session_id: "grok-session", cwd, hook_event_name: event, tool_name: "Bash", tool_input: { command: start }, agent_type: "explore" },
      cursor: { ...sample("cursor-pre-tool-use.json"), cwd, tool_input: { command: start, working_directory: cwd }, subagent_type: "explore" },
      grok: { ...sample("grok-pre-tool-use.json"), cwd, hook_event_name: event, ...(grokTool ? { toolInput: { command: start } } : {}), sessionId: "grok-session", ...grokFields },
    });
    const everyHook = (event: string, nativeEvent: string, payload: ReturnType<typeof payloads>, agents: readonly ("claude" | "cursor" | "grok")[] = ["claude", "cursor", "grok"]) =>
      agents.map((agent) => ({ agent, run: runHook(stack, grok, agent === "cursor" ? legacyCursor(home, event) : installed[agent](home, event), payload[agent], GROK_ENV(nativeEvent)) }));

    // B4: one subagent start reaches Hide's Claude Code hook and Grok file, as Grok runs them (Cursor's entry has no
    // subagent event). One count results, in the hook's own record and in the core's snapshot, and none prints anything.
    const started = everyHook("SubagentStart", "subagent_start", payloads("SubagentStart", { subagentType: "explore" }), ["claude", "grok"]);
    for (const { agent, run } of started) expect({ agent, status: run.status, stdout: run.stdout }).toEqual({ agent, status: 0, stdout: "" });
    // The pane's record is the only one: no hook counted under another pane.
    const records = path.join(home, ".hide", "agent-hooks", "panes");
    const names = fs.readdirSync(records).filter((name) => name.endsWith(".json"));
    expect(names).toHaveLength(1);
    expect(json(path.join(records, names[0]!))).toMatchObject({ working: 1, done: 0 });
    await expect.poll(() => wire.counts(grok), { timeout: 20_000 }).toEqual([1, 0]);

    // B4: a direct agent start is refused once: only Hide's Grok file answers, and the guard logs one refusal.
    const answers = Object.fromEntries(everyHook("PreToolUse", "pre_tool_use", payloads("PreToolUse", {}, "shell")).map(({ agent, run }) => [agent, run.stdout]));
    expect(answers.claude).toBe("");
    expect(answers.cursor).toBe("");
    expect(JSON.parse(answers.grok!).hookSpecificOutput.permissionDecision).toBe("deny");
    expect(refusals(home)).toEqual(["grok"]);
  } finally {
    stop(stack);
  }
});
