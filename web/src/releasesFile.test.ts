// The releases file built into the app (`web/src/releases.json`) is checked
// on every pull request that could change it: the app reads every version of
// it, names its drawings and practice chapters, and draws its words in every
// interface language (PRD whats-new B29). The release skill adds a version
// here before the release candidate is built, and runs this on that commit.

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { INTERFACE_LANGUAGES } from "./i18n/locale";
import { CHAPTERS } from "./practice/chapters";
import { parseReleaseFeed, RELEASE_DRAWINGS } from "./releases";

const FILE = path.resolve(__dirname, "releases.json");
/** The version a release is about to publish, set by `scripts/check-releases-entry.sh`; unset on an ordinary run. */
const RELEASING = process.env.HIDE_RELEASE_ENTRY;
const raw: unknown = JSON.parse(fs.readFileSync(FILE, "utf8"));
const { feed, dropped } = parseReleaseFeed(raw);

describe("web/src/releases.json", () => {
  it("reads whole: no version is dropped and each appears once, newest first", () => {
    expect(dropped).toEqual([]);
    const listed = (raw as { versions: { version: string }[] }).versions.map((row) => row.version);
    expect(listed).toEqual(feed.versions.map((row) => row.version));
  });

  it("names only drawings and practice chapters the app has", () => {
    const chapters = new Set(CHAPTERS.map((chapter) => chapter.id));
    for (const row of feed.versions) {
      for (const highlight of row.highlights) {
        expect(RELEASE_DRAWINGS, `${row.version} drawing`).toContain(highlight.drawing);
        if (highlight.chapter !== null) expect(chapters.has(highlight.chapter), `${row.version} chapter ${highlight.chapter}`).toBe(true);
      }
    }
  });

  it("marks every practice chapter a main feature opens with the version it came with", () => {
    for (const row of feed.versions) {
      for (const highlight of row.highlights) {
        if (highlight.chapter === null) continue;
        expect(CHAPTERS.find((chapter) => chapter.id === highlight.chapter)?.since, `chapter ${highlight.chapter} since`).toBeDefined();
      }
    }
  });

  it.runIf(RELEASING !== undefined)("has the version being released as its newest entry (B29)", () => {
    expect(feed.versions[0]?.version, `the releases file's newest entry, for release ${RELEASING}`).toBe(RELEASING);
  });

  it("gives every line in every interface language", () => {
    for (const row of feed.versions) {
      for (const [at, highlight] of row.highlights.entries()) {
        for (const language of INTERFACE_LANGUAGES) expect(highlight.text[language], `${row.version} highlight ${at} ${language}`).toBeDefined();
      }
      for (const [at, change] of row.changes.entries()) {
        for (const language of INTERFACE_LANGUAGES) expect(change.text[language], `${row.version} change ${at} ${language}`).toBeDefined();
      }
    }
  });

  it("names the app's lists and buttons in each language's own words, never in Korean outside Korean", () => {
    const hangul = /[\uac00-\ud7a3]/;
    for (const row of feed.versions) {
      const lines = [
        ...row.highlights.flatMap((highlight) => Object.entries(highlight.text).flatMap(([language, words]) => [[language, words!.title], [language, words!.line]] as const)),
        ...row.changes.flatMap((change) => Object.entries(change.text).map(([language, line]) => [language, line!] as const)),
      ];
      for (const [language, line] of lines) if (language !== "ko") expect(line, `${row.version} ${language}`).not.toMatch(hangul);
    }
  });

  it("uses no em or en dash", () => {
    expect(fs.readFileSync(FILE, "utf8")).not.toMatch(/[–—]/);
  });
});
