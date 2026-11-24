import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { APP_IDENTITY, identityDrift } from "./app-identity.mjs";
import { APP_ID } from "./installers.mjs";

const app = (extra = {}) => ({ CFBundleIdentifier: APP_IDENTITY.identifier, CFBundleName: APP_IDENTITY.name, CFBundleVersion: "1", ...APP_IDENTITY.usage, ...extra });

describe("the app identity contract", () => {
  it("names the identifier the app is packaged and installed under", () => {
    expect(APP_IDENTITY.identifier).toBe(APP_ID);
    expect(APP_IDENTITY.requirement.startsWith(`identifier "${APP_ID}" and anchor apple generic`)).toBe(true);
  });

  it("finds no difference in an app that says what it says", () => {
    expect(identityDrift(app())).toEqual([]);
  });

  it("names a sentence the app gained, lost or changed, and another identifier or name", () => {
    const withoutCamera = app();
    delete withoutCamera.NSCameraUsageDescription;
    expect(identityDrift(withoutCamera)).toEqual(['NSCameraUsageDescription is undefined, the contract says "This app needs access to the camera"']);
    expect(identityDrift(app({ NSSpeechRecognitionUsageDescription: "speech" }))).toEqual(['NSSpeechRecognitionUsageDescription is "speech", the contract says undefined']);
    expect(identityDrift(app({ NSMicrophoneUsageDescription: "another sentence" }))).toHaveLength(1);
    expect(identityDrift(app({ CFBundleIdentifier: "another.app", CFBundleName: "another" }))).toHaveLength(2);
  });

  // Electron's own sentences are the app's: the package adds only the local
  // network one. An Electron that changes them fails here, on the Mac lane
  // where its app bundle is installed, before a package would.
  const electron = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "node_modules", "electron", "dist", "Electron.app", "Contents", "Info.plist");
  it.skipIf(process.platform !== "darwin" || !fs.existsSync(electron))("carries every usage sentence of the Electron the app is built on", () => {
    const info = JSON.parse(execFileSync("/usr/bin/plutil", ["-convert", "json", "-o", "-", electron], { encoding: "utf8" }));
    const fromElectron = { ...APP_IDENTITY.usage };
    delete fromElectron.NSLocalNetworkUsageDescription;
    const usage = Object.fromEntries(Object.entries(info).filter(([key]) => /^NS.*UsageDescription$/.test(key)));
    expect(usage).toEqual(fromElectron);
  });
});
