// What the fixture's `claude` stand-ins do (PRD engine-bundle-3-core-owns
// D-24): the screen that tells the engine an agent works, asks or rests, a new
// session as Claude Code reports one, a finished turn the operator has not
// seen. Each goes in through the daemon as the web shell sends it (`key`,
// `create_tab`, `focus_tab`) and is read back from the snapshot's agent rows.
// The engine reads the stand-in's screen with the rules for Claude Code
// (`hide-agent-adapter/agents/claude-code.toml`).

import path from "node:path";
import { prepareAgentInput, waitForAgentInput } from "./agent-pane";
import { claudeProjects, writeFixtureTranscript, type FixtureLabel } from "./pane-fixture";
import { agentIn, checkoutAt, closePane, focusTab, newTab, preparePaneInput, sendKeys, startedAgentIn, tabPanes, withControl, type AgentRow, type OpenedProject } from "./daemon-actions";
import type { Daemon } from "./hided-fixture";
import type { AgentFixture, EngineServer } from "./engine-fixture";

export type AgentState = "working" | "blocked" | "idle";

/** Whether `row` reads as `state`: `idle` is a rest, seen or not (Herdr's idle and done). */
function reads(row: AgentRow | undefined, state: AgentState): boolean {
  if (!row) return false;
  if (state === "blocked") return row.blocked === true;
  if (state === "working") return row.activity === "working" && !row.blocked;
  // A turn that ended unseen with no label verdict reads `stopped` (status-marks D-03).
  return !row.blocked && row.activity !== "working" && (row.status_code === "idle" || row.status_code === "done" || row.status_code === "stopped");
}

/**
 * Puts the stand-in in `pane` on the screen Claude Code shows while it
 * works, asks for a permission, or rests, and waits until its row reads so.
 * The stand-in echoes what it is typed, so the engine's screen rules see
 * the title and prompt a real Claude Code draws; leaving a permission prompt
 * clears it off the screen, where the rules would still read it.
 */
export async function setAgentState(daemon: Pick<Daemon, "origin" | "token">, pane: string, state: AgentState): Promise<AgentRow> {
  return withControl(daemon, async (control) => {
    await preparePaneInput(control, pane);
    const now = await control.until((rest) => agentIn(rest, pane), `an agent in ${pane}`, 20_000);
    const clear = now.blocked ? "\x1b[2J\x1b[H" : "";
    const screen = state === "working"
      ? `${clear}\x1b]0;⠋ Working\x07`
      : state === "idle"
        ? `${clear}\x1b]0;✳ Claude Code\x07`
        : "\x1b]0;Fixture\x07\x1b[2J\x1b[Hdo you want to proceed?\n" + "bash command\n❯ 1. Yes\n2. No\n";
    sendKeys(control, pane, screen);
    return control.until((rest) => {
      const row = agentIn(rest, pane);
      return reads(row, state) ? row : undefined;
    }, `${pane} reading ${state}`, 20_000);
  });
}

/**
 * Gives the stand-in in `pane` a new session, as Claude Code reports one
 * after `/clear`, and waits until its row names it, through Hide's
 * SessionStart hook the stand-in runs from inside its pane
 * (`shims/claude-shim.c`, `session_request`), and its start has settled
 * (`startedAgentIn`).
 */
export async function setAgentSession(fixture: Pick<EngineServer, "root">, daemon: Pick<Daemon, "origin" | "token">, pane: string, session: string): Promise<void> {
  await withControl(daemon, async (control) => {
    await waitForAgentInput(fixture.root, pane);
    sendKeys(control, pane, `\x1b]7770;session=${session}\x07`);
    await control.until((rest) => startedAgentIn(rest, pane)?.session_id === session || undefined, `${pane} on session ${session}, started`, 20_000);
  });
}

let labelSessions = 0;

/**
 * Gives `pane` a new Claude session whose transcript the core analyzes into
 * `label`; a new session, because the same session is read again only when
 * the agent's state changes. Returns the session.
 */
export async function labelAgentPane(fixture: AgentFixture, daemon: Pick<Daemon, "origin" | "token">, pane: string, label: FixtureLabel): Promise<string> {
  const session = `label-${process.pid}-${(labelSessions += 1)}`;
  writeFixtureTranscript(claudeProjects(fixture), session, label);
  await setAgentSession(fixture, daemon, pane, session);
  return session;
}

/** A new tab in the fixture's checkout that no agent runs in, made as New tab makes it; returns its id. */
export async function elsewhereTab(fixture: AgentFixture, daemon: Pick<Daemon, "origin" | "token">): Promise<string> {
  return withControl(daemon, async (control) => (await newTab(control, fixture, "elsewhere")).tab);
}

