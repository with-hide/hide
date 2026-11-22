// What each released version of the desktop app brought, as the app carries
// it (`web/src/releases.json`, docs/UI_BEHAVIOR.md, What's new and Update
// history): the shape, its parse, the version order, and which versions the
// window after an update offers. The desktop host parses the file
// (`desktop/src/main/whatsNew.ts`); the shell draws what the host hands it. Pure: no DOM, no Node, no network.

import { INTERFACE_LANGUAGES, type InterfaceLanguage } from "./i18n/locale";

/** The file's own shape number; a file of another number is not read at all. */
export const RELEASES_SCHEMA = 1;
/** The most a version names as its main features (the window's list). */
export const HIGHLIGHT_LIMIT = 3;
/** The most changes and fixes one version lists. */
export const CHANGE_LIMIT = 80;
/** The most versions the history keeps, newest first; older ones are dropped. */
export const VERSION_LIMIT = 100;

/**
 * The drawings a highlight can name: a feature's own scene
 * (`components/whats-new-scenes/`), else one of the practice cards' moves
 * (`PracticePreviewKind`). `components/whats-new.tsx` holds this list equal to
 * the two at compile time; the desktop host reads this file without JSX.
 */
export const RELEASE_DRAWINGS = ["automation", "pair", "skills", "bots", "mission", "zones", "split", "switch", "rows", "children", "search", "lane", "focus"] as const;
export type ReleaseDrawing = (typeof RELEASE_DRAWINGS)[number];

export type Words<T> = { en: T } & Partial<Record<Exclude<InterfaceLanguage, "en">, T>>;
/** `settings`: the Settings page that holds a feature no chapter teaches, which its 설정에서 보기 opens (a tab id, `skills`). */
export type Highlight = { drawing: string; chapter: string | null; settings: string | null; text: Words<{ title: string; line: string }> };
export type Change = { kind: "change" | "fix"; text: Words<string> };
export type ReleaseEntry = { version: string; date: string; highlights: Highlight[]; changes: Change[] };
export type ReleaseFeed = { versions: ReleaseEntry[] };

/** The versions a release is published under (`releaseKind` in scripts/check-release-assets.mjs). */
const RELEASE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-rc\.([1-9]\d*))?$/;

export type Version = { major: number; minor: number; patch: number; rc: number | null };

export function parseVersion(text: string): Version | null {
  const match = RELEASE_VERSION.exec(text);
  if (!match) return null;
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]), rc: match[4] === undefined ? null : Number(match[4]) };
}

/** Semver order for the release shapes: a release is newer than every rc of the same version. */
export function compareVersions(a: Version, b: Version): number {
  const rank = (rc: number | null) => (rc === null ? Number.MAX_SAFE_INTEGER : rc);
  return a.major - b.major || a.minor - b.minor || a.patch - b.patch || rank(a.rc) - rank(b.rc);
}

/** `a` against `b` as version text; both must be release versions. */
function order(a: string, b: string): number {
  return compareVersions(parseVersion(a)!, parseVersion(b)!);
}

const isStable = (text: unknown): text is string => typeof text === "string" && parseVersion(text)?.rc === null;
const isText = (value: unknown, max: number): value is string => typeof value === "string" && value.trim().length > 0 && value.length <= max;
const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);

/** Words in every language the file gives, English required; a language with a bad value is left out. */
function words<T>(raw: unknown, read: (value: unknown) => T | null): Words<T> | null {
  if (!isObject(raw)) return null;
  const en = read(raw.en);
  if (en === null) return null;
  const out: Words<T> = { en };
  for (const language of INTERFACE_LANGUAGES) {
    if (language === "en" || raw[language] === undefined) continue;
    const value = read(raw[language]);
    if (value !== null) out[language] = value;
  }
  return out;
}

function highlight(raw: unknown): Highlight | null {
  if (!isObject(raw) || !isText(raw.drawing, 32)) return null;
  const chapter = raw.chapter === undefined || raw.chapter === null ? null : isText(raw.chapter, 32) ? raw.chapter : undefined;
  if (chapter === undefined) return null;
  const settings = raw.settings === undefined || raw.settings === null ? null : isText(raw.settings, 32) ? raw.settings : undefined;
  if (settings === undefined) return null;
  const text = words(raw.text, (value) => (isObject(value) && isText(value.title, 80) && isText(value.line, 200) ? { title: value.title, line: value.line } : null));
  return text ? { drawing: raw.drawing, chapter, settings, text } : null;
}

