// What each released version of the desktop app brought, as withhide.dev
// publishes it (`site/public/app/releases.json`, docs/UI_BEHAVIOR.md, What's
// new and Update history): the shape, its parse, the version order, and which
// versions the window after an update offers. The desktop host parses the
// file before it keeps a copy (`desktop/src/main/whatsNew.ts`); the shell
// draws what the host hands it. Pure: no DOM, no Node, no network.

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
 * The drawings a highlight can name: the practice cards' moves
 * (`PracticePreviewKind`, which `components/whats-new.tsx` holds equal to
 * this list at compile time; the desktop host reads this file without JSX).
 */
export const RELEASE_DRAWINGS = ["mission", "zones", "split", "switch", "rows", "children", "search", "lane"] as const;
export type ReleaseDrawing = (typeof RELEASE_DRAWINGS)[number];

export type Words<T> = { en: T } & Partial<Record<Exclude<InterfaceLanguage, "en">, T>>;
export type Highlight = { drawing: string; chapter: string | null; text: Words<{ title: string; line: string }> };
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
  const text = words(raw.text, (value) => (isObject(value) && isText(value.title, 80) && isText(value.line, 200) ? { title: value.title, line: value.line } : null));
  return text ? { drawing: raw.drawing, chapter, text } : null;
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
 * The file as withhide.dev serves it. A file that is not this shape at all
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
 * (below it for an rc), plus `ready`, a downloaded version waiting for the
 * restart, when the file has it. A development build, whose version is no
 * release, lists them all.
 */
export function historyVersions(feed: ReleaseFeed, appVersion: string, ready: string | null): ReleaseEntry[] {
  const app = parseVersion(appVersion);
  return feed.versions.filter((row) => !app || compareVersions(parseVersion(row.version)!, app) <= 0 || row.version === ready);
}

/** What the host remembers: the last version whose window closed, and the newest version opened in the history. */
export type WhatsNewRecord = { shown: string | null; seen: string | null };

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
 * `record` null means no record was ever written: a new install when the
 * first run is not done (the first-run wizard introduces Hide), an app from
 * before this window when it is (only this version is offered).
 */
export function whatsNewOffer(
  feed: ReleaseFeed,
  appVersion: string,
  record: WhatsNewRecord | null,
  firstRunDone: boolean,
): { kind: "none" } | { kind: "quiet"; save: WhatsNewRecord } | { kind: "show"; offer: Offer; save: WhatsNewRecord } {
  if (!announces(appVersion)) return { kind: "none" };
  const current = feed.versions.find((row) => row.version === appVersion);
  if (!current) return { kind: "none" };
  const earlier = feed.versions.filter((row) => order(row.version, appVersion) < 0);
  if (record === null && !firstRunDone) return { kind: "quiet", save: { shown: appVersion, seen: appVersion } };
  if (record !== null && record.shown !== null && isStable(record.shown) && order(record.shown, appVersion) >= 0) return { kind: "none" };
  const since = record?.shown && isStable(record.shown) ? record.shown : null;
  const rows = record === null ? [current] : feed.versions.filter((row) => order(row.version, appVersion) <= 0 && (since === null ? row === current : order(row.version, since) > 0));
  // Seen stays where it was, except on the first record, where only this version is new.
  const save: WhatsNewRecord = { shown: appVersion, seen: record === null ? (earlier[0]?.version ?? null) : record.seen };
  const highlights = rows.flatMap((row) => row.highlights.map((item) => ({ ...item, version: row.version }))).slice(0, HIGHLIGHT_LIMIT);
  if (highlights.length === 0) return { kind: "quiet", save };
  return { kind: "show", offer: { from: rows.length > 1 ? since : null, to: appVersion, highlights, changes: rows.flatMap((row) => row.changes) }, save };
}

/** Whether `version` carries the history's NEW mark: newer than the newest version opened there; no mark before any record exists. */
export function unseen(version: string, record: WhatsNewRecord | null): boolean {
  if (record === null) return false;
  return record.seen === null || !isStable(record.seen) || order(version, record.seen) > 0;
}
