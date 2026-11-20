import { describe, expect, it } from "vitest";
import { historyVersions, parseReleaseFeed, unseen, whatsNewOffer, type ReleaseFeed } from "./releases";

const words = (title: string) => ({ en: { title, line: `${title} line` } });
const version = (v: string, highlights: string[], changes = 1) => ({
  version: v,
  date: "2026-11-20",
  highlights: highlights.map((title) => ({ drawing: "mission", chapter: null, text: words(title) })),
  changes: Array.from({ length: changes }, (_, at) => ({ kind: at % 2 ? "fix" : "change", text: { en: `${v} change ${at}` } })),
});
const feedOf = (...versions: ReturnType<typeof version>[]): ReleaseFeed => parseReleaseFeed({ schema: 1, versions }).feed;

describe("parseReleaseFeed", () => {
  it("refuses a file of another shape number, so an old app never misreads a new layout", () => {
    expect(() => parseReleaseFeed({ schema: 2, versions: [] })).toThrow("schema 2");
    expect(() => parseReleaseFeed([])).toThrow("not an object");
  });

  it("drops one wrong version and keeps the rest, newest first (B22)", () => {
    const { feed, dropped } = parseReleaseFeed({
      schema: 1,
      versions: [version("1.2.9", ["a"]), { ...version("1.2.10", ["b"]), date: "yesterday" }, version("1.2.11", ["c"]), version("1.2.11-rc.1", [])],
    });
    expect(feed.versions.map((row) => row.version)).toEqual(["1.2.11", "1.2.9"]);
    expect(dropped).toEqual([{ version: "1.2.10", reason: "date" }, { version: "1.2.11-rc.1", reason: "version" }]);
  });

  it("refuses a version naming more than three main features", () => {
    const { dropped } = parseReleaseFeed({ schema: 1, versions: [version("1.2.11", ["a", "b", "c", "d"])] });
    expect(dropped).toEqual([{ version: "1.2.11", reason: "highlights" }]);
  });

  it("keeps a language's words only when they are well formed, English being required", () => {
    const row = { ...version("1.2.11", []), changes: [{ kind: "fix", text: { en: "Fixed", ko: "고침", ja: "" } }] };
    const { feed } = parseReleaseFeed({ schema: 1, versions: [row] });
    expect(feed.versions[0]!.changes[0]!.text).toEqual({ en: "Fixed", ko: "고침" });
    expect(parseReleaseFeed({ schema: 1, versions: [{ ...row, changes: [{ kind: "fix", text: { ko: "고침" } }] }] }).dropped[0]!.reason).toBe("changes");
  });

  it("keeps the newest hundred versions", () => {
    const many = Array.from({ length: 105 }, (_, at) => version(`1.0.${at}`, []));
    const { feed } = parseReleaseFeed({ schema: 1, versions: many });
    expect(feed.versions).toHaveLength(100);
    expect(feed.versions.at(-1)!.version).toBe("1.0.5");
  });
});

describe("whatsNewOffer", () => {
  const feed = feedOf(version("1.2.8", ["eight"]), version("1.2.9", ["nine"]), version("1.2.10", ["ten-a", "ten-b"]), version("1.2.11", ["eleven"]), version("1.2.12", [], 2));

  it("offers the new version's main features once, after a one-step update (B1)", () => {
    const result = whatsNewOffer(feed, "1.2.11", { shown: "1.2.10", seen: "1.2.10" }, true);
    expect(result).toMatchObject({ kind: "show", offer: { from: null, to: "1.2.11", highlights: [{ version: "1.2.11", text: { en: { title: "eleven" } } }] }, save: { shown: "1.2.11", seen: "1.2.10" } });
    expect(whatsNewOffer(feed, "1.2.11", { shown: "1.2.11", seen: "1.2.10" }, true)).toEqual({ kind: "none" });
  });

  it("opens nothing for a version with no main features and records it, so a later version does not reach back (B6)", () => {
    expect(whatsNewOffer(feed, "1.2.12", { shown: "1.2.11", seen: "1.2.11" }, true)).toEqual({ kind: "quiet", save: { shown: "1.2.12", seen: "1.2.11" } });
  });

  it("merges skipped versions newest first, at most three, with the version it came from (B9)", () => {
    const result = whatsNewOffer(feed, "1.2.11", { shown: "1.2.8", seen: "1.2.8" }, true);
    expect(result.kind).toBe("show");
    if (result.kind !== "show") return;
    expect(result.offer.from).toBe("1.2.8");
    expect(result.offer.highlights.map((row) => [row.version, row.text.en.title])).toEqual([["1.2.11", "eleven"], ["1.2.10", "ten-a"], ["1.2.10", "ten-b"]]);
    expect(result.offer.changes).toHaveLength(3);
  });

  it("opens nothing on a preview or development build (B7)", () => {
    expect(whatsNewOffer(feed, "1.2.11-rc.1", null, true)).toEqual({ kind: "none" });
    expect(whatsNewOffer(feed, "0.4.0-main.f77c2a43", null, true)).toEqual({ kind: "none" });
  });

  it("leaves a new install to the first-run wizard and never offers that version later (B8)", () => {
    expect(whatsNewOffer(feed, "1.2.11", null, false)).toEqual({ kind: "quiet", save: { shown: "1.2.11", seen: "1.2.11" } });
  });

  it("offers an app from before this window only its own version, marking only it new (B8)", () => {
    const result = whatsNewOffer(feed, "1.2.11", null, true);
    expect(result).toMatchObject({ kind: "show", offer: { from: null, highlights: [{ version: "1.2.11" }] }, save: { shown: "1.2.11", seen: "1.2.10" } });
  });

  it("waits while the file does not have the running version yet (B11)", () => {
    expect(whatsNewOffer(feed, "1.2.13", { shown: "1.2.12", seen: "1.2.12" }, true)).toEqual({ kind: "none" });
  });
});

describe("historyVersions", () => {
  const feed = feedOf(version("1.2.10", ["a"]), version("1.2.11", ["b"]), version("1.2.12", []));

  it("lists versions up to the one in use, below it for a preview build (B7, B18)", () => {
    expect(historyVersions(feed, "1.2.11", null).map((row) => row.version)).toEqual(["1.2.11", "1.2.10"]);
    expect(historyVersions(feed, "1.2.11-rc.2", null).map((row) => row.version)).toEqual(["1.2.10"]);
  });

  it("adds a downloaded version waiting for the restart (B18)", () => {
    expect(historyVersions(feed, "1.2.11", "1.2.12").map((row) => row.version)).toEqual(["1.2.12", "1.2.11", "1.2.10"]);
  });

  it("lists every version on a development build", () => {
    expect(historyVersions(feed, "0.4.0-main.f77c2a43", null)).toHaveLength(3);
  });
});

describe("unseen", () => {
  it("marks versions newer than the newest opened in the history, and none before any record (B17)", () => {
    expect(unseen("1.2.11", { shown: "1.2.11", seen: "1.2.10" })).toBe(true);
    expect(unseen("1.2.10", { shown: "1.2.11", seen: "1.2.10" })).toBe(false);
    expect(unseen("1.2.11", null)).toBe(false);
  });
});
