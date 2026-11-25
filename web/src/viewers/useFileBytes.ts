// One file's bytes, read through hided and kept until the path changes or the
// file changes on disk (`file_changed`). The viewers share this: an image, a
// PDF and a video all ask the same question, and a read that fails leaves the
// reason for the caller to draw in one line.

import { useEffect, useRef, useState } from "react";
import { requestFileBytes, type FileSource } from "../fileBytes";
import { explorerContext, localDeviceId } from "../snapshot";
import { useShellStore } from "../store";

export type FileBytesState =
  | { status: "loading" }
  | { status: "ready"; bytes: Uint8Array }
  | { status: "failed"; reason: string };

/**
 * The device and checkout root a shown file is read from: the checkout in
 * front on the device in front, or null for this Hide host's own files.
 */
export function useFileSource(): FileSource {
  const device = useShellStore((s) => explorerContext(s.rest).device);
  const root = useShellStore((s) => explorerContext(s.rest).checkout?.path ?? null);
  const node = useShellStore((s) => localDeviceId(s.rest));
  return device === node || !root ? null : { device, root };
}

export function useFileBytes(path: string): FileBytesState {
  return useFileBytesFrom(path, useFileSource());
}

/**
 * A file's bytes from a named source, for a viewer showing a file outside the checkout in front (Desk's 결과물).
 * `attempt` is the caller's retry count: a new one reads the file again.
 * A change on disk reads it again too, keeping the last bytes on screen until
 * the new ones arrive, so a file an agent rewrites does not blink, and for
 * good when that read fails, as for a file removed since.
 */
export function useFileBytesFrom(path: string, source: FileSource, attempt = 0): FileBytesState {
  const [state, setState] = useState<FileBytesState>({ status: "loading" });
  const device = source?.device ?? null;
  const root = source?.root ?? null;
  const changes = useShellStore((s) => s.fileChanges[path] ?? 0);
  const asked = useRef<string | null>(null);
  useEffect(() => {
    let live = true;
    const ask = `${device ?? ""}\n${root ?? ""}\n${path}\n${attempt}`;
    const again = asked.current === ask;
    if (!again) setState({ status: "loading" });
    asked.current = ask;
    requestFileBytes(path, undefined, device && root ? { device, root } : null)
      .then((bytes) => {
        if (live) setState({ status: "ready", bytes });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const failed: FileBytesState = { status: "failed", reason: error instanceof Error ? error.message : "read_failed" };
        setState((shown) => (again && shown.status === "ready" ? shown : failed));
      });
    return () => {
      live = false;
    };
  }, [path, device, root, attempt, changes]);
  return state;
}
