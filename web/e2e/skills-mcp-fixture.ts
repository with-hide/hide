// A machine whose skills and MCP servers Settings › 스킬·MCP shows (PRD
// skill-mcp-manager): an isolated HOME where the seven supported agents are
// installed and set up, local git repositories a skill installs from over
// `file://`, and a hided that runs the install kit, so every store pass is the
// real kit's on that HOME and nothing reaches the operator's own folders.

import { expect, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import type { Rest } from "./hided-control.mjs";
import { copyFixtureShim } from "./shims/build";
import { enterWorkspace } from "./wire";

/** The seven agents the store reaches, by id. */
export const AGENTS = ["claude-code", "codex", "grok", "opencode", "pi", "omp", "antigravity"] as const;

/** A store change as the screen sends it (`web/src/snapshot.ts` `StoreChange`). */
export type Change = Record<string, unknown> & { op: string };

type Answer = { request_id: string; state: string; outcomes: { refused?: { cause: string } | null }[] };
type Report = {
  items: { id: string; name: string; kind: string; update?: unknown; cells: { column: string; state: string }[] }[];
  previews: { id: string; at_unix_ms: number; skills: { name: string }[] }[];
  mcp_previews?: { id: string; at_unix_ms: number }[];
  trash: { id: string; name: string }[];
  unmanaged: { name: string }[];
};

export type StoreFixture = { engine: AgentFixture; daemon: Daemon; home: string; root: string; repos: { archify: string; pdf: string; pdfWork: string }; folder: string };

const GIT = ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", "-c", "init.defaultBranch=main"];
const git = (cwd: string, ...args: string[]) => execFileSync("git", [...GIT, ...args], { cwd, stdio: "pipe" });

function write(file: string, text: string, mode?: number) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text, mode === undefined ? undefined : { mode });
}

const skill = (name: string, description: string) => `---\nname: ${name}\ndescription: "${description}"\n---\n\n${description}.\n`;

/** A repository with `files`, committed (and tagged), and its bare clone a source reads over `file://`. */
function repository(root: string, name: string, files: Record<string, string>, tag?: string): { url: string; work: string } {
  const work = path.join(root, `${name}-work`);
  for (const [file, text] of Object.entries(files)) write(path.join(work, file), text, file.endsWith(".py") || file.endsWith(".sh") ? 0o755 : undefined);
  git(work, "init", "-q", ".");
  git(work, "add", ".");
  git(work, "commit", "-q", "-m", "first");
  if (tag) git(work, "tag", tag);
  const bare = path.join(root, `${name}.git`);
  git(root, "clone", "-q", "--bare", work, bare);
  git(work, "remote", "add", "origin", bare);
  return { url: `file://${bare}`, work };
}

/**
 * The HOME and sources: every agent's program in `~/.local/bin` and its own
 * folder made, so each column is set up; archify (tagged, with an executable
 * script and a second skill to pick), pdf-tools (a bare commit, updated later)
 * and a folder skill to follow.
 */
export async function startStore(label: string): Promise<StoreFixture> {
  const engine = await startFixture();
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(engine.root, "st-")));
  const home = path.join(root, "home");
  for (const folder of [".claude", ".agents", ".codex", ".grok", ".config/opencode", ".pi/agent", ".omp/agent", ".gemini/config", ".local/bin", ".hide/kit"]) fs.mkdirSync(path.join(home, folder), { recursive: true });
  for (const program of ["codex", "grok", "opencode", "pi", "omp", "agy", "antigravity", "gemini"]) write(path.join(home, ".local", "bin", program), "#!/bin/sh\n", 0o755);
  copyFixtureShim("claude-shim", path.join(home, ".local", "bin", "claude"));
  write(path.join(home, ".hide", "kit", "installed.json"), JSON.stringify({ format: 1, installed: [] }));
  const archify = repository(root, "archify", {
    "archify/SKILL.md": skill("archify", "Draws architecture diagrams"),
    "archify/scripts/render.py": "#!/usr/bin/env python3\nprint('render')\n",
    "archify/templates/base.svg": "<svg/>\n",
    "archify-export/SKILL.md": skill("archify-export", "Exports a diagram"),
    "archify-legacy/SKILL.md": skill("archify-legacy", "The old renderer"),
  }, "v0.4.1");
  const pdf = repository(root, "pdf-tools", { "SKILL.md": skill("pdf-tools", "Merges and splits PDF files") });
  const folder = path.join(root, "mine", "aside-browser");
  write(path.join(folder, "SKILL.md"), skill("aside-browser", "Does web work in the Aside browser"));
  write(path.join(folder, "scripts", "open.sh"), "#!/bin/sh\necho open\n", 0o755);
  const daemon = await startHided(engine, label, home, {}, true);
  return { engine, daemon, home, root, repos: { archify: archify.url, pdf: pdf.url, pdfWork: pdf.work }, folder };
}

export function stopStore(fixture: StoreFixture) {
  fixture.daemon.stop();
  fixture.engine.stop();
}

function deviceKit(rest: Rest, node: string): { store?: Report | null; agents?: { id: string; enabled: boolean }[] } | undefined {
  const devices = (rest.navigator as { devices?: { id: string; kit?: { store?: Report | null; agents?: { id: string; enabled: boolean }[] } }[] } | undefined)?.devices;
  return devices?.find((device) => device.id === node)?.kit;
}

/** The machine's store report once `ready` holds for it. */
export async function storeWhen(daemon: Daemon, ready: (report: Report) => boolean, what: string): Promise<Report> {
  return withControl(daemon, (control) => control.until((rest) => {
    const report = deviceKit(rest, daemon.node)?.store;
    return report && ready(report) ? report : null;
  }, what, 60_000));
}

