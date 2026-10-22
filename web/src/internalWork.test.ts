import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import type { InternalWork, WorkRead } from "./snapshot";

type Execution = Extract<InternalWork["executions"], { state: "current" }>["value"]["records"][number];

it("accepts the internal work values emitted by the snapshot contract", () => {
  const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
  const kinds = ["subagent", "background"] satisfies NonNullable<Execution["kind"]>[];
  const states = ["running", "ended", "stopped", "expired"] satisfies Execution["state"][];
  const sources = ["hooks", "session"] satisfies Extract<WorkRead<unknown>, { state: "current" }>["source"][];
  const reads = ["unsupported", "unknown", "current", "stale", "lost"] satisfies WorkRead<unknown>["state"][];
  expect(kinds).toEqual(contract.internal_work_kind);
  expect(states).toEqual(contract.internal_work_state);
  expect(sources).toEqual(contract.internal_work_source);
  expect(reads).toEqual(contract.internal_work_read_state);
});