/**
 * A finished turn the operator has not seen: the stand-in in `pane` works,
 * then rests while `elsewhere` is the tab shown, so its row reads unread.
 */
export async function finishUnseenTurn(fixture: AgentFixture, daemon: Pick<Daemon, "origin" | "token">, pane: string, elsewhere: string): Promise<void> {
  await withControl(daemon, (control) => focusTab(control, fixture, elsewhere));
  await setAgentState(daemon, pane, "working");
  await setAgentState(daemon, pane, "idle");
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.unread === true, `${pane}'s finished turn unread`, 20_000));
}

/**
 * Starts the `claude` stand-in at the shell prompt of `pane`, typed as the
 * operator starts one, and waits until the engine lists it; with `session`,
 * the stand-in then reports that session as Claude Code does
 * (`setAgentSession`).
 */
export async function startAgentIn(fixture: Pick<EngineServer, "root">, daemon: Pick<Daemon, "origin" | "token">, pane: string, session?: string): Promise<AgentRow> {
  const row = await withControl(daemon, async (control) => {
    await preparePaneInput(control, pane);
    prepareAgentInput(fixture.root, pane);
    sendKeys(control, pane, "claude\r");
    return control.until((rest) => startedAgentIn(rest, pane), `an agent started in ${pane}`, 30_000);
  });
  if (!session) return row;
  await setAgentSession(fixture, daemon, pane, session);
  return withControl(daemon, (control) => control.until((rest) => startedAgentIn(rest, pane), `the agent in ${pane}`));
}

let projectAgents = 0;

/**
 * Starts the `claude` stand-in in a tab of its own in the project `project`
 * (`openProject`), as the start panel's start does, resuming a session whose
 * transcript names it `label`, and replaces the project's first shell with
 * it, as `startFixture` does. Waits until the agent is listed and returns its
 * tab and pane. Unlike a typed `startAgentIn`, the session is known from the
 * start on both engines.
 */
export async function startAgentInProject(fixture: EngineServer, daemon: Pick<Daemon, "origin" | "token">, project: OpenedProject, label: FixtureLabel): Promise<{ tab: string; pane: string; session: string }> {
  const session = `project-agent-${process.pid}-${(projectAgents += 1)}`;
  writeFixtureTranscript(claudeProjects(fixture), session, label);
  return withControl(daemon, async (control) => {
    control.send("agent_start_in_checkout", {
      checkout_path: project.folder, provider: "claude", resume_session_id: session, request_id: `e2e-${session}`,
    });
    const made = await control.until((rest) => {
      const tab = checkoutAt(rest, project.folder)?.tabs.find((id) => id !== project.tab);
      const pane = tab ? tabPanes(rest, tab)[0] : undefined;
      return tab && pane && agentIn(rest, pane) ? { tab, pane } : undefined;
    }, `an agent in a tab of ${project.folder}`, 30_000);
    // The engine learns the pane's session from the start's resume.
    await closePane(control, project.pane);
    await control.until((rest) => startedAgentIn(rest, made.pane)?.session_id === session || undefined, `${made.pane} on session ${session}, started`, 20_000);
    return { ...made, session };
  });
}

/** The session the agent in `pane` runs now, as the daemon publishes it. */
export async function sessionIn(daemon: Pick<Daemon, "origin" | "token">, pane: string): Promise<string> {
  return withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.session_id ?? undefined, `the session of ${pane}`, 20_000));
}

/**
 * Has the agent in `parent`, an agent of a family fixture
 * (`startFixture({ family: true })`), spawn a child as a real agent
 * delegates (`hide agent spawn --parent here`), in the checkout of `repo` on
 * `branch` (its own tab), or beside the parent in the parent's own checkout
 * with `beside`. With `session`, the child resumes that session, whose
 * transcript the spec wrote. Waits until the daemon lists the child as
 * `parent`'s and returns its pane.
 */
