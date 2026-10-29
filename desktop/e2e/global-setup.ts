// Builds the web fixtures' C programs and the fused Electron copy once, before any test
// (web/e2e/shims/build.ts, fused-electron.ts).
// This file lives in the desktop package so Playwright loads it, and what it
// imports, the way it loads the desktop specs.
import path from "node:path";
import type { FullConfig } from "@playwright/test";
import { buildFixtureShims } from "../../web/e2e/shims/build";
import { makeFusedElectron } from "./fused-electron";

export default async function globalSetup(config: FullConfig): Promise<void> {
  await buildFixtureShims(path.dirname(config.configFile!));
  await makeFusedElectron();
}
