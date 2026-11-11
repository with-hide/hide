// A mission's close bringing the operator's checkout up to its destination,
// and a landed direct change's worktree retired (PRD mission-destination B34,
// B51), on an isolated hided and a real repository with a bare remote of its
// own. The project's background fetch is off, and the remote takes one more
// commit after the change landed, so only the close's own fetch can bring the
// checkout two commits forward. The runtime and real-repository tests state
// each case; only this crosses the CLI's close, the core's worker, the node
// and git together.

import { expect, test } from "@playwright/test";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { HideAnswer } from "./agent-pane";
import { closePane, untilLogged, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";
import { git } from "./work-prs-fixture";

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; result?: Record<string, unknown> };
type Shown = { mission: { id: string; status: string; destination?: string; report?: string | null } };

async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const got = JSON.parse(ran.out.trim().split("\n").at(-1) ?? "null") as Answer;
  expect(got?.ok, `${args.join(" ")}: ${ran.out}`).toBe(true);
  return got;
}

test.describe.configure({ timeout: 180_000 }); // wait-ok: a daemon, a page and a real fetch from a bare remote finish their real startup

test("a closed mission fast-forwards the checkout on its destination by its own fetch and retires its landed direct change's worktree", async ({ page }) => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "mission-pull");
    const live = daemon;
    const [lead, beside] = fixture.panes;
    await withControl(live, (control) => closePane(control, beside));
    const conductor = fixture.runAs(lead);
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));

    // The checkout on `feat/login`, which tracks the remote's `feat/login`.
    const origin = path.join(fixture.root, "origin.git");
    git(fixture.root, ["init", "-q", "--bare", "-b", "main", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["checkout", "-q", "-b", "feat/login"]);
    git(repo, ["push", "-q", "-u", "origin", "feat/login"]);
    const start = git(repo, ["rev-parse", "HEAD"]).trim();

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await withControl(live, async (control) => {
      await control.until((rest) => {
        const workspaces = (rest.navigator as { workspaces?: { checkouts: { branch?: string | null }[] }[] } | undefined)?.workspaces ?? [];
        return workspaces.some((workspace) => workspace.checkouts.some((checkout) => checkout.branch === "feat/login")) || undefined;
      }, "the checkout read on feat/login", 30_000);
      // The project's background fetch is off: nothing but the close fetches.
      control.send("project_settings_set", {
        device_id: live.node,
        root: repo,
        request_id: `${Date.now()}-${crypto.randomBytes(16).toString("hex")}`,
        patch: { auto_fetch: false },
      });
      await control.until((rest) => {
        const settings = (rest.ui_state as { project_settings?: Record<string, { auto_fetch?: boolean | null }> } | undefined)?.project_settings ?? {};
        return Object.values(settings).some((record) => record.auto_fetch === false) || undefined;
      }, "the project's fetch turned off", 20_000);
    });

    await ok(conductor, ["mission", "create", "--title", "로그인 고치기"]);
    const created = (await ok(conductor, ["mission", "show", "--json"])).result as Shown;
    expect(created.mission.destination).toBe("feat/login");
    await ok(conductor, ["mission", "plan", "--json", JSON.stringify({ scope: "merge", nodes: [{ key: "fix", title: "로그인 · 세션 유지", kind: "change", delivery: "direct", criteria: ["로그인이 유지된다"] }] })]);
    await ok(conductor, ["mission", "start"]);

    // The node's worktree, its commit, and its merge into the destination on
    // the remote, which the push also records as the remote's copy here.
    const worktree = path.join(fixture.root, "fixture-fix");
    git(repo, ["worktree", "add", "-q", "-b", "fix", worktree, "feat/login"]);
    fs.writeFileSync(path.join(worktree, "login.txt"), "session kept\n");
    git(worktree, ["add", "login.txt"]);
    git(worktree, ["commit", "-q", "-m", "keep the session"]);
    git(repo, ["push", "-q", "origin", "fix:feat/login"]);
    // Another change reaches the remote after it, which this clone has not fetched.
    const other = path.join(fixture.root, "other");
    git(fixture.root, ["clone", "-q", "-b", "feat/login", origin, other]);
    git(other, ["commit", "-q", "--allow-empty", "-m", "a later change"]);
    git(other, ["push", "-q", "origin", "feat/login"]);
    const newest = git(other, ["rev-parse", "HEAD"]).trim();
    expect(git(repo, ["rev-parse", "HEAD"]).trim()).toBe(start);

    await ok(conductor, ["mission", "node", "fix", "--status", "working", "--branch", "fix"]);
    await ok(conductor, ["mission", "record", "fix", "c1", "--pass", "--evidence", "login.txt keeps the session"]);
    // Hide finishes the direct change on its passed line (B7) and closes the
    // mission by itself: as done once the branch reads in the destination,
    // at once or after a `merge_wait` that pulls nothing (D-47), and the
    // close as done owes the pull.
    // The close's pull: fetched though the project's fetch is off, the
    // checkout moved forward by both commits, and the report's last line.
    await untilLogged(live, { kind: "mission.folder_pull", outcome: "updated" }, 1, "the close's pull", 60_000);
    expect(git(repo, ["rev-parse", "HEAD"]).trim()).toBe(newest);
    expect(git(repo, ["branch", "--show-current"]).trim()).toBe("feat/login");
    const closed = (await ok(conductor, ["mission", "show", "--json"])).result as Shown;
    expect(closed.mission.status).toBe("done");
    expect(closed.mission.report?.trim().split("\n").at(-1)).toBe("내 폴더도 최신으로 당겨왔어요 (2커밋)");

    // The landed direct change's worktree is gone and its branch stays.
    await untilLogged(live, { kind: "mission.worktree_retired", branch: "fix", outcome: "removed" }, 1, "the direct change's worktree retired", 60_000);
    expect(fs.existsSync(worktree)).toBe(false);
    expect(git(repo, ["branch", "--list", "fix"]).trim()).toContain("fix");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
