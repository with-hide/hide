// An external tool's journey through the real `hide` (PRD
// hide-external-control c1, B1-B4, B21): a program outside every pane, with
// a home of its own and no pane identity, asks to connect, the operator's
// window approves it at Act, and the tool starts a stand-in agent with no
// parent, writes to it, reads the agent's reply in its inbox once, asks the
// same letter again, types into the agent's pane and ends it.

import { expect, test } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { connectControl } from "./hided-control.mjs";
import { startHided, type Daemon } from "./hided-fixture";
import { claudeProjects, writeFixtureTranscript } from "./pane-fixture";
import { fixtureExecutable } from "./platform-fixture";

test.describe.configure({ timeout: 240_000 }); // wait-ok: a fixture with agent panes and a daemon, started and stopped whole

type Answer = { ok: boolean; value?: Record<string, unknown>; result?: Record<string, unknown>; error?: { code?: string; message?: string }; reason?: string };
type Listed = { id: string; pane?: string; origin_tool?: string; life?: string };

/**
 * The built `hide` as a tool runs it: in its own home, with that home as
 * HOME, no `HIDE_*` of this process but the daemon's state folder, and so no
 * pane to run as.
 */
function toolRunner(daemon: Daemon, home: string): (args: string[]) => Answer {
  const env: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (!key.toUpperCase().startsWith("HIDE_") && key !== "XDG_STATE_HOME") env[key] = value;
  }
  Object.assign(env, { HOME: home, HIDE_STATE_DIR: daemon.stateDir });
  const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
  return (args) => {
    const ran = spawnSync(hide, args, { cwd: home, env, encoding: "utf8", timeout: 30_000 }); // wait-ok: one hide command's answer, bounded so a hung daemon fails the spec
    const line = `${ran.stdout ?? ""}`.trim().split("\n").at(-1) ?? "";
    try {
      return JSON.parse(line) as Answer;
    } catch {
      return { ok: false, error: { code: `exit ${ran.status}`, message: `${ran.stdout}${ran.stderr}` } };
    }
  };
}

function ok(answer: Answer, what: string): Record<string, unknown> {
  expect(answer.ok, `${what}: ${JSON.stringify(answer)}`).toBe(true);
  return (answer.value ?? answer.result) as Record<string, unknown>;
}

test("a tool outside Hide's panes is approved at Act, starts an agent, writes to it, reads its reply once and ends it", async () => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  const home = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hde-tool-")));
  try {
    daemon = await startHided(fixture, "access-tool");
    const live = daemon;
    const tool = toolRunner(live, home);

    // B1-B4: the request waits for the operator, the window approves it, and the same request run again holds the key.
    const asked = ok(tool(["access", "request", "--name", "nightly", "--json"]), "request");
    expect(asked.state).toBe("pending");
    const window = await connectControl({ origin: live.origin, token: live.token, clientKind: "web" });
    try {
      window.send("access_approve", { id: asked.id as string, level: "act" });
      await window.until((rest) => (rest as { access?: { connections?: { id: string }[] } }).access?.connections?.some((connection) => connection.id === asked.id), "the tool connected");
    } finally {
      window.close();
    }
    const connected = ok(tool(["access", "request", "--name", "nightly", "--json"]), "request again");
    expect(connected).toMatchObject({ state: "connected", level: "act" });

    // B21: an agent with no parent, its pane in the fixture's checkout, on a session the stand-in resumes.
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));
    const session = `tool-helper-${process.pid}`;
    writeFixtureTranscript(claudeProjects(fixture), session, { task: "Tool helper", at: Date.now() });
    const spawn = ["agent", "spawn", "--name", "helper", "--intent", "help", "--kind", "claude", "--repo", repo, "--branch", "helper-task", "--", "--resume", session];
    let pane = "";
    const deadline = Date.now() + 30_000;
    for (;;) {
      const spawned = tool(spawn);
      if (spawned.ok) {
        pane = ok(spawned, "spawn").pane as string;
        break;
      }
      // A child started fresh reports its session after the spawn answers; the same intent then answers it (docs/delivery.md).
      expect(spawned.error?.code, JSON.stringify(spawned)).toBe("native_identity_unavailable");
      expect(Date.now() < deadline, "the helper never reported its session").toBe(true);
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    const listed = ok(tool(["agent", "list"]), "agent list").items as Listed[];
    const helper = listed.find((agent) => agent.pane === pane);
    expect(helper, JSON.stringify(listed)).toBeDefined();
    expect(helper?.origin_tool).toBe("nightly");
    const helperId = helper!.id;

    // A letter to it, and the same intent again is the same letter.
    const sent = ok(tool(["request", "send", helperId, "--intent", "look", "--body", "please look at the fixture"]), "send");
    const again = ok(tool(["request", "send", helperId, "--intent", "look", "--body", "please look at the fixture"]), "send again");
    expect(again.id).toBe(sent.id);

    // The agent answers from its pane; the reply waits in the tool's inbox, once.
    const reply = await fixture.runAs(pane)(["request", "reply", sent.id as string, "--intent", "looked", "--body", "looked at the fixture"]);
    expect(reply.status, reply.out).toBe(0);
    let inbox: Record<string, unknown> = {};
    await expect.poll(() => {
      inbox = ok(tool(["inbox"]), "inbox");
      return JSON.stringify(inbox);
    }, { message: "the reply in the tool's inbox", timeout: 20_000 }).toContain("looked at the fixture"); // wait-ok: the reply the store wrote, read by the tool's next inbox
    // The one command that acknowledges what this answer listed: `hide inbox --ack <bundle>`.
    const bundle = `${inbox.ack_command ?? ""}`.split(" ").at(-1) ?? "";
    expect(inbox.ack_command, JSON.stringify(inbox)).toBe(`hide inbox --ack ${bundle}`);
    ok(tool(["inbox", "--ack", bundle]), "inbox --ack");
    expect(JSON.stringify(ok(tool(["inbox"]), "inbox after ack"))).not.toContain("looked at the fixture");

    // It types into the agent's pane, and ends the agent.
    ok(tool(["pane", "send", pane, "hello from the tool"]), "pane send");
    ok(tool(["agent", "end", helperId]), "agent end");
    const after = ok(tool(["agent", "list"]), "agent list after end").items as Listed[];
    expect(after.some((agent) => agent.id === helperId)).toBe(false);
  } finally {
    daemon?.stop();
    fixture.stop();
    fs.rmSync(home, { recursive: true, force: true });
  }
});