/** One store request, as one press sends it, answered with no change refused; the report it left. */
export async function store(daemon: Daemon, changes: Change[]): Promise<Report> {
  const id = `e2e-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  return withControl(daemon, async (control) => {
    control.send("kit_store", { device_id: daemon.node, request_id: id, changes });
    return control.until((rest) => {
      const answers = (rest.status as { store_answers?: Answer[] } | undefined)?.store_answers;
      const answer = answers?.find((row) => row.request_id === id);
      if (!answer || answer.state === "pending") return null;
      if (answer.state !== "done" || answer.outcomes.some((outcome) => outcome.refused)) throw new Error(`store request refused: ${JSON.stringify(answer)}`);
      return deviceKit(rest, daemon.node)?.store ?? null;
    }, `store answer ${id}`, 90_000);
  });
}

/** Every agent switched on in Settings › Agents, so each has its column. */
export async function everyAgentOn(daemon: Daemon) {
  await withControl(daemon, async (control) => {
    for (const agent of AGENTS) control.send("kit_agent_set", { device_id: daemon.node, agent, enabled: true });
    await control.until((rest) => {
      const agents = deviceKit(rest, daemon.node)?.agents ?? [];
      return AGENTS.every((id) => agents.find((agent) => agent.id === id)?.enabled) && deviceKit(rest, daemon.node)?.store;
    }, "every agent on with a store report", 90_000);
  });
}

/** A skill source read and the skills `names` of it installed. */
export async function install(daemon: Daemon, source: Record<string, string>, names: string[]): Promise<Report> {
  const read = await store(daemon, [{ op: "inspect", source }]);
  const preview = [...read.previews].sort((a, b) => b.at_unix_ms - a.at_unix_ms)[0];
  if (!preview) throw new Error(`no preview of ${JSON.stringify(source)}`);
  return store(daemon, [{ op: "install", preview: preview.id, names }]);
}

/**
 * The machine of the mockup's scene 1-3: archify on everywhere, the folder
 * skill off for agy, pdf-tools off with a newer commit found, context7 on
 * everywhere, github with a key and a Grok file it cannot read, linear over
 * SSE that Codex and Pi do not take, and a skill Claude holds by hand.
 */
export async function seedStore(fixture: StoreFixture) {
  const { daemon, home } = fixture;
  await everyAgentOn(daemon);
  await install(daemon, { kind: "git", url: fixture.repos.archify }, ["archify"]);
  await install(daemon, { kind: "folder", path: fixture.folder }, ["aside-browser"]);
  await install(daemon, { kind: "git", url: fixture.repos.pdf }, ["pdf-tools"]);
  await store(daemon, [{ op: "set", item: "skill:aside-browser", column: "antigravity", on: false }, { op: "set", item: "skill:pdf-tools", on: false }]);
  await store(daemon, [{ op: "add_mcp", server: { name: "context7", reach: { kind: "local", command: "npx", args: ["-y", "@upstash/context7-mcp"], env: [] } }, keys: {} }]);
  await store(daemon, [{ op: "add_mcp", server: { name: "linear", reach: { kind: "remote", transport: "sse", url: "https://mcp.linear.app/sse", headers: ["Authorization"] } }, keys: { Authorization: "Bearer e2e-fixture-value" } }]);
  // A Grok file Hide cannot read leaves that one cell failed (B26).
  write(path.join(home, ".grok", "config.toml"), "this is = = not toml\n");
  const github = { name: "github", reach: { kind: "local", command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: ["GITHUB_TOKEN"] } };
  const id = `e2e-github-${Date.now().toString(36)}`;
  await withControl(daemon, async (control) => {
    control.send("kit_store", { device_id: daemon.node, request_id: id, changes: [{ op: "add_mcp", server: github, keys: { GITHUB_TOKEN: "e2e-fixture-value" } }] });
    await control.until((rest) => (rest.status as { store_answers?: Answer[] } | undefined)?.store_answers?.find((row) => row.request_id === id && row.state !== "pending"), "github added", 90_000);
  });
  // A skill Claude and the shared folder hold that Hide did not write (B19).
  for (const folder of [".claude/skills", ".agents/skills"]) write(path.join(home, folder, "ast-grep", "SKILL.md"), skill("ast-grep", "Searches code by its syntax"));
  // pdf-tools moves on upstream, and the check finds it (B36).
  write(path.join(fixture.repos.pdfWork, "scripts", "merge.py"), "#!/usr/bin/env python3\nprint('merge')\n", 0o755);
  write(path.join(fixture.repos.pdfWork, "SKILL.md"), `${skill("pdf-tools", "Merges and splits PDF files")}\nUse merge.py to merge.\n`);
  git(fixture.repos.pdfWork, "add", ".");
  git(fixture.repos.pdfWork, "commit", "-q", "-m", "Add merge command");
  git(fixture.repos.pdfWork, "push", "-q", "origin", "main");
  await store(daemon, [{ op: "check_updates" }]);
  return storeWhen(daemon, (report) => report.items.some((item) => item.name === "pdf-tools" && item.update) && report.unmanaged.some((row) => row.name === "ast-grep"), "the update and the hand-made skill");
}

/** Settings › 스킬·MCP of this machine, opened on the fixture's workspace. */
export async function openStore(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="skills"]').click();
  const machine = page.locator(`[data-store-machine="${daemon.node}"]`);
  await expect(machine).toBeVisible();
  return machine;
}
