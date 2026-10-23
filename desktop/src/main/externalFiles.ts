// Files outside every checkout that the operator clicked in this window's
// terminal, shown in Hide read-only (PRD outside-path-in-hide). The core never
// holds such a path: this process grants the clicked file, and its folder for
// an HTML page's own files, to this window alone, and the core holds only the
// grant's id, the file's name and which viewer it opens in (D-05).
//
// A grant lives in memory, as long as some tab stands on it: the shell names
// the grants its tabs use on every sync, and a grant no tab names is let go,
// so closing a tab ends what it may read (B14). It survives a reload of the
// window, whose first sync names nothing until it has the core's tabs again
// (B15), and ends with the app. At most `MAX_EXTERNAL_GRANTS` are held; past
// that a click goes to the operating system as it did before (B17).
//
// What opens here is judged on the file itself, the way `localPath.ts` judges
// what macOS is handed: a program, an installer or an app bundle is only
// revealed, audio, a folder and a file past the viewers' size go to the
// system, and every other file opens in its viewer, a script as read-only
// text, since nothing shown here runs (D-08, D-10). The log names a grant, a
// kind and an outcome, never a path or a file name (B22).

import { randomBytes } from "node:crypto";
import fs from "node:fs/promises";
import { constants, realpathSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { executableHeader } from "./localPath";
import type { ExternalGrant, ExternalKind, ExternalRead } from "../../../web/src/host";
import type { LogFields } from "./log";

export type { ExternalKind };

/** Grants held at once; a click past it goes to the operating system. */
export const MAX_EXTERNAL_GRANTS = 32;
/** The largest file a viewer is handed, as `MAX_FILE_BYTES` caps a checkout file's bytes. */
export const MAX_EXTERNAL_BYTES = 256 * 1024 * 1024;
/** The largest text a tab holds, as `MAX_EDITABLE_BYTES` caps a checkout document; past it the tab is preview-only. */
export const MAX_EXTERNAL_TEXT_BYTES = 16 * 1024 * 1024;
/** How much of a file is read to tell a program, a PDF and text apart. */
const HEAD_BYTES = 8192;


/** The image kinds a checkout file opens as (`hide-host/src/document.rs`); the extension decides. */
const IMAGES = new Set(["png", "jpg", "jpeg", "gif", "webp", "tiff", "heic", "avif"]);
/** The videos the View plays (`web/src/viewers/video.ts`). */
const VIDEOS = new Set(["mp4", "m4v", "webm", "mov"]);
const HTML = new Set(["html", "htm", "xhtml"]);
/** Audio has no player in the View (D-10). */
const AUDIO = new Set(["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "caf", "ogg", "oga", "opus", "wma"]);
/** Installers and packages a handler would install or run; revealed on every system. */
const INSTALLERS = new Set(["pkg", "mpkg", "dmg", "app", "msi", "msix", "appx", "exe", "bat", "cmd", "com", "scr", "deb", "rpm", "appimage", "snap", "flatpak", "apk", "ipa", "jar"]);

const PDF_SIGNATURE = "%PDF-";

/** Where a clicked outside file goes: a viewer in Hide, the operating system's default route, or the file manager. */
export type ExternalRoute =
  | { action: "grant"; kind: ExternalKind }
  | { action: "system"; reason: "folder" | "audio" | "size" | "binary" | "special" }
  | { action: "reveal"; reason: "installer" | "program" };

function extension(file: string): string {
  // A leading dot is a hidden name, not an extension.
  const base = path.basename(file);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}

/** Whether `head` reads as text: no NUL, and UTF-8 but for a sequence the cut may have split at its end. */
export function looksLikeText(head: Uint8Array, whole: boolean): boolean {
  if (head.includes(0)) return false;
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(head, { stream: !whole });
    return true;
  } catch {
    return false;
  }
}

/**
 * The route for an outside file the operator clicked: `real` is its physical
 * path on this computer. `source` asks for an HTML file's text. A file that
 * cannot be read is still granted by its extension, so its tab says it cannot
 * be opened (B16) rather than the click doing nothing.
 */
export async function externalRoute(real: string, source: boolean): Promise<ExternalRoute> {
  const ext = extension(real);
  if (INSTALLERS.has(ext)) return { action: "reveal", reason: "installer" };
  let stat;
  try {
    stat = await fs.lstat(real);
  } catch {
    return { action: "grant", kind: byExtension(ext, source) };
  }
  if (stat.isDirectory()) return { action: "system", reason: "folder" };
  if (!stat.isFile()) return { action: "system", reason: "special" };
  if (AUDIO.has(ext)) return { action: "system", reason: "audio" };
  if (stat.size > MAX_EXTERNAL_BYTES) return { action: "system", reason: "size" };
  let head: Uint8Array;
  try {
    head = await readHead(real);
  } catch {
    return { action: "grant", kind: byExtension(ext, source) };
  }
  // A script's `#!` is text to read; a compiled program is not.
  if (executableHeader(head) && !(head[0] === 0x23 && head[1] === 0x21)) return { action: "reveal", reason: "program" };
  if (HTML.has(ext)) return { action: "grant", kind: source ? "text" : "html" };
  if (IMAGES.has(ext)) return { action: "grant", kind: "image" };
  if (new TextDecoder().decode(head.subarray(0, PDF_SIGNATURE.length)) === PDF_SIGNATURE) return { action: "grant", kind: "pdf" };
  if (VIDEOS.has(ext)) return { action: "grant", kind: "video" };
  if (looksLikeText(head, stat.size <= head.length)) return { action: "grant", kind: "text" };
  return { action: "system", reason: "binary" };
}

function byExtension(ext: string, source: boolean): ExternalKind {
  if (HTML.has(ext)) return source ? "text" : "html";
  if (IMAGES.has(ext)) return "image";
  if (ext === "pdf") return "pdf";
  if (VIDEOS.has(ext)) return "video";
  return "text";
}

async function readHead(file: string): Promise<Uint8Array> {
  const handle = await fs.open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const buffer = new Uint8Array(HEAD_BYTES);
    const { bytesRead } = await handle.read(buffer, 0, HEAD_BYTES, 0);
    return buffer.subarray(0, bytesRead);
  } finally {
    await handle.close();
  }
}

/** What the core and the shell are told of a grant: never its path. */
export type GrantedFile = ExternalGrant;

type Grant = GrantedFile & {
  /** The file's physical path on this computer. */
  real: string;
  /** Its folder, the one an HTML page loads its own files from (D-03). */
  folder: string;
};


export type GrantLog = (event: string, fields: LogFields) => void;

export class ExternalFiles {
  private readonly grants = new Map<string, Grant>();

  constructor(private readonly log: GrantLog) {}

  /**
   * A grant for the file at `real` (physical) opened in `kind`'s viewer, or
   * null at the cap. The same file clicked again keeps its grant, so its one
   * tab comes forward (B10).
   */
  grant(real: string, kind: ExternalKind): GrantedFile | null {
    for (const held of this.grants.values()) {
      if (held.real !== real) continue;
      held.kind = kind;
      this.log("external.granted", { grant: held.id, kind, again: true });
      return { id: held.id, name: held.name, kind };
    }
    if (this.grants.size >= MAX_EXTERNAL_GRANTS) {
      this.log("external.cap_reached", { kind, cap: MAX_EXTERNAL_GRANTS });
      return null;
    }
    const id = randomBytes(16).toString("hex");
    const name = path.basename(real);
    this.grants.set(id, { id, name, kind, real, folder: path.dirname(real) });
    this.log("external.granted", { grant: id, kind, again: false });
    return { id, name, kind };
  }

  /** The grant `id` names, or undefined once it is gone. */
  get(id: string): Readonly<Grant> | undefined {
    return this.grants.get(id);
  }

  /** Every grant held now, as the core is told when the window attaches. */
  live(): string[] {
    return [...this.grants.keys()];
  }

  /** Lets go of every grant `named` does not hold; returns the ids let go. */
  keep(named: ReadonlySet<string>): string[] {
    const gone = [...this.grants.keys()].filter((id) => !named.has(id));
    for (const id of gone) {
      const held = this.grants.get(id)!;
      this.grants.delete(id);
      this.log("external.released", { grant: id, kind: held.kind });
    }
    return gone;
  }

  /** The page address an HTML grant loads. */
  pageUrl(id: string): string | null {
    const held = this.grants.get(id);
    return held ? pathToFileURL(held.real).href : null;
  }

  /**
   * Whether `file` is inside the folder of grant `id`, links resolved, so a
   * link out of the folder carries nothing out; false once the grant is gone.
   */
  inside(id: string, file: string): boolean {
    const held = this.grants.get(id);
    if (!held) return false;
    let real = file;
    try {
      real = realpathSync.native(file);
    } catch {
      // A file that is not there answers 404 inside the folder it names.
    }
    const relative = path.relative(held.folder, real);
    return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
  }

  /**
   * The granted file's bytes, read now so a second click shows what is on the
   * disk (B10). The path must still be its own physical spelling, so a link
   * swapped in since the click is not followed elsewhere.
   */
  async read(id: string): Promise<ExternalRead> {
    const held = this.grants.get(id);
    if (!held) return { failure: "grant_gone" };
    const fail = (reason: string): ExternalRead => {
      this.log("external.read_failed", { grant: id, kind: held.kind, reason });
      return { failure: reason };
    };
    try {
      if ((await fs.realpath(held.real)) !== held.real) return fail("not_physical");
    } catch (error) {
      return fail(errorCode(error));
    }
    let handle;
    try {
      handle = await fs.open(held.real, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    } catch (error) {
      return fail(errorCode(error));
    }
    try {
      const stat = await handle.stat();
      if (!stat.isFile()) return fail("not_a_file");
      if (held.kind === "text" && stat.size > MAX_EXTERNAL_TEXT_BYTES) return { preview_only: true };
      if (stat.size > MAX_EXTERNAL_BYTES) return fail("too_large");
      const bytes = new Uint8Array(stat.size);
      let filled = 0;
      while (filled < bytes.length) {
        const { bytesRead } = await handle.read(bytes, filled, bytes.length - filled, filled);
        if (bytesRead === 0) break;
        filled += bytesRead;
      }
      return { bytes: bytes.subarray(0, filled) };
    } catch (error) {
      return fail(errorCode(error));
    } finally {
      await handle.close();
    }
  }
}

/** The platform's reason code alone: a message can carry the path. */
function errorCode(error: unknown): string {
  const code = (error as NodeJS.ErrnoException | null)?.code;
  return typeof code === "string" && /^[A-Z][A-Z0-9_]{0,31}$/u.test(code) ? code : "read_failed";
}
