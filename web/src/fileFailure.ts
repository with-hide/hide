import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { fileReasonsEnglish } from "./i18n/resources/fileReasons";
import type { CodedFailure } from "./snapshot";
import type { DirectoryUnavailable } from "./store";

/**
 * Why a change to a checkout's files was not made, or a folder not listed, in the operator's language: an
 * `explorer.*`, `host.*` or `link.*` code with the limit its sentence names. A code this build does not word reads as
 * the general line, and the code stays in `data-` and the log.
 */
export function fileFailureText(failure: CodedFailure, t: TFunction<"translation">, fallback: "fileReason.other" | "fileReason.unreadable" = "fileReason.other"): string {
  const key = `fileReason.${failure.code}`;
  return key in fileReasonsEnglish ? t(key as MessageKey, { limit: failure.limit }) : t(fallback);
}

/** Why a read (History, the file index) failed, from its code; one this build does not word says only that it was not read. */
export function readFailureText(code: string, t: TFunction<"translation">): string {
  return fileFailureText({ code }, t, "fileReason.unreadable");
}

/** Why a download failed: hided's `file_bytes_error` reason, a `path_refused` reason, or the page's own (`not_connected`, `socket_closed`, ...). */
export function downloadFailureText(reason: string, t: TFunction<"translation">): string {
  return readFailureText(`fileReason.download.${reason}` in fileReasonsEnglish ? `download.${reason}` : `path.${reason}`, t);
}

/** A device folder its helper did not list, as the code `fileFailureText` words: the helper's refusal, or the link's state. */
export function listingFailure(unavailable: DirectoryUnavailable): CodedFailure {
  if (unavailable.code === "refused" && unavailable.reason) return { code: `host.${unavailable.reason}` };
  return { code: unavailable.code === "not_ready" ? "link.not_connected" : `link.${unavailable.code}` };
}
