import { test } from "@playwright/test";
import { e2eEngine } from "./e2e-engine";

/** Each named caller needs the Windows foreground proof deferred to bundle 4 PR A. */
export function skipOwnWindowsAgentStart(): void {
  test.skip(process.platform === "win32" && e2eEngine() === "own",
    "own engine Windows foreground proof unavailable; bundle 4 PR A");
}
