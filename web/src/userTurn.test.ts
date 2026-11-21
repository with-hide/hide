import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import { ASK_SUBJECT_CAUSES, HUMAN_KINDS, USER_TURN_KINDS } from "./snapshot";

it("types every native user-turn kind emitted by the snapshot contract", () => {
  const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
  expect(USER_TURN_KINDS).toEqual(contract.user_turn_kind);
});

it("types every cause of a question the core asks for an agent", () => {
  const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
  expect(ASK_SUBJECT_CAUSES).toEqual(contract.ask_subject_cause);
});

it("types every kind of a person's turn an agent declares", () => {
  const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
  expect(HUMAN_KINDS).toEqual(contract.human_kind);
});
