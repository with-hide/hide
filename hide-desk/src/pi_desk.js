// Hide Desk's ask mode for Pi, which asks for nothing on its own: every
// tool call, reads included, waits for Hide's permission, sent to the ACP
// client as a confirm, and a confirm that does not come back true blocks
// the call. Hide answers from Settings › Desk › 권한, so a read runs without
// a card unless the operator set reads to ask. A handler failure blocks the
// call too (Pi's fail-safe).
import { writeFileSync } from "node:fs";

export default function (pi) {
  // Proves to Hide that this process loaded the gate before any prompt.
  if (process.env.HIDE_DESK_PI_READY) {
    writeFileSync(process.env.HIDE_DESK_PI_READY, String(process.pid));
  }
  pi.on("tool_call", async (event, ctx) => {
    const message = JSON.stringify({ hide_desk: 1, tool: event.toolName, input: event.input ?? null });
    const confirmed = await ctx.ui.confirm("hide-desk.permission", message);
    if (confirmed !== true) return { block: true, reason: "Hide did not allow this tool call" };
  });
}
