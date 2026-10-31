// Hide Desk's ask mode for Pi, which asks for nothing on its own: every
// tool call that is not a read waits for Hide's permission, sent to the
// ACP client as a confirm, and a confirm that does not come back true
// blocks the call. A handler failure blocks the call too (Pi's fail-safe).
import { writeFileSync } from "node:fs";

const READS = new Set(["read", "grep", "find", "ls"]);

export default function (pi) {
  // Proves to Hide that this process loaded the gate before any prompt.
  if (process.env.HIDE_DESK_PI_READY) {
    writeFileSync(process.env.HIDE_DESK_PI_READY, String(process.pid));
  }
  pi.on("tool_call", async (event, ctx) => {
    if (READS.has(event.toolName)) return;
    const message = JSON.stringify({ hide_desk: 1, tool: event.toolName, input: event.input ?? null });
    const confirmed = await ctx.ui.confirm("hide-desk.permission", message);
    if (confirmed !== true) return { block: true, reason: "Hide did not allow this tool call" };
  });
}
