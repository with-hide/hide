// The file-bytes reader (PRD B6, B7, D-07): one request per path, answered by
// one or more binary frames on the token-gated socket. A viewer asks for the
// bytes of an image, a PDF or a video and gets a promise; the frames arrive
// through `ws.ts`, which knows nothing about viewers. A request that is
// abandoned (the tab closed) resolves to nothing.

import type { DispatchFn } from "./ws";

export type BytesHeader = {
  type: "file_bytes";
  request_id: string;
  path: string;
  offset: number;
  total: number;
  eof: boolean;
};

type Pending = {
  path: string;
  total: number;
  chunks: Uint8Array[];
  received: number;
  resolve: (answer: { bytes: Uint8Array; total: number }) => void;
  reject: (error: Error) => void;
};

let dispatchFn: DispatchFn | null = null;
const pending = new Map<string, Pending>();
let nextRequest = 1;

export function configureFileBytes(dispatch: DispatchFn): void {
  dispatchFn = dispatch;
}

/** A header for a binary frame: 4-byte big-endian length, JSON, then bytes. */
function parseFrame(data: ArrayBuffer): { header: BytesHeader; bytes: Uint8Array } | null {
  if (data.byteLength < 4) return null;
  const view = new DataView(data);
  const headerLength = view.getUint32(0, false);
  if (4 + headerLength > data.byteLength) return null;
  try {
    const header = JSON.parse(new TextDecoder().decode(new Uint8Array(data, 4, headerLength))) as BytesHeader;
    return { header, bytes: new Uint8Array(data, 4 + headerLength) };
  } catch {
    return null;
  }
}

export function receiveBytes(data: ArrayBuffer): void {
  const parsed = parseFrame(data);
  if (!parsed) return;
  const { header, bytes } = parsed;
  const entry = pending.get(header.request_id);
  if (!entry) return;
  entry.chunks.push(bytes);
  entry.received += bytes.byteLength;
  entry.total = header.total;
  if (header.eof) {
    pending.delete(header.request_id);
    const joined = new Uint8Array(entry.received);
    let at = 0;
    for (const chunk of entry.chunks) {
      joined.set(chunk, at);
      at += chunk.byteLength;
    }
    entry.resolve({ bytes: joined, total: entry.total });
  }
}

export function receiveBytesError(payload: { request_id: string; reason: string }): void {
  const entry = pending.get(payload.request_id);
  if (!entry) return;
  pending.delete(payload.request_id);
  entry.reject(new Error(payload.reason));
}

/**
 * A boundary refusal names the path, not a request id, so every read of that
 * path is settled with the daemon's reason: the viewer shows one line instead
 * of waiting forever.
 */
export function receiveBytesRefusal(path: string, reason: string): void {
  for (const [requestId, entry] of pending) {
    if (entry.path !== path) continue;
    pending.delete(requestId);
    entry.reject(new Error(reason));
  }
}

/** A socket that is gone cannot answer; every in-flight read fails at once. */
export function clearPending(reason: string): void {
  for (const [requestId, entry] of pending) {
    pending.delete(requestId);
    entry.reject(new Error(reason));
  }
}

/**
 * Where a file lives when it is not on this Hide host: the SSH device and the
 * checkout root there. hided reads it through that device's helper.
 */
export type FileSource = { device: string; root: string } | null;

/**
 * Reads a file under a registered checkout root. `length` absent reads to the
 * end; a range read names an offset and a length. The promise rejects with the
 * daemon's reason code (`outside_checkout`, `too_large`, `read_failed`, ...).
 */
export function requestFileBytesWithTotal(path: string, range?: { offset?: number; length?: number }, source: FileSource = null): Promise<{ bytes: Uint8Array; total: number }> {
  return new Promise((resolve, reject) => {
    if (!dispatchFn) {
      reject(new Error("not_connected"));
      return;
    }
    const request_id = `bytes-${nextRequest}`;
    nextRequest += 1;
    pending.set(request_id, { path, total: 0, chunks: [], received: 0, resolve, reject });
    const sent = dispatchFn({
      schema_version: 2,
      kind: "file_bytes",
      payload: {
        request_id,
        path,
        offset: range?.offset ?? 0,
        length: range?.length ?? null,
        ...(source ? { device_id: source.device, root: source.root } : {}),
      },
    });
    if (sent === false) {
      pending.delete(request_id);
      reject(new Error("not_connected"));
    }
  });
}

export async function requestFileBytes(path: string, range?: { offset?: number; length?: number }, source: FileSource = null): Promise<Uint8Array> {
  return (await requestFileBytesWithTotal(path, range, source)).bytes;
}

/**
 * Saves a file's bytes through the browser's download path (PRD S3 D-12).
 * A bare browser cannot prove that the daemon is on the viewer's machine,
 * so the bytes come down the same `file_bytes` stream the viewers use.
 */
export async function downloadFile(path: string, source: FileSource = null): Promise<void> {
  const name = path.split("/").pop() || "download";
  // Chromium and the eventual desktop shell can write one range at a time to
  // the chosen file. Invoke the picker inside the click's user activation.
  const picker = (window as Window & { showSaveFilePicker?: (options: { suggestedName: string }) => Promise<FileSystemFileHandle> }).showSaveFilePicker;
  if (picker) {
    const handle = await picker.call(window, { suggestedName: name });
    const writer = await handle.createWritable();
    try {
      const rangeSize = 4 * 1024 * 1024;
      let offset = 0;
      let total: number | null = null;
      do {
        const answer = await requestFileBytesWithTotal(path, { offset, length: rangeSize }, source);
        if (total !== null && total !== answer.total) throw new Error("file_changed_during_download");
        total = answer.total;
        if (answer.bytes.length === 0 && offset < total) throw new Error("incomplete_download");
        await writer.write(answer.bytes as BufferSource);
        offset += answer.bytes.length;
      } while (total === null || offset < total);
      await writer.close();
    } catch (error) {
      await writer.abort().catch(() => {});
      throw error;
    }
    return;
  }
  // Browsers without a streaming file writer retain the existing Blob path.
  // Its protocol read cap is explicit; a failure is shown beside the button.
  const bytes = await requestFileBytes(path, undefined, source).catch((error: unknown) => {
    if (error instanceof Error && error.message === "too_large") {
      throw new Error("needs_save_picker");
    }
    throw error;
  });
  const url = blobUrl(bytes, "application/octet-stream");
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // The download reads the blob asynchronously; revoking at once cancels it.
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

/** A blob URL for bytes, and the one place a viewer revokes it. */
export function blobUrl(bytes: Uint8Array, type: string): string {
  return URL.createObjectURL(new Blob([bytes as BlobPart], { type }));
}
