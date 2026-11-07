// A mission node's worker reads the letter its node's reviewer wrote, on the
// core's own engine and an isolated hided, through the real `hide` command
// from each agent's pane (with-hide/hide-private#656): the worker of the node
// reads it with `hide request show`, another node's worker is refused, and
// reading is all the worker is given.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import type { HideAnswer } from "./agent-pane";
import { startFixture } from "./engine-fixture";
import { closePane, withControl } from "./daemon-actions";
import { spawnChild } from "./fixture-agents";
import { claudeProjects, writeFixtureTranscript } from "./pane-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 240_000 });

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; reason?: string; error?: { code?: string }; value?: Record<string, unknown>; result?: Record<string, unknown> };

/** The last JSON line one `hide` command printed. */
async function answer(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  if (!line) return { ok: ran.status === 0, reason: ran.status === 0 ? undefined : `exit ${ran.status}` };
  return JSON.parse(line) as Answer;
}

async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const got = await answer(hide, args);
  expect(got.ok, `${args.join(" ")}: ${JSON.stringify(got)}`).toBe(true);
  return got;
}

/** What a failed command refused with, wherever the answer keeps the code. */
function refusal(got: Answer): string | undefined {
  return got.ok ? undefined : (got.error?.code ?? got.reason);
}

/** The id a send answered with. */
function idOf(got: Answer): string {
  return ((got.value ?? got.result) as { id: string }).id;
}

test("the worker of a mission node reads its node's reviewer's letter and another node's worker is refused", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "mission-letters");
    const live = daemon;
    const [lead, beside] = fixture.panes;
    // The Conductor is alone in its project, as a mission's lead is.
    await withControl(live, (control) => closePane(control, beside));
    const conductor = fixture.runAs(lead);
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));
    const conductorId = idOf(await ok(conductor, ["agent", "show", "here"]));

    // Three agents the Conductor delegated to, each with the conversation file a real Claude writes.
    const agents: Record<"worker" | "reviewer" | "neighbor", { id: string; pane: string }> = {} as never;
    for (const name of ["worker", "reviewer", "neighbor"] as const) {
      const session = `${name}-session-${process.pid}`;
      writeFixtureTranscript(claudeProjects(fixture), session, { task: `Mission ${name}`, at: Date.now() });
      const pane = await spawnChild(fixture, live, lead, { name, repo, branch: `${name}-task`, session });
      const listed = (await ok(conductor, ["agent", "list"])).value as { items: { id: string; pane: string }[] };
      agents[name] = { id: listed.items.find((agent) => agent.pane === pane)!.id, pane };
    }
    const as = (name: keyof typeof agents) => fixture.runAs(agents[name].pane);

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // One mission with two nodes: `fix` is worked by the worker and checked by the reviewer, `docs` by the neighbor.
    const check = { as: "review", by: [{ agent: "claude", model: "opus" }] };
    await expect.poll(async () => (await conductor(["mission", "create", "--title", "편지 확인"])).out, { timeout: 20_000 }).toContain('"ok":true');
    await ok(conductor, ["mission", "plan", "--json", JSON.stringify({ scope: "pr", check: "true", nodes: [
      { key: "fix", title: "고치기", kind: "change", delivery: "pr", criteria: ["고쳐졌다"], checks: [check] },
      { key: "docs", title: "문서", kind: "change", delivery: "pr", criteria: ["적었다"] },
    ] })]);
    await ok(conductor, ["mission", "start"]);
    await ok(conductor, ["mission", "node", "fix", "--status", "working", "--agent-id", agents.worker.id, "--reviewer", agents.reviewer.id]);
    await ok(conductor, ["mission", "node", "docs", "--status", "working", "--agent-id", agents.neighbor.id]);

    // The reviewer writes its verdict to the Conductor, and the worker reads it.
    const verdict = idOf(await ok(as("reviewer"), ["request", "send", conductorId, "--intent", "verdict", "--kind", "report", "--body", "리뷰 결과: 고칠 곳 하나"]));
    const read = await ok(as("worker"), ["request", "show", verdict]);
    expect(JSON.stringify(read)).toContain("리뷰 결과: 고칠 곳 하나");
    // Another node's worker is refused, and the letter's own two parties read it as before.
    expect(refusal(await answer(as("neighbor"), ["request", "show", verdict]))).toBe("letter_unavailable");
    expect((await answer(as("reviewer"), ["request", "show", verdict])).ok).toBe(true);
    expect((await answer(conductor, ["request", "show", verdict])).ok).toBe(true);
    // Reading is all the node's worker is given.
    expect(refusal(await answer(as("worker"), ["request", "ack", verdict]))).toBe("letter_unavailable");
    expect(refusal(await answer(as("worker"), ["request", "cancel", verdict]))).toBe("letter_unavailable");
    // The grant runs one way: the reviewer reads nothing of the worker's but what was sent to it.
    const reported = idOf(await ok(as("worker"), ["request", "send", conductorId, "--intent", "fix-done", "--kind", "report", "--body", "작업을 마쳤습니다"]));
    expect(refusal(await answer(as("reviewer"), ["request", "show", reported]))).toBe("letter_unavailable");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
