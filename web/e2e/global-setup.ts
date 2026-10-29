// The e2e entry point for both packages: build the fixtures' C programs once,
// before any test starts, so a test never compiles one. On Windows the
// compilers' telemetry helper is then ended, after the run's last compile, so
// no pane shell can be mistaken for its parent (`endVctip`).
import path from "node:path";
import type { FullConfig } from "@playwright/test";
import { endVctip } from "./platform-fixture";
import { buildFixtureShims } from "./shims/build";

export default async function globalSetup(config: FullConfig): Promise<void> {
  await buildFixtureShims(path.dirname(config.configFile!));
  if (process.platform === "win32") {
    const ended = endVctip();
    if (ended.length > 0) console.log(`ended vctip.exe ${ended.join(", ")} before the first test`);
  }
}
