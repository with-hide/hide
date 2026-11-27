// The names of a node's kind and a general node's presets are one set of words on every side (PRD node-presets
// D-01, D-02, D-10, D-13): the core's preset table (`PRESETS`, hide-core/src/missions.rs) names the presets as the
// screens say them in Korean, and `hide mission show` (hided/src/mission_show.rs), which an agent reads in English,
// names a node's kind and preset as the screens say them in English. This reads every source as written, so a word
// changed on one side alone fails here.
import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import { missionsCatalogs } from "./i18n/resources/missions";
import { nodeKindWord } from "./missionLine";

const source = (file: string) => fs.readFileSync(path.resolve(__dirname, "../..", file), "utf8");

it("names each preset in Korean with the core's own words, and a node's kind in English as hide mission show does", () => {
  const table = source("hide-core/src/missions.rs");
  const rows = [...table.slice(table.indexOf("pub const PRESETS")).matchAll(/key: "([a-z-]+)",\s*word: "([^"]+)"/g)].map(([, key = "", word = ""]): [string, string] => [key, word]);
  expect(rows.map(([key]) => key)).toEqual(["research", "verify", "deploy", "general"]);
  const word = (key: string) => nodeKindWord(key === "general" ? { kind: "general", preset: null } : { kind: "general", preset: key as "research" | "verify" | "deploy" });
  const ko = missionsCatalogs.ko as Record<string, string>;
  expect(rows.map(([key]) => [key, ko[word(key)]])).toEqual(rows);
  // `hide mission show` says a preset by its key, which is the English screen's word.
  const en = missionsCatalogs.en as Record<string, string>;
  expect(rows.map(([key]) => en[word(key)]?.toLowerCase())).toEqual(rows.map(([key]) => key));
  const show = source("hided/src/mission_show.rs");
  const cli = (kind: string) => show.match(new RegExp(`"${kind}" => "([^"]+)"\\.to_owned\\(\\)`))?.[1];
  expect([en["mission.kind.change"]?.toLowerCase(), en["mission.kind.external"]?.toLowerCase()]).toEqual([cli("change"), cli("external")]);
});
