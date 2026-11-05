// The answers file of the helper stand-in the host script tests run
// (`tests/it/host_helper.rs`): one line per operation, its name, a delay in
// seconds, an exit status and the output, by tabs with `-` for none. It is
// replaced whole, so the stand-in never reads half of it.

import { renameSync, writeFileSync } from "node:fs";

/**
 * Writes `answers` by operation: `delay_ms` holds an answer back, `exit` ends the helper without one, `raw` is
 * printed unparsed, and the rest is the answer.
 */
export function writeAnswers(file, answers) {
  const lines = Object.entries(answers).map(([operation, { delay_ms: delay, exit, raw, ...answer }]) => {
    const fields = [operation, delay ? String(delay / 1000) : "-", exit === undefined ? "-" : String(exit), raw ?? JSON.stringify(answer)];
    if (fields.some((field) => field === "" || /[\t\n]/.test(field))) {
      throw new Error(`not one line of the stand-in's answers: ${JSON.stringify(fields)}`);
    }
    return `${fields.join("\t")}\n`;
  });
  writeFileSync(`${file}.part`, lines.join(""));
  renameSync(`${file}.part`, file);
}
