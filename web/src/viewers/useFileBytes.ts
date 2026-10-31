// One file's bytes, read through hided and kept until the path changes. The
// viewers share this: an image, a PDF and a video all ask the same question,
// and a read that fails leaves the reason for the caller to draw in one line.

import { useEffect, useState } from "react";
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

/** A file's bytes from a named source, for a viewer showing a file outside the checkout in front (Desk's 결과물). */
export function useFileBytesFrom(path: string, source: FileSource): FileBytesState {
  const [state, setState] = useState<FileBytesState>({ status: "loading" });
  const device = source?.device ?? null;
  const root = source?.root ?? null;
  useEffect(() => {
    let live = true;
    setState({ status: "loading" });
    requestFileBytes(path, undefined, device && root ? { device, root } : null)
      .then((bytes) => {
        if (live) setState({ status: "ready", bytes });
      })
      .catch((error: unknown) => {
        if (live) setState({ status: "failed", reason: error instanceof Error ? error.message : "read_failed" });
      });
    return () => {
      live = false;
    };
  }, [path, device, root]);
  return state;
}
