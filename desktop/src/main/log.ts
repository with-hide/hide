// The host's structured log: one JSON object per line, capped by size.
// Discovery failures land here with their detail, never on the screen
// (design 13); a URL is never logged because it carries the daemon token.

import fs from "node:fs";
import path from "node:path";
import { isSecretField, redact } from "../../../web/src/redact";

/** Past this size the log rotates once to `desktop.log.1` (engineering 15). */
export const MAX_LOG_BYTES = 5 * 1024 * 1024;

export type LogFields = Record<string, string | number | boolean | null | undefined>;

export class HostLog {
  private readonly file: string;

  constructor(dir: string) {
    fs.mkdirSync(dir, { recursive: true });
    this.file = path.join(dir, "desktop.log");
  }

  get path(): string {
    return this.file;
  }

  event(event: string, fields: LogFields = {}): void {
    const safeFields = Object.fromEntries(Object.entries(fields).map(([key, value]) => [key, isSecretField(key) ? "[redacted]" : typeof value === "string" ? redact(value, key) : value]));
    const line = `${JSON.stringify({ ts: new Date().toISOString(), event: redact(event), ...safeFields })}\n`;
    process.stderr.write(line);
    try {
      if (fs.existsSync(this.file) && fs.statSync(this.file).size > MAX_LOG_BYTES) fs.renameSync(this.file, `${this.file}.1`);
      fs.appendFileSync(this.file, line);
    } catch (error) {
      process.stderr.write(`${JSON.stringify({ event: "log.write_failed", detail: redact(String(error)) })}\n`);
    }
  }
}

/**
 * The fields a failed page load may log. Electron's rejection message quotes
 * the URL it tried, and the daemon URL carries the token, so only the error
 * code is kept.
 */
export function loadFailureFields(error: unknown): { code: string } {
  const code = (error as { code?: unknown } | null)?.code;
  return { code: typeof code === "string" ? code : "unknown" };
}
