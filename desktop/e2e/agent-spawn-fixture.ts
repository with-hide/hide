// What the native spawn specs (`agent-spawn.spec.ts`, `agent-handoff.spec.ts`)
// share: a Git repository with one linked worktree, and the answer of a
// `hide agent` command run by a real agent process in its pane (`agentPane`
// in `web/e2e/agent-pane.ts`, whose stand-in reports its native session the
// way the engine learns one).

import { expect } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { HideRunner } from "../../web/e2e/engine-fixture";
import { corePaneFocus, withControl } from "../../web/e2e/daemon-actions";
import type { Daemon } from "../../web/e2e/hided-fixture";

export type SpawnedAgent = { id: string; pane: string; machine: string; parent: string | null; origin: string | null; watch: string | null; session: string };

/**
 * A repository `<root>/repo` on `main` with one commit and a linked
 * worktree `<root>/existing-checkout` on the branch `existing-task`, the
 * checkout a spawn onto that branch reuses.
 */
export function makeSpawnRepository(root: string, env: NodeJS.ProcessEnv): { repo: string; existingCheckout: string } {
  const repo = path.join(root, "repo");
  const existingCheckout = path.join(root, "existing-checkout");
  fs.mkdirSync(repo, { recursive: true });
  execFileSync("git", ["init", "-b", "main", repo], { env, timeout: 20_000 });
  execFileSync("git", ["-C", repo, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "--allow-empty", "-m", "Fixture"], { env, timeout: 20_000 });
  execFileSync("git", ["-C", repo, "worktree", "add", "-b", "existing-task", existingCheckout], { env, timeout: 20_000 });
  return { repo, existingCheckout };
}

type Answer = { ok?: boolean; value?: SpawnedAgent; error?: { code?: string } } | null;

/**
 * Runs `args` as the agent `caller` runs them and returns the answer's value.
 * A child started fresh reports its session after the spawn answers, and the
 * same intent then answers it (docs/delivery.md), so that one refusal is asked
 * again.
 */
export async function hideAnswer(caller: HideRunner, args: string[]): Promise<SpawnedAgent> {
  const deadline = Date.now() + 30_000;
  for (;;) {
    const ran = await caller(args);
    const answer = JSON.parse(ran.out.trim().split("\n").at(-1) ?? "null") as Answer;
    if (ran.status === 0 && answer?.ok && answer.value) return answer.value;
    if (answer?.error?.code !== "native_identity_unavailable" || Date.now() > deadline) throw new Error(`hide ${args.join(" ")} failed: ${ran.out}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
}

/** The pane the core holds focused, as the daemon publishes it. */
export async function coreFocus(daemon: Pick<Daemon, "origin" | "token">): Promise<string | null> {
  return withControl(daemon, (control) => control.until((rest) => ({ pane: corePaneFocus(rest) }), "the focused pane")).then((focus) => focus.pane);
}

/** Waits until the core holds `pane` focused. */
export async function expectCoreFocus(daemon: Pick<Daemon, "origin" | "token">, pane: string): Promise<void> {
  await expect.poll(() => coreFocus(daemon), { message: `the core's keyboard focus on ${pane}` }).toBe(pane);
}
