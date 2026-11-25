import type { ReactNode } from "react";
import type { DeskOutput } from "../../desk";
import { useFileSource, useImageFile } from "../../imageFile";

/**
 * Whether an output is drawn as a picture: one the core sent a thumbnail
 * for (an image or a page), or an image file a part can read.
 */
export function outputPictured(output: DeskOutput): boolean {
  return output.kind === "image" ? Boolean(output.thumb || output.path) : output.kind === "page" && Boolean(output.thumb);
}

/**
 * An output's picture, the one part every place that draws one uses (the
 * session panel's shelf, its conversation, its folded turns): the core's
 * thumbnail when it sent one, else an image file read from the machine the
 * session runs on, shared by every part that draws it at once
 * (`useImageFile`); `fallback` while there is none to draw.
 */
export function OutputPicture({ output, className, fallback }: { output: DeskOutput; className: string; fallback: ReactNode }) {
  if (output.thumb) return <img src={output.thumb} alt="" aria-hidden="true" draggable={false} className={className} />;
  if (output.kind !== "image" || !output.path) return fallback;
  return <FilePicture path={output.path} device={output.device_id} root={output.checkout} className={className} fallback={fallback} />;
}

function FilePicture({ path, device, root, className, fallback }: { path: string; device: string | null; root: string | null; className: string; fallback: ReactNode }) {
  const url = useImageFile(path, useFileSource(device, root));
  if (url === null || url === "failed") return fallback;
  return <img src={url} alt="" aria-hidden="true" draggable={false} className={className} data-desk-output-picture="file" />;
}