export async function spawnChild(
  fixture: AgentFixture,
  daemon: Pick<Daemon, "origin" | "token">,
  parent: string,
  { name, repo, branch = "main", beside = false, session }: { name: string; repo: string; branch?: string; beside?: boolean; session?: string },
): Promise<string> {
  const args = ["agent", "spawn", "--parent", "here", "--name", name, "--intent", name, "--kind", "claude", "--repo", repo, "--branch", branch, ...(beside ? ["--beside"] : []), ...(session ? ["--", "--resume", session] : [])];
  type Spawned = { ok?: boolean; value?: { pane?: string }; error?: { code?: string } } | null;
  const deadline = Date.now() + 30_000;
  let pane: string | undefined;
  for (;;) {
    const answer = await fixture.runAs(parent)(args);
    const spawned = JSON.parse(answer.out.trim().split("\n").at(-1) ?? "null") as Spawned;
    pane = spawned?.ok ? spawned.value?.pane : undefined;
    if (answer.status === 0 && pane) break;
    // A child started fresh reports its session after the spawn answers; the same intent then answers it (docs/delivery.md).
    if (spawned?.error?.code !== "native_identity_unavailable" || Date.now() > deadline) throw new Error(`${parent} could not spawn ${name}: ${answer.out}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.lineage_parent_pane_id === parent || undefined, `${pane} listed as ${parent}'s child`, 30_000));
  return pane;
}

/**
 * Has the agent in `pane`, an agent of a family fixture
 * (`startFixture({ family: true })`), take a new session as Claude Code does
 * after `/clear`, and waits until its row names it. The family's stand-in
 * does not read the session escape `setAgentSession` types, so the agent
 * reports through Hide's SessionStart hook, run as the agent from inside its
 * pane.
 * A child's relationship ends with its session, which is the point.
 */
export async function takeOverSession(fixture: AgentFixture, daemon: Pick<Daemon, "origin" | "token">, pane: string, session: string, cwd = path.join(fixture.root, "fixture")): Promise<void> {
  const hooks = fixture.daemonEnv?.HIDE_E2E_HOOKS;
  if (!hooks) throw new Error("the fixture names no hook helper to report a session with");
  const answer = await fixture.runAs(pane)(["hook", "--runtime", "claude-code", "--event", "SessionStart"], {
    program: hooks,
    input: JSON.stringify({ session_id: session, hook_event_name: "SessionStart", source: "clear", cwd }),
    // The report leaves the agent's screen as it was: a permission prompt the spec drew keeps the agent asking.
    hook: true,
  });
  if (answer.status !== 0) throw new Error(`${pane} could not report ${session}: ${answer.out}`);
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.session_id === session, `${pane} on session ${session}`, 20_000));
}

/**
 * Rests the stand-in in `pane` after a turn whose transcript ends in a
 * question (`continueFixtureTranscript` with `question: true`), and waits
 * until its row asks: a rest the operator is asked to answer reads
 * `question`, not `idle`, so `setAgentState` would never see it.
 */
export async function setAgentAsking(daemon: Pick<Daemon, "origin" | "token">, pane: string): Promise<AgentRow> {
  return withControl(daemon, async (control) => {
    await preparePaneInput(control, pane);
    await control.until((rest) => agentIn(rest, pane), `an agent in ${pane}`, 20_000);
    sendKeys(control, pane, "\x1b]0;✳ Claude Code\x07");
    return control.until((rest) => {
      const row = agentIn(rest, pane);
      return row && !row.blocked && row.activity !== "working" && (row as AgentRow & { demand?: string }).demand === "question" ? row : undefined;
    }, `${pane} asking`, 20_000);
  });
}

/**
 * Has the agent in `from`, an agent of a family fixture, hand work to the
 * operator as a real agent does (`hide agent spawn` without `--parent`): an
 * independent root with no relationship to `from`, in its own tab of the
 * checkout of `repo` on `branch`. With `session`, it resumes that session,
 * whose transcript the spec wrote. Waits until the daemon lists it and
 * returns its pane.
 */
export async function spawnRoot(
  fixture: AgentFixture,
  daemon: Pick<Daemon, "origin" | "token">,
  from: string,
  { name, repo, branch = "main", session, kind = "claude", nativeArgs = [] }: { name: string; repo: string; branch?: string; session?: string; kind?: string; nativeArgs?: string[] },
): Promise<string> {
  const passed = session ? ["--resume", session] : nativeArgs;
  const args = ["agent", "spawn", "--name", name, "--intent", name, "--kind", kind, "--repo", repo, "--branch", branch, ...(passed.length ? ["--", ...passed] : [])];
  type Spawned = { ok?: boolean; value?: { pane?: string }; error?: { code?: string } } | null;
  const deadline = Date.now() + 30_000;
  let pane: string | undefined;
  for (;;) {
    const answer = await fixture.runAs(from)(args);
    const spawned = JSON.parse(answer.out.trim().split("\n").at(-1) ?? "null") as Spawned;
    pane = spawned?.ok ? spawned.value?.pane : undefined;
    if (answer.status === 0 && pane) break;
    if (spawned?.error?.code !== "native_identity_unavailable" || Date.now() > deadline) throw new Error(`${from} could not spawn ${name}: ${answer.out}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane) || undefined, `an agent in ${pane}`, 30_000));
  return pane;
}

/**
 * Waits until the stand-in a family fixture's agent runs in `pane` has
 * configured its terminal for input: a child `spawnChild`
 * lists is registered before its stand-in runs, and keys typed before it
 * reads are dropped by the terminal mode it sets.
 */
export async function agentReading(fixture: Pick<EngineServer, "root">, pane: string): Promise<void> {
  await waitForAgentInput(fixture.root, pane);
}