function change(raw: unknown): Change | null {
  if (!isObject(raw) || (raw.kind !== "change" && raw.kind !== "fix")) return null;
  const text = words(raw.text, (value) => (isText(value, 300) ? value : null));
  return text ? { kind: raw.kind, text } : null;
}

/** One version, or why it is not read. */
function entry(raw: unknown): ReleaseEntry | string {
  if (!isObject(raw)) return "not an object";
  if (!isStable(raw.version)) return "version";
  if (typeof raw.date !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(raw.date)) return "date";
  if (!Array.isArray(raw.highlights) || raw.highlights.length > HIGHLIGHT_LIMIT) return "highlights";
  if (!Array.isArray(raw.changes) || raw.changes.length > CHANGE_LIMIT) return "changes";
  const highlights = raw.highlights.map(highlight);
  if (highlights.some((row) => row === null)) return "highlights";
  const changes = raw.changes.map(change);
  if (changes.some((row) => row === null)) return "changes";
  return { version: raw.version, date: raw.date, highlights: highlights as Highlight[], changes: changes as Change[] };
}

export type ParsedFeed = { feed: ReleaseFeed; dropped: { version: string | null; reason: string }[] };

/**
 * The releases file as the app carries it. A file that is not this shape at all
 * throws; a version that is wrong is dropped alone and named in `dropped`, so
 * one bad entry does not cost the others. Newest first, a version once, at
 * most `VERSION_LIMIT`.
 */
export function parseReleaseFeed(raw: unknown): ParsedFeed {
  if (!isObject(raw)) throw new Error("releases file: not an object");
  if (raw.schema !== RELEASES_SCHEMA) throw new Error(`releases file: schema ${String(raw.schema)} is not ${RELEASES_SCHEMA}`);
  if (!Array.isArray(raw.versions)) throw new Error("releases file: no versions list");
  const dropped: ParsedFeed["dropped"] = [];
  const seen = new Set<string>();
  const versions: ReleaseEntry[] = [];
  for (const row of raw.versions) {
    const read = entry(row);
    const named = isObject(row) && typeof row.version === "string" ? row.version.slice(0, 32) : null;
    if (typeof read === "string") dropped.push({ version: named, reason: read });
    else if (seen.has(read.version)) dropped.push({ version: named, reason: "repeated" });
    else {
      seen.add(read.version);
      versions.push(read);
    }
  }
  versions.sort((a, b) => order(b.version, a.version));
  return { feed: { versions: versions.slice(0, VERSION_LIMIT) }, dropped };
}

/** `words` in `language`, English where the file has none in it. */
export function inLanguage<T>(value: Words<T>, language: InterfaceLanguage): T {
  return value[language] ?? value.en;
}

/** Whether this app's version is a release that announces itself: a stable release, not an rc or a development build. */
export function announces(appVersion: string): boolean {
  return isStable(appVersion);
}

/**
 * The versions the history lists for an app at `appVersion`: those up to it
 * (below it for an rc). A development build, whose version is no release,
 * lists them all.
 */
export function historyVersions(feed: ReleaseFeed, appVersion: string): ReleaseEntry[] {
  const app = parseVersion(appVersion);
  return feed.versions.filter((row) => !app || compareVersions(parseVersion(row.version)!, app) <= 0);
}

/**
 * What the host remembers: the last version whose window closed (`shown`),
 * and which versions the history has opened, as a version at or below which
 * every version counts as opened (`seen`) and the opened ones above it
 * (`opened`, newest first, at most `VERSION_LIMIT`).
 */
export type WhatsNewRecord = { shown: string | null; seen: string | null; opened: string[] };
/** What the shell asks the host to add to the record. */
export type RecordUpdate = { shown?: string; seen?: string; opened?: string };

export type OfferHighlight = Highlight & { version: string };
export type Offer = { from: string | null; to: string; highlights: OfferHighlight[]; changes: Change[] };

