import { useEffect, useState } from "react";
import { blobUrl, requestFileBytesWithTotal, type FileSource } from "./fileBytes";
import { localDeviceId } from "./snapshot";
import { useShellStore } from "./store";

/** Bytes past this draw the file's mark instead: a tile is no place to read a large file. */
const IMAGE_BYTES = 8 * 1024 * 1024;

/**
 * Where a file a session showed is read from: this machine's own channel
 * when the file is on it (or the record names no machine), else the device
 * it is on, under the checkout it belongs to. The tab panel's output, the
 * session record's tiles and the shelf's pictures all ask this.
 */
export function useFileSource(device: string | null, root: string | null): FileSource {
  const here = useShellStore((state) => localDeviceId(state.rest));
  return device && root && device !== here ? { device, root } : null;
}

type Picture = string | "failed" | null;

/** One image file's picture while any part draws it: read once, handed to each, let go when the last one goes. */
type Held = { users: number; picture: Picture; listeners: Set<(picture: Picture) => void>; gone: boolean };
const held = new Map<string, Held>();

function hold(key: string, path: string, source: FileSource): Held {
  const known = held.get(key);
  if (known) return known;
  const entry: Held = { users: 0, picture: null, listeners: new Set(), gone: false };
  held.set(key, entry);
  const settle = (picture: Picture) => {
    if (entry.gone) {
      if (picture && picture !== "failed") URL.revokeObjectURL(picture);
      return;
    }
    entry.picture = picture;
    for (const listener of entry.listeners) listener(picture);
  };
  requestFileBytesWithTotal(path, { length: IMAGE_BYTES }, source)
    .then(({ bytes, total }) => settle(total > IMAGE_BYTES ? "failed" : blobUrl(bytes, path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "image/*")))
    .catch(() => settle("failed"));
  return entry;
}

/**
 * An image file's picture while the part that draws it is shown (the
 * session record's tile, the session panel's shelf): read once over the
 * file channel, of the machine `source` names, shared by every part that
 * draws the same file at the same time, and let go when the last of them
 * goes. `"failed"` when it could not be read or is too large to draw.
 */
export function useImageFile(path: string, source: FileSource): Picture {
  const device = source?.device ?? null;
  const root = source?.root ?? null;
  const key = JSON.stringify([device, root, path]);
  const [picture, setPicture] = useState<Picture>(() => held.get(key)?.picture ?? null);
  useEffect(() => {
    const entry = hold(key, path, device && root ? { device, root } : null);
    entry.users += 1;
    entry.listeners.add(setPicture);
    setPicture(entry.picture);
    return () => {
      entry.listeners.delete(setPicture);
      entry.users -= 1;
      if (entry.users > 0) return;
      entry.gone = true;
      held.delete(key);
      if (entry.picture && entry.picture !== "failed") URL.revokeObjectURL(entry.picture);
    };
  }, [key, path, device, root]);
  return picture;
}
