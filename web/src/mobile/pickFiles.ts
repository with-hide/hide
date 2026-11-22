// The `+` of the reply bar (Doop 08): the system's own picker (the photo library, a new photo, a file), and each picked photo made
// smaller on the phone before it is uploaded. Text, code, logs and PDFs go as they are.

import { FILE_BYTES, addFile, fileName, leaveFile, reserveFile, type PreparedFile } from "./uploads";

/**
 * A photo's longer side once made smaller: the size an agent reads an image at (Claude scales anything longer down to about 1,568 px,
 * a GPT image is read at up to 2,048), so nothing an agent would see is lost and a phone photo of 12 MP leaves as about 0.5 MB.
 */
export const PHOTO_LONG_SIDE = 2048;
/** JPEG quality of a photo made smaller: small text in a screenshot stays sharp. */
export const PHOTO_QUALITY = 0.85;
/** The thumbnail kept on this phone: sharp at the bubble's width on a 3x screen. */
const THUMB_LONG_SIDE = 320;
const THUMB_QUALITY = 0.7;

const PHOTO_TYPES = new Set(["image/jpeg", "image/png", "image/webp", "image/heic", "image/heif"]);
const PHOTO_EXTENSIONS = /\.(jpe?g|png|webp|heic|heif)$/i;

/** Whether a picked file is a photo to make smaller: a still image the phone can draw, never a GIF (it moves) or an SVG (it is text). */
export function isPhoto(file: { name: string; type: string }): boolean {
  return PHOTO_TYPES.has(file.type.toLowerCase()) || (file.type === "" && PHOTO_EXTENSIONS.test(file.name));
}

/** The name a photo made smaller leaves with: its own, as a JPEG. */
export function photoName(name: string): string {
  const dot = name.lastIndexOf(".");
  return `${dot > 0 ? name.slice(0, dot) : name}.jpg`;
}

/** The size a picture of `width` by `height` is drawn at so that its longer side is at most `longSide`. */
export function scaledSize(width: number, height: number, longSide: number): { width: number; height: number } {
  const scale = Math.min(1, longSide / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** Opens the system's picker for any number of files; resolves with none when it was closed without a pick. */
export function pickFiles(): Promise<File[]> {
  return new Promise((resolve) => {
    // Made here rather than drawn: the picker is the system's, and the field only opens it. iOS shows the photo library, the camera
    // and Files for a field that names no type; Android shows its chooser.
    const field = document.createElement("input");
    field.type = "file";
    field.multiple = true;
    field.hidden = true;
    const done = (picked: File[]) => {
      field.remove();
      resolve(picked);
    };
    field.addEventListener("change", () => done(Array.from(field.files ?? [])), { once: true });
    field.addEventListener("cancel", () => done([]), { once: true });
    document.body.append(field);
    field.click();
  });
}

function encode(bitmap: ImageBitmap, longSide: number, quality: number): Promise<Blob> {
  const { width, height } = scaledSize(bitmap.width, bitmap.height, longSide);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return Promise.reject(new Error("no_canvas"));
  context.drawImage(bitmap, 0, 0, width, height);
  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("no_blob"))), "image/jpeg", quality));
}

function dataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/**
 * A photo made smaller, as a JPEG without the original's metadata (a photo's place goes with it), and its thumbnail; null when the
 * phone cannot draw it, and it goes as the file it is.
 */
async function smallerPhoto(file: File): Promise<PreparedFile | null> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return null;
  }
  try {
    const photo = await encode(bitmap, PHOTO_LONG_SIDE, PHOTO_QUALITY);
    const thumb = await dataUrl(await encode(bitmap, THUMB_LONG_SIDE, THUMB_QUALITY));
    return { name: photoName(file.name), bytes: new Uint8Array(await photo.arrayBuffer()), kind: "photo", thumb };
  } catch {
    return null;
  } finally {
    bitmap.close();
  }
}

/** Puts the picked files on the bar in the order picked, each one uploading once it is ready. */
export async function attachPicked(picked: readonly File[]): Promise<void> {
  const slots = picked.map((file) => ({ file, id: reserveFile(file.name, isPhoto(file) ? "photo" : "file") }));
  for (const { file, id } of slots) {
    if (!id) continue;
    const photo = isPhoto(file) ? await smallerPhoto(file) : null;
    if (photo) {
      addFile(id, photo);
      continue;
    }
    // A file past the cap is never read whole: its size says enough.
    if (file.size > FILE_BYTES) {
      leaveFile(id, { kind: "too_large", name: fileName(file.name), size: file.size });
      continue;
    }
    try {
      addFile(id, { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), kind: "file", thumb: null });
    } catch {
      leaveFile(id, { kind: "unreadable", name: fileName(file.name) });
    }
  }
}
