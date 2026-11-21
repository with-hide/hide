// A session an agent's hook reports from inside its pane reaches the core's
// own engine (PRD engine-bundle-3-core-owns D-9, D-18, B18): the `claude` stand-in
// runs Hide's SessionStart hook as Claude Code does after `/clear`
// (`shims/claude-shim.c`, `session_request`), and the agent's row names the
// new session. The pane's program once inherited the holder's own launch
// metadata (`HIDE_PROCESS_OWNER_FD`), which the hook helper took for its
// own owner, so it reported nothing and the row kept its old session.

import { expect, test } from "@playwright/test";
import { agentIn, sendKeys, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test("a SessionStart hook run inside an agent's pane names the agent's new session", async () => {
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "pane-hook-session", fixture.home);
    const pane = fixture.panes[0];
    const session = await withControl(daemon, async (control) => {
      const before = await control.until((rest) => agentIn(rest, pane)?.session_id ?? undefined, `the session of ${pane}`, 20_000);
      sendKeys(control, pane, "\x1b]7770;session=hook-reported-session\x07");
      return control.until((rest) => {
        const now = agentIn(rest, pane)?.session_id;
        return now && now !== before ? now : undefined;
      }, `${pane} on the session its hook reported`, 20_000);
    });
    expect(session).toBe("hook-reported-session");
  } finally {
    try { daemon?.stop(); } finally { fixture.stop(); }
  }
});
