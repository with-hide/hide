// What the fixture's `claude` stand-ins do on either engine (PRD
// engine-bundle-3-core-owns D-24): the screen that tells an engine an agent
// works, asks or rests, a new session as Claude Code reports one, a finished
// turn the operator has not seen. Each goes in through the daemon as the web
// shell sends it (`key`, `create_tab`, `focus_tab`) and is read back from the
// snapshot's agent rows, never from an engine's CLI, so a spec on
// `startFixture` (`engine-fixture.ts`) says the same thing on Herdr and on
// the core's own engine. Both engines read the stand-in's screen with
// Herdr's rules for Claude Code (`hide-agent-adapter/agents/claude-code.toml`).

import { claudeProjects, declareAgentSession, writeFixtureTranscript, type FixtureLabel } from "./herdr-fixture";
import { agentIn, focusTab, newTab, sendKeys, withControl, type AgentRow } from "./daemon-actions";
import type { Daemon } from "./hided-fixture";
import type { AgentFixture, EngineServer } from "./engine-fixture";

export type AgentState = "working" | "blocked" | "idle";

/** Whether `row` reads as `state`: `idle` is a rest, seen or not (Herdr's idle and done). */
function reads(row: AgentRow | undefined, state: AgentState): boolean {
  if (!row) return false;
  if (state === "blocked") return row.blocked === true;
  if (state === "working") return row.activity === "working" && !row.blocked;
  return !row.blocked && row.activity !== "working" && (row.status_code === "idle" || row.status_code === "done");
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
 * after `/clear`, and waits until its row names it: on Herdr through
 * Herdr's integration report, on the core's own engine through Hide's
 * SessionStart hook the stand-in runs from inside its pane
 * (`shims/claude-shim.c`, `session_request`).
 */
export async function setAgentSession(fixture: EngineServer, daemon: Pick<Daemon, "origin" | "token">, pane: string, session: string): Promise<void> {
  await withControl(daemon, async (control) => {
    if (fixture.engine === "herdr") {
      declareAgentSession(fixture as EngineServer & { env: NodeJS.ProcessEnv; bin: string }, pane, session, true);
    } else {
      sendKeys(control, pane, `\x1b]7770;session=${session}\x07`);
    }
    await control.until((rest) => agentIn(rest, pane)?.session_id === session, `${pane} on session ${session}`, 20_000);
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
export async function startAgentIn(fixture: EngineServer, daemon: Pick<Daemon, "origin" | "token">, pane: string, session?: string): Promise<AgentRow> {
  const row = await withControl(daemon, async (control) => {
    sendKeys(control, pane, "claude\r");
    return control.until((rest) => agentIn(rest, pane), `an agent in ${pane}`, 30_000);
  });
  if (!session) return row;
  await setAgentSession(fixture, daemon, pane, session);
  return withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane), `the agent in ${pane}`));
}
