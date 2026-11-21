// The names of a node's kind and a general node's presets as the screens say them in Korean are the core's
// (PRD node-presets D-01, D-02, D-10, D-13): the core's preset table (`PRESETS`, hide-core/src/missions.rs) and
// `hide mission show`'s kind words (hided/src/mission_show.rs) name them for the Conductor and the CLI, the web's
// catalog for the screens in four languages. This reads both sources as written, so a word changed on one side alone fails here.
import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import { missionsCatalogs } from "./i18n/resources/missions";
import { nodeKindWord } from "./missionLine";

const source = (file: string) => fs.readFileSync(path.resolve(__dirname, "../..", file), "utf8");

it("names each preset, and a code and a waiting node, in Korean with the core's own words", () => {
  const table = source("hide-core/src/missions.rs");
  const rows = [...table.slice(table.indexOf("pub const PRESETS")).matchAll(/key: "([a-z-]+)",\s*word: "([^"]+)"/g)].map(([, key, word]) => [key, word]);
  expect(rows.map(([key]) => key)).toEqual(["research", "verify", "deploy", "general"]);
  const ko = missionsCatalogs.ko as Record<string, string>;
  const screen = rows.map(([key]) => [key, ko[nodeKindWord(key === "general" ? { kind: "general", preset: null } : { kind: "general", preset: key as "research" | "verify" | "deploy" })]]);
  expect(screen).toEqual(rows);
  const show = source("hided/src/mission_show.rs");
  const cli = (kind: string) => show.match(new RegExp(`"${kind}" => "([^"]+)"\\.to_owned\\(\\)`))?.[1];
  expect([ko["mission.kind.change"], ko["mission.kind.external"]]).toEqual([cli("change"), cli("external")]);
});
