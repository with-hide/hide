// Direct Herdr prompts and starts seen by the passive Post observation (PRD
// direct-prompt-observation-widen B1, B4, B6, B7, B10, B14, B16), on an
// isolated pinned Herdr and hided. A Claude Code stand-in in the first pane
// runs the hook helper from inside its own process, as Claude Code runs its
// PostToolUse hook, with the payload of a Bash call that ran the real
// `herdr agent prompt` against the second pane's stand-in. Each proven
// recipient reaches the Overview as a direct-input line; a variable target
// with discarded output and a refused prompt reach nothing. A start the call
// made is recorded as the operator's root with the first agent as origin.
// The spawn guard refuses a start in a registered checkout before it runs;
// this exercises the Post side alone, for a start the guard did not see.

import { expect, test } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { fixtureExecutable } from "./platform-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 180_000 });

type Agent = { pane_id: string; name?: string | null; agent_session?: { value: string } | null };
type DirectInput = { id: string; sender: { pane_id: string }; recipient: { pane_id: string } };

const agents = (herdr: HerdrFixture) => (herdr.run(["agent", "list"]) as { result: { agents: Agent[] } }).result.agents;

/** Herdr's own answer to a CLI call, as the agent's shell would print it: stdout and stderr. */
function herdrCall(herdr: HerdrFixture, args: string[]): { stdout: string; stderr: string } {
  const ran = spawnSync(herdr.bin, args, { env: herdr.env, encoding: "utf8", timeout: 40_000 });
  return { stdout: ran.stdout, stderr: ran.stderr };
}

/** A pane of its own in the fixture's workspace, at its shell prompt. */
async function newPane(herdr: HerdrFixture): Promise<string> {
  const created = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--label", "started", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  const screen = () => execFileSync(herdr.bin, ["pane", "read", pane, "--source", "recent", "--lines", "10"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  await expect.poll(screen, { message: `a prompt in pane ${pane}`, timeout: 20_000 }).toContain("fixture %");
  return pane;
}

test("piped, named and loop-free direct prompts draw lines and an unseen start keeps only its origin", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "direct-observation");
    // The latest overview graph facts the page received.
    let inputs: DirectInput[] = [];
    page.on("websocket", (socket) =>
      socket.on("framereceived", ({ payload }) => {
        if (typeof payload !== "string") return;
        const visit = (value: unknown): void => {
          if (!value || typeof value !== "object") return;
          const graph = (value as { overview_graph?: { direct_inputs?: DirectInput[] } }).overview_graph;
          if (graph?.direct_inputs) inputs = graph.direct_inputs;
          Object.values(value).forEach(visit);
        };
        visit(JSON.parse(payload));
      }),
    );
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const [sender, recipient] = herdr.panes;
    const run = await agentPane(herdr, daemon.stateDir, sender);
    herdr.run(["agent", "start", "reviewer", "--kind", "claude", "--pane", recipient]);
    await expect.poll(() => agents(herdr).find((agent) => agent.pane_id === recipient)?.agent_session?.value ?? null, { timeout: 20_000 }).toBeTruthy();
    const session = agents(herdr).find((agent) => agent.pane_id === sender)!.agent_session!.value;
    await openOverview(page, "work");

    const hooks = path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks"));
    let calls = 0;
    /** The agent's PostToolUse hook for one Bash call: its command and what its shell printed. */
    const post = async (command: string, printed: { stdout: string; stderr: string }) => {
      calls += 1;
      const payload = {
        session_id: session,
        tool_use_id: `toolu_direct_${calls}`,
        tool_name: "Bash",
        hook_event_name: "PostToolUse",
        cwd: path.join(herdr.root, "fixture"),
        tool_input: { command },
        tool_response: { ...printed, interrupted: false },
      };
      const answer = await run(["hook", "--runtime", "claude-code", "--event", "PostToolUse"], { program: hooks, input: JSON.stringify(payload) });
      expect(answer.status).toBe(0);
      return payload;
    };
    const directTo = (pane: string) => inputs.filter((input) => input.sender.pane_id === sender && input.recipient.pane_id === pane);

    // B1: `| jq .` prints Herdr's answer over several lines; the answer names the recipient.
    const piped = herdrCall(herdr, ["agent", "prompt", "reviewer", "first note"]);
    const pretty = { stdout: `${JSON.stringify(JSON.parse(piped.stdout), null, 2)}\n`, stderr: "" };
    const first = await post("herdr agent prompt reviewer 'first note' | jq .", pretty);
    await expect.poll(() => directTo(recipient).length, { message: "a direct input from the piped call", timeout: 20_000 }).toBe(1);

    // B10: the same hook event again changes nothing.
    await run(["hook", "--runtime", "claude-code", "--event", "PostToolUse"], { program: hooks, input: JSON.stringify(first) });

    // B4: the answer was discarded; the pane the command wrote stands in.
    herdrCall(herdr, ["agent", "prompt", recipient, "second note"]);
    await post(`herdr agent prompt ${recipient} "second note" >/dev/null; echo done`, { stdout: "done\n", stderr: "" });
    await expect.poll(() => directTo(recipient).length, { message: "a direct input from the written pane", timeout: 20_000 }).toBe(2);

    // B6, B7: a variable target with no output, and a refused prompt, add nothing;
    // the daemon has recorded each before its hook returns, and the page is checked after the start below.
    await post('herdr agent prompt "$p" "third note" >/dev/null', { stdout: "", stderr: "" });
    await post("herdr agent prompt nobody 'fourth note'", herdrCall(herdr, ["agent", "prompt", "nobody", "fourth note"]));

    // The Overview draws the pair as one direct-input line.
    await expect(page.locator(".og-direct-input").first()).toBeAttached({ timeout: 20_000 });
    await screenshot(page, "direct-prompt-observation-overview");

    // B14: a start whose answer names the pane and its session records the started agent as a root
    // whose origin is the sender; it has no parent and nothing watches it.
    const started = await newPane(herdr);
    const answer = herdrCall(herdr, ["agent", "start", "helper", "--kind", "claude", "--pane", started]);
    const herdrStarted = JSON.parse(answer.stdout) as { result: { agent: { agent_session?: { value: string } | null } } };
    test.info().annotations.push({ type: "start session at hook time", description: herdrStarted.result.agent.agent_session ? "present" : "absent" });
    await post(`herdr agent start helper --kind claude --pane ${started} | jq .`, answer);
    type Record = { id: string; pane: string; parent: string | null; origin: string | null };
    const ledger = async () => JSON.parse((await run(["agent", "list"])).out) as { value: { items: Record[] } };
    await expect.poll(async () => (await ledger()).value.items.find((item) => item.pane === started) ?? null, { message: "the started agent is registered", timeout: 20_000 }).toBeTruthy();
    const { items } = (await ledger()).value;
    const starter = items.find((item) => item.pane === sender)!;
    expect(items.find((item) => item.pane === started)).toMatchObject({ parent: null, origin: starter.id });
    const shown = JSON.parse((await run(["agent", "show", items.find((item) => item.pane === started)!.id])).out) as { value: { watch?: unknown } };
    expect(shown.value.watch ?? null).toBeNull();
    expect(directTo(recipient)).toHaveLength(2);
    expect(inputs.filter((input) => input.recipient.pane_id !== recipient)).toHaveLength(0);
    expect(fs.existsSync(daemon.stateDir)).toBe(true);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
