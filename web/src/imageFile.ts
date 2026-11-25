import { useEffect, useState } from "react";
import { blobUrl, requestFileBytesWithTotal, type FileSource } from "./fileBytes";

/** Bytes past this draw the file's mark instead: a tile is no place to read a large file. */
const IMAGE_BYTES = 8 * 1024 * 1024;

/**
 * An image file's picture while the part that draws it is shown (the
 * session record's tile, the session panel's shelf): read once over the
 * file channel, of the machine `source` names, and let go when the part
 * goes. `"failed"` when it could not be read or is too large to draw.
 */
export function useImageFile(path: string, source: FileSource): string | "failed" | null {
  const [url, setUrl] = useState<string | "failed" | null>(null);
  const device = source?.device ?? null;
  const root = source?.root ?? null;
  useEffect(() => {
    let live = true;
    let made: string | null = null;
    setUrl(null);
    requestFileBytesWithTotal(path, { length: IMAGE_BYTES }, device && root ? { device, root } : null)
      .then(({ bytes, total }) => {
        if (!live) return;
        if (total > IMAGE_BYTES) return setUrl("failed");
        made = blobUrl(bytes, path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "image/*");
        setUrl(made);
      })
      .catch(() => { if (live) setUrl("failed"); });
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [path, device, root]);
  return url;
}
