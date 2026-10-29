// The e2e entry point for both packages: build the fixtures' C programs once,
// before any test starts, so a test never compiles one. On Windows the run is
// then prepared after the last compile (`prepareWindowsRun`): the job's first
// PowerShell start is paid outside every test's bounds, and the compilers'
// telemetry helper is ended, so no pane shell can be mistaken for its parent.
import path from "node:path";
import type { FullConfig } from "@playwright/test";
import { prepareWindowsRun } from "./platform-fixture";
import { buildFixtureShims } from "./shims/build";

export default async function globalSetup(config: FullConfig): Promise<void> {
  await buildFixtureShims(path.dirname(config.configFile!));
  if (process.platform === "win32") for (const line of await prepareWindowsRun(path.dirname(config.configFile!))) console.log(line);
}
