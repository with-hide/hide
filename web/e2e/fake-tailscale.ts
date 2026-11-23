// A fake `tailscale` CLI for the specs that open Settings › Devices: nothing here
// runs the operator's Tailscale. HIDE_TAILSCALE_BIN names the script in the
// test's own directory, and every call the daemon makes is recorded.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { endWindowsProcesses, fixtureExecutable, fixtureProgram } from "./platform-fixture";

export const DNS = "mac.tailnet-name.ts.net";

/** A `tailscale` CLI whose answers the test writes; it records every call. */
export class FakeTailscale {
  readonly dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-ts-"));
  readonly bin = path.join(this.dir, fixtureExecutable("tailscale"));

  install(): void {
    fixtureProgram(
      this.dir,
      "tailscale",
      `const fs = require("fs");
const path = require("path");
const file = (name) => path.join(${JSON.stringify(this.dir)}, name);
const args = process.argv.slice(2);
fs.appendFileSync(file("calls.log"), args.join(" ") + "\\n");
if (args[0] === "status") { process.stdout.write(fs.readFileSync(file("status.json"), "utf8")); process.exit(0); }
if (args[0] === "serve") {
  const rest = args.slice(1);
  if (rest[0] === "status") {
    process.stdout.write(fs.existsSync(file("serve.json")) ? fs.readFileSync(file("serve.json"), "utf8") : "{}\\n");
    process.exit(0);
  }
  const last = rest[rest.length - 1];
  fs.writeFileSync(file("serve.json"), last === "off" ? "{}\\n" : '{"TCP":{"443":{"HTTPS":true}},"Web":{"${DNS}:443":{"Handlers":{"/":{"Proxy":"' + last + '"}}}}}');
  process.exit(0);
}
process.exit(2);
`,
    );
  }

  status(value: unknown): void {
    fs.writeFileSync(path.join(this.dir, "status.json"), JSON.stringify(value));
  }

  loggedOut(): void {
    this.status({ BackendState: "NeedsLogin", Self: { DNSName: "", HostName: "mac" } });
  }

  httpsOff(): void {
    this.status({ BackendState: "Running", Self: { DNSName: `${DNS}.`, HostName: "mac" }, CurrentTailnet: { MagicDNSEnabled: true } });
  }

  ready(): void {
    this.status({ BackendState: "Running", Self: { DNSName: `${DNS}.`, HostName: "mac" }, CurrentTailnet: { MagicDNSEnabled: true }, CertDomains: [DNS] });
  }

  proxy(): string | null {
    try {
      const serve = JSON.parse(fs.readFileSync(path.join(this.dir, "serve.json"), "utf8")) as { Web?: Record<string, { Handlers?: Record<string, { Proxy?: string }> }> };
      return serve.Web?.[`${DNS}:443`]?.Handlers?.["/"]?.Proxy ?? null;
    } catch {
      return null;
    }
  }

  calls(): string {
    try {
      return fs.readFileSync(path.join(this.dir, "calls.log"), "utf8");
    } catch {
      return "";
    }
  }

  remove(): void {
    // Windows keeps a running fake's executable locked; end what runs from this folder first.
    if (process.platform === "win32") endWindowsProcesses([], this.dir);
    fs.rmSync(this.dir, { recursive: true, force: true });
  }
}
