import { test } from "@playwright/test";
import { e2eEngine } from "./e2e-engine";

/** Each named caller needs the Windows foreground proof owned by win-agent-start. */
export function skipOwnWindowsAgentStart(): void {
  test.skip(process.platform === "win32" && e2eEngine() === "own",
    "win-agent-start 대기: own engine Windows foreground proof unavailable");
}
