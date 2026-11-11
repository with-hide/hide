// A mission's destination from its creation to its worker's pull request
// (PRD mission-destination B2, B5, B6), on an isolated hided and a real
// repository with a remote of its own: the mission made in a checkout on
// `feat/x` records `feat/x`, its worker's new worktree starts from the newest
// commit the remote has of `feat/x` rather than from `main`, and the worker
// reads `feat/x` from `hide mission show <node>` as the base of the pull
// request it opens. The runtime tests state each rule; only this crosses the
// CLI, the spawn's executor and git together.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import type { HideAnswer } from "./agent-pane";
import { closePane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { spawnChild } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { enterWorkspace } from "./wire";
import { git } from "./work-prs-fixture";

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; value?: Record<string, unknown>; result?: Record<string, unknown>; error?: { code?: string } };

async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const got = JSON.parse(ran.out.trim().split("\n").at(-1) ?? "null") as Answer;
  expect(got?.ok, `${args.join(" ")}: ${ran.out}`).toBe(true);
  return got;
}

test.describe.configure({ timeout: 180_000 }); // wait-ok: a daemon, two registered stand-ins and a spawn into a new worktree each finish their real startup

test("a mission made on a feature branch starts its worker there and names it as the pull request's base", async ({ page }) => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "mission-destination");
    const live = daemon;
    const [lead, beside] = fixture.panes;
    await withControl(live, (control) => closePane(control, beside));
    const conductor = fixture.runAs(lead);
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));

    // The repository's remote, and `feat/x` there one commit ahead of the checkout's own.
    const origin = path.join(fixture.root, "origin.git");
    git(fixture.root, ["init", "-q", "--bare", "-b", "main", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["checkout", "-q", "-b", "feat/x"]);
    git(repo, ["commit", "-q", "--allow-empty", "-m", "feature begins"]);
    git(repo, ["push", "-q", "-u", "origin", "feat/x"]);
    const other = path.join(fixture.root, "other");
    git(fixture.root, ["clone", "-q", "-b", "feat/x", origin, other]);
    git(other, ["commit", "-q", "--allow-empty", "-m", "feature moves on"]);
    git(other, ["push", "-q", "origin", "feat/x"]);
    git(repo, ["fetch", "-q", "origin"]);
    const newest = git(repo, ["rev-parse", "origin/feat/x"]).trim();
    expect(newest).not.toBe(git(repo, ["rev-parse", "feat/x"]).trim());
    expect(newest).not.toBe(git(repo, ["rev-parse", "main"]).trim());

    // The mission copies the branch from what Hide last read of the checkout, which it reads for a window that shows it.
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await withControl(live, (control) => control.until((rest) => {
      const workspaces = (rest.navigator as { workspaces?: { checkouts: { branch?: string | null }[] }[] } | undefined)?.workspaces ?? [];
      return workspaces.some((workspace) => workspace.checkouts.some((checkout) => checkout.branch === "feat/x")) || undefined;
    }, "the checkout read on feat/x", 30_000));

    await ok(conductor, ["mission", "create", "--title", "도착 브랜치 확인"]);
    const { mission } = (await ok(conductor, ["mission", "show", "--json"])).result as { mission: { id: string; destination?: string } };
    expect(mission.destination).toBe("feat/x");
    expect((await conductor(["mission", "show"])).out).toContain("도착 · feat/x");

    await ok(conductor, ["mission", "plan", "--json", JSON.stringify({ scope: "pr", nodes: [{ key: "fix", title: "기능 수정", kind: "change", delivery: "pr", criteria: ["수정이 들어갔다"] }] })]);
    await ok(conductor, ["mission", "start"]);
    // The checkout moves on to another branch: the worker still starts from the mission's destination, not from what is out now.
    git(repo, ["checkout", "-q", "main"]);

    // The worker, in a new worktree on a new branch.
    const pane = await spawnChild(fixture, live, lead, { name: "worker", repo, branch: "fix-task", intent: `${mission.id}:fix`, brief: "기능을 고치고 PR을 엽니다" });

    expect(git(repo, ["rev-parse", "fix-task"]).trim()).toBe(newest);
    // The new branch starts at the commit and takes no upstream from it, so the worker's push makes its own.
    expect(git(repo, ["for-each-ref", "--format=%(upstream:short)", "refs/heads/fix-task"]).trim()).toBe("");

    // What the worker reads for its pull request once the Conductor names it the node's, and the command the guide has it run with it.
    const listed = (await ok(conductor, ["agent", "list"])).value as { items: { id: string; pane: string }[] };
    await ok(conductor, ["mission", "node", "fix", "--status", "working", "--agent-id", listed.items.find((agent) => agent.pane === pane)!.id, "--line", "고치는 중"]);
    const worker = fixture.runAs(pane);
    const node = (await ok(worker, ["mission", "show", "fix", "--json"])).result as { destination?: string };
    expect(node.destination).toBe("feat/x");
    expect((await worker(["mission", "show", "fix"])).out).toContain("도착 feat/x");
    const log = path.join(fixture.root, "gh-writes.log");
    const gh = fixtureProgram(path.join(fixture.root, "gh-bin"), "gh", `require("fs").appendFileSync(${JSON.stringify(log)}, JSON.stringify(process.argv.slice(2)) + "\\n");`);
    const opened = await worker(["pr", "create", "--base", node.destination!, "--head", "fix-task", "--title", "fix: 기능 수정", "--body", ""], { program: gh });
    expect(opened.status).toBe(0);
    const written = JSON.parse(fs.readFileSync(log, "utf8").trim()) as string[];
    expect(written.slice(0, 4)).toEqual(["pr", "create", "--base", "feat/x"]);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
