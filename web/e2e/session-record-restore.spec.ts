// The record restores the exact retained native conversation after its pane closes.
import { expect, test } from "@playwright/test";
import { CLAUDE_FULL_ACCESS_LINE } from "./herdr-fixture";
import { agentIn, closePane, tabOfPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { labelAgentPane } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace } from "./wire";

test.describe.configure({ timeout: 120_000 });

test("a closed pane's record restores the same native session in a fresh pane", async ({ page }) => {
  const fixture = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(fixture, "record-restore");
    const source = fixture.panes[0];
    const session = await labelAgentPane(fixture, daemon, source, { task: "Restore this conversation", end: "done", cwd: `${fixture.root}/fixture` });
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator(`[data-pane-view="${source}"] [data-pane-record]`).click();
    const record = page.locator('[data-session-record]');
    await expect(record).toContainText("Restore this conversation 진행해줘", { timeout: 30_000 });
    await expect(record.locator('[data-record-now-button="pane"]')).toBeEnabled();
    // The pane closes only once its label is made: a close mid-analysis discards it, and the record would have no title to restore.
    const sourceRow = page.locator(`nav[data-sidebar] [data-pane="${source}"] [data-agent-title]`);
    await expect(sourceRow).toHaveText("Restore this conversation", { timeout: 30_000 });
    await withControl(daemon, (control) => closePane(control, source));
    const restore = record.getByRole("button", { name: "Restore session", exact: true });
    await expect(restore).toBeEnabled({ timeout: 30_000 });
    await restore.click();
    // The start asks for the record's own session, and the fresh pane is the one an agent runs in that the fixture did not make.
    await expect.poll(() => last.get("agent_start_in_checkout")?.resume_session_id).toBe(session);
    const pane = await withControl(daemon, (control) => control.until((rest) => {
      const rows = ((rest.navigator as { agents?: { pane_id: string }[] } | undefined)?.agents ?? []).map((row) => row.pane_id);
      return rows.find((id) => id !== source && !fixture.panes.includes(id) && agentIn(rest, id) && tabOfPane(rest, id) !== undefined);
    }, "a fresh pane running an agent", 45_000));
    expect(pane).not.toBe(source);
    // The shell echoes the command line the core typed, which resumes that same session with the default full-access options (shell quotes and wrapping left out).
    await expect.poll(async () => (await page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane)).replace(/[\s']/g, ""), { timeout: 30_000 })
      .toContain(`claude--resume${session}${CLAUDE_FULL_ACCESS_LINE}`.replace(/[\s']/g, ""));
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toBeVisible({ timeout: 30_000 });
    // The actual fixture process was launched with --resume, but sends no
    // hook identity. This is a locator test, not a real-agent observation.
    const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"]`);
    await expect(row.locator("[data-agent-title]")).toHaveText("Restore this conversation", { timeout: 30_000 });
    if (fixture.engine === "herdr") {
      // Herdr is told no session by the stand-in, so the record stays unconfirmed. The own engine takes the session
      // from the start's --resume, so its row is confirmed: it names the record's session and carries no pending mark.
      await expect(row.locator('[data-record-connection="unconfirmed"]')).toBeVisible();
    } else {
      await expect(row.locator("[data-record-connection]")).toHaveCount(0);
      await withControl(daemon, (control) => control.until((rest) => agentIn(rest, pane)?.session_id === session || undefined, `${pane} on session ${session}`, 20_000));
    }
    await expect(row.locator('[data-agent-status-mark="turn"]')).toHaveCount(0);
  } finally {
    await page.goto("about:blank");
    await daemon?.stop();
    fixture.stop();
  }
});