/**
 * What the window after an update shows, or why it shows nothing yet:
 *   `none`  nothing to do now: this version was already shown, it is no
 *           release that announces itself, or the file does not have it yet
 *           (a later launch asks again);
 *   `quiet` nothing to show for this version, and nothing ever will be:
 *           record it as shown (`save`) so a later version does not reach back;
 *   `show`  the offer, after which the record says this version was shown.
 * `record` null means no record was ever written though the app ran before:
 * an app from before this window, offered only this version. A new install
 * never reaches here without one: the host records its first version at
 * launch, so the first-run wizard alone introduces Hide (B8).
 */
export function whatsNewOffer(
  feed: ReleaseFeed,
  appVersion: string,
  record: WhatsNewRecord | null,
): { kind: "none" } | { kind: "quiet"; save: RecordUpdate } | { kind: "show"; offer: Offer; save: RecordUpdate } {
  if (!announces(appVersion)) return { kind: "none" };
  const current = feed.versions.find((row) => row.version === appVersion);
  if (!current) return { kind: "none" };
  if (record !== null && record.shown !== null && isStable(record.shown) && order(record.shown, appVersion) >= 0) return { kind: "none" };
  const since = record?.shown && isStable(record.shown) ? record.shown : null;
  const rows = record === null ? [current] : feed.versions.filter((row) => order(row.version, appVersion) <= 0 && (since === null ? row === current : order(row.version, since) > 0));
  // On the first record only this version is new in the history; the versions before it count as opened.
  const before = record === null ? feed.versions.find((row) => order(row.version, appVersion) < 0)?.version : undefined;
  const save: RecordUpdate = before === undefined ? { shown: appVersion } : { shown: appVersion, seen: before };
  const highlights = rows.flatMap((row) => row.highlights.map((item) => ({ ...item, version: row.version }))).slice(0, HIGHLIGHT_LIMIT);
  if (highlights.length === 0) return { kind: "quiet", save };
  return { kind: "show", offer: { from: rows.length > 1 ? since : null, to: appVersion, highlights, changes: rows.flatMap((row) => row.changes) }, save };
}

/**
 * The record after `update`, moved forward only, or why the update is
 * refused: a value that is no release version, or one past the version the
 * app runs (a buggy page could otherwise silence every later window). An
 * opened version joins `opened`; `seen` then climbs over every version of
 * `feed` that has been opened in a row, so `opened` keeps only the gaps.
 */
export function mergeRecord(
  before: WhatsNewRecord | null,
  update: { shown?: unknown; seen?: unknown; opened?: unknown },
  feed: ReleaseFeed | null,
  appVersion: string,
): { record: WhatsNewRecord } | { refused: "version" | "ahead" } {
  const values = [update.shown, update.seen, update.opened].filter((value) => value !== undefined);
  if (!values.every(isStable)) return { refused: "version" };
  if (parseVersion(appVersion) && values.some((value) => order(value as string, appVersion) > 0)) return { refused: "ahead" };
  const newer = (a: string | null, b: unknown): string | null => (typeof b !== "string" ? a : a === null || order(b, a) > 0 ? b : a);
  const record = before ?? { shown: null, seen: null, opened: [] };
  let seen = newer(record.seen, update.seen);
  const opened = new Set([...record.opened, ...(typeof update.opened === "string" ? [update.opened] : [])]);
  const above = (version: string) => seen === null || order(version, seen) > 0;
  for (const row of [...(feed?.versions ?? [])].reverse()) {
    if (!above(row.version)) continue;
    if (!opened.has(row.version)) break;
    seen = row.version;
  }
  const kept = [...opened].filter(above).sort((a, b) => order(b, a)).slice(0, VERSION_LIMIT);
  return { record: { shown: newer(record.shown, update.shown), seen, opened: kept } };
}

/** Whether `version` carries the history's NEW mark: not opened there yet; no mark before any record exists. */
export function unseen(version: string, record: WhatsNewRecord | null): boolean {
  if (record === null) return false;
  if (record.opened.includes(version)) return false;
  return record.seen === null || !isStable(record.seen) || order(version, record.seen) > 0;
}
