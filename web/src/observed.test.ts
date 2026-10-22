import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import { FRESHNESS } from "./observed";

it("reads every freshness the core writes", () => {
  const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
  expect([...FRESHNESS]).toEqual(contract.observed_freshness);
});
