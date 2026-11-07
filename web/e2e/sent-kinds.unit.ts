import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, expect, test } from "vitest";
import { acceptedKinds, staleReads } from "../scripts/check-e2e-sent-kinds.mjs";

let directory: string | undefined;
afterEach(() => {
  if (directory) fs.rmSync(directory, { recursive: true, force: true });
  directory = undefined;
});

/** An e2e folder holding `wire.ts`'s counter and the given spec, checked against `focus_pane` only. */
function check(spec: string): string[] {
  directory = fs.mkdtempSync(path.join(os.tmpdir(), "sent-kinds-"));
  fs.writeFileSync(path.join(directory, "wire.ts"), "export function countSent(_page: unknown, _last: Map<string, Record<string, unknown>> = new Map()): Map<string, number> { return new Map(); }\n");
  fs.writeFileSync(path.join(directory, "a.spec.ts"), `import { countSent } from "./wire";\n${spec}`);
  const { errors } = staleReads(directory, new Set(["focus_pane"]), path.resolve(import.meta.dirname, ".."));
  return (errors as string[]).map((error) => error.replace(/^.*a\.spec\.ts:/, ""));
}

test("the accepted kinds are the daemon's dispatch arms, never a payload field or a removed event", () => {
  const kinds: Set<string> = acceptedKinds();
  for (const kind of ["terminal_mouse", "key", "file_list", "ui_state_update", "view_layout"]) expect(kinds.has(kind), kind).toBe(true);
  for (const kind of ["terminal_click", "sidebar_tab_expanded", "pane_id", "type"]) expect(kinds.has(kind), kind).toBe(false);
});

test("a counter's read of a kind the daemon does not accept fails, by line, through a helper and a destructuring", () => {
  expect(check(`
function flow(page: unknown) { const last = new Map<string, Record<string, unknown>>(); return { sent: countSent(page, last), last }; }
export function a(page: unknown) {
  const sent = countSent(page);
  sent.get("focus_pane"); sent.get("focus_pane.split");
  sent.get("terminal_click");
  const { sent: counted, last } = flow(page);
  counted.get("pane_id"); last.get("terminal_click");
  flow(page).sent.get("sidebar_tab_expanded");
}
`)).toEqual([
    expect.stringMatching(/^7 reads "terminal_click"/),
    expect.stringMatching(/^9 reads "pane_id"/),
    expect.stringMatching(/^9 reads "terminal_click"/),
    expect.stringMatching(/^10 reads "sidebar_tab_expanded"/),
  ]);
});

test("a map that only shares a counter's name is not a counter", () => {
  expect(check(`
export function a(page: unknown) { const sent = countSent(page); sent.get("focus_pane"); }
export function b() { const sent = new Map<string, string>(); sent.get("customer-key"); }
`)).toEqual([]);
});
