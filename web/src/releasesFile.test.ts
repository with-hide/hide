// The releases file withhide.dev serves (`site/public/app/releases.json`) is
// checked on every pull request that could change it: the app reads every
// version of it, names its drawings and practice chapters, and draws its
// words in every interface language (PRD whats-new B29). The release skill
// adds a version here and its pull request runs this before the file is
// deployed.

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { INTERFACE_LANGUAGES } from "./i18n/locale";
import { CHAPTERS } from "./practice/chapters";
import { parseReleaseFeed, RELEASE_DRAWINGS } from "./releases";

const FILE = path.resolve(__dirname, "../../site/public/app/releases.json");
const raw: unknown = JSON.parse(fs.readFileSync(FILE, "utf8"));
const { feed, dropped } = parseReleaseFeed(raw);

describe("site/public/app/releases.json", () => {
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

  it("uses no em or en dash", () => {
    expect(fs.readFileSync(FILE, "utf8")).not.toMatch(/[–—]/);
  });
});
