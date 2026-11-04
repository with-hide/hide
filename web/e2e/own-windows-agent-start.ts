import { test } from "@playwright/test";

/** Each named caller needs the Windows foreground proof deferred to bundle 4 PR A. */
export function skipOwnWindowsAgentStart(): void {
  test.skip(process.platform === "win32",
    "own engine Windows foreground proof unavailable; bundle 4 PR A");
}
