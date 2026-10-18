// A browser display's device size (PRD browser-width-zoom D-10..D-12): the
// page lays out at a phone's, a tablet's or a desktop's width instead of its
// area's, centered in the area when it is narrower and scaled down to fit
// when it is wider. The choice is the display's own and page-local
// (`ui.ts` `browserDevices`); the host emulates the viewport
// (`desktop/src/main/browser.ts`).

import type { BrowserDevicePlacement } from "./host";

export type BrowserDevice = "fit" | "mobile" | "tablet" | "desktop";

/** The menu's order: fit first, then the sizes from the narrowest. */
export const BROWSER_DEVICES: readonly BrowserDevice[] = ["fit", "mobile", "tablet", "desktop"];

/** Each size's viewport width in CSS pixels, and whether it lays out as a phone or tablet does. */
const SIZES: Record<Exclude<BrowserDevice, "fit">, BrowserDevicePlacement> = {
  mobile: { width: 390, mobile: true },
  tablet: { width: 768, mobile: true },
  desktop: { width: 1280, mobile: false },
};

/** What the host emulates for a device size; null fits the area. */
export function devicePlacement(device: BrowserDevice): BrowserDevicePlacement | null {
  return device === "fit" ? null : SIZES[device];
}

/**
 * What the device button says beside its icon: nothing while the page fits,
 * the width while the page shows at its own size, and the width with the
 * scale it is drawn at when the area is narrower than it (`1280 · 39%`).
 * `drawn` is the width the page's frame takes in the area, 0 before it is
 * measured.
 */
export function deviceLabel(device: BrowserDevice, drawn: number): string | null {
  const size = devicePlacement(device);
  if (!size) return null;
  const scale = drawn > 0 ? drawn / size.width : 1;
  return scale < 0.995 ? `${size.width} · ${Math.round(scale * 100)}%` : `${size.width}`;
}
